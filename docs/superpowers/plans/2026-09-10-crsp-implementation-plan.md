# crsp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a Rust single-crate `crsp` CLI that is drop-in compatible with clasp v3.4.1 across its 29 commands, local files, OAuth credentials, API semantics, output, exit codes, security behavior, and the specified intentional differences.

**Architecture:** Build a Tokio-based binary with a thin typed HTTP API layer, a UI-independent core domain layer, command modules driven by clap, and a separate rmcp stdio server. Establish configuration, credential, path-jail, file-pipeline, error, output, and test-fixture contracts before adding command breadth. Keep every external API behind injectable clients so unit, wiremock, CLI, and MCP tests do not require live Google credentials.

**Tech Stack:** Rust stable via mise; clap 4.5 derive; Tokio with targeted features (`rt-multi-thread`, `macros`, `net`, `time`, `io-util`, `sync`, `fs`, `signal`); futures; reqwest 0.12 with rustls (`default-features = false`, `rustls-tls-native-roots`, `json`, `multipart`, `stream`); serde/serde_json with local `indexmap` only where key order is required; json5; globset; walkdir; notify; `demand` for all interactive prompts and TTY spinners; open; home; rand/sha2/base64; url/regex; time; tracing; rmcp; thiserror; wiremock/assert_cmd/predicates/tempfile.

**Spec:** `docs/superpowers/specs/2026-09-09-crsp-design.md`

## Global Constraints

- Implement all 29 canonical commands and the aliases in spec §2.5; `start-mcp-server`/`mcp` is included.
- Preserve `.clasp.json`, `.clasprc.json`, `appsscript.json`, and `.claspignore` compatibility exactly as specified; read `.clasp.json` as JSON5 and write plain 2-space JSON.
- Preserve clasp CLI names, aliases, defaults, English output, API request semantics, and exit code 0/1 except the ten intentional differences in spec §5.
- Usage and unknown-command failures must exit 1, not clap’s default exit 2.
- Use Tokio async execution; API requests use 30s request and 10s connect timeouts. Transient retries apply only to GET/HEAD/PUT/OPTIONS/DELETE on statuses 1xx/408/429/5xx, with max 3 retries; network errors with no response use max 2 retries, 100ms exponential backoff, and no `Retry-After` handling. POST is excluded from this layer. A separate 401 token-refresh layer retries the failed request once for every HTTP method, including POST.
- Preserve direct-write/O_NOFOLLOW behavior, path jails, symlink handling, skip reasons, and security regression coverage from spec §§2.6, 8, and 9.4.
- Keep live Google tests opt-in behind `CRSP_LIVE_TEST=1`; normal tests use fixtures and wiremock.
- Do not expose secrets in logs, fixtures, transcripts, or tests; replace authorization and token values with placeholders.
- Use `mise` for Rust/toolchain setup and project commands; inspect project-local mise configuration before installing or selecting versions.
- Use `demand` as the sole interactive UI dependency: replace `dialoguer` and `indicatif` rather than adding them alongside demand.
- Implement the one-shot OAuth localhost callback with `tokio::net::TcpListener`; do not add `tiny_http` or use `spawn_blocking` for it.
- Do not add `inflector`; use an internal title-humanization helper matching clasp’s `inflection.humanize` behavior: remove trailing `_id`/`_ids`, replace underscores with spaces, lowercase the full string, uppercase only the first character, and preserve hyphens.
- Use `thiserror` exclusively for `CrspError`; do not add `anyhow`.
- Configure Tokio with only `rt-multi-thread`, `macros`, `net`, `time`, `io-util`, `sync`, `fs`, and `signal` features.
- Interactive prompts and spinners must be TTY-aware; in noninteractive mode preserve clasp-compatible behavior (manifest confirmation rejects, pull deletion skips with warning, delete is a silent no-op).

---

