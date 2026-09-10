//! The authenticated HTTP client (spec §2.2, §7.1, §7.2, §7.4; clasp's
//! googleapis-common + google-auth-library behavior).
//!
//! Two independent layers drive every request:
//!
//! * **Transient retries** (gaxios defaults, spec §7.1): only idempotent
//!   methods (GET/HEAD/PUT/OPTIONS/DELETE) retry on statuses 100-199, 408,
//!   429, and 500-599 — at most 3 times. Network/no-response failures
//!   (connection refused, timeouts — also §7.2) retry at most 2 times with a
//!   separate counter. POST is never transient-retried. `Retry-After` is
//!   ignored.
//! * **401 refresh** (spec §7.4): any 401, regardless of method, triggers one
//!   token refresh through the injected callback and one retry of the
//!   original request. A failed refresh surfaces as an `Auth` error without a
//!   retry, and a 401 after a refresh is a normal `NotAuthenticated` API
//!   error (no second refresh).
//!
//! Backoff (spec §7.1, gaxios 7.3.1 `getNextRetryDelay`): the first retry
//! waits 100ms; each subsequent retry waits `((2^n − 1) / 2) × 1000ms` with
//! `n` the completed-retry count — the series [100, 500, 1500]. Both clamps
//! of the gaxios `min()` (a `totalTimeout` remaining budget and
//! `maxRetryDelay`) are injectable and unlimited by default. The sleeper and
//! clock are injectable so tests assert delay sequences without real waiting.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::api::BaseUrls;
use crate::api::error::{ApiErrorKind, api_error_from_response, network_error, parse_error};
use crate::error::CrspError;

/// Request timeout (spec §7.2).
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Connect timeout (spec §7.2).
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Maximum transient retries for retryable HTTP statuses (spec §7.1).
pub const MAX_STATUS_RETRIES: u32 = 3;

/// Maximum transient retries for network/no-response failures (spec §7.1).
pub const MAX_NETWORK_RETRIES: u32 = 2;

/// google-auth-library `DEFAULT_EAGER_REFRESH_THRESHOLD_MILLIS` (authclient.js):
/// a token whose `expiry_date` falls within this window is refreshed before
/// the request (`isTokenExpiring`).
pub const EAGER_REFRESH_THRESHOLD_MS: i64 = 5 * 60 * 1000;

/// Shared token-expiry cell (milliseconds since the Unix epoch), wired by the
/// production context so the client can lazy-refresh expiring tokens before
/// sending. `None` = unknown expiry (library semantics: treated as never
/// expiring).
pub type ExpiryCell = Arc<StdMutex<Option<i64>>>;

/// Callback producing a fresh access token (spec §7.4). Production wires this
/// to [`crate::auth::flow::refresh_and_save`]; wiremock tests inject a
/// counting stub.
pub type RefreshFn = Arc<dyn Fn() -> BoxFuture<'static, Result<String, CrspError>> + Send + Sync>;

/// Injectable backoff sleeper (spec §7.1): records or performs the delay.
pub type SleepFn = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

/// Injectable monotonic-ish clock returning milliseconds (gaxios `Date.now()`
/// equivalent), used for the `totalTimeout` remaining-budget clamp.
pub type ClockFn = Arc<dyn Fn() -> u128 + Send + Sync>;

/// Parameters for building an authenticated [`ApiClient`].
#[derive(Clone)]
pub struct ApiClientConfig {
    /// Current access token (sent as a masked bearer header).
    pub access_token: String,
    /// One-shot 401 refresh callback (spec §7.4).
    pub refresh: RefreshFn,
    /// Shared expiry cell for the lazy first-use/expiry refresh (google-auth
    /// library `isTokenExpiring`). `None` = unknown (never expiring).
    pub token_expiry: Option<ExpiryCell>,
    /// Optional injectable sleeper; defaults to `tokio::time::sleep`.
    pub sleeper: Option<SleepFn>,
    /// `maxRetryDelay` clamp for the backoff `min()` (gaxios default:
    /// effectively unlimited). `None` = unlimited.
    pub max_retry_delay: Option<Duration>,
    /// `totalTimeout` budget for the backoff `min()` (gaxios default:
    /// effectively unlimited). `None` = unlimited.
    pub total_timeout: Option<Duration>,
    /// Optional injectable clock (milliseconds) for the `totalTimeout`
    /// remaining-budget computation; defaults to the wall clock.
    pub clock: Option<ClockFn>,
}

