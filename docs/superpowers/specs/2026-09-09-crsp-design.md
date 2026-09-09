# crsp 設計仕様 — clasp (Google Apps Script CLI) の Rust 再実装

- 日付: 2026-09-09
- 状態: 承認済み (ユーザーレビュー後、writing-plans へ移行)
- 参照実装: `/Users/sin/Desktop/GitHub/manage-clasp/clasp` (@google/clasp v3.4.1, Apache-2.0)
- 対象: `/Users/sin/Desktop/GitHub/manage-clasp/crsp`

## 1. 目的とスコープ

Google Apps Script CLI「clasp」を Rust で **フルパリティ + drop-in 互換** に再実装する。

### 決定事項 (ユーザー確定)

| 項目 | 決定 |
|---|---|
| 再現範囲 | 全29コマンド + MCPサーバー (フルパリティ) |
| ローカルファイル互換 | `.clasp.json` / `.clasprc.json` / `appsscript.json` は clasp と完全互換 (読み書き) |
| CLI UX | コマンド名 / エイリアス / フラグ / 既定値 / 出力文言 (英語) を clasp と同一にする drop-in 互換 |
| アーキテクチャ | シングルクレート + tokio async + Google API 薄クライアント自作 (案A) |
| clasp のバグ的挙動 | 意図的に修正し、ドキュメントに差分を明記 |

### 非スコープ

- i18n (claspは英語ロケールのみ使用 → crsp も英語ハードコード)
- Node版 clasp とのソース共有
- clasp の実験的機能のうち `CLASP_ENABLE_USER_HINTS` 以外 (experiments.ts の他フラグは未使用)

## 2. 参照実装から得た再現要件 (clasp v3.4.1 の分析結果)

### 2.1 全体構造

- エントリ: `src/index.ts` → commander プログラム組み立て (`program.ts`) → 全コマンド共通の `preAction` フックで認証初期化 (`initAuth`) と `Clasp` インスタンス構築 (`initClaspInstance`) を行い、コマンドへ注入。
- グローバルオプション: `-A/--auth <file>` (env `clasp_config_auth`), `-u/--user <name>` (既定 `default`), `--adc`, `--json`, `--allow-symlinks`, `-I/--ignore <file>` (env `clasp_config_ignore`), `-P/--project <file>` (env `clasp_config_project`), `-v/--version`。
- 出力規約: 人間向け文言は ICU (英語)。スピナーは TTY 時のみ。`--json` で JSON 出力 (原則2スペース pretty)。
- API エラー正規化 `handleApiError`: 400→INVALID_ARGUMENT, 401→NOT_AUTHENTICATED, 403→NOT_AUTHORIZED, 404→NOT_FOUND, その他→UNEXPECTED_API_ERROR。
- ページネーション `fetchWithPages`: pageSize 100 / maxPages 10 (最大1000件)、`{results, partialResults}` を返す。
- 未知コマンド → `Unknown command "clasp {command}"` + help + exit 1。

### 2.2 REST エンドポイント一覧 (crsp/api/ の実装対象)

