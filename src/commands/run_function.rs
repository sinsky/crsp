//! `run-function` (alias `run`) — clasp commands/run-function.ts.
//!
//! `--params` is parsed as JSON and forwarded verbatim (clasp's
//! `parameters ?? []` never validates the top-level type; `null` parses to a
//! nullish value and sends `[]`). A missing function name is selected from
//! the project content interactively; noninteractive runs forward the
//! missing name exactly like clasp (the `function` key is dropped from the
//! request body). `NOT_AUTHORIZED`/`NOT_FOUND` map to clasp's notices.

use std::io::Write;

use serde::Serialize;
use serde_json::Value;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::project::assert_script_configured;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptSelect, Ui};

/// Arguments for [`run_function`] (clasp `run-function [functionName]
/// --nondev -p/--params`).
#[derive(Debug, Clone, Copy, Default)]
pub struct RunFunctionArgs<'a> {
    pub function_name: Option<&'a str>,
    pub nondev: bool,
    pub params: Option<&'a str>,
}

/// The run `--json` payload (clasp run-function.ts:73-85): `response` and
/// `error` keys are omitted when undefined (`JSON.stringify` semantics).
#[derive(Serialize)]
struct RunJson<'a> {
    #[serde(rename = "response", skip_serializing_if = "Option::is_none")]
    response: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RunErrorJson<'a>>,
}

#[derive(Serialize)]
struct RunErrorJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

pub async fn run_function<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: RunFunctionArgs<'_>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    // clasp parses `--params` before any script-configuration assert.
    let mut parameters = Value::Array(Vec::new());
    if let Some(params) = args.params.filter(|params| !params.is_empty()) {
        parameters = serde_json::from_str(params)
            .map_err(|error| CrspError::Validation(error.to_string()))?;
        // `parameters ?? []`: JSON null is nullish.
        if parameters.is_null() {
            parameters = Value::Array(Vec::new());
        }
    }

    let mut function_name = args.function_name.map(str::to_string);
    if function_name.is_none() && ui.is_interactive() {
        let script_id = assert_script_configured(config).await?.to_string();
        let content = client.script().get_content(&script_id, None).await?;
        let names: Vec<String> = content
            .files()
            .iter()
            .flat_map(|file| file.function_names())
            .collect();
        function_name = Some(
            ui.select(PromptSelect {
                prompt: i18n::SELECT_A_FUNCTION_NAME.to_string(),
                options: names
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect(),
                default: None,
            })?,
        );
    }

    let script_id = assert_script_configured(config).await?.to_string();
    let outcome = ui.with_spinner(
        &i18n::running_function(function_name.as_deref().unwrap_or_default()),
        move || {
            crate::ui::drive_isolated(async move {
                client
                    .script()
                    .run(
                        &script_id,
                        function_name.as_deref(),
                        parameters,
                        !args.nondev,
                    )
                    .await
            })
        },
    )?;
    let result = match outcome {
        // clasp run-function.ts:111-125: `error.cause?.code` special cases.
        Err(CrspError::Api {
            kind: crate::api::error::ApiErrorKind::NotAuthorized,
            ..
        }) => {
            return Err(CrspError::Validation(
                i18n::RUN_FUNCTION_NOT_AUTHORIZED.to_string(),
            ));
        }
        Err(CrspError::Api {
            kind: crate::api::error::ApiErrorKind::NotFound,
            ..
        }) => {
            return Err(CrspError::Validation(
                i18n::RUN_FUNCTION_NOT_FOUND.to_string(),
            ));
        }
        other => other?,
    };
    if result.is_null() {
        // clasp `if (!res.data) throw new Error('Function returned undefined')`.
        return Err(CrspError::Validation(
            i18n::FUNCTION_RETURNED_UNDEFINED.to_string(),
        ));
    }

    if output.is_json() {
        output.print_json(&RunJson {
            response: result
                .get("response")
                .and_then(|response| response.get("result")),
            error: result.get("error").map(|error| RunErrorJson {
                code: error.get("code"),
                message: error.get("message").and_then(Value::as_str),
                details: error.get("details"),
            }),
        })?;
        return Ok(());
    }

    // Human branches (clasp run-function.ts:88-108). The Exception path is a
    // script-level error: stderr line, exit 0.
    let error = result.get("error");
    let details = error.and_then(|error| error.get("details"));
    if error.is_some() && js_truthy(details) {
        let first = details
            .and_then(Value::as_array)
            .and_then(|details| details.first())
            .ok_or_else(|| {
                CrspError::Validation(
                    "Cannot destructure property 'errorMessage' of \
                     'result.error.details[0]' as it is undefined."
                        .to_string(),
                )
            })?;
        let error_message = match first.get("errorMessage") {
            Some(Value::String(message)) => message.clone(),
            Some(other) => js_inspect(other),
            None => "undefined".to_string(),
        };
        let stack = match first.get("scriptStackTraceElements") {
            Some(stack) if js_truthy(Some(stack)) => js_inspect(stack),
            _ => "[]".to_string(),
        };
        output.warn(&format!("{} {error_message} {stack}", i18n::RUN_EXCEPTION));
        return Ok(());
    }

    let response_result = result
        .get("response")
        .and_then(|response| response.get("result"));
    match response_result {
        Some(value) => output.message(&js_inspect(value)),
        None => output.message(i18n::RUN_NO_RESPONSE),
    }
    Ok(())
}