### Task 1: Bootstrap the crate, error model, output, and CLI shell

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `src/cli.rs`
- Create: `src/constants.rs`
- Create: `src/i18n.rs`
- Create: `src/error.rs`
- Create: `src/output.rs`
- Create: `src/ui.rs`
- Create: `src/text.rs`
- Create: `tests/cli_surface.rs`
- Create: `tests/ui.rs`
- Create: `mise.toml` if no repository/project configuration provides Rust
- Modify: `Cargo.toml` release profile and explicit dependency feature declarations

**Interfaces:**
- `Cli` owns global options: auth path, user, ADC, JSON, allow-symlinks, ignore path, project path, and version.
- `Commands` contains all 29 canonical variants with exact aliases, positional arguments, and option names from spec §2.5; handlers can initially return a typed “not implemented” error only for commands not yet wired.
- `CrspError` contains `Api`, `Config`, `Auth`, `Io`, `Validation`, and `Aborted` categories and renders one English message to stderr.
- `Output` writes normal results to stdout, warnings/spinners to stderr, and supports compact/pretty JSON according to command-level callers.
- `Ui` wraps `demand` for `Input`, `Select`, `MultiSelect`, `Confirm`, `Dialog`, and `Spinner`; it exposes TTY detection and deterministic noninteractive fallbacks without leaking prompt logic into command modules.
- `humanize_title(name: &str) -> String` matches clasp’s `inflection.humanize`: remove a trailing `_id`/`_ids`, replace underscores with spaces, lowercase the full string, then uppercase only the first character; hyphens remain unchanged (`my-project-folder` → `My-project-folder`, `My_App` → `My app`, `blog_id` → `Blog`).
- `run(cli)` returns `Result<(), CrspError>` and `main` maps every failure, including clap usage/unknown-command failures, to exit code 1.