| エンドポイント | メソッド | 用途 |
|---|---|---|
| `https://script.googleapis.com/v1/projects` | POST | create-script |
| `.../v1/projects/{scriptId}/content` | GET (`?versionNumber=N`) | push(差分比較), pull, clone, create, run(関数一覧) |
| `.../v1/projects/{scriptId}/content` | PUT (body `{files:[{name,type,source}]}`) | push (フル置換) |
| `.../v1/projects/{scriptId}/versions` | POST / GET | create-version, deploy(暗黙作成), list-versions |
| `.../v1/projects/{scriptId}/deployments` | POST / GET | create-deployment, list, open-web-app, undeploy |
| `.../v1/projects/{scriptId}/deployments/{depId}` | GET / PUT / DELETE | open-web-app(entryPoints), redeploy, undeploy |
| `https://script.googleapis.com/v1/scripts/{scriptId}/run` | POST (`{function, parameters, devMode}`) | run |
| `https://www.googleapis.com/drive/v3/files` | GET (`q: mimeType="application/vnd.google-apps.script"`) | list, clone/create 対話リスト |
| `https://www.googleapis.com/drive/v3/files` | POST (multipart upload) | create-script (コンテナ作成) |
| `https://www.googleapis.com/drive/v3/files/{fileId}` | PATCH (`{trashed: true}`) | delete |
| `https://serviceusage.googleapis.com/v1/projects/{p}/services?filter=state:ENABLED&pageSize=200` | GET | list-apis |
| `https://serviceusage.googleapis.com/v1/projects/{p}/services/{api}.googleapis.com:enable` / `:disable` | POST | enable/disable-api |
| `https://discovery.googleapis.com/discovery/v1/apis?preferred=true` | GET | list-apis (available一覧) |
| `https://logging.googleapis.com/v2/entries:list` | POST | logs / tail-logs |
| `https://www.googleapis.com/oauth2/v2/userinfo` | GET | login, show-authorized-user, open系ユーザーヒント |
| `https://oauth2.googleapis.com/token` (+ `accounts.google.com/o/oauth2/v2/auth`) | POST | 認可コード交換 / トークンリフレッシュ |

### 2.3 ローカルファイル

**`.clasp.json`**
- 発見: `-P` がパス/ディレクトリ指定ならそこから、なければ cwd から find-up。
- パースは **JSON5** (コメント・シングルクォート許容)。書き込みはプレーン JSON 2スペース。
- キー: `scriptId`, `projectId`, `parentId` (配列なら先頭), `srcDir`|`rootDir` → contentDir (プロジェクトルート外はエラー), `filePushOrder`, `fileExtension`(レガシー), `scriptExtensions` (既定 `[".js",".gs"]`), `htmlExtensions` (既定 `[".html"]`), `jsonExtensions` (既定 `[".json"]`), `skipSubdirectories`, `allowSymlinks`。

**`.claspignore`**
- プロジェクトルート (または `-I`)。BOM除去→行分割。micromatch で `dot: true` 評価。
- 既定値 (不在時): `**/**`, `!**/appsscript.json`, `!**/*.gs`, `!**/*.js`, `!**/*.ts`, `!**/*.html`, `.git/**`, `node_modules/**`。

**`.clasprc.json`** (既定 `~/.clasprc.json`)
- V3形式: `{"tokens": {"<user>": {client_id, client_secret, refresh_token, access_token, expiry_date, type: "authorized_user"}}}`
- レガシー読み込み対応: V1ローカル (`token` + `oauth2ClientSettings`)、V1グローバル (トップレベル `access_token`、`exprity_date` スペルミスも再現)。
- 書き込み: chmod 0600 + `O_NOFOLLOW` (symlink防御)。トークンリフレッシュで自動再保存。
- デフォルト OAuth クライアント (claspと同一、既存トークン互換):
  - ID: `1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com`
  - Secret: `v6V3fKV_zWU7iw1DrpO1rknX` (公開前提のGoogle提供デスクトップクライアント)
  - redirect: `http://localhost`
  - クライアント分類: ID一致 → `google-provided`、それ以外 → `user-provided`

**`appsscript.json`** — contentDir に置かれるマニフェスト。push/pull では `type: JSON`・リモート名 `appsscript`。enable/disable-api は `dependencies.enabledAdvancedServices` (`userSymbol`, `serviceId`, `version`) を読み書き (キー順保存)。

### 2.4 認証フロー

- OAuth2 Authorization Code + **PKCE (S256)**、`access_type=offline`、state は 256bit ランダム (base64url 32バイト)。
- デフォルト = **localhost flow**: ローカルHTTPサーバー (`--redirect-port` 未指定ならランダムポート) でリダイレクトを受け、ブラウザ自動起動。
- `--no-localhost` = serverless flow: redirect URI `http://localhost:8888`、認証後のURLをユーザーがコピペ。
- スコープ既定10個:
  1. `https://www.googleapis.com/auth/script.deployments`
  2. `https://www.googleapis.com/auth/script.projects`
  3. `https://www.googleapis.com/auth/script.webapp.deploy`
  4. `https://www.googleapis.com/auth/drive.metadata.readonly`
  5. `https://www.googleapis.com/auth/drive.file`
  6. `https://www.googleapis.com/auth/service.management`
  7. `https://www.googleapis.com/auth/logging.read`
  8. `https://www.googleapis.com/auth/userinfo.email`
  9. `https://www.googleapis.com/auth/userinfo.profile`
  10. `https://www.googleapis.com/auth/cloud-platform`
