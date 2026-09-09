//! `tail-logs` (alias `logs`) — clasp commands/tail-logs.ts.
//!
//! Fetches Cloud Logging entries (clasp's exact request body), prints them
//! oldest-first with the padded `severity time function payload` columns,
//! deduplicates by `insertId` across polls, and updates the `since` filter
//! from each entry's timestamp. The watch loop polls every 6000ms (clasp
//! `POLL_INTERVAL`); the interval and sleeper are injectable so the loop is
//! testable (the full watch wiring with signal handling lands with CLI
//! dispatch). clasp's `console.log('PAST', projectId)` debug line is removed
//! (spec §5 #6).

use std::collections::HashSet;
use std::io::Write;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::Value;
use time::OffsetDateTime;
use time::UtcOffset;
use time::format_description::well_known::Rfc3339;

use crate::api::client::SleepFn;
use crate::api::{ApiClient, LogEntry};
use crate::commands::shared::{
    UrlOpener, assert_gcp_project_configured, maybe_prompt_for_project_id, pad_end,
};
use crate::core::config::ProjectConfig;
use crate::core::project::assert_script_configured;
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// Watch poll interval (clasp tail-logs.ts:88: `POLL_INTERVAL = 6000`).
pub const POLL_INTERVAL_MS: u64 = 6000;

/// Arguments for [`tail_logs`] (clasp `tail-logs --watch --simplified`).
/// `poll_interval` is clasp's 6000ms by default and injectable for tests.
#[derive(Debug, Clone, Copy)]
pub struct TailLogsArgs {
    pub watch: bool,
    pub simplified: bool,
    pub poll_interval: Duration,
}

impl Default for TailLogsArgs {
    fn default() -> Self {
        Self {
            watch: false,
            simplified: false,
            poll_interval: Duration::from_millis(POLL_INTERVAL_MS),
        }
    }
}

/// Per-command tail state (clasp `seenEntries` + `since`).
#[derive(Debug, Default)]
pub struct PollState {
    /// Entry ids already printed (clasp `seenEntries`).
    pub seen: HashSet<String>,
    /// The `timestamp >= "…"` filter origin, as an ISO string with
    /// millisecond precision (clasp `since.toISOString()`).
    pub since: Option<String>,
}

/// The process-local UTC offset (clasp `new Date(...)` local getters).
/// Falls back to UTC when the platform cannot determine the offset.
pub fn local_utc_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

fn parse_timestamp(timestamp: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

/// clasp `getLocalISODateTime(new Date(timestamp))`: the local-time
/// `YYYY-MM-DDTHH:MM:SS` rendering of an RFC 3339 timestamp.
pub fn format_local_time(timestamp: &str, offset: UtcOffset) -> Option<String> {
    let dt = parse_timestamp(timestamp)?.to_offset(offset);
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    ))
}

/// clasp `since.toISOString()`: UTC with exactly three fractional digits.
fn to_iso_utc_millis(timestamp: &str) -> Option<String> {
    let dt = parse_timestamp(timestamp)?.to_offset(UtcOffset::UTC);
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        dt.nanosecond() / 1_000_000
    ))
}

/// clasp `formatEntry` (tail-logs.ts:108-150). Returns `None` when the entry
/// lacks a resource or timestamp (or, outside `--json`, a payload).
/// `--json` embeds the whole entry as `JSON.stringify(entry, null, 2)`
/// (multiline; the formatted line keeps the quirk of prefixing only the
/// first line, spec §6.2).
pub fn format_entry(
    entry: &LogEntry,
    json: bool,
    simplified: bool,
    offset: UtcOffset,
) -> Option<String> {
    let severity = entry.severity().unwrap_or("");
    let timestamp = entry.timestamp().unwrap_or_default();
    if entry.raw().get("resource").is_none() || timestamp.is_empty() {
        return None;
    }

    let function_name = pad_end(entry.function_name_label().unwrap_or("N/A"), 15);
    let payload_data = if json {
        let text = serde_json::to_string_pretty(entry.raw())
            .map_err(|error| CrspError::Validation(error.to_string()))
            .ok()?;
        pad_end(&text, 20)
    } else if let Some(text) = entry.text_payload() {
        pad_end(text, 20)
    } else if let Some(Value::String(message)) = entry
        .json_payload()
        .and_then(|payload| payload.get("message"))
        .filter(|message| !message.as_str().unwrap_or_default().is_empty())
    {
        pad_end(message, 20)
    } else {
        // A null jsonPayload is falsy (clasp skips the entry); any other
        // payload is compact-stringified.
        let payload = entry.json_payload().filter(|payload| !payload.is_null())?;
        let text = serde_json::to_string(payload)
            .map_err(|error| CrspError::Validation(error.to_string()))
            .unwrap_or_default();
        pad_end(&text, 20)
    };

    let localized_time = format_local_time(timestamp, offset).unwrap_or_default();
    if simplified {
        return Some(format!("{severity:<20} {function_name} {payload_data}"));
    }
    Some(format!(
        "{severity:<20} {localized_time} {function_name} {payload_data}"
    ))
}

