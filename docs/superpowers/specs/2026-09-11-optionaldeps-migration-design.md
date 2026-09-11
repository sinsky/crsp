# optionalDependencies 移行 Design

## Goal

postinstallダウンロード方式を廃止し、platform package＋`optionalDependencies`方式へ移行する。

## Architecture

`npm/<platform>/`配下に5 platform packageを同リポジトリ管理する。ルート`@sinsky-gh/crsp`は`optionalDependencies`で参照し、npmのos/cpu解決にバイナリ配布を委ねる。`postinstall`と`scripts/install.js`は廃止し、install script不要化によりallowScripts問題を解消する。

## Tech Stack

- npm optionalDependencies (os/cpu制約付きplatform package)
- Node.js >= 18 (binラッパー)
- GitHub Actions (release.yamlで6パッケージstage publish)

## Platform packages

配置とnpm名：

- `npm/darwin-arm64/` → `@sinsky-gh/crsp-darwin-arm64` (`os: [darwin]`, `cpu: [arm64]`)
- `npm/darwin-x64/` → `@sinsky-gh/crsp-darwin-x64` (`os: [darwin]`, `cpu: [x64]`)
- `npm/linux-x64/` → `@sinsky-gh/crsp-linux-x64` (`os: [linux]`, `cpu: [x64]`)
- `npm/linux-arm64/` → `@sinsky-gh/crsp-linux-arm64` (`os: [linux]`, `cpu: [arm64]`)
- `npm/win32-x64/` → `@sinsky-gh/crsp-win32-x64` (`os: [win32]`, `cpu: [x64]`)

各`package.json`は`version`をルートと同期、`files: [bin]`、`publishConfig.access: public`を持つ。バイナリ配置先は各`bin/crsp-bin`（Windowsは`bin/crsp-bin.exe`）。

## Root package changes

- `optionalDependencies`に上記5件を同期バージョンで追加
- `scripts.postinstall`を削除
- `files`から`scripts`を除外
- `bin/crsp.js`を`require.resolve`＋直接パス探索で`@sinsky-gh/crsp-<platform>/bin/crsp-bin`解決に変更。未解決時は導入済みplatform一覧つきエラーを出す

## Release workflow changes

- matrixに`darwin-x64`（macos-15-intel, `x86_64-apple-darwin`）を追加。Linuxはmusl target継続（`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`）
- 各matrixジョブはビルド成果物を対応`npm/<platform>/bin/`へ配置
- 本体＋platform5の計6パッケージを`npm stage publish`
- 初回のみplatform5件は新規パッケージのため通常publishが必要（Staged Publishingは既存パッケージのみ可）

## Versioning

ルートとplatform5の`version`は常に同期する。リリース時は6件まとめて同一バージョンで公開する。

## Testing

- `node --check bin/crsp.js`
- 6件の`npm pack --dry-run`で内容物確認
- ローカル`npm install`で該当platformのみ導入されること、`crsp --help`が動作すること
- 未対応platformでは明瞭なエラーメッセージが出ること

## Rollback

移行後問題が出た場合、`postinstall`方式への復帰は`scripts/install.js`と`release.yaml`のGitHub Releases添付を復活させれば可能。ただし本設計では復帰手順の実装は行わない。
