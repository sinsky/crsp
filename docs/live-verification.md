# Live verification

Opt-in live tests are gated by `CRSP_LIVE_TEST=1`; normal CI never sets it.

| Date | Target | Command | Result |
|------|--------|---------|--------|
| 2026-09-11 | `live` | `CRSP_LIVE_TEST=1 cargo test --test live -- --ignored` | NOT EXECUTED (`CRSP_TEST_SCRIPT_ID` unset) |

If not executed, record the reason (for example: credentials or `CRSP_TEST_SCRIPT_ID` unavailable). Never weaken the gate to make a run pass.
