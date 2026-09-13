# crsp

Rust port of [clasp](https://github.com/google/clasp) — develop Apps Script projects locally from the command line.

## Install

### npm (recommended)

```sh
npm install -g @sinsky-gh/crsp
```

Requires Node.js 18 or later. Prebuilt binaries are included for
darwin-arm64, darwin-x64, linux-x64, linux-arm64, and win32-x64.

Or run without installing:

```sh
npx @sinsky-gh/crsp --version
```

### cargo

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
- `completion` `<shell>` — Generate a shell completion script (`bash`, `zsh`, `fish`, `powershell`)

## Shell completion

```sh
# bash
crsp completion bash > ~/.bash_completion.d/crsp

# zsh
crsp completion zsh > "${fpath[1]}/_crsp"

# fish
crsp completion fish > ~/.config/fish/completions/crsp.fish

# powershell
crsp completion powershell | Out-String | Invoke-Expression
```

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
