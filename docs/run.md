# Running crsp

## Install

```sh
cargo install google-clasp-rs
```

Or from a checkout:

```sh
cargo install --path .
```

## Login

```sh
crsp login
```

By default this starts a local HTTP server and opens the browser for the Google OAuth consent screen. Use `--no-localhost` to copy the consent URL manually, `--creds <file>` to use your own OAuth client JSON, `--adc` for Application Default Credentials, or `crsp login --user work` to store credentials under a named account.

## Create or clone

Create a new project, then push it to a new Apps Script project:

```sh
crsp create-script --title "My script" --type standalone
```

Clone an existing project into a new directory:

```sh
crsp clone-script --script-id <scriptId>
```

Cloning works from any directory; `crsp` finds the project settings as it does for every other command, so `--project` can point at the target directory:

```sh
crsp clone-script --script-id <scriptId> --project ./my-script
```

## Push and pull

```sh
crsp push
crsp pull
```

`push` uploads the tracked local files; `pull` downloads the remote project into the content directory. To watch for local changes and push automatically:

```sh
crsp push --watch
```

The `--json` flag produces machine-readable output where supported:

```sh
crsp push --json
```

To select a project or a credential explicitly, use the global flags:

```sh
crsp push --project /path/to/project
crsp push --user work
```

## Deployments

```sh
crsp create-version --description "First version"
crsp list-versions
crsp create-deployment --description "Production" --versionNumber 1
crsp list-deployments
```

## Logs and functions

```sh
crsp run-function --function myFunction
crsp tail-logs
crsp setup-logs
```

`tail-logs` polls Cloud Logging for the current project. `setup-logs` prepares logging for the project.

## API management

```sh
crsp list-apis
crsp enable-api drive
crsp disable-api drive
```

## JSON output

Commands that support structured output accept the global `--json` flag:

```sh
crsp list-scripts --json
crsp list-versions --json
```

The JSON shape is stable per command; commands without a defined JSON contract keep their human-readable output.

## MCP server

Start a Model Context Protocol server over stdio:

```sh
crsp start-mcp-server
```

The server exposes `push_files`, `pull_files`, `create_project`, `clone_project`, and `list_projects`. It identifies itself as `Crsp`.

## Troubleshooting

- `No access, refresh token, API key or refresh handler callback is set.` — run `crsp login` (or pass `--creds` / `--adc`).
- `Script ID is required.` — run the command inside a project directory with a `.clasp.json`, or pass `-P, --project`.
- Files are missing from `push` — check `.claspignore`; the tracked set is the complement of the ignored set, so a broad positive pattern ignores more files.
- Push reports a syntax error — `crsp` prints the message, line, and file, and stops before uploading.
- A symlinked file is skipped — symlinks are skipped unless the project enables `allowSymlinks`.