- [ ] **Step 1: Write failing CLI contract tests** for canonical command parsing, aliases (`clone`, `create`, `deploy`, `mcp`, etc.), global options, and exit code 1 on malformed/unknown input.
- [ ] **Step 2: Run the focused tests** with `mise exec -- cargo test --test cli_surface --test ui`; expect failures because the crate and parser do not exist.
- [ ] **Step 3: Add the crate manifest and minimal module tree** with pinned major/minor constraints from spec §3.1 and a binary target named `crsp`; configure `[profile.release]` with `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true`, use targeted Tokio features including `signal`, and configure reqwest with `default-features = false`, `rustls-tls-native-roots`, `json`, `multipart`, and `stream`.
- [ ] **Step 4: Implement the clap surface** with long canonical names, aliases, exact short flags, default values, and `--version` reporting the crate version; keep parsing synchronous so `--version`, `--help`, and usage errors do not initialize Tokio.
- [ ] **Step 5: Implement the shared English message constants in `i18n.rs`, typed errors with `thiserror` only, and the stdout/stderr output boundary**, then add `Ui` as the only prompt/spinner boundary using `demand`.
- [ ] **Step 6: Add `demand` UI and title-humanization tests** for TTY/non-TTY selection, confirmation, input, multi-select, dialog, spinner lifecycle, noninteractive fallback values, and clasp-compatible title conversion (`my-project-folder` → `My-project-folder`, `My_App` → `My app`, `blog_id` → `Blog`); use injected test adapters where terminal interaction cannot be deterministic.
- [ ] **Step 7: Override clap error handling** so parse and usage errors render compatible English diagnostics and exit 1.
- [ ] **Step 8: Run focused tests and format/lint** with `mise exec -- cargo test --test cli_surface --test ui`, `mise exec -- cargo fmt --check`, and `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] **Step 9: Commit** with message `feat: bootstrap crsp cli shell`.

### Task 2: Implement configuration, manifest, ignore matching, and path-jail primitives

**Files:**
- Create: `src/core/mod.rs`
- Create: `src/core/config.rs`
- Create: `src/core/manifest.rs`
- Create: `src/core/ignore.rs`
- Create: `src/core/path.rs`
- Create: `src/core/pagination.rs`
- Create: `tests/core_config.rs`
- Create: `tests/core_security.rs`

**Interfaces:**
- `ProjectConfig::discover(project_arg, cwd) -> Result<ProjectConfig, CrspError>` finds `.clasp.json` by explicit path or find-up.
- `ProjectConfig::load(path) -> Result<Self, CrspError>` parses JSON5 and applies the exact defaults and empty-string behavior from spec §§2.3 and 4.
- `PathJail::is_inside(base, candidate)` uses component-aware `strip_prefix`/component checks, never raw string-prefix checks; concurrent parent creation verifies `EEXIST` paths are directories and their resolved realpaths remain inside contentDir.
- `ProjectConfig::update_settings(...) -> Result<(), CrspError>` writes 2-space JSON, recomputes `rootDir`, resets `filePushOrder` to `[]`, and omits undefined `parentId`/`projectId`.
- `IgnoreMatcher::from_file(path) -> Result<Self, CrspError>` evaluates micromatch-compatible patterns as match-set then complement, with dot files and slash normalization enabled.
- `PathJail` exposes `is_inside(base, candidate)`, `resolve_content_dir`, and remote-name validation for both project and MCP jails.
- `fetch_pages` exposes page size/max page/max result controls and returns `{ results, partial_results }` without swallowing mid-page HTTP errors.

- [ ] **Step 1: Write unit tests** for JSON5 comments/single quotes, default fields, empty `srcDir`/`rootDir`, rootDir recomputation, omitted undefined keys, ignore complement semantics, `.ts` tracking, component-safe path containment (`/app_fake` must not be inside `/app`), Windows slash normalization before ignore/change detection/PUT payloads, content-dir escape rejection, and MCP `projectDir`/`sourceDir` jail rules.
- [ ] **Step 2: Run the focused tests** with `mise exec -- cargo test --test core_config --test core_security`; expect failures.
- [ ] **Step 3: Implement config parsing/serialization** using local ordered maps only for `.clasp.json` fields whose key order must be preserved, with explicit optional-field serialization; keep API response models strongly typed.
- [ ] **Step 4: Implement ignore matching** and normalize all evaluated paths to `/` before glob evaluation.
- [ ] **Step 5: Implement canonical path resolution and jail checks** against real paths, including project-root and content-dir containment; use component-aware `strip_prefix`/`Path::components` checks rather than string prefixes.
- [ ] **Step 6: Implement generic pagination** with the standard 100/10 limits and Service Usage override parameters.
- [ ] **Step 7: Run tests, formatting, and clippy** and fix all failures before proceeding.
- [ ] **Step 8: Commit** with message `feat: add project config and path safety primitives`.

### Task 3: Implement credential storage and OAuth flows

**Files:**
- Create: `src/auth/mod.rs`
- Create: `src/auth/credential_store.rs`
- Create: `src/auth/oauth_client.rs`
- Create: `src/auth/flow.rs`
- Create: `src/auth/localhost_flow.rs`
- Create: `src/auth/serverless_flow.rs`
- Create: `tests/auth_credentials.rs`
- Create: `tests/auth_flow.rs`
- Modify: `src/auth/localhost_flow.rs` to cover callback probes, favicon requests, cancellation, and timeout

**Interfaces:**
- `CredentialStore::load(path, user) -> Result<Option<Credentials>, CrspError>` reads V3 and default-user-only V1 formats, including the `exprity_date` typo.
- `CredentialStore::save(path, credentials) -> Result<(), CrspError>` performs direct writes; under `#[cfg(unix)]` it enforces mode 0600 and O_NOFOLLOW and propagates chmod failures, while under `#[cfg(windows)]` permission changes are best-effort and unsupported chmod/O_NOFOLLOW behavior does not abort the operation.
- `OAuthClient` refreshes access tokens once after 401/`invalid_grant`, saves successful refreshes, and preserves the old token on failure.
- `AuthOptions` models ADC, default client, user-provided credentials, scopes, PKCE, redirect port, and localhost/serverless choices.
- `login`, `logout`, and `show_authorized_user` service functions provide the command layer with typed results and JSON payloads.

