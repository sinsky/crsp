# crsp Forward Plan — parity lock and release readiness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining clasp→crsp gaps and ship a distributable 0.1.0→1.0 candidate.

**Architecture:** Freeze CLI/API behavior with the existing golden, surface, and opt-in live tests, then add user documentation and cross-platform release packaging. Avoid core refactors.

**Tech Stack:** Rust stable via mise; cargo-nextest; cargo fmt/clippy; GitHub Actions; cargo publish; tar/zip release archives.

**Spec:** `docs/superpowers/specs/2026-09-09-crsp-design.md`

## Global Constraints

- Preserve `.clasp.json`, `.clasprc.json`, `appsscript.json`, and `.claspignore` compatibility.
- Preserve clasp command names, aliases, defaults, English output, API semantics, and exit codes except intentional spec differences.
- Usage and unknown-command failures exit 1.
- Live Google tests remain opt-in behind `CRSP_LIVE_TEST=1`; normal tests never require credentials.
- Do not expose secrets in logs, fixtures, transcripts, or tests.
- Use `mise exec -- ...` for project commands.
- Release assets must contain only the executable and required license/readme files, with no `.d`, `.pdb`, or build intermediates.

---

## Evaluation (2026-09-11時点)

- **Baseline:** plan作成時点の HEAD は `f899084` で working tree は clean。pull の未使用ファイル削除修正は既にコミット済みなので、各タスクはこの clean な状態から開始する。
- **Tests:** 454 passed / 0 failed; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean.
- **Commands:** 29/29 canonical commands and aliases are defined in `src/cli.rs:104-271`; existing `tests/commands_surface.rs` already covers command outcomes and aliases.
- **Golden coverage:** 34 fixtures; live integration target is `tests/live/main.rs`, whose Cargo target name is `live`.
- **Platform gating:** `src` 側の `std::os::unix` 依存 (`src/auth/credential_store.rs`, `src/core/files.rs`, `src/commands/pull.rs`) はすべて `#[cfg(unix)]` 済みで Windows ビルドが通る。未ゲートなのは `tests/` の一部のみで、Task 3 Step 2 の対象はこれらに限られる。
- **Confirmed gaps:** README は最小限で、いくつかのコマンドと migration/config/run ドキュメントが欠落; README の `cargo install crsp` は crate 名 `google-clasp-rs` と不一致; CI は ubuntu のみ; release は crates.io のみ; versioning policy が未文書化; live 実行の検証記録が未作成。

### Task 1: Parity lock — deterministic surface checks plus a recorded live-verification artifact

**Files:**
- Modify: `tests/commands_surface.rs` (add the canonical-command registration test; mandatory, not conditional)
- Modify: `tests/live/live_smoke.rs` only if live assertions need correction
- Create: `docs/live-verification.md` (recorded live-run evidence)

**Interfaces:**
- Consumes: `tests/golden_runner.rs`, `tests/commands_surface.rs`, `tests/cli_surface.rs`, and `tests/live/main.rs`.
- Produces: deterministic local parity coverage, a mandatory 29-name command-registration assertion, plus a committed record of whether the opt-in live target ran.

- [ ] **Step 1: Confirm the existing live target and gate**

Run:

```bash
mise exec -- cargo test --test live -- --list
mise exec -- cargo nextest list -p google-clasp-rs 2>/dev/null | rg '^live::'
```

Expected: the target name is `live`, `tests/live/main.rs` is `mod live_smoke;`, and the listed tests are the three in `tests/live/live_smoke.rs`. Note `cargo test -- --list` lists ignored tests too (listing is not filtering), so pass `--ignored` only when actually running. Do not add a test that asserts `CRSP_LIVE_TEST` is set: that would make normal CI fail and duplicate `live_gate_open()`.

- [ ] **Step 2: Run the normal verification suite**

Run:

```bash
mise exec -- cargo nextest run
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
```

Expected: all normal tests pass without `CRSP_LIVE_TEST` or Google credentials.

- [ ] **Step 3: Run the opt-in live smoke target when credentials are available, then record it**

Run:

```bash
CRSP_LIVE_TEST=1 mise exec -- cargo test --test live -- --ignored
```

Then create `docs/live-verification.md` so the result is committed evidence rather than an unrecorded claim. Use exactly this structure:

```markdown
# Live verification

Opt-in live tests are gated by `CRSP_LIVE_TEST=1`; normal CI never sets it.

| Date | Target | Command | Result |
|------|--------|---------|--------|
| YYYY-MM-DD | `live` | `CRSP_LIVE_TEST=1 cargo test --test live -- --ignored` | PASS / NOT EXECUTED (reason) |

If not executed, record the reason (for example: credentials or `CRSP_TEST_SCRIPT_ID` unavailable). Never weaken the gate to make a run pass.
```

