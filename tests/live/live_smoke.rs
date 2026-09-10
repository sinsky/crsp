//! Opt-in live smoke tests against the real Google Apps Script API
//! (spec §9.3).
//!
//! These tests are SKIPPED BY DEFAULT. Two independent gates must both open
//! before any network byte leaves the machine:
//!
//! 1. The `#[ignore]` attribute keeps them out of the normal suite; they run
//!    only under an explicit opt-in (`cargo test --test live -- --ignored`).
//! 2. Inside each test, `live_gate_open()` requires `CRSP_LIVE_TEST=1`, a
//!    `CRSP_TEST_SCRIPT_ID`, and a real `.clasprc.json` access token in
//!    `$HOME`. Without those, the test returns immediately (no network).
//!
//! CI stays hermetic: the default `cargo test` run reports every test in this
//! binary as ignored, matching the Task 12 "skipped by default" gate.

use assert_cmd::Command;

/// True only when the full live gate is open: the `CRSP_LIVE_TEST=1`
/// opt-in flag, a non-empty `CRSP_TEST_SCRIPT_ID`, and real credentials
/// (an `access_token` inside `$HOME/.clasprc.json`).
fn live_gate_open() -> bool {
    if std::env::var("CRSP_LIVE_TEST").as_deref() != Ok("1") {
        return false;
    }
    if std::env::var("CRSP_TEST_SCRIPT_ID").ok().is_none() {
        return false;
    }
    let Some(home) = home::home_dir() else {
        return false;
    };
    let clasprc = std::fs::read_to_string(home.join(".clasprc.json")).unwrap_or_default();
    clasprc.contains("\"access_token\"")
}

fn crsp_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crsp"))
}

#[test]
#[ignore]
fn live_show_authorized_user_with_real_credentials() {
    if !live_gate_open() {
        return;
    }
    let output = crsp_bin()
        .arg("show-authorized-user")
        .output()
        .expect("live crsp run");
    assert!(
        output.status.success(),
        "live show-authorized-user failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore]
fn live_deployments_of_the_configured_script() {
    if !live_gate_open() {
        return;
    }
    let script = std::env::var("CRSP_TEST_SCRIPT_ID").expect("live script id");
    let output = crsp_bin()
        .arg("deployments")
        .arg(&script)
        .output()
        .expect("live crsp run");
    assert!(
        output.status.success(),
        "live deployments check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "live deployments output must not be empty"
    );
}

#[test]
#[ignore]
fn live_versions_of_the_configured_script() {
    if !live_gate_open() {
        return;
    }
    let script = std::env::var("CRSP_TEST_SCRIPT_ID").expect("live script id");
    let output = crsp_bin()
        .arg("versions")
        .arg(&script)
        .output()
        .expect("live crsp run");
    assert!(
        output.status.success(),
        "live versions check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "live versions output must not be empty"
    );
}
