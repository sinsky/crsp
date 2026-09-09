//! stdout/stderr output boundary (spec §2.1: artifacts to stdout, warnings to
//! stderr) with clasp-compatible two-space pretty JSON for `--json`.

use std::io::{self, Write};

use serde::Serialize;

/// Command-level output helper: JSON mode plus normal/warning channels.
pub struct Output<W: Write = io::Stdout, E: Write = io::Stderr> {
    json: bool,
    out: W,
    err: E,
}

impl Output<io::Stdout, io::Stderr> {
    /// Output bound to the process's real stdout/stderr.
    pub fn stdout(json: bool) -> Self {
        Self {
            json,
            out: io::stdout(),
            err: io::stderr(),
        }
    }
}

impl<W: Write, E: Write> Output<W, E> {
    /// Output over explicit writers (tests).
    pub fn new(json: bool, out: W, err: E) -> Self {
        Self { json, out, err }
    }

    /// Whether `--json` was requested for the command.
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// Writes a normal result line to stdout.
    pub fn message(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
    }

    /// Writes a warning line to stderr.
    pub fn warn(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// Writes a two-space pretty JSON document to stdout (clasp's `--json`
    /// convention, spec §6.2).
    pub fn print_json<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        self.write_json(value, serde_json::to_string_pretty)
    }

    /// Writes a single-line JSON document to stdout.
    pub fn print_json_compact<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        self.write_json(value, serde_json::to_string)
    }

    fn write_json<T: Serialize, F>(&mut self, value: &T, render: F) -> io::Result<()>
    where
        F: FnOnce(&T) -> serde_json::Result<String>,
    {
        let text = render(value).map_err(|error| io::Error::other(error.to_string()))?;
        writeln!(self.out, "{text}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Output;

    #[test]
    fn message_writes_stdout_and_warn_writes_stderr() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut output = Output::new(false, &mut out, &mut err);
        output.message("Pushed 3 files.");
        output.warn("Skipped symlink: a.js");
        assert_eq!(String::from_utf8(out).unwrap(), "Pushed 3 files.\n");
        assert_eq!(String::from_utf8(err).unwrap(), "Skipped symlink: a.js\n");
    }

    #[test]
    fn print_json_renders_two_space_pretty_documents() {
        let mut out = Vec::new();
        let mut output = Output::new(true, &mut out, Vec::new());
        output
            .print_json(&json!({ "deploymentId": "dep1", "versionNumber": 3 }))
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\n  \"deploymentId\": \"dep1\",\n  \"versionNumber\": 3\n}\n"
        );
    }

    #[test]
    fn print_json_compact_renders_single_line_documents() {
        let mut out = Vec::new();
        let mut output = Output::new(false, &mut out, Vec::new());
        output
            .print_json_compact(&json!({ "success": true }))
            .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "{\"success\":true}\n");
    }

    #[test]
    fn is_json_reflects_the_constructor_flag() {
        let mut out = Vec::new();
        assert!(!Output::new(false, &mut out, Vec::new()).is_json());
        assert!(Output::new(true, &mut out, Vec::new()).is_json());
    }
}