impl ApiClientConfig {
    pub fn new(access_token: impl Into<String>, refresh: RefreshFn) -> Self {
        Self {
            access_token: access_token.into(),
            refresh,
            token_expiry: None,
            sleeper: None,
            max_retry_delay: None,
            total_timeout: None,
            clock: None,
        }
    }
}

/// A request body for [`ApiClient::request`].
#[derive(Debug, Clone)]
pub enum ApiBody {
    /// Serialized as JSON with an `application/json` content type.
    Json(Value),
}

/// A fully resolved API request (method, absolute URL, optional body).
#[derive(Debug, Clone)]
pub struct ApiRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub body: Option<ApiBody>,
}

/// A successful (2xx) response from [`ApiClient::request`].
pub struct ApiResponse(reqwest::Response);

impl std::fmt::Debug for ApiResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately opaque: response bodies may contain sensitive data.
        f.debug_struct("ApiResponse")
            .field("status", &self.0.status().as_u16())
            .finish()
    }
}

impl ApiResponse {
    pub fn status(&self) -> reqwest::StatusCode {
        self.0.status()
    }

    /// Parses the body as `T` (clasp degrade semantics: empty or non-JSON
    /// bodies behave like an object with every field absent, so typed
    /// accessors never fail on transport noise; only structurally unexpected
    /// JSON produces an error).
    pub async fn json<T: DeserializeOwned>(self) -> Result<T, CrspError> {
        let text = self.0.text().await.map_err(|error| CrspError::Api {
            kind: ApiErrorKind::UnexpectedApiError,
            message: error.to_string(),
        })?;
        parse_body(&text)
    }

    /// Parses the body as a raw JSON value (`null` for empty bodies).
    pub async fn json_value(self) -> Result<Value, CrspError> {
        self.json::<Value>().await
    }
}

fn parse_body<T: DeserializeOwned>(text: &str) -> Result<T, CrspError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        // Empty bodies degrade to `null`/empty-object, matching the tolerant
        // clasp parsing (missing fields default via serde).
        return serde_json::from_value::<T>(Value::Null)
            .or_else(|_| serde_json::from_value::<T>(Value::Object(Default::default())))
            .map_err(parse_error);
    }
    // Fast path: deserialize straight from the raw text into the target type
    // with no intermediate `Value` AST and no deep clone (the common case for
    // every API response, including multi-MB content payloads).
    match serde_json::from_str::<T>(trimmed) {
        Ok(parsed) => Ok(parsed),
        Err(_) => {
            // Tolerant fallback: some fixtures carry unknown shapes (extra
            // fields are ignored by the direct path already); degrade to
            // `null`/empty-object for structurally unexpected JSON, exactly
            // as the old `Value::Null` fallback did.
            serde_json::from_value::<T>(Value::Null)
                .or_else(|_| serde_json::from_value::<T>(Value::Object(Default::default())))
                .map_err(parse_error)
        }
    }
}

/// The authenticated Google API client (spec §2.2). Typed service accessors
/// (`script()`, `drive()`, …) build on [`ApiClient::request`]. Clone shares
/// the token/refresh state (used by the MCP server's CLI-context reuse).
pub struct ApiClient {
    pub(crate) http: reqwest::Client,
    base_urls: BaseUrls,
    access_token: Arc<StdMutex<String>>,
    token_expiry: Option<ExpiryCell>,
    refresh: RefreshFn,
    sleeper: SleepFn,
    max_retry_delay: Option<Duration>,
    total_timeout: Option<Duration>,
    clock: ClockFn,
}