Expected: if credentials or `CRSP_TEST_SCRIPT_ID` are unavailable, the file records `NOT EXECUTED` with the reason; otherwise all configured live tests pass and the row records `PASS`.

- [ ] **Step 4: Add the mandatory canonical-command registration test**

Add this to `tests/commands_surface.rs` rather than creating a new target. Assert against the clap command tree via `CommandFactory` so required positional arguments (`update-deployment <deploymentId>`, `enable-api <api>`, `disable-api <api>`) do not cause parse failures. The list must contain exactly these 29 names:

```rust
use clap::CommandFactory;

use crate::Cli; // or google_clasp_rs::Cli for integration tests

const CANONICAL_COMMANDS: &[&str] = &[
    "login", "logout", "show-authorized-user", "clone-script", "create-script",
    "push", "pull", "create-deployment", "update-deployment", "delete-deployment",
    "delete-script", "create-version", "list-versions", "list-deployments",
    "list-scripts", "run-function", "tail-logs", "setup-logs", "show-file-status",
    "list-apis", "enable-api", "disable-api", "open-script", "open-container",
    "open-web-app", "open-logs", "open-api-console", "open-credentials-setup",
    "start-mcp-server",
];

#[test]
fn all_canonical_commands_are_registered() {
    let names: Vec<_> = Cli::command().get_subcommands().map(|s| s.get_name()).collect();
    assert_eq!(CANONICAL_COMMANDS.len(), 29);
    for command in CANONICAL_COMMANDS {
        assert!(names.contains(command), "missing subcommand `{command}`");
    }
}
```

The test must assert `CANONICAL_COMMANDS.len() == 29`, and aliases must be checked from the existing `ALIAS_PAIRS` instead of comparing help formatting.

Run the focused test to verify it compiles and passes:

```bash
mise exec -- cargo nextest run -E 'binary(commands_surface)'
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
```

Expected: PASS. If the nextest `binary(...)` filter does not match, fall back to `mise exec -- cargo test --test commands_surface`.

- [ ] **Step 5: Do not use raw help-output diff as a gate**

`../clasp/src/index.ts` is TypeScript and there is no `build/` directory in the checkout. If a manual comparison is useful, first run clasp through its documented npm/ts-node development command and compare normalized command names only. The automated gate is the exact Rust command list and existing behavior tests, not `diff ... || true`.

- [ ] **Step 6: Commit**

```bash
git add tests/commands_surface.rs docs/live-verification.md
git add tests/live/live_smoke.rs   # only if Step 3 required an assertion fix
git commit -m "test: lock command parity and record live verification"
```

### Task 2: User docs and CHANGELOG with explicit content

**Files:**
- Modify: `README.md`
- Create: `docs/config-files.md`
- Create: `docs/run.md`
- Create: `docs/migration-clasp-to-crsp.md`
- Create: `CHANGELOG.md`
- Create: `tests/doc_surface.rs`

**Interfaces:**
- Consumes: `src/cli.rs:104-271`, `docs/superpowers/specs/2026-09-09-crsp-design.md`, and the existing README.
- Produces: install, operation, configuration, migration, and release-history documentation.

- [ ] **Step 1: Add a strict documentation surface test**

Create `tests/doc_surface.rs` with exact command checks, not substring aliases:

