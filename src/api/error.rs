//! API error normalization (spec §2.1, §6.1; clasp `handleApiError`,
//! `core/utils.ts:265-303`, gaxios `extractAPIErrorFromResponse`).
//!
//! HTTP statuses map to [`ApiErrorKind`] (400 → `InvalidArgument`,
//! 401 → `NotAuthenticated`, 403 → `NotAuthorized`, 404 → `NotFound`,
//! everything else → `UnexpectedApiError`) and the message is extracted from
//! the response body the way gaxios does for Google AIP-193 payloads:
//! `error.errors[].message` joined with newlines, else `error.message`, else
//! the string `error`, else the raw body, else the status code.

use serde_json::Value;

use crate::error::CrspError;

/// API failure categories normalized from HTTP status codes (spec §2.1,
/// §6.1; clasp `ERROR_CODES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorKind {
    InvalidArgument,
    NotAuthenticated,
    NotAuthorized,
    NotFound,
    UnexpectedApiError,
}

/// Maps an HTTP status to its normalized kind (clasp `handleApiError`).
pub fn api_error_kind(status: u16) -> ApiErrorKind {
    match status {
        400 => ApiErrorKind::InvalidArgument,
        401 => ApiErrorKind::NotAuthenticated,
        403 => ApiErrorKind::NotAuthorized,
        404 => ApiErrorKind::NotFound,
        _ => ApiErrorKind::UnexpectedApiError,
    }
}

/// Extracts the gaxios-style error message from a response body (gaxios
/// `extractAPIErrorFromResponse`; clasp `isDetailedError` then reads
/// `error.errors[0].message`, and gaxios joins all detailed messages).
pub fn api_error_message(status: u16, body: &str) -> String {
    let default = format!("Request failed with status code {status}");
    let Ok(value) = serde_json::from_str::<Value>(body.trim()) else {
        return if body.trim().is_empty() {
            default
        } else {
            body.trim().to_string()
        };
    };
    let Some(error) = value.get("error") else {
        return default;
    };
    match error {
        Value::String(text) => text.clone(),
        Value::Object(_) => {
            let base = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(default);
            let detailed: Vec<String> = error
                .get("errors")
                .and_then(Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .filter_map(|entry| entry.get("message").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if detailed.is_empty() {
                base
            } else {
                detailed.join("\n")
            }
        }
        _ => default,
    }
}

/// Builds the [`CrspError::Api`] value for a non-success response.
pub fn api_error(status: u16, body: &str) -> CrspError {
    CrspError::Api {
        kind: api_error_kind(status),
        message: api_error_message(status, body),
    }
}

/// Reads the response body and converts a non-success response into
/// [`CrspError::Api`].
pub(crate) async fn api_error_from_response(response: reqwest::Response) -> CrspError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    api_error(status, &body)
}

/// Normalizes a network-level failure (no response: refused connection,
/// timeout, reset) the way clasp's `handleApiError` normalizes no-response
/// gaxios errors: `UNEXPECTED_API_ERROR` with the raw error text.
pub(crate) fn network_error(error: reqwest::Error) -> CrspError {
    CrspError::Api {
        kind: ApiErrorKind::UnexpectedApiError,
        message: error.to_string(),
    }
}

/// Normalizes a response-body parse failure (clasp degrades to absent fields,
/// so this only fires on structurally unexpected JSON).
pub(crate) fn parse_error(error: serde_json::Error) -> CrspError {
    CrspError::Api {
        kind: ApiErrorKind::UnexpectedApiError,
        message: format!("Failed to parse API response: {error}"),
    }
}