impl Clone for ApiClient {
    fn clone(&self) -> Self {
        Self {
            http: self.http.clone(),
            base_urls: self.base_urls.clone(),
            access_token: Arc::clone(&self.access_token),
            token_expiry: self.token_expiry.clone(),
            refresh: Arc::clone(&self.refresh),
            sleeper: Arc::clone(&self.sleeper),
            max_retry_delay: self.max_retry_delay,
            total_timeout: self.total_timeout,
            clock: Arc::clone(&self.clock),
        }
    }
}

impl ApiClient {
    /// Builds a client with production base URLs plus the documented
    /// `CRSP_*_BASE_URL` runtime overrides (spec §9.1).
    pub fn new(config: ApiClientConfig) -> Result<Self, CrspError> {
        Self::with_base_urls(config, BaseUrls::from_env())
    }

    /// Builds a client with explicit base URLs for in-process wiremock tests;
    /// never touches process environment variables.
    pub fn with_base_urls(config: ApiClientConfig, base_urls: BaseUrls) -> Result<Self, CrspError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            // Spinner-wrapped calls may be driven on the shared isolated
            // runtime (see `Ui::drive_isolated`) while earlier requests ran
            // on the ambient runtime. A pooled idle connection established on
            // one runtime hangs when re-driven on another, stalling for the
            // full request timeout, so idle reuse is disabled: every request
            // opens a fresh connection on the driving runtime.
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|error| {
                CrspError::Config(format!("Failed to initialize HTTP client: {error}"))
            })?;
        Ok(Self {
            http,
            base_urls,
            access_token: Arc::new(StdMutex::new(config.access_token)),
            token_expiry: config.token_expiry,
            refresh: config.refresh,
            sleeper: config.sleeper.unwrap_or_else(default_sleeper),
            max_retry_delay: config.max_retry_delay,
            total_timeout: config.total_timeout,
            clock: config.clock.unwrap_or_else(default_clock),
        })
    }

    /// Replaces the injectable sleeper (spec §7.1 tests).
    pub fn with_sleeper(self, sleeper: SleepFn) -> Self {
        Self { sleeper, ..self }
    }

    pub fn base_urls(&self) -> &BaseUrls {
        &self.base_urls
    }

    /// Typed Apps Script API methods.
    pub fn script(&self) -> crate::api::script::ScriptApi<'_> {
        crate::api::script::ScriptApi(self)
    }

    /// Typed Drive v3 API methods.
    pub fn drive(&self) -> crate::api::drive::DriveApi<'_> {
        crate::api::drive::DriveApi(self)
    }

    /// Typed Service Usage API methods.
    pub fn service_usage(&self) -> crate::api::service_usage::ServiceUsageApi<'_> {
        crate::api::service_usage::ServiceUsageApi(self)
    }

    /// Typed Discovery API methods.
    pub fn discovery(&self) -> crate::api::discovery::DiscoveryApis<'_> {
        crate::api::discovery::DiscoveryApis(self)
    }

    /// Typed Cloud Logging API methods.
    pub fn logging(&self) -> crate::api::logging::LoggingApi<'_> {
        crate::api::logging::LoggingApi(self)
    }

    /// Userinfo and token endpoint methods.
    pub fn oauth2(&self) -> crate::api::oauth2::OAuth2Api<'_> {
        crate::api::oauth2::OAuth2Api(self)
    }

    /// Executes a request through the retry and 401-refresh layers (spec
    /// §7.1, §7.4), converting failures into [`CrspError`]. Success is any
    /// 2xx response; every other final outcome is an error.
    pub async fn request(&self, request: ApiRequest) -> Result<ApiResponse, CrspError> {
        // clasp parity (google-auth-library 10.5.0 `getRequestMetadataAsync`):
        // the token is refreshed lazily at FIRST USE — before any API request
        // — when it is missing, or when its stored expiry falls within the
        // eager-refresh threshold (`isTokenExpiring`). Without a refresh
        // mechanism the closure fails locally — the `No access, refresh
        // token, API key or refresh handler callback is set.` error — so an
        // unauthenticated run fails fast instead of sending an empty bearer
        // (spec §4.3 contract). A refresh that completes without a token is
        // the library's `Could not refresh access token.` error.
        if self.needs_pre_send_refresh() {
            let new_token = (self.refresh)().await?;
            if new_token.is_empty() {
                return Err(CrspError::Auth(
                    crate::i18n::COULD_NOT_REFRESH_ACCESS_TOKEN.to_string(),
                ));
            }
            *self.access_token.lock().expect("access token lock") = new_token;
        }
        let response = self.execute(&request).await?;
        if response.status().is_success() {
            Ok(ApiResponse(response))
        } else {
            Err(api_error_from_response(response).await)
        }
    }

    /// google-auth-library `getAccessToken`/`isTokenExpiring` semantics: a
    /// refresh is required before sending when the access token is missing,
    /// or when the known expiry falls within the eager-refresh threshold.
    /// Without expiry information the token is treated as never expiring.
    fn needs_pre_send_refresh(&self) -> bool {
        if self
            .access_token
            .lock()
            .expect("access token lock")
            .is_empty()
        {
            return true;
        }
        let Some(cell) = &self.token_expiry else {
            return false;
        };
        let Some(expiry) = *cell.lock().expect("token expiry lock") else {
            return false;
        };
        let now = (self.clock)().min(i64::MAX as u128) as i64;
        expiry <= now + EAGER_REFRESH_THRESHOLD_MS
    }

    /// The retry/refresh state machine. Returns the final response for both
    /// success and error statuses; transport failures become `CrspError`.
    async fn execute(&self, request: &ApiRequest) -> Result<reqwest::Response, CrspError> {
        let idempotent = is_idempotent(&request.method);
        let mut status_retries: u32 = 0;
        let mut network_retries: u32 = 0;
        let mut completed_retries: u32 = 0;
        let mut refreshed = false;
        let time_of_first_request = (self.clock)();

        loop {
            match self.attempt(request).await {
                Err(error) => {
                    // Network/no-response failure (timeouts included, §7.2).
                    if idempotent && network_retries < MAX_NETWORK_RETRIES {
                        network_retries += 1;
                        let elapsed = (self.clock)().saturating_sub(time_of_first_request);
                        self.wait(completed_retries, elapsed).await;
                        completed_retries += 1;
                        continue;
                    }
                    return Err(network_error(error));
                }
                Ok(response) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        if refreshed {
                            // 401 after the refresh retry: no second refresh.
                            return Err(api_error_from_response(response).await);
                        }
                        // One token refresh + one retry of the original
                        // request, for every method (spec §7.4). A failed
                        // refresh propagates as an Auth error immediately.
                        let new_token = (self.refresh)().await?;
                        *self.access_token.lock().expect("access token lock") = new_token;
                        refreshed = true;
                        continue;
                    }
                    if idempotent
                        && is_retryable_status(status)
                        && status_retries < MAX_STATUS_RETRIES
                    {
                        status_retries += 1;
                        let elapsed = (self.clock)().saturating_sub(time_of_first_request);
                        self.wait(completed_retries, elapsed).await;
                        completed_retries += 1;
                        // Drain the body so the connection can be reused.
                        let _ = response.bytes().await;
                        continue;
                    }
                    return Ok(response);
                }
            }
        }
    }

    /// One request attempt with the current token.
    async fn attempt(&self, request: &ApiRequest) -> Result<reqwest::Response, reqwest::Error> {
        let token = self.access_token.lock().expect("access token lock").clone();
        let mut builder = self
            .http
            .request(request.method.clone(), &request.url)
            .bearer_auth(token);
        if let Some(ApiBody::Json(value)) = &request.body {
            builder = builder.json(value);
        }
        builder.send().await
    }

    /// Backoff before the next retry (spec §7.1). `elapsed` is measured from
    /// the first request via the injectable clock.
    async fn wait(&self, completed_retries: u32, elapsed_ms: u128) {
        let remaining_total_ms = self
            .total_timeout
            .map(|total| total.as_millis() as i128 - elapsed_ms as i128);
        let max_retry_delay_ms = self.max_retry_delay.map(|max| max.as_millis());
        let millis = retry_delay_ms(completed_retries, remaining_total_ms, max_retry_delay_ms);
        (self.sleeper)(Duration::from_millis(millis)).await;
    }
}