```rust
const COMMANDS: &[&str] = &[
    "login", "logout", "show-authorized-user", "clone-script", "create-script",
    "push", "pull", "create-deployment", "update-deployment", "delete-deployment",
    "delete-script", "create-version", "list-versions", "list-deployments",
    "list-scripts", "run-function", "tail-logs", "setup-logs", "show-file-status",
    "list-apis", "enable-api", "disable-api", "open-script", "open-container",
    "open-web-app", "open-logs", "open-api-console", "open-credentials-setup",
    "start-mcp-server",
];

#[test]
fn readme_documents_all_canonical_commands() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    assert_eq!(COMMANDS.len(), 29);
    for command in COMMANDS {
        let exact = format!("`{command}`");
        let bullet = format!("- `{command}`");
        assert!(
            readme.lines().any(|line| {
                let trimmed = line.trim();
                trimmed == exact || trimmed.starts_with(&bullet)
            }),
            "missing `{command}`"
        );
    }
}

#[test]
fn readme_has_required_sections_in_order() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    let headings = [
        "## Install",
        "## Quick start",
        "## Commands",
        "## Global options",
        "## Compatibility",
        "## Versioning",
        "## Development",
        "## License",
    ];
    let mut cursor = 0;
    for heading in headings {
        let found = readme[cursor..]
            .find(heading)
            .unwrap_or_else(|| panic!("README is missing `{heading}`"));
        cursor += found + heading.len();
    }
}

#[test]
fn required_docs_have_required_sections() {
    let sections = [
        ("docs/config-files.md", &["## Overview", "## .clasp.json", "## .clasprc.json", "## appsscript.json", "## .claspignore", "## Project discovery", "## Security"] as &[&str]),
        ("docs/run.md", &["## Install", "## Login", "## Create or clone", "## Push and pull", "## Deployments", "## Logs and functions", "## API management", "## JSON output", "## MCP server", "## Troubleshooting"]),
        ("docs/migration-clasp-to-crsp.md", &["## Compatibility", "## Install", "## Command mapping", "## Configuration", "## Authentication", "## Output and exit codes", "## MCP", "## Known intentional differences"]),
        ("CHANGELOG.md", &["# Changelog", "## 0.1.0", "### Added"]),
    ];
    for (path, required) in sections {
        let body = std::fs::read_to_string(path).unwrap();
        for section in required {
            assert!(body.contains(section), "{path} is missing {section}");
        }
    }
}
```

- [ ] **Step 2: Run the focused test and verify it fails for the current README**

Run:

```bash
mise exec -- cargo test --test doc_surface
```

Expected: FAIL because the current README does not contain 29 bullet or bare backtick entries and lacks the required `##` headings, and the four docs/CHANGELOG files do not exist yet.

- [ ] **Step 3: Write `README.md` with these exact sections**

Replace the current README with the following content (the `##` headings must match the `readme_has_required_sections_in_order` test exactly, and every Commands bullet must keep the canonical name inside the first pair of backticks):

````markdown
# crsp

