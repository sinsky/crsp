# Changelog

All notable changes to `crsp` are documented in this file.

## 0.1.1

### Added

- npm distribution via `optionalDependencies` platform packages (`@sinsky-gh/crsp-darwin-arm64`, `-darwin-x64`, `-linux-x64`, `-linux-arm64`, `-win32-x64`).
- linux-arm64 prebuilt binary and musl static linking for Linux targets.
- Staged Publishing release workflow with Trusted Publishing (OIDC).

### Removed

- `postinstall` GitHub Releases download (`scripts/install.js`).

## 0.1.0

### Added

- Rust CLI (`crsp`) porting the `clasp` command surface, with 29 canonical commands and their `clasp` aliases.
- clasp-compatible local files: `.clasp.json`, `.clasprc.json`, `appsscript.json`, and `.claspignore`.
- OAuth and Google Apps Script / Drive API clients with named-user credential support.
- Golden, command-surface, and opt-in live test suites.
- `start-mcp-server` for MCP clients, with display name `Crsp`.