fn default_sleeper() -> SleepFn {
    Arc::new(|duration| Box::pin(tokio::time::sleep(duration)))
}

fn default_clock() -> ClockFn {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default()
    })
}

/// gaxios 7.3.1 `getNextRetryDelay` (build/cjs/src/retry.js — the binding
/// authority for spec §7.1), verbatim semantics:
///
/// ```js
/// const retryDelay = config.currentRetryAttempt ? 0 : (config.retryDelay ?? 100);
/// const calculatedDelay = retryDelay + ((Math.pow(2, config.currentRetryAttempt) - 1) / 2) * 1000;
/// return Math.min(calculatedDelay, maxAllowableDelay, config.maxRetryDelay);
/// ```
///
/// The 100ms base applies only to the first retry (`currentRetryAttempt ==
/// 0`); the pow term uses the completed-retry count, giving the series
/// [100, 500, 1500]. `remaining_total_ms` is `totalTimeout − elapsed`
/// (unlimited when `None`); a negative remaining budget clamps to an
/// immediate retry, matching gaxios's negative `setTimeout`. The
/// `(2^n − 1) × 500` form keeps the `/ 2 × 1000` arithmetic integer-exact.
fn retry_delay_ms(
    completed_retries: u32,
    remaining_total_ms: Option<i128>,
    max_retry_delay_ms: Option<u128>,
) -> u64 {
    let n = completed_retries.min(62);
    let base: u128 = u128::from(completed_retries == 0) * 100;
    let calculated = base + (2u128.pow(n) - 1) * 500;
    let mut delay = calculated;
    if let Some(remaining) = remaining_total_ms {
        delay = delay.min(remaining.max(0) as u128);
    }
    if let Some(max) = max_retry_delay_ms {
        delay = delay.min(max);
    }
    delay as u64
}