- [ ] **Step 1: Write tests** for V3 round trips, V1 local/global fallback only for `default`, logout cleanup rules, file mode/symlink rejection, explicit POSIX mode reset to `0600` for pre-existing credential files, Windows best-effort behavior, refresh success/failure, PKCE S256/state properties, redirect-port validation, `--include-clasp-scopes` preconditions, callback probes/favicon handling, and cancellation/timeout.
- [ ] **Step 2: Run focused auth tests** and verify the expected failures.
- [ ] **Step 3: Implement the credential schema and compatibility parser.**
- [ ] **Step 4: Implement secure direct writes and refresh persistence**, using `#[cfg(unix)]` for 0600/O_NOFOLLOW enforcement and propagated chmod errors; explicitly apply `PermissionsExt::set_mode(0o600)` after writing `.clasprc.json`, including pre-existing files. Use `#[cfg(windows)]` for best-effort permission/symlink protections that do not abort on unsupported POSIX operations.
- [ ] **Step 5: Implement OAuth URL creation and token exchange**, including the default client constants and ten default scopes.
- [ ] **Step 6: Implement localhost and serverless redirect flows**; use `tokio::net::TcpListener` with an accept loop that ignores probes, returns 204/404 for `/favicon.ico` and other invalid paths, accepts only a valid `GET /?code=...&state=...` callback, returns a minimal HTTP 200 HTML response, closes the connection, and integrates callback/Ctrl+C/timeout with `tokio::select!` without `spawn_blocking`.
- [ ] **Step 7: Run focused tests plus `cargo clippy`** and verify no token values appear in test output.
- [ ] **Step 8: Commit** with message `feat: implement clasp-compatible auth storage and oauth`.

### Task 4: Build the HTTP/API client layer and retry/error contracts

**Files:**
- Create: `src/api/mod.rs`
- Create: `src/api/client.rs`
- Create: `src/api/error.rs`
- Create: `src/api/script.rs`
- Create: `src/api/drive.rs`
- Create: `src/api/service_usage.rs`
- Create: `src/api/discovery.rs`
- Create: `src/api/logging.rs`
- Create: `src/api/oauth2.rs`
- Create: `tests/api_client.rs`

**Interfaces:**
- `ApiClient` owns authenticated reqwest configuration and exposes typed methods for Script, Drive, Service Usage, Discovery, Logging, userinfo, and token endpoints listed in spec §2.2; endpoint payloads use strongly typed serde structs rather than globally ordered JSON maps.
- `ApiClient::with_base_urls(config, base_urls)` injects service base URLs directly for in-process WireMock tests without mutating process environment variables.
- `ApiErrorKind` maps 400/401/403/404/other statuses to the exact normalized kinds.
- `ApiClient::request` applies transient timeout/status retry only to GET/HEAD/PUT/OPTIONS/DELETE; network/no-response failures have a separate maximum of 2 retries. Backoff is injectable and follows `100ms` for the first retry, then `100 + ((2^n - 1) / 2) * 1000ms` for subsequent attempts (`n=currentRetryAttempt`); `Retry-After` is ignored.
- `ApiClient::request` has a separate authentication layer: any 401 response triggers one token refresh and one retry of the original request regardless of method, including POST; this does not enable general POST retries.
- Runtime/golden subprocesses may use `CRSP_API_BASE_URL` and service-specific overrides (`CRSP_SCRIPT_BASE_URL`, `CRSP_DRIVE_BASE_URL`, `CRSP_SERVICE_USAGE_BASE_URL`, `CRSP_DISCOVERY_BASE_URL`, `CRSP_LOGGING_BASE_URL`, `CRSP_OAUTH2_BASE_URL`, and `CRSP_USERINFO_BASE_URL`), with service-specific values taking precedence. In-process tests must use `ApiClient::with_base_urls` instead of mutating environment variables. Production defaults remain the Google URLs in spec §2.2; overrides never change request paths or query semantics.
- API request structs preserve exact paths, query names, bodies, pagination, and multipart upload semantics.