- `--use-project-scopes`: マニフェスト `oauthScopes` を使用。`--include-clasp-scopes` は `--use-project-scopes` と併用必須 (単独使用はエラー)。`--extra-scopes`: カンマ区切り、空要素でエラー。
- ADC (`--adc`): Application Default Credentials。

### 2.5 コマンドサーフェス (29コマンド)

エイリアス: `clone`, `create`, `deploy`, `redeploy`, `undeploy`, `delete`, `version`, `versions`, `deployments`, `list`, `run`, `logs`, `status`, `apis`, `mcp` (`start-mcp-server`)。

| コマンド | 引数 | 主要オプション | 概要 |
|---|---|---|---|
| `login` | なし | `--no-localhost`, `--creds <file>`, `--use-project-scopes`, `--include-clasp-scopes`, `--extra-scopes <scopes>`, `--redirect-port <n>` | OAuth認証して `.clasprc.json` 保存 |
| `logout` | なし | なし | 現ユーザーkeyのトークン削除 (V1レガシートップレベルキーも除去)。未ログインでも成功 |
| `show-authorized-user` | なし | なし | ログイン状態・email・client ID・client種別表示 |
| `clone` | `[scriptId] [versionNumber]` | `--rootDir <dir>` | URLからscript ID抽出 (`https://script.google.com/d/([^/]+)/`)。対話: Driveスクリプト一覧から選択。pull後 `.clasp.json` 作成 |
| `create` | なし | `--type <t>` (既定 `standalone`), `--title <t>` (既定: cwd名をhumanize), `--parentId <id>`, `--rootDir <dir>` | container型 (docs/forms/sheets/slides) はDrive作成→bound script。webapp/api はdeployment tip表示 |
| `push` | なし | `-f/--force`, `-w/--watch` | 変更差分のみ説明用に比較し、フルコンテンツ置換PUT。マニフェスト変更時はconfirm (非interactive=拒否)。watchは500ms debounce |
| `pull` | なし | `--versionNumber <n>`, `-d/--deleteUnusedFiles`, `-f/--force` | content取得→ローカル書き込み (アトミック)。`-d` はリモートに無いローカルファイルを対話確認後削除 |
| `deploy` | なし | `-V/--versionNumber`, `-d/--description`, `-i/--deploymentId` | version未指定なら暗黙作成。`-i` 指定なら redeploy |
| `redeploy` | `<deploymentId>` (必須) | `-V`, `-d` | 既存deployment更新 |
| `undeploy` | `[deploymentId]` | `-a/--all` | versionedのみ対象。1件なら自動選択、複数なら対話 |
| `delete` | `[scriptId]` | `-f/--force` | Driveファイルをtrash。confirm必須 (非interactive+`-f`なし=サイレントno-op) |
| `version` | `[description]` | なし | version作成 (対話入力フォールバック) |
| `versions` | `[scriptId]` | なし | 逆順リスト `N - desc` |
| `deployments` | `[scriptId]` | なし | `- {id} @{v|HEAD} {- desc}` |
| `list` | なし | `--noShorten` | Drive上スクリプト一覧 (既定20文字truncate) |
| `run` | `[functionName]` | `--nondev`, `-p/--params <json>` | `scripts/{id}/run`。devMode既定true。関数名省略時fuzzy選択。NOT_AUTHORIZED→API実行可能deployの注意喚起 |
| `logs` | なし | `--watch`, `--simplified` | Cloud Logging取得。insertId重複排除。watchは6000msポーリング |
| `setup-logs` | なし | なし | projectId確認・対話設定のみ (API呼び出しなし) |
| `status` | なし | なし | push対象/未追跡ファイル一覧 (未追跡は共通親ディレクトリに圧縮) |
| `apis` | なし | なし | enabled (Service Usage) / available (Discovery) を Advanced Services 31種でフィルタ |
| `enable-api` | `<api>` | なし | マニフェスト更新 + Service Usage有効化 |
| `disable-api` | `<api>` | なし | マニフェスト更新 + Service Usage無効化 |
| `open-script` | `[scriptId]` | なし | `https://script.google.com/d/{id}/edit` |
| `open-container` | なし | なし | `parentId` 必須 → `https://drive.google.com/open?id={id}` |
| `open-web-app` | `[deploymentId]` | なし | deployments から WEB_APP entry point のURL取得 |
| `open-logs` | なし | なし | Cloud Console logs viewer |
| `open-api-console` | なし | なし | GCP APIs dashboard |
| `open-credentials-setup` | なし | なし | GCP credentials ページ |
| `mcp` | なし | なし | MCP stdio サーバー起動 |

