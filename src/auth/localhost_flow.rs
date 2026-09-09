//! Localhost redirect flow (spec §2.4; clasp `localhost_auth_code_flow.ts`).
//!
//! Binds a `tokio::net::TcpListener`, ignores probes and `/favicon.ico`
//! (404), and resolves only a valid `GET /?code=...&state=...` callback with
//! a minimal HTTP 200 HTML response. Callback completion, cancellation, and
//! an optional timeout race through `tokio::select!` — no blocking server,
//! no `spawn_blocking`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::CrspError;
use crate::i18n;

/// Upper bound for reading a request head before treating the connection as
/// a dead probe (browsers deliver the head immediately).
const REQUEST_HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum request-head size before a connection is dropped as a probe.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// The bound localhost redirect listener.
#[derive(Debug)]
pub struct LocalhostListener {
    listener: TcpListener,
    port: u16,
}

impl LocalhostListener {
    /// Binds `127.0.0.1:port` (`0` = random port, clasp's default). Port
    /// conflicts surface clasp's `--redirect-port` guidance.
    pub async fn bind(port: u16) -> Result<Self, CrspError> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await;
        match listener {
            Ok(listener) => {
                let bound = listener.local_addr()?.port();
                Ok(Self {
                    listener,
                    port: bound,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                Err(CrspError::Auth(i18n::port_already_in_use(port)))
            }
            Err(_) => Err(CrspError::Auth(i18n::unable_to_start_server_on_port(port))),
        }
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The runtime redirect URI (`http://localhost:<port>`).
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Accepts connections until a valid callback resolves the code, the
    /// cancel signal fires (Ctrl+C wiring), or the optional timeout expires.
    ///
    /// Probes, non-GET requests, and `/favicon.ico` get a 404 and are
    /// ignored. OAuth `error` params, state mismatches, and missing codes
    /// respond 400 and abort the flow with clasp's messages.
    pub async fn wait_for_code(
        &self,
        expected_state: &str,
        cancel: tokio::sync::oneshot::Receiver<()>,
        timeout: Option<Duration>,
    ) -> Result<String, CrspError> {
        let accept_loop = async {
            loop {
                let (stream, _) = self.listener.accept().await.map_err(CrspError::Io)?;
                match handle_connection(stream, expected_state).await? {
                    Some(outcome) => return outcome,
                    None => continue,
                }
            }
        };

        let timeout_future = async {
            match timeout {
                Some(duration) => tokio::time::sleep(duration).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            result = accept_loop => result,
            _ = cancel => Err(CrspError::Aborted),
            _ = timeout_future => Err(CrspError::Auth(i18n::AUTH_TIMED_OUT.to_string())),
        }
    }
}

/// Outcome of one connection: `None` keeps listening (probe), `Some` ends
/// the flow.
type ConnectionOutcome = Option<Result<String, CrspError>>;

async fn handle_connection(
    mut stream: TcpStream,
    expected_state: &str,
) -> Result<ConnectionOutcome, CrspError> {
    let head = match read_head(&mut stream).await {
        Some(head) => head,
        None => return Ok(None),
    };
    let Some(request_line) = head.lines().next() else {
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    // Only a GET callback on `/` carries an authorization result; everything
    // else (probes, favicons, other paths) is answered 404 and ignored.
    let path_ok = target == "/" || target.starts_with("/?");
    if method != "GET" || !path_ok {
        respond(&mut stream, "HTTP/1.1 404 Not Found", None, "").await;
        return Ok(None);
    }

    let url = match url::Url::parse(&format!("http://localhost{target}")) {
        Ok(url) => url,
        Err(_) => {
            respond(&mut stream, "HTTP/1.1 404 Not Found", None, "").await;
            return Ok(None);
        }
    };

    let get_param = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    };

    // OAuth errors from the authorization server abort the flow (clasp
    // ts:112-121).
    if let Some(error) = get_param("error") {
        respond(
            &mut stream,
            "HTTP/1.1 400 Bad Request",
            Some("text/plain; charset=utf-8"),
            i18n::AUTHORIZATION_FAILED_RETRY,
        )
        .await;
        return Ok(Some(Err(CrspError::Auth(error))));
    }

    // Connections without any OAuth parameters are probes (health checks,
    // scanners): answer 404 and keep waiting (spec §2.4).
    let code = get_param("code").filter(|code| !code.is_empty());
    let state = get_param("state");
    if code.is_none() && state.is_none() {
        respond(&mut stream, "HTTP/1.1 404 Not Found", None, "").await;
        return Ok(None);
    }

    // State validation against CSRF (clasp ts:123-132, RFC 6749 §10.12).
    let state_valid = state.as_deref() == Some(expected_state);
    if !state_valid {
        respond(
            &mut stream,
            "HTTP/1.1 400 Bad Request",
            Some("text/plain; charset=utf-8"),
            i18n::STATE_MISMATCH_CSRF,
        )
        .await;
        return Ok(Some(Err(CrspError::Auth(
            i18n::STATE_MISMATCH_CSRF.to_string(),
        ))));
    }

    let Some(code) = code else {
        respond(
            &mut stream,
            "HTTP/1.1 400 Bad Request",
            Some("text/plain; charset=utf-8"),
            i18n::MISSING_AUTHORIZATION_CODE,
        )
        .await;
        return Ok(Some(Err(CrspError::Auth(
            i18n::MISSING_AUTHORIZATION_CODE.to_string(),
        ))));
    };

    // Minimal HTML success page (spec §2.4).
    let body = format!(
        "<html><body>{}</body></html>",
        i18n::LOGGED_IN_YOU_MAY_CLOSE
    );
    respond(
        &mut stream,
        "HTTP/1.1 200 OK",
        Some("text/html; charset=utf-8"),
        &body,
    )
    .await;
    Ok(Some(Ok(code)))
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let read = tokio::time::timeout(REQUEST_HEAD_TIMEOUT, async {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let count = stream.read(&mut chunk).await.ok()?;
            if count == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..count]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n")
                || buffer.len() > MAX_HEAD_BYTES
            {
                break;
            }
        }
        Some(buffer)
    });
    match read.await {
        Ok(Some(buffer)) => Some(String::from_utf8_lossy(&buffer).into_owned()),
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, content_type: Option<&str>, body: &str) {
    let mut response = format!("{status}\r\n");
    if let Some(content_type) = content_type {
        response.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(body);
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}
