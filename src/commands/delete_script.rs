//! `delete-script` (alias `delete`) — clasp commands/delete-script.ts.
//!
//! Trashes the script's Drive file. Deletion requires confirmation: `-f`
//! bypasses the prompt; noninteractive runs without `-f` are a silent no-op
//! (clasp behavior). In crsp, `--json` emits `{"success": true}` after a
//! confirmed deletion (spec §5 #8; clasp prints nothing).

use std::io::Write;

use serde::Serialize;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::project::trash_script;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

/// Delete result: `deleted` is false for a refused/silent no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteScriptResult {
    pub deleted: bool,
}

/// crsp `--json` payload (spec §5 #8).
#[derive(Serialize)]
struct DeletedJson {
    success: bool,
}

pub async fn delete_script<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    script_id: Option<&str>,
    force: bool,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<DeleteScriptResult, CrspError> {
    let script_id = match script_id.or(config.script_id.as_deref()) {
        Some(script_id) => script_id.to_string(),
        None => {
            return Err(CrspError::Validation(
                i18n::SCRIPT_ID_NOT_SET_UNABLE_TO_DELETE.to_string(),
            ));
        }
    };

    // clasp delete-script.ts:34-52: `--force` skips the prompt; an
    // interactive terminal confirms; anything else stays unconfirmed and the
    // command returns silently.
    let mut confirmed = force;
    if !confirmed && ui.is_interactive() {
        confirmed = ui.confirm(PromptConfirm {
            prompt: i18n::ARE_YOU_SURE_YOU_WANT_TO_DELETE_SCRIPT.to_string(),
            default: false,
        })?;
    }
    if !confirmed {
        return Ok(DeleteScriptResult { deleted: false });
    }

    trash_script(client, &script_id).await?;
    if output.is_json() {
        output.print_json(&DeletedJson { success: true })?;
    } else {
        output.message(&i18n::deleted_script(&script_id));
    }
    Ok(DeleteScriptResult { deleted: true })
}