- [ ] **Step 1: Write wiremock tests** for endpoint paths/body/query, status normalization, idempotent-method retries, POST no-transient-retry, separate no-response retry count, injectable delay sequence, `Retry-After` ignore, 401 refresh-and-retry for GET and POST, refresh failure, and each base URL override including the shared override precedence.
- [ ] **Step 2: Run `mise exec -- cargo test --test api_client`** and confirm failures.
- [ ] **Step 3: Implement the authenticated HTTP client** with 30s request and 10s connect timeouts.
- [ ] **Step 4: Implement two independent retry state machines**: transient idempotent-method/status retries with max 3 attempts (network/no-response max 2), and a one-shot 401 token refresh retry for all methods; inject the clock/sleeper so tests assert delays without real waiting.
- [ ] **Step 5: Implement typed endpoint modules** and pagination adapters, including Service Usage page size 200/max 10000, plus production Google base URLs and the documented `CRSP_*_BASE_URL` test/runtime overrides.
- [ ] **Step 6: Run tests and inspect wiremock transcripts** to ensure authorization values are masked, request bodies match the spec, and every mockable Google service is reachable through the documented child-process base URL overrides.
- [ ] **Step 7: Commit** with message `feat: add google api clients and retry policy`.

### Task 5: Implement the secure file pipeline for push, pull, and status

**Files:**
- Create: `src/core/files.rs`
- Create: `src/core/project.rs`
- Create: `src/commands/shared.rs`
- Create: `src/commands/push.rs`
- Create: `src/commands/pull.rs`
- Create: `src/commands/show_file_status.rs`
- Create: `tests/files_pipeline.rs`
- Create: `tests/fixtures/files/` with small source trees and expected snapshots

**Interfaces:**
- `collect_local_files(config, options) -> Result<Vec<LocalFile>, CrspError>` walks the content directory, applies ignore and extension rules, handles symlinks, detects SERVER_JS basename collisions, and sorts by `filePushOrder` then locale-equivalent name order.
- `get_changed_files(local, remote) -> Vec<LocalFile>` compares only local files against remote and does not report remote-only deletions.
- `push_files(...)` skips PUT when zero changed files; otherwise sends every collected local file in a full replacement body.
- `pull_files(...)` writes remote files directly with jail/symlink/race protections and optionally deletes unused local files after confirmation.
- `show_file_status(...)` produces the exact human and pretty JSON payloads.

- [ ] **Step 1: Write tests** for default ignore behavior, `.ts` unsupported skip, symlink skip/follow, special-file skip, collision errors, ordering, changed-vs-full-PUT split, `/`-normalized remote names and PUT payloads, direct-write mode with mode `0644`, all pull skip reasons, content jail, noninteractive confirmations, compressed untracked parents, and syntax-error extraction/snippet output (`Syntax error: {msg} line: {n} file: {name}`).
- [ ] **Step 2: Run the focused pipeline/security tests** and confirm failures.
- [ ] **Step 3: Implement local discovery and file-type mapping** with explicit skip-reason values, normalizing every relative path to `/` before ignore matching and API payload construction.
- [ ] **Step 4: Implement secure remote path validation, parent creation, direct writes with mode `0644`, O_NOFOLLOW, and race-condition handling; cap concurrent writes at 32 with a Tokio semaphore/buffer, treat concurrent parent-directory `EEXIST` as success only after verifying the path is a directory and its resolved realpath remains inside contentDir, and test TOCTOU symlink substitution.**
- [ ] **Step 5: Implement push comparison/full replacement and pull deletion semantics.**
- [ ] **Step 6: Add command adapters** for `push`, `pull`, and `show-file-status`, including 500ms watch debounce scaffolding.
- [ ] **Step 7: Run all file/security tests and verify fixture snapshots.**
- [ ] **Step 8: Commit** with message `feat: implement secure push pull file pipeline`.

### Task 6: Implement project lifecycle and version/deployment commands