Rust port of [clasp](https://github.com/google/clasp) — develop Apps Script projects locally from the command line.

## Install

```sh
cargo install google-clasp-rs
```

Or install from a checkout:

```sh
cargo install --path .
```

## Quick start

```sh
crsp login
crsp clone-script --script-id <scriptId>
crsp push
crsp pull
```

## Commands

- `login` — Log in to script.google.com
- `logout` — Log out of the current account
- `show-authorized-user` — Show the current authorization state
- `clone-script` — Clone an existing script
- `create-script` — Create a new script
- `push` — Update the remote project
- `pull` — Fetch a remote project
- `create-deployment` — Deploy a project
- `update-deployment` `<deploymentId>` — Update a deployment to a new version
- `delete-deployment` — Delete a deployment of a project
- `delete-script` — Delete a project
- `create-version` — Create an immutable version of the script
- `list-versions` — List versions of a script
- `list-deployments` — List deployment ids of a script
- `list-scripts` — List Apps Script projects
- `run-function` — Run a function in your Apps Script project
- `tail-logs` — Print the most recent log entries
- `setup-logs` — Set up Cloud Logging
- `show-file-status` — List files that will be pushed by clasp
- `list-apis` — List enabled APIs for the current project
- `enable-api` `<api>` — Enable a service for the current project
- `disable-api` `<api>` — Disable a service for the current project
- `open-script` — Open the Apps Script IDE
- `open-container` — Open the container-bound Apps Script IDE
- `open-web-app` — Open a deployed web app in the browser
- `open-logs` — Open logs in the developer console
- `open-api-console` — Open the API console for the current project
- `open-credentials-setup` — Open the credentials page for the script's GCP project
- `start-mcp-server` — Start an MCP server for Apps Script

## Global options

- `-P, --project <path>` — Use a specific project directory instead of discovery
- `-A, --auth <user>` — Use a named OAuth credential
- `-I, --ignore <glob>` — Add an ignore pattern
- `--json` — Emit machine-readable JSON where supported
- `--adc` — Use Application Default Credentials
- `--user <user>` — Select the credential user

## Compatibility

The local `clasp` files (`.clasp.json`, `.clasprc.json`, `appsscript.json`, `.claspignore`) are compatible with `clasp`. The only intentional display change is the MCP server display name: `Crsp` instead of `Clasp`.

## Versioning

The crate follows Semantic Versioning. Pre-1.0 releases may include breaking changes; from 1.0, the CLI contract (command names, aliases, defaults, and exit codes) and local file formats are treated as stable.

## Development

```sh
mise run fmt:check
mise run lint
mise run test
```

Requires Rust stable (see `mise.toml`).

## License

Apache-2.0 — see [LICENSE](LICENSE).
This project is a Rust port of [google/clasp](https://github.com/google/clasp) (Apache-2.0).
````

- [ ] **Step 4: Write `docs/config-files.md` with these exact sections and facts**

Use level-2 headings (`## Overview`, `## .clasp.json`, `## .clasprc.json`, `## appsscript.json`, `## .claspignore`, `## Project discovery`, `## Security`) because `tests/doc_surface.rs` asserts them verbatim. Document JSON5 input and two-space JSON output for `.clasp.json`, `srcDir`/`rootDir` defaults and `rootDir` recomputation, OAuth named users and `--auth`, manifest behavior, ignore complement semantics, `-P`/`-I`, path-jail and symlink rules, and secret-handling guidance.

- [ ] **Step 5: Write `docs/run.md` with these exact sections and examples**

Use level-2 headings (`## Install`, `## Login`, `## Create or clone`, `## Push and pull`, `## Deployments`, `## Logs and functions`, `## API management`, `## JSON output`, `## MCP server`, `## Troubleshooting`) because `tests/doc_surface.rs` asserts them verbatim. Include runnable examples for `login`, `create-script`, `clone-script`, `push`, `pull`, `create-deployment`, `run-function`, `tail-logs`, `list-apis`, `start-mcp-server`, `--json`, `--project`, and `--user`.

- [ ] **Step 6: Write `docs/migration-clasp-to-crsp.md` with these exact sections and mapping**

Use level-2 headings (`## Compatibility`, `## Install`, `## Command mapping`, `## Configuration`, `## Authentication`, `## Output and exit codes`, `## MCP`, `## Known intentional differences`) because `tests/doc_surface.rs` asserts them verbatim. State that command aliases and local file formats remain compatible, the binary changes from `clasp` to `crsp`, MCP display name changes from `Clasp` to `Crsp`, and intentional differences must be copied from spec §5 rather than guessed.

- [ ] **Step 7: Write `CHANGELOG.md`**

Create the file with exactly this content. Do not claim live verification here; that evidence lives in `docs/live-verification.md` (Task 1 Step 3).

```markdown
# Changelog

All notable changes to `crsp` are documented in this file.

## 0.1.0

### Added

- Rust CLI (`crsp`) porting the `clasp` command surface, with 29 canonical commands and their `clasp` aliases.
- clasp-compatible local files: `.clasp.json`, `.clasprc.json`, `appsscript.json`, and `.claspignore`.
- OAuth and Google Apps Script / Drive API clients with named-user credential support.
- Golden, command-surface, and opt-in live test suites.
- `start-mcp-server` for MCP clients, with display name `Crsp`.
```

- [ ] **Step 8: Run tests**

Run:

```bash
mise exec -- cargo nextest run -E 'binary(doc_surface)'
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
```

Expected: PASS. Fall back to `mise exec -- cargo test --test doc_surface` if the nextest `binary(...)` filter does not match.

- [ ] **Step 9: Commit**

```bash
git add README.md docs/config-files.md docs/run.md docs/migration-clasp-to-crsp.md CHANGELOG.md tests/doc_surface.rs
git commit -m "docs: add user and migration documentation"
```

### Task 3: Separate CI checks from cross-platform tests and publish release assets

**Files:**
- Modify: `.github/workflows/ci.yaml`
- Modify: `.github/workflows/release.yaml`
- Modify: `tests/commands_lifecycle.rs` (POSIX-only test gating)
- Modify: `tests/auth_credentials.rs` (POSIX-only test gating)
- Modify: `tests/files_pipeline.rs` (POSIX-only test gating)
- Modify: `Cargo.toml` only if package metadata or version policy is changed

**Interfaces:**
- Consumes: Cargo package `google-clasp-rs`, binary target `crsp`, and the existing release trigger.
- Produces: one lint job, a Windows/macOS/Linux test matrix, and GitHub Release assets plus crates.io publication.

- [ ] **Step 1: Add a static workflow validation check before editing**

Run:

```bash
! grep -q 'windows-latest' .github/workflows/ci.yaml
! grep -q 'softprops/action-gh-release' .github/workflows/release.yaml
```

Expected: PASS for the current known gaps. This is a precondition check, not a committed test.

- [ ] **Step 2: Gate POSIX-only tests before enabling Windows CI**

Discover the call sites dynamically instead of trusting stale line numbers (the source changes as pull work lands):

```bash
rg -n 'std::os::unix' tests/
```

At the time of writing this matches only tests: `tests/commands_lifecycle.rs`, `tests/auth_credentials.rs`, and `tests/files_pipeline.rs`. The `src/` matches (`src/auth/credential_store.rs`, `src/core/files.rs`, `src/commands/pull.rs`) are already behind `#[cfg(unix)]`; do not re-gate them. For each test function that uses `std::os::unix`, add `#[cfg(unix)]` to the function, and move any function-local `use std::os::unix::...` inside the gated function body. Do not add Windows symlink equivalents in this plan; non-Unix runners simply skip these tests.

Verify every remaining test hit is inside a gated function. Because Windows cannot be built locally, inspect the surrounding context instead:

```bash
rg -n -B3 'std::os::unix' tests/
```

Expected: each match is preceded within a few lines by `#[cfg(unix)]` (either on the `fn` or on the enclosing `mod`/`impl`). If a hit is not, add the gate.

- [ ] **Step 3: Split CI responsibilities**

Make `ci.yaml` contain:

```yaml
name: ci

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test
      - run: cargo build --release
```

CI intentionally uses `cargo test` (which also runs doctests) rather than nextest; locally the repo standard is `mise run test` (`cargo nextest run`) plus `mise run test:doc`. Keep the job names `lint` and `test` exactly as above.

- [ ] **Step 4: Build deterministic release archives**

Add top-level write permission so the release job can upload assets under the default read-only token:

```yaml
name: release

on:
  release:
    types: [created]

permissions:
  contents: write
```

Use an explicit include matrix pairing runner, target triple, and archive format. `macos-latest` runners are Apple Silicon, so default to `aarch64-apple-darwin`:

```yaml
jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            archive: tar.gz
          - os: macos-latest
            target: aarch64-apple-darwin
            archive: tar.gz
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            archive: zip
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - name: Stage dist directory
        shell: bash
        run: |
          mkdir -p dist/crsp-${{ matrix.target }}
          if [ "${{ matrix.os }}" = "windows-latest" ]; then
            cp target/release/crsp.exe dist/crsp-${{ matrix.target }}/
          else
            cp target/release/crsp dist/crsp-${{ matrix.target }}/
          fi
          cp LICENSE README.md dist/crsp-${{ matrix.target }}/
      - name: Create Unix archive
        if: matrix.archive == 'tar.gz'
        run: tar -czf dist/crsp-${{ matrix.target }}.tar.gz -C dist crsp-${{ matrix.target }}
      - name: Create Windows archive
        if: matrix.archive == 'zip'
        shell: pwsh
        run: Compress-Archive -Path dist/crsp-${{ matrix.target }} -DestinationPath dist/crsp-${{ matrix.target }}.zip
      - uses: softprops/action-gh-release@v2
        with:
          files: dist/crsp-${{ matrix.target }}.${{ matrix.archive }}
```

Each `dist/crsp-<target>` directory must contain only the executable (`crsp` or `crsp.exe`), `LICENSE`, and `README.md`; do not glob `target/release/crsp*`.

- [ ] **Step 5: Publish crates.io in a separate job**

Keep `cargo publish` in a `publish` job that `needs: build` and uses `CARGO_REGISTRY_TOKEN`, so it runs only after the archive builds succeed. Do not use `actions/upload-artifact` as a substitute for Release assets.

- [ ] **Step 6: Validate workflow structure and local commands**

Run:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo nextest run
mise exec -- cargo test --doc
```

Then gate the workflow text mechanically with `rg` (a missing match is a failure):

```bash
# ci.yaml must define both jobs and all three OS labels
rg -q '^  lint:' .github/workflows/ci.yaml
rg -q '^  test:' .github/workflows/ci.yaml
rg -q 'ubuntu-latest' .github/workflows/ci.yaml
rg -q 'windows-latest' .github/workflows/ci.yaml
rg -q 'macos-latest' .github/workflows/ci.yaml

# release.yaml must write assets and upload them
rg -q 'contents: write' .github/workflows/release.yaml
rg -q 'softprops/action-gh-release@v2' .github/workflows/release.yaml
rg -q 'tar -czf' .github/workflows/release.yaml
rg -q 'Compress-Archive' .github/workflows/release.yaml
rg -q 'cargo publish' .github/workflows/release.yaml
rg -q 'needs: build' .github/workflows/release.yaml
```

Expected: all `rg -q` commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yaml .github/workflows/release.yaml
git add Cargo.toml   # only if package metadata or the version policy changed
git add tests/commands_lifecycle.rs tests/auth_credentials.rs tests/files_pipeline.rs
git commit -m "ci: add cross-platform checks and release assets"
```