open系共通: `CLASP_ENABLE_USER_HINTS=true|1` のとき `?authUser=<userId>` を付与 (userinfo取得必要)。表示は `Opening {url} in your browser.` / JSONモードでもURL行表示。

### 2.6 files パイプラインのセキュリティ機構 (最難所・移植必須)

- push/pull とも symlink のスキップ+警告 (`Security Warning: Skipping symbolic link {file}...`)。**親ディレクトリが symlink** の場合もスキップ。
- pull の書き込み: **アトミック** (temp → rename)、`O_NOFOLLOW`、親symlink検出 (realpath事前確認)、`0o644`、親ディレクトリ `mkdir -p` (親のsymlinkチェック込み)、race時 (`EEXIST`/`ELOOP`) は安全側フォールバック。
- リモート由来ファイル名の jail (`isInside`): contentDir 外への脱出禁止 (`../../` 攻撃防御)。
- SERVER_JS 名衝突検出: 同一basenameの `.js`/`.gs` 併存でエラー。
- push ソート: `filePushOrder` 優先 → アルファベット (`localeCompare` 相当)。
- 構文エラーパース: APIエラーから `Syntax error: {msg} line: {n} file: {name}` を抽出しコードスニペット表示。
- contentDir はプロジェクトルート内に制限 (脱出禁止)。

### 2.7 MCP サーバー

- transport: **stdio** のみ。サーバー名 `Clasp`、バージョン = CLIバージョン。
- ツール5つ: `push_files`, `pull_files`, `create_project`, `clone_project`, `list_projects`
  - 共通入力: `projectDir` (必須) (+`sourceDir`/`projectName`/`scriptId`)。
  - `projectDir` の jail: resolve後、homedir または cwd 配下のみ許可 (`Security Error: projectDir must be within the user home directory or current working directory.`)。
  - ツール呼び出しごとに `Clasp` インスタンスを新規構築 (ステートレス)。
  - レスポンス: text content (人間向け行) + structuredContent (`{scriptId, projectDir, files[]}` / `{scripts: [{scriptId, name}]}`)。エラーは `isError: true` + text。
- clasp のコピペバグ (pull_files の文言等) は crsp では修正した文言を使用。

## 3. crsp アーキテクチャ (承認済み設計)

