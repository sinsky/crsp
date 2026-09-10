# crsp

Rust port of [clasp](https://github.com/google/clasp) — develop Apps Script projects locally from the command line.

## Install

```sh
cargo install crsp
```

Or build from source:

```sh
cargo build --release
```

## Usage

```sh
crsp login
crsp clone-script --script-id <id>
crsp push
crsp pull
crsp --help
```

Key commands: `login`, `logout`, `clone-script`, `create-script`, `push`, `pull`,
`create-version`, `list-versions`, `create-deployment`, `list-deployments`,
`update-deployment`, `delete-deployment`, `run-function`, `tail-logs`,
`list-apis`, `enable-api`, `disable-api`, `open-script`, `start-mcp-server`.

Global flags: `-P/--project`, `-A/--auth`, `-I/--ignore`, `--json`, `--adc`, `--user`.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Requires Rust stable (see `mise.toml`).

## License

Apache-2.0 — see [LICENSE](LICENSE).
This project is a Rust port of [google/clasp](https://github.com/google/clasp) (Apache-2.0).
