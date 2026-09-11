# Migrating from clasp to crsp

## Compatibility

`crsp` is a Rust port of `clasp`. It keeps the command names, aliases, defaults, English output, API semantics, and exit codes, and it reads and writes the same local files (`.clasp.json`, `.clasprc.json`, `appsscript.json`, `.claspignore`). You can point `crsp` at an existing clasp project without changing anything on disk.

Day-to-day migration is therefore: install `crsp`, run your existing commands with `crsp` instead of `clasp`, and keep using the same project directories.

## Install

```sh
cargo install google-clasp-rs
```

Or from a checkout:

```sh
cargo install --path .
```

The installed binary is named `crsp`.

## Command mapping

The binary changes from `clasp` to `crsp`; the command vocabulary is unchanged. Every short form is registered as an alias of the canonical long name.

| clasp short form | crsp canonical | Aliases |
|---|---|---|
| `clasp login` | `crsp login` | |
| `clasp logout` | `crsp logout` | |
| `clasp show-authorized-user` | `crsp show-authorized-user` | |
| `clasp clone` | `crsp clone-script` | `clone` |
| `clasp create` | `crsp create-script` | `create` |
| `clasp push` | `crsp push` | |
| `clasp pull` | `crsp pull` | |
| `clasp deploy` | `crsp create-deployment` | `deploy` |
| `clasp redeploy` | `crsp update-deployment` | `redeploy` |
| `clasp undeploy` | `crsp delete-deployment` | `undeploy` |
| `clasp delete` | `crsp delete-script` | `delete` |
| `clasp version` | `crsp create-version` | `version` |
| `clasp versions` | `crsp list-versions` | `versions` |
| `clasp deployments` | `crsp list-deployments` | `deployments` |
| `clasp list` | `crsp list-scripts` | `list` |
| `clasp run` | `crsp run-function` | `run` |
| `clasp logs` | `crsp tail-logs` | `logs` |
| `clasp status` | `crsp show-file-status` | `status` |
| `clasp apis` | `crsp list-apis` | `apis` |
| `clasp enable-api` | `crsp enable-api` | |
| `clasp disable-api` | `crsp disable-api` | |
| `clasp open-script` | `crsp open-script` | |
| `clasp open-container` | `crsp open-container` | |
| `clasp open-web-app` | `crsp open-web-app` | |
| `clasp open-logs` | `crsp open-logs` | |
| `clasp open-api-console` | `crsp open-api-console` | |
| `clasp open-credentials-setup` | `crsp open-credentials-setup` | |
| | `crsp setup-logs` | |
| | `crsp start-mcp-server` | `mcp` |

## Configuration

`.clasp.json` is read as JSON5 and written as two-space JSON. The `srcDir`/`rootDir` settings, `filePushOrder`, extension lists, `skipSubdirectories`, and `allowSymlinks` all behave as in clasp. When `crsp` rewrites project settings it stores the recomputed relative `rootDir` and resets `filePushOrder` to an empty array, exactly as clasp does.

## Authentication

The default OAuth client is the same public Google desktop client that clasp ships, so existing `.clasprc.json` tokens continue to work. Named users are selected with the global `--auth <user>` (or `-A`) flag, and `--creds`, `--adc`, `--no-localhost`, and the OAuth scope flags behave as in clasp. See [Configuration files](config-files.md) for the credential file format.

## Output and exit codes

Success exits `0`; errors and usage failures exit `1` (clap's default `2` is rewritten to match commander). Output stays in English and matches clasp except where the intentional differences below apply. Commands with a `--json` contract emit the documented JSON; commands without one keep their human-readable output.

## MCP

`crsp start-mcp-server` runs an MCP server over stdio with the tools `push_files`, `pull_files`, `create_project`, `clone_project`, and `list_projects`. The structured output and error shapes match the spec; the one display change is the server name, which is `Crsp` instead of `Clasp`. The `sourceDir` argument is resolved relative to `projectDir` and validated to stay inside it.

## Known intentional differences

These differences come from spec §5. Do not guess at them; consult the spec for the full output contract.

1. Messages show the binary name `crsp` rather than `clasp` (for example `Unknown command "crsp ..."`). Exit code `1` is unchanged.
2. The MCP server display name is `Crsp` instead of `Clasp`; tool names, schemas, and annotations are unchanged.
3. MCP message bugs are fixed: `pull_files` success says `Pulled project in {dir} to local filesystem successfully.`, its error says `Error pulling project`, and `clone_project` uses the title `Clone Apps Script project`.
4. The non-standard `status: 'success'` field is removed from the `push_files` structured output.
5. MCP `sourceDir` is resolved relative to `projectDir` and validated inside it (clasp validated against homedir/cwd but resolved against the project root). Absolute paths behave the same.
6. The `tail-logs` debug line `PAST <projectId>` is removed.
7. `show-file-status --json` uses two-space pretty JSON like the other commands.
8. `delete --json` emits `{"success": true}`.
9. The `pull -d` flag definition drops a trailing space in its display name; behavior is unchanged.
10. `list_projects` declares the correct read-only annotations (`destructiveHint: false`, `readOnlyHint: true`).