```
crsp/
├── Cargo.toml               # シングルクレート
├── src/
│   ├── main.rs              # エントリ: clapパース → auth/Clasp初期化 (preAction相当) → 実行 → エラー処理
│   ├── cli.rs               # clap derive: グローバルオプション + 27コマンド (名前/エイリアス/フラグ完全一致)
│   ├── constants.rs         # PROJECT_NAME="crsp" ※バイナリ名/表示名, マニフェスト名, 既定OAuthクライアント
│   ├── i18n.rs              # claspと同一の英語メッセージ定数
│   ├── commands/            # 1コマンド1ファイル (clasp 1:1)
│   │   ├── shared.rs        # assertScriptConfigured / assertGcpProjectConfigured / ellipsize / openUrl / spinner
│   │   └── (29 files)
│   ├── core/                # ドメインロジック (UI非依存)
│   │   ├── clasp.rs         # Clasp: config + credentials + APIクライアントを束ねる実行コンテキスト
│   │   ├── project.rs       # createScript / listScripts / updateSettings (.clasp.json書込)
│   │   ├── files.rs         # push/pullパイプライン + セキュリティ機構
│   │   ├── manifest.rs      # appsscript.json 読書 (キー順保持: serde_json::Valueラウンドトリップ)
│   │   ├── ignore.rs        # micromatch互換マッチャー (globset + 否定パターン, dot有効)
│   │   └── pagination.rs    # fetchWithPages (pageSize=100, maxPages=10)
│   ├── auth/
│   │   ├── credential_store.rs  # .clasprc.json 読書 (V1/V3互換, 0600, O_NOFOLLOW)
│   │   ├── oauth_client.rs      # リフレッシュ, クライアント分類
│   │   ├── flow.rs              # PKCE(S256) + state + auth URL
│   │   ├── localhost_flow.rs    # axum ローカルサーバー
│   │   └── serverless_flow.rs   # URLコピペ
│   ├── api/
│   │   ├── script.rs / drive.rs / service_usage.rs / logging.rs / oauth2.rs / discovery.rs
│   │   └── error.rs         # ApiErrorKind (InvalidArgument/NotAuthenticated/NotAuthorized/NotFound/Unexpected)
│   └── mcp/server.rs        # rmcp stdio, 5ツール
└── tests/                   # 統合テスト (assert_cmd + wiremock + tempfile)
```

### 3.1 技術選定

| 用途 | クレート |
|---|---|
| CLI | `clap 4` (derive) |
| async | `tokio` (full) |
| HTTP | `reqwest` + `rustls` (静的リンク, openssl回避) |
| JSON | `serde` + `serde_json` (書き出し), `json5` (`.clasp.json`読み込みのみ) |
| キー順保存JSON | `serde_json::Value` (preserve_order feature — indexmap経由で挿入順維持) |
| ignore | `globset` + 自前否定パターン処理 (micromatch互換) |
| ファイル走査 | `walkdir` |
| watch | `notify` (FSEvents/inotify) + 500ms debounce |
| 対話UI | `dialoguer` (`FuzzySelect` + `fuzzy-matcher`), `indicatif` (spinner, TTY時のみ) |
| ブラウザ | `open` |
| ホームディレクトリ | `dirs` |
| OAuth暗号 | `rand` + `sha2` (PKCE), `base64` |
| localhost redirect | `axum` (最小1ルート) |
| MCP | `rmcp` (公式SDK, stdio transport) |
| エラー | `thiserror` + `anyhow` (mainでの表示用) |
| ellipsize | `unicode-width` |
| テスト | `wiremock` / `httpmock`, `assert_cmd`, `predicates`, `tempfile` |

### 3.2 実行フロー

1. `main()`: `#[tokio::main]`、clapパース。
2. preAction相当: グローバルオプションから `auth::init()` (credential store読み込み → 必要ならトークンリフレッシュ+再保存 or ADC) → `Clasp::init()` (`-P` または find-up で `.clasp.json` をJSON5パース → contentDir検証 → ignore matcher構築)。
3. コマンドハンドラへ `Clasp` (Arc) を注入して実行。
4. エラーはtyped error → mainでexit code 1 + stderrにmessage。未知コマンドはclapがhelp表示。

### 3.3 並行性

- pull の並列書き込み: `futures::stream` + bounded concurrency (claspの p-map 相当、8並列)。
- watch / MCP / logs polling は tokio 上のasyncタスク。
- ファイルIOは少量なので直接同期実行 (必要時に `spawn_blocking`)。

## 4. 重要な互換性ルール (実装時の契約)