/// The per-poll collaborators (client, project, print mode, and state) so
/// the poll/watch entry points stay small.
pub struct LogPoller<'a, W: Write, E: Write> {
    client: &'a ApiClient,
    project_id: &'a str,
    simplified: bool,
    json: bool,
    state: &'a mut PollState,
    output: &'a mut Output<W, E>,
}

impl<'a, W: Write, E: Write> LogPoller<'a, W, E> {
    pub fn new(
        client: &'a ApiClient,
        project_id: &'a str,
        simplified: bool,
        json: bool,
        state: &'a mut PollState,
        output: &'a mut Output<W, E>,
    ) -> Self {
        Self {
            client,
            project_id,
            simplified,
            json,
            state,
            output,
        }
    }

    /// One fetch-and-print cycle (clasp `fetchAndPrintLogs`): the `since`
    /// filter from the previous cycle, reverse iteration (oldest first),
    /// per-entry `since` update, and insertId dedupe.
    pub async fn poll_once(&mut self) -> Result<(), CrspError> {
        let filter = self
            .state
            .since
            .as_deref()
            .map(|since| format!("timestamp >= \"{since}\""))
            .unwrap_or_default();
        let entries = self
            .client
            .logging()
            .list_entries(self.project_id, &filter)
            .await?
            .results;
        let offset = local_utc_offset();
        // clasp: `entries.results.reverse().forEach(...)`.
        for entry in entries.iter().rev() {
            if let Some(iso) = entry
                .timestamp()
                .filter(|timestamp| !timestamp.is_empty())
                .and_then(to_iso_utc_millis)
            {
                self.state.since = Some(iso);
            }
            let Some(id) = entry.insert_id() else {
                continue;
            };
            if self.state.seen.contains(id) {
                continue;
            }
            self.state.seen.insert(id.to_string());
            if let Some(line) = format_entry(entry, self.json, self.simplified, offset) {
                self.output.message(&line);
            }
        }
        Ok(())
    }

    /// The watch loop (clasp `setInterval(fetchAndPrintLogs,
    /// POLL_INTERVAL)`): sleeps `interval` before every poll. `sleep` and the
    /// `stop` predicate are injectable for tests; production passes a real
    /// sleeper and `|| false` (the full watch wiring with signal handling
    /// lands with CLI dispatch).
    pub async fn watch(
        &mut self,
        interval: Duration,
        sleep: &SleepFn,
        stop: impl Fn() -> bool,
    ) -> Result<(), CrspError> {
        while !stop() {
            (sleep)(interval).await;
            self.poll_once().await?;
        }
        Ok(())
    }
}

pub async fn tail_logs<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &mut ProjectConfig,
    args: TailLogsArgs,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    maybe_prompt_for_project_id(config, ui, opener, output).await?;
    assert_gcp_project_configured(config)?;
    // clasp core `getLogEntries` asserts the script configuration too.
    assert_script_configured(config).await?;
    let project_id = config.project_id.clone().unwrap_or_default();

    let mut state = PollState::default();
    let mut poller = LogPoller::new(
        client,
        &project_id,
        args.simplified,
        output.is_json(),
        &mut state,
        output,
    );
    poller.poll_once().await?;

    if args.watch {
        let sleep: SleepFn = std::sync::Arc::new(|duration| {
            Box::pin(tokio::time::sleep(duration)) as BoxFuture<'static, ()>
        });
        poller.watch(args.poll_interval, &sleep, || false).await?;
    }
    Ok(())
}