/// JS truthiness for JSON values (clasp `if (result.error.details)`).
fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(string)) => !string.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// Renders a JSON value the way Node's `console.log`/`console.error` format
/// top-level arguments (util.inspect without width-based line breaking):
/// strings print raw at the top level, containers print with single-quoted
/// string elements, `{ key: value }` objects, `[ a, b ]` arrays.
fn js_inspect(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(string) => string.clone(),
        Value::Array(entries) => {
            if entries.is_empty() {
                "[]".to_string()
            } else {
                let items: Vec<String> = entries.iter().map(js_inspect_element).collect();
                format!("[ {} ]", items.join(", "))
            }
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                "{}".to_string()
            } else {
                let items: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| {
                        format!("{}: {}", inspect_key(key), js_inspect_element(value))
                    })
                    .collect();
                format!("{{ {} }}", items.join(", "))
            }
        }
    }
}

/// Inspects a nested element (strings get single quotes).
fn js_inspect_element(value: &Value) -> String {
    match value {
        Value::String(string) => format!("'{}'", escape_inspect_string(string)),
        other => js_inspect(other),
    }
}

fn inspect_key(key: &str) -> String {
    let valid_identifier = !key.is_empty()
        && key.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric()
                || character == '_'
                || character == '$'
                || (index == 0 && character.is_ascii_alphabetic())
                || (index > 0 && character.is_ascii_digit())
        })
        && !key
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit());
    if valid_identifier {
        key.to_string()
    } else {
        format!("'{}'", escape_inspect_string(key))
    }
}

fn escape_inspect_string(string: &str) -> String {
    let mut escaped = String::with_capacity(string.len());
    for character in string.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\'' => escaped.push_str("\\'"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::js_inspect;
    use serde_json::json;

    #[test]
    fn inspect_matches_console_log_for_common_values() {
        assert_eq!(js_inspect(&json!("raw")), "raw");
        assert_eq!(js_inspect(&json!(42)), "42");
        assert_eq!(js_inspect(&json!(1.5)), "1.5");
        assert_eq!(js_inspect(&json!(null)), "null");
        assert_eq!(js_inspect(&json!(true)), "true");
        assert_eq!(js_inspect(&json!([])), "[]");
        assert_eq!(js_inspect(&json!({})), "{}");
        assert_eq!(js_inspect(&json!(["Code:3"])), "[ 'Code:3' ]");
        assert_eq!(js_inspect(&json!({"a": 1})), "{ a: 1 }");
        assert_eq!(
            js_inspect(&json!([{"function": "f", "line": 3}])),
            "[ { function: 'f', line: 3 } ]"
        );
        assert_eq!(
            js_inspect(&json!({"weird key": "x"})),
            "{ 'weird key': 'x' }"
        );
    }

    #[test]
    fn inspect_stays_single_line_for_long_arrays_pinned_divergence() {
        // Pinned divergence (parked audit item 11): Node's util.inspect wraps
        // long arrays at breakLength (~80 columns); crsp keeps the
        // single-line rendering regardless of length.
        let long: Vec<serde_json::Value> = (0..40)
            .map(|index| json!(format!("element-{index:02}")))
            .collect();
        let inspected = js_inspect(&json!(long));
        assert!(!inspected.contains('\n'), "must stay single-line");
        assert!(inspected.starts_with("[ 'element-00',"));
        assert!(inspected.ends_with("'element-39' ]"));
    }
}