/// Methods that gaxios considers retryable (spec §7.1). POST is excluded.
fn is_idempotent(method: &reqwest::Method) -> bool {
    matches!(
        *method,
        reqwest::Method::GET
            | reqwest::Method::HEAD
            | reqwest::Method::PUT
            | reqwest::Method::OPTIONS
            | reqwest::Method::DELETE
    )
}

/// Statuses that trigger transient retries (spec §7.1): 100-199, 408, 429,
/// 500-599.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    let code = status.as_u16();
    (100..=199).contains(&code) || code == 408 || code == 429 || (500..=599).contains(&code)
}

// ---------------------------------------------------------------------------
// URL building (qs RFC3986-compatible query encoding, clasp parameter order)
// ---------------------------------------------------------------------------

/// Joins a service base URL with an endpoint path.
pub fn service_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// Percent-encodes a query component exactly like `qs`'s default RFC3986
/// encoder (gaxios `paramsSerializer`): unreserved characters only
/// (`A-Za-z0-9-._~`), everything else percent-encoded — so spaces become
/// `%20` and `/'()!*` are escaped.
pub fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Appends `key=value` to a URL, encoding like gaxios's `qs` serializer.
pub fn append_query(url: &str, key: &str, value: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!(
        "{url}{separator}{}={}",
        encode_query_component(key),
        encode_query_component(value)
    )
}

/// Appends `key=value` only when the value exists (clasp omits absent
/// query parameters such as `pageToken` on the first page).
pub fn append_query_if_some(url: &str, key: &str, value: Option<&str>) -> String {
    match value {
        Some(value) => append_query(url, key, value),
        None => url.to_string(),
    }
}