**Files:**
- Create: `src/commands/clone_script.rs`
- Create: `src/commands/create_script.rs`
- Create: `src/commands/create_version.rs`
- Create: `src/commands/list_versions.rs`
- Create: `src/commands/create_deployment.rs`
- Create: `src/commands/update_deployment.rs`
- Create: `src/commands/delete_deployment.rs`
- Create: `src/commands/list_deployments.rs`
- Create: `src/commands/list_scripts.rs`
- Create: `src/commands/delete_script.rs`
- Modify: `src/core/project.rs`
- Create: `tests/commands_lifecycle.rs`

**Interfaces:**
- Project services call the typed API layer and update local settings through `ProjectConfig::update_settings`.
- Clone accepts script URL/ID, optional version, and root directory; create supports standalone/webapp/api and Drive-bound container MIME types.
- Deployment services preserve implicit version creation, update-vs-create behavior, versioned detection, selection rules, and all JSON payloads.
- `list-versions` reverses API order only for human-readable text; `--json` preserves raw API order.
- Delete-script performs Drive trash and, in crsp, emits `{"success":true}` in JSON mode.

- [ ] **Step 1: Write wiremock/CLI tests** for clone URL extraction, HEAD/version content, create type normalization and clasp-compatible title defaults (`my-project-folder`, `My_App`, `blog_id`), config output, version/deployment request bodies, human-readable reverse version ordering versus raw `--json` API order, deployment selection, delete behavior, `list-scripts --noShorten` and default 20-character truncation, open URL fallback behavior, and JSON contracts.
- [ ] **Step 2: Run the focused lifecycle tests** and verify failures.
- [ ] **Step 3: Implement project create/clone/list/delete services** and content initialization.
- [ ] **Step 4: Implement version and deployment services** with exact request ordering and output formatting.
- [ ] **Step 5: Add command handlers and noninteractive behavior.**
- [ ] **Step 6: Run tests, clippy, and fixture comparisons.**
- [ ] **Step 7: Commit** with message `feat: add project version and deployment commands`.

### Task 7: Implement run, API-management, logs, and open commands

**Files:**
- Create: `src/commands/run_function.rs`
- Create: `src/commands/list_apis.rs`
- Create: `src/commands/enable_api.rs`
- Create: `src/commands/disable_api.rs`
- Create: `src/commands/tail_logs.rs`
- Create: `src/commands/setup_logs.rs`
- Create: `src/commands/open_script.rs`
- Create: `src/commands/open_container.rs`
- Create: `src/commands/open_web_app.rs`
- Create: `src/commands/open_logs.rs`
- Create: `src/commands/open_api_console.rs`
- Create: `src/commands/open_credentials_setup.rs`
- Create: `tests/commands_misc.rs`

**Interfaces:**
- `run-function` parses `--params` as JSON and forwards the parsed value with clasp-compatible `parameters ?? []` behavior without client-side top-level array validation; JSON syntax errors exit 1, defaults `devMode=true`, and maps NOT_AUTHORIZED/NOT_FOUND to clasp-compatible messages.
- API-management commands update `appsscript.json` while preserving key order and call Service Usage with the correct API name.
- `tail-logs` deduplicates `insertId`, polls at 6000ms in watch mode, removes the specified debug line, and preserves the non-pure JSON formatted-line quirk.
- Open commands build exact URLs, sort `open-web-app` candidates by `updateTime` ascending, add `authUser` only when `CLASP_ENABLE_USER_HINTS` is `true`/`1`, and preserve the open-message behavior even in JSON mode. They check stdout TTY before calling `open::that`: non-TTY prints the manual-open message and returns success (exit 0); TTY calls `open::that`, and a browser-launch error propagates as exit 1.

- [ ] **Step 1: Write tests** for function parameter parsing/errors including JSON syntax failures and clasp-compatible non-array forwarding, API manifest mutations, enabled/available filtering, log formatting/deduplication/poll interval, URL construction, `open-web-app` `updateTime` ascending selection, web-app entry point selection, hints, non-TTY manual-open success, TTY browser-launch failure with exit 1, and JSON exceptions.
- [ ] **Step 2: Run focused tests** and confirm failures.
- [ ] **Step 3: Implement run/API/log services and command handlers.**
- [ ] **Step 4: Implement open URL construction and browser invocation behind a testable abstraction.**
- [ ] **Step 5: Run tests and verify exact stdout/stderr fixtures.**
- [ ] **Step 6: Commit** with message `feat: add execution api management logs and open commands`.

