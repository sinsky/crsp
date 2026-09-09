//! Serverless redirect flow (spec §2.4; clasp
//! `serverless_auth_code_flow.ts`): no local server — the user authorizes on
//! another device and pastes the final redirect URL, which is parsed for the
//! code and validated against the expected state.

use crate::auth::oauth_client::SERVERLESS_REDIRECT_PORT;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptInput, Ui};

/// The redirect URI for the serverless flow: `http://localhost:8888` unless
/// `--redirect-port` overrides the port (clasp `getRedirectUri`).
pub fn redirect_uri(port: Option<u16>) -> String {
    format!(
        "http://localhost:{}",
        port.unwrap_or(SERVERLESS_REDIRECT_PORT)
    )
}

/// Prints the authorization URL banner, prompts for the pasted redirect URL,
/// and returns the authorization code (clasp `promptAndReturnCode`).
pub async fn obtain_code<W: std::io::Write, E: std::io::Write, A: crate::ui::PromptAdapter>(
    authorization_url: &str,
    expected_state: &str,
    ui: &Ui<A>,
    output: &mut Output<W, E>,
) -> Result<String, CrspError> {
    output.message(&i18n::authorize_url_serverless(authorization_url));
    let answer = ui.input(PromptInput {
        prompt: i18n::PASTE_AUTH_URL_PROMPT.to_string(),
        placeholder: None,
        default: None,
    })?;
    parse_pasted_response(&answer, expected_state)
}

/// Parses a pasted redirect URL the way clasp's `parseAuthResponseUrl` does
/// (relative or malformed inputs degrade to an empty query): OAuth errors
/// surface verbatim, state mismatches are CSRF rejections, and a missing
/// code is an error.
pub fn parse_pasted_response(raw: &str, expected_state: &str) -> Result<String, CrspError> {
    let params = url::Url::parse(raw)
        .or_else(|_| url::Url::parse(&format!("http://localhost/{raw}")))
        .ok()
        .map(|url| {
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let get_param = |key: &str| {
        params
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.to_string())
    };

    if let Some(error) = get_param("error") {
        return Err(CrspError::Auth(error));
    }
    let state_valid = get_param("state")
        .map(|state| state == expected_state)
        .unwrap_or(false);
    if !state_valid {
        return Err(CrspError::Auth(i18n::STATE_MISMATCH_CSRF.to_string()));
    }
    get_param("code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| CrspError::Auth(i18n::MISSING_CODE_IN_RESPONSE_URL.to_string()))
}