1. `.clasp.json` は JSON5 で読み、プレーン JSON (2スペース) で書く。
2. `.clasprc.json` は V3 形式で書き、V1 2形式を読める。書き込みは0600 + O_NOFOLLOW。
3. 既定OAuthクライアントは clasp と同一の ID/secret (既存トークンがそのまま動く)。
4. スコープ既定10種は clasp と完全一致。
5. ignore の既定パターンと評価を再現: dot有効。micromatch互換の意味論は「いずれかの肯定パターンに一致し、いずれの否定 (`!`) パターンにも一致しない」こと。
6. ページネーション: pageSize=100, maxPages=10。
7. push は差分ありならリクエストに全ファイルを含む**フルコンテンツ置換PUT** (増分PUTではない)。差分なしの場合はPUTせず `Script is already up to date.` を表示。
8. pull 書き込みはアトミック + symlink/jail 防御。
9. 非interactive時のフォールバック: push manifest確認=拒否 / pull `-d` 削除=スキップ+警告 / delete=サイレントno-op。
10. `--json` 出力は2スペース pretty (統一)。
11. ログポーリング6000ms、watch debounce 500ms。
12. マニフェスト名はリモートから `appsscript` (拡張子なし)、ローカルは `appsscript.json`。

## 5. crsp における意図的な差分 (claspからの修正)

1. tail-logs の余計な `console.log('PAST', ...)` デバッグ出力 → 削除。
2. MCP ツール文言のコピペバグ (`pull_files` が "Pushed..." と表示等) → 正しい文言。
3. `status` のJSONが単一行 → pretty に統一。
4. `delete` が `--json` を無視 → JSON出力対応 (`{"success": true}`)。
5. pull の `-d, --deleteUnusedFiles ` 末尾スペースフラグ定義 → 修正。
6. `open-*` のJSONモードでもURL行を必ず表示する clasp 挙動 → **維持** (挙動差分を起こさないため。JSONには含めず人間向け行として表示)。
7. その他CLASP_ENABLE_USER_HINTS以外のexperiments未使用フラグ → 実装しない。
8. MCPサーバー名は `Crsp` (claspは `Clasp`)、バージョンはcrsp自身のもの。

## 6. エラー処理・出力

- `crsp::Error` (thiserror): `Api(ApiErrorKind, message)`, `Config`, `Auth`, `Io`, `Validation`, `Aborted` (ユーザー拒否)。
- APIエラーはHTTP statusから `ApiErrorKind` へ正規化し、コマンドはkindで分岐 (例: run → NotAuthorized で「API executable deploy」注意喚起)。
- spinner/警告はstderr、成果物はstdout。
- exit code: 成功0、エラー1、clapusageエラーはclap既定。

## 7. テスト戦略

| レイヤ | 手法 |
|---|---|
| core unit | ignoreマッチャー・config発見・manifest・衝突検出・paginationを網羅 (純粋関数) |
| APIクライアント | `wiremock` でエンドポイントモック。トークンリフレッシュ・401処理含む |
| CLI統合 | `assert_cmd` + `tempfile` 仮想プロジェクト。主要コマンドをモックAPIに対しE2E |
| セキュリティ回帰 | symlink親攻撃・`../../` jail・O_NOFOLLOW・0600・名前衝突のテストを重点記述 |
| 互換フィクスチャ | 実claspが書いた `.clasp.json` / `.clasprc.json` (V1/V3) を読んで同一解釈できるか検証 |

## 8. 実装順序 (リスク降順)

1. 基盤: constants, config (.clasp.json JSON5), credential_store (.clasprc.json V1/V3), OAuth flow (login/logout/show-authorized-user)
2. files: ignoreマッチャー, push/pull, セキュリティ機構 ← 最難所
3. script APIコマンド: clone/create/deploy系/list系/run/version/versions/deployments/undeploy/delete
4. 周辺: open系/status/setup-logs/list-apis/enable/disable/logs/setup-logs
5. MCP (rmcp stdio, 5ツール)
6. 仕上げ: watch, fuzzy picker, メッセージ文言のclasp完全一致監査, リリース (cargo-dist想定)

## 9. 成功基準

- 実claspのプロジェクトディレクトリ (`clasp` でclone済みの状態) に対し、crspの `pull` / `push` / `deployments` / `run` が同一結果で動作する。
- `.clasprc.json` に clasp で login したトークンがある状態で crsp が再ログインなしで動く。
- `crsp push --watch`・`crsp logs --watch` が clasp と同等の挙動。
- `crsp mcp` が MCP Inspector で5ツール動作確認できる。
- セキュリティ回帰テスト全グリーン。