### Task 8: Implement the MCP stdio server and intentional MCP differences

**Files:**
- Create: `src/mcp/mod.rs`
- Create: `src/mcp/server.rs`
- Create: `tests/mcp_server.rs`

**Interfaces:**
- `start_server(...)` starts rmcp over stdio with server name `Crsp` and crate version.
- The five tools are `push_files`, `pull_files`, `create_project`, `clone_project`, and `list_projects`, with exact schemas and structured outputs from spec §2.7.
- `validate_project_dir` permits only homedir/cwd descendants; `sourceDir` resolves relative to projectDir and must remain inside it.
- MCP behavior includes all specified crsp fixes: corrected pull text/errors/title, removed push `status`, and read-only annotations for `list_projects`.

- [ ] **Step 1: Write rmcp-client integration tests** for tool schemas, annotations, success/error text, structured content, required clone scriptId, sourceDir jail, projectDir jail, and stateless Clasp construction.
- [ ] **Step 2: Run focused MCP tests** and confirm failures.
- [ ] **Step 3: Implement tool handlers over the existing project/file services.**
- [ ] **Step 4: Implement stdio startup and exact annotations/server metadata.**
- [ ] **Step 5: Run MCP tests and a manual protocol smoke test with a local client fixture.**
- [ ] **Step 6: Commit** with message `feat: add crsp mcp stdio server`.

### Task 9: Complete watch, interactive selection, shared initialization, and all command wiring

**Files:**
- Modify: `src/main.rs`
- Modify: `src/cli.rs`
- Modify: `src/commands/shared.rs`
- Create/modify: `src/commands/mod.rs`
- Modify: `src/commands/push.rs`
- Modify: `src/commands/tail_logs.rs`
- Modify: every command module as needed for shared initialization
- Create: `tests/commands_surface.rs`

**Interfaces:**
- `init_context` performs auth initialization and `Clasp` construction before command execution, equivalent to clasp’s preAction hook.
- `Clasp` bundles project config, credentials/API clients, ignore matcher, and output settings behind an `Arc`-compatible context.
- Interactive selectors use the shared `Ui`/`demand` adapter only when stdin/stdout are interactive; noninteractive fallbacks follow spec §4.
- Every parser variant dispatches to a real command handler; no placeholder command remains.

- [ ] **Step 1: Write an end-to-end command-surface test** that invokes every canonical command and alias with fixture context, asserting dispatch rather than “not implemented”.
- [ ] **Step 2: Run the surface test** and record each missing handler/initialization failure.
- [ ] **Step 3: Implement `Clasp` context construction** and shared assertions for configured script/project; dispatch async commands through a lazily built Tokio multi-thread runtime while retaining synchronous fast paths for help/version/usage and local inspection commands.
- [ ] **Step 4: Wire every command and alias** through the same typed context/error/output path.
- [ ] **Step 5: Implement push watch 500ms debounce and persistent force/confirmation state; implement logs watch 6000ms polling.**
- [ ] **Step 6: Implement fuzzy/interactive selection through the shared `Ui`/`demand` adapter with deterministic noninteractive errors/no-ops; use `std::sync::LazyLock` for static regexes and `Cow<'_, str>` for zero-allocation slash normalization when no backslash exists.**
- [ ] **Step 7: Run all unit/integration tests and verify command exit codes.**
- [ ] **Step 8: Commit** with message `feat: wire complete crsp command surface`.

### Task 10: Build golden fixtures and compatibility harness

**Files:**
- Create: `tests/golden/mod.rs`
- Create: `tests/golden_runner.rs`
- Create: `tests/golden/fixtures/<case>/fixture/...`
- Create: `tests/golden/fixtures/<case>/mock-transcript.toml`
- Create: `tests/golden/fixtures/<case>/expected.json`
- Create: `scripts/record-golden-transcript` only if needed for reproducible fixture capture

**Interfaces:**
- A golden case contains fixture tree, masked HTTP transcript, expected stdout/stderr, exit code, resulting tree, and request bodies exactly as spec §9.2.
- The runner starts wiremock, runs the binary in a temporary copied fixture, captures stdout/stderr/exit code, and compares normalized snapshots.
- Authorization headers and access/refresh tokens are always replaced by placeholders before persistence.

- [ ] **Step 1: Add representative golden cases** for login state, config discovery, push no-change/full PUT, pull/delete, clone/create, deployments, run, APIs, logs, open URLs, unknown command, and all intentional differences.
- [ ] **Step 2: Implement transcript loading and wiremock registration.**
- [ ] **Step 3: Implement process execution and filesystem snapshot comparison.**
- [ ] **Step 4: Run the golden suite** and update only fixture data that is proven to match the reference clasp behavior.
- [ ] **Step 5: Add a live-test gate** that skips unless `CRSP_LIVE_TEST=1` and credentials are present.
- [ ] **Step 6: Commit** with message `test: add clasp compatibility golden harness`.

### Task 11: Security, output, and reference parity audit

**Files:**
- Modify: affected `src/**` modules
- Modify: `tests/core_security.rs`
- Modify: `tests/files_pipeline.rs`
- Modify: `tests/golden/**`
- Create: `docs/superpowers/plans/` only for this plan; no additional product docs unless required

- [ ] **Step 1: Enumerate every spec §5 difference** and add one assertion for output, structured content, annotations, or exit code.
- [ ] **Step 2: Enumerate every spec §9.4 security case** and ensure each skip reason has a test asserting stderr, exit code, and filesystem state.
- [ ] **Step 3: Run `mise exec -- cargo test --all-targets --all-features`** and resolve failures without weakening compatibility assertions.
- [ ] **Step 4: Run `mise exec -- cargo fmt --check` and `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`.**
- [ ] **Step 5: Search source and fixtures for accidental secrets and debug output**, including `PAST`, raw authorization values, refresh tokens, and access tokens.
- [ ] **Step 6: Commit** with message `test: lock down crsp parity and security behavior`.

### Task 12: Release verification and opt-in live smoke tests

**Files:**
- Modify: `Cargo.toml` if package metadata/bin configuration is incomplete
- Create: `tests/live/` only for gated live tests
- Modify: CI configuration if the repository already has one; otherwise do not add unrelated CI infrastructure

- [ ] **Step 1: Run the complete normal verification set:**
  `mise exec -- cargo test --all-targets --all-features`
  `mise exec -- cargo fmt --check`
  `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`
- [ ] **Step 2: Run the binary smoke checks** for `crsp --version`, `crsp --help`, unknown command, usage error, `crsp status`, and `crsp mcp` with fixture/noninteractive inputs.
- [ ] **Step 3: Run live tests only when explicitly opted in** with `CRSP_LIVE_TEST=1 mise exec -- cargo test --test live -- --ignored`; verify they are skipped by default.
- [ ] **Step 4: Compare the final implementation against every success criterion in spec §11** and record any unmet criterion as a failing test rather than a prose exception.
- [ ] **Step 5: Commit** with message `chore: verify crsp release readiness`.

## Final Verification Checklist

- [ ] All 29 commands and aliases parse and dispatch.
- [ ] `.clasp.json`, `.clasprc.json`, `appsscript.json`, and `.claspignore` fixtures round-trip compatibly.
- [ ] Standard, API, MCP, security, CLI, and golden tests pass.
- [ ] `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` pass through mise.
- [ ] Normal tests make no live network calls and no secrets are committed.
- [ ] Live tests are skipped unless `CRSP_LIVE_TEST=1` and credentials exist.
- [ ] The ten intentional differences are explicit and covered by tests; no other output/API/exit-code difference is accepted.
