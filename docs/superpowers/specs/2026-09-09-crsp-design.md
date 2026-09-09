# crsp 設計仕様 — clasp (Google Apps Script CLI) の Rust 再実装

- 日付: 2026-09-09 (rev2 — 外部レビュー反映)
- 状態: ユーザーレビュー待ち (承認後 writing-plans へ移行)
- 参照実装: `/Users/sin/Desktop/GitHub/manage-clasp/clasp` (@google/clasp v3.4.1, Apache-2.0)
- 対象: `/Users/sin/Desktop/GitHub/manage-clasp/crsp`

## 1. 目的とスコープ

Google Apps Script CLI「clasp」を Rust で **フルパリティ** に再実装する。

### 1.1 決定事項 (ユーザー確定)

| 項目 | 決定 |
|---|---|
| 再現範囲 | 全29コマンド + MCPサーバー (フルパリティ) |
| ローカルファイル互換 | `.clasp.json` / `.clasprc.json` / `appsscript.json` は clasp と完全互換 (読み書き) |
| CLI UX | コマンド名 / エイリアス / フラグ / 既定値 / 出力文言 (英語) を clasp と同一にする drop-in 互換 |
| アーキテクチャ | シングルクレート + tokio async + Google API 薄クライアント自作 |
| MCPサーバー名 | **`Crsp`** に変更 (claspは `Clasp`)。ツール名・スキーマ・annotations は不変 (公開契約の互換部分) |
| clasp のバグ的挙動 | §5 に列挙した差分のみ意図的に修正、それ以外は完全再現 |

### 1.2 互換モードの定義 (compatibility boundary)

「drop-in互換」を以下の3層で定義する:

- **レイヤA — 契約 (完全互換・差分禁止)**: ローカルファイル形式 (`.clasp.json`, `.clasprc.json`, `appsscript.json`, `.claspignore`)、コマンド/エイリアス/オプション名、既定値、OAuth (既定クライアント・スコープ・トークン形式)、APIリクエストセマンティクス (メソッド・パス・ボディ・ページネーション)、exit code (0=成功, 1=エラー/usageエラー)。
- **レイヤB — 出力テキスト (文言一致)**: stdout/stderr の英語文言・フォーマット。§5 の差分のみ変更を許容。差分は exit code に影響しない。
- **レイヤC — 非公開実装詳細 (自由)**: 内部アーキテクチャ、並行実装、依存関係、デバッグ出力。

§5 の各差分には「期待される出力/exit code」を明記する。差分に該当しない限り clasp と同一挙動を golden test で保証する。

### 1.3 非スコープ

- i18n (claspは英語ロケールのみ使用 → crsp も英語ハードコード)
- clasp の `CLASP_ENABLE_USER_HINTS` 以外の実験的フラグ
- Node版 clasp とのソース共有

## 2. 参照実装から得た再現要件 (clasp v3.4.1 の分析結果・実コード検証済み)

### 2.1 全体構造

- エントリ: `src/index.ts` → commander プログラム組み立て (`program.ts`) → 全コマンド共通の `preAction` フックで認証初期化 (`initAuth`) と `Clasp` インスタンス構築 (`initClaspInstance`) を行い、コマンドへ注入。
- グローバルオプション: `-A/--auth <file>` (env `clasp_config_auth`), `-u/--user <name>` (既定 `default`), `--adc`, `--json`, `--allow-symlinks`, `-I/--ignore <file>` (env `clasp_config_ignore`), `-P/--project <file>` (env `clasp_config_project`), `-v/--version`。
- 出力規約: スピナーは TTY 時のみ。`--json` で JSON 出力 (§6.2 にコマンド別契約)。スピナー/警告は stderr、成果物は stdout。
- API エラー正規化 `handleApiError`: 400→INVALID_ARGUMENT, 401→NOT_AUTHENTICATED, 403→NOT_AUTHORIZED, 404→NOT_FOUND, その他→UNEXPECTED_API_ERROR。
- ページネーション `fetchWithPages`: 既定 pageSize 100 / maxPages 10 (最大1000件)、`{results, partialResults}` を返す。**例外**: Service Usage (list-apis) は pageSize 200 / maxResults 10000。
- 未知コマンド: `program.ts:188` のメッセージは **ハードコードで `clasp` と表示** される (`Unknown command "clasp {command}"`)。crsp は `crsp` に置換 (§5 #1)。usageエラーも exit 1。

### 2.2 REST エンドポイント一覧 (crsp/api/ の実装対象)

| エンドポイント | メソッド | 用途 |
|---|---|---|
| `https://script.googleapis.com/v1/projects` | POST | create-script |
| `.../v1/projects/{scriptId}/content` | GET (`?versionNumber=N`) | push(差分比較), pull, clone, create, run(関数一覧) |
| `.../v1/projects/{scriptId}/content` | PUT (body `{files:[{name,type,source}]}`) | push (フル置換) |
| `.../v1/projects/{scriptId}/versions` | POST / GET | create-version, deploy(暗黙作成), list-versions |
| `.../v1/projects/{scriptId}/deployments` | POST / GET | create-deployment, list, open-web-app, undeploy |
| `.../v1/projects/{scriptId}/deployments/{depId}` | GET / PUT / DELETE | open-web-app(entryPoints), redeploy, undeploy |
| `https://script.googleapis.com/v1/scripts/{scriptId}/run` | POST (`{function, parameters, devMode}`) | run-function |
| `https://www.googleapis.com/drive/v3/files` | GET (`q: mimeType="application/vnd.google-apps.script"`) | list, clone/create 対話リスト |
| `https://www.googleapis.com/drive/v3/files` | POST (multipart upload) | create-script (コンテナ作成) |
| `https://www.googleapis.com/drive/v3/files/{fileId}` | PATCH (`{trashed: true}`) | delete-script |
| `https://serviceusage.googleapis.com/v1/projects/{p}/services?filter=state:ENABLED&pageSize=200` | GET | list-apis (pagination: pageSize 200 / maxResults 10000) |
| `https://serviceusage.googleapis.com/v1/projects/{p}/services/{api}.googleapis.com:enable` / `:disable` | POST | enable/disable-api |
| `https://discovery.googleapis.com/discovery/v1/apis?preferred=true` | GET | list-apis (available一覧) |
| `https://logging.googleapis.com/v2/entries:list` | POST | tail-logs |
| `https://www.googleapis.com/oauth2/v2/userinfo` | GET | login, show-authorized-user, open系ユーザーヒント |
| `https://oauth2.googleapis.com/token` (+ `accounts.google.com/o/oauth2/v2/auth`) | POST | 認可コード交換 / トークンリフレッシュ |

### 2.3 ローカルファイル

**`.clasp.json` (プロジェクト設定)**
- 発見: `-P` がパス/ディレクトリ指定ならそこから、なければ cwd から find-up。
- パースは **JSON5** (コメント・シングルクォート許容)。書き込みはプレーン JSON 2スペース。
- キー: `scriptId`, `projectId`, `parentId` (配列なら先頭), `srcDir`|`rootDir` → contentDir (プロジェクトルート外はエラー), `filePushOrder`, `fileExtension`(レガシー), `scriptExtensions` (既定 `[".js",".gs"]`), `htmlExtensions` (既定 `[".html"]`), `jsonExtensions` (既定 `[".json"]`), `skipSubdirectories`, `allowSymlinks`。
- **空文字列のedge**: `srcDir`/`rootDir` が空文字列 `""` の場合は未設定扱い (falsy) → contentDir = プロジェクトルート (`clasp.ts:208` の `config.srcDir || config.rootDir || '.'` 互換)。
- `updateSettings` (clone/create/MCP create/clone 時の書き込み) は `scriptId`, `rootDir` (=srcDir, 常に書く), `parentId`, `projectId`, `scriptExtensions`, `htmlExtensions`, `jsonExtensions`, `filePushOrder: []` (常に空配列リセット), `skipSubdirectories` を対象とする。**値が `undefined` のキーは省略される** (`JSON.stringify` 挙動、`project.ts:447-459`) — 実質: `parentId`/`projectId` は未設定時キーごと欠落、それ以外は常に出力。

**`.claspignore`**
- プロジェクトルート (または `-I`)。BOM除去→行分割。
- 評価は `micromatch.not(files, patterns, {dot: true})` — **この戻り値が「追跡対象ファイル」** (実測確認済み: `files.ts:129`)。意味論: `match = (いずれかの肯定パターンに一致) かつ (いずれの否定 `!` パターンにも一致しない)`、**tracked = 全ファイル集合から match集合を差し引いた補集合**。
  - 実測例: files=`{a.js, x.txt, appsscript.json, .git/config.js, node_modules/f.js, sub/Code.gs}`, patterns=既定 → `match = {x.txt, .git/config.js, node_modules/f.js}` (無視対象), **`not = {a.js, appsscript.json, sub/Code.gs}` (追跡対象)**。
  - crsp は同一アルゴリズム (match計算→補集合) を `ignore.rs` に実装する。
- 既定値 (不在時): `**/**`, `!**/appsscript.json`, `!**/*.gs`, `!**/*.js`, `!**/*.ts`, `!**/*.html`, `.git/**`, `node_modules/**`。

**`.clasprc.json`** (既定 `~/.clasprc.json`)
- V3形式: `{"tokens": {"<user>": {client_id, client_secret, refresh_token, access_token, expiry_date, type: "authorized_user"}}}`
- レガシー読み込み対応: V1ローカル (`token` + `oauth2ClientSettings`)、V1グローバル (トップレベル `access_token`、`exprity_date` スペルミスも再現)。**V1 フォールバックは `user === 'default'` のときのみ** 実行 (`file_credential_store.ts:144`)。
- `logout` は当該ユーザーのトークンを削除。**V1 レガシートップレベルキーの掃除は `default` ユーザー削除時のみ** (`file_credential_store.ts:96-112`)。
- 書き込み: chmod 0600 + `O_NOFOLLOW` (symlink防御) の **直接書き込み** (temp→renameではない)。トークンリフレッシュで自動再保存。
- Windows: 0600/O_NOFOLLOW は POSIX 専用 → ベストエフォート (chmodはno-op相当、symlink防御は書き込み前の `symlink_metadata` 事前チェックのみ)。
- デフォルト OAuth クライアント (claspと同一、既存トークン互換):
  - ID: `1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com`
  - Secret: `v6V3fKV_zWU7iw1DrpO1rknX` (公開前提のGoogle提供デスクトップクライアント)
  - redirect: `http://localhost`
  - クライアント分類: ID一致 → `google-provided`、それ以外 → `user-provided`

**`appsscript.json`** — contentDir に置かれるマニフェスト。push/pull では `type: JSON`・リモート名 `appsscript`。enable/disable-api は `dependencies.enabledAdvancedServices` (`userSymbol`, `serviceId`, `version`) を読み書き (キー順保持)。

### 2.4 認証フロー

- OAuth2 Authorization Code + **PKCE (S256)**、`access_type=offline`、state は 256bit ランダム (base64url 32バイト)。
- デフォルト = **localhost flow**: ローカルHTTPサーバー (`--redirect-port` 未指定ならランダムポート) でリダイレクトを受け、ブラウザ自動起動。
- `--no-localhost` = serverless flow: redirect URI `http://localhost:8888`、認証後のURLをユーザーがコピペ。
- `--creds <file>`: ユーザー提供クライアントJSON。localhost redirect URI を含むこと必須。
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
- `--use-project-scopes`: マニフェスト `oauthScopes` を使用 (マニフェスト未設定時は既定スコープにフォールバック)。`--include-clasp-scopes` は `--use-project-scopes` と併用必須 (単独使用はエラー)。`--extra-scopes`: カンマ区切り、空要素でエラー。
- ADC (`--adc`): Application Default Credentials。
- `--redirect-port`: 0–65535 の整数検証。

### 2.5 コマンドサーフェス (29コマンド・正規名/エイリアス)

正規コマンド名は長形式 (claspのファイル名と一致)。短形式は **エイリアス** として同一コマンドに登録される (`help` 表示・`Unknown command` 判定は正規名基準)。

| # | 正規名 | エイリアス | 引数 | 主要オプション | 概要 |
|---|---|---|---|---|---|
| 1 | `login` | — | なし | `--no-localhost`, `--creds <f>`, `--use-project-scopes`, `--include-clasp-scopes`, `--extra-scopes <s>`, `--redirect-port <n>` | OAuth認証して `.clasprc.json` 保存 |
| 2 | `logout` | — | なし | なし | 現ユーザーkeyのトークン削除 (`default` のみV1掃除)。未ログインでも成功 |
| 3 | `show-authorized-user` | — | なし | なし | ログイン状態・email・client ID・種別表示 |
| 4 | `clone-script` | `clone` | `[scriptId] [versionNumber]` | `--rootDir <dir>` | URLからID抽出 (`https://script.google.com/d/([^/]+)/`)。対話: Drive一覧選択。pull後 `.clasp.json` 作成 |
| 5 | `create-script` | `create` | なし | `--type <t>` (既定 `standalone`), `--title <t>` (既定: cwd名humanize), `--parentId <id>`, `--rootDir <dir>` | container型 (docs/forms/sheets/slides) はDrive作成→bound script。webapp/api はdeployment tip表示 |
| 6 | `push` | — | なし | `-f/--force`, `-w/--watch` | 差分なし→`Script is already up to date.`、あり→全ファイル含むフル置換PUT。マニフェスト変更時confirm (非interactive=拒否)。watchは500ms debounce |
| 7 | `pull` | — | なし | `--versionNumber <n>`, `-d/--deleteUnusedFiles`, `-f/--force` | content取得→直接書き込み (§2.6)。`-d` は対話確認後削除 |
| 8 | `create-deployment` | `deploy` | なし | `-V/--versionNumber`, `-d/--description`, `-i/--deploymentId` | version未指定なら暗黙作成。`-i` 指定なら更新 |
| 9 | `update-deployment` | `redeploy` | `<deploymentId>` (必須) | `-V`, `-d`, `--json` (ローカル定義・グローバルと同義) | 既存deployment更新 |
| 10 | `delete-deployment` | `undeploy` | `[deploymentId]` | `-a/--all` | versionedのみ対象。1件自動選択、複数は対話 |
| 11 | `delete-script` | `delete` | `[scriptId]` | `-f/--force` | Drive trash。confirm必須 (非interactive+`-f`なし=サイレントno-op) |
| 12 | `create-version` | `version` | `[description]` | なし | version作成 (対話入力フォールバック) |
| 13 | `list-versions` | `versions` | `[scriptId]` | なし | 逆順リスト `N - desc` |
| 14 | `list-deployments` | `deployments` | `[scriptId]` | なし | `- {id} @{v|HEAD} {- desc}` |
| 15 | `list-scripts` | `list` | なし | `--noShorten` | Driveスクリプト一覧 (既定20文字truncate) |
| 16 | `run-function` | `run` | `[functionName]` | `--nondev`, `-p/--params <json>` | `scripts/{id}/run`。devMode既定true。関数名省略時fuzzy選択。`NOT_AUTHORIZED`→API実行可能deploy注意喚起、`NOT_FOUND`→関数不在メッセージ |
| 17 | `tail-logs` | `logs` | なし | `--watch`, `--simplified` | Cloud Logging取得。insertId重複排除。watchは6000msポーリング |
| 18 | `setup-logs` | — | なし | なし | projectId確認・対話設定のみ (API呼び出しなし) |
| 19 | `show-file-status` | `status` | なし | なし | push対象/未追跡ファイル一覧 (未追跡は共通親ディレクトリに圧縮) |
| 20 | `list-apis` | `apis` | なし | なし | enabled (Service Usage) / available (Discovery) を Advanced Services 31種でフィルタ |
| 21 | `enable-api` | — | `<api>` | なし | マニフェスト更新 + Service Usage有効化 |
| 22 | `disable-api` | — | `<api>` | なし | マニフェスト更新 + Service Usage無効化 |
| 23 | `open-script` | — | `[scriptId]` | なし | `https://script.google.com/d/{id}/edit` |
| 24 | `open-container` | — | なし | なし | `parentId` 必須 → `https://drive.google.com/open?id={id}` |
| 25 | `open-web-app` | — | `[deploymentId]` | なし | deployments から WEB_APP entry point のURL取得 |
| 26 | `open-logs` | — | なし | なし | Cloud Console logs viewer |
| 27 | `open-api-console` | — | なし | なし | GCP APIs dashboard |
| 28 | `open-credentials-setup` | — | なし | なし | GCP credentials ページ |
| 29 | `start-mcp-server` | `mcp` | なし | なし | MCP stdio サーバー起動 |

open系共通: `CLASP_ENABLE_USER_HINTS=true|1` のとき `?authUser=<userId>` を付与 (userinfo取得必要)。表示は `Opening {url} in your browser.`。

### 2.6 ファイルパイプラインとセキュリティ機構 (最難所・移植必須)

**push (読み取り側) の skip reason コード** (`files.ts:61`): `symlink`, `unsupported_type`

- 走査: contentDir を再帰走査 (`skipSubdirectories` なら depth 0)。
- `unsupported_type`: 通常ファイル以外 (ディレクトリ・symlink以外の特殊ファイル) は skip+警告。
- symlink (`allowSymlinks=false`): **skip+警告**。contentDir 自体が symlink の場合も同様。
- **`allowSymlinks=true`**: symlink を **follow する** (`crawlLogicalSymlinks`, `files.ts:116-117`)。書き込み側も `O_NOFOLLOW` を省略。
- SERVER_JS名衝突: 同一basenameの `.js`/`.gs` 併存でエラー。
- ソート: `filePushOrder` 優先 → アルファベット (`localeCompare` 相当)。
- 差分計算: リモートHEAD (`GET content`) と比較。差分なし → PUTせず `Script is already up to date.`。あり → **全ファイルを含むフル置換PUT**。
- マニフェスト変更 & `--force` 無し → confirm (非interactive=拒否)。
- 構文エラーパース: `Syntax error: {msg} line: {n} file: {name}` を抽出しコードスニペット表示。
- contentDir はプロジェクトルート内に制限 (脱出禁止)。

**pull (書き込み側) の skip reason コード** (`files.ts:53`): `outside_content_dir`, `parent_symlink`, `target_symlink`, `race_condition`, `symlink_loop`

- 書き込み = **直接書き込み** (`O_WRONLY|O_CREAT|O_TRUNC` + `allowSymlinks=false` 時は `O_NOFOLLOW`、mode 0644、`files.ts:765-778`)。**temp→rename ではない**。
- `race_condition`: open時の `EEXIST` → skip。`symlink_loop`: `ELOOP` → skip。
- 親ディレクトリ: mkdir -p (親のsymlinkチェック込み) + realpath照合で `parent_symlink` 検出 (`files.ts:723,740`)。
- リモート由来パスの jail (`isInside`): contentDir 外脱出禁止 → `outside_content_dir`。
- contentDir 自体: `lstat` でsymlink確認 (allowSymlinks=false時) + realpath取得 (`files.ts:679-689`)。
- `-d/--deleteUnusedFiles`: リモートに無いローカルファイルを削除 (`-f` なしなら対話確認、非interactiveならスキップ警告)。削除対象も jail チェック。

**watch (`push -w`)**: 500ms debounce、`.claspignore` フィルタ、起動時 `Waiting for changes...`、マニフェスト拒否でwatch終了。

**並列度**: 書き込みは `pMap(files, mapper)` = **並列数無制限** (`files.ts:793`、p-mapのデフォルト)。crsp も無制限で再現 (§1.2 レイヤA: 動作互換)。

### 2.7 MCP サーバー (crsp契約 — clasp参照値は明記)

- transport: **stdio** のみ。**crspのサーバー名は `Crsp`** (clasp実装は `Clasp`, `server.ts:73` — §5 #2で変更)。バージョン = CLIバージョン。
- ツール5つと annotations (claspでは全ツール共通 `openWorldHint: false, destructiveHint: true, idempotentHint: false, readOnlyHint: false`。crspは `list_projects` のみ修正 — §5 #10):

| ツール | 入力 | 出力 (text + structuredContent) |
|---|---|---|
| `push_files` | `projectDir` (必須) | text: 成功文言 + `Updated file: {abs path}` ×N。structured: `{scriptId, projectDir, files[]}`。**crspは非標準フィールド `status: 'success'` を廃止** (§5 #4) |
| `pull_files` | `projectDir` (必須) | 同構造。claspは文言バグ (`Pushed project...` と成功表示 `server.ts:226`、`Error cloning project` とエラー表示 `server.ts:244`) → **crspは修正** (§5 #3) |
| `create_project` | `projectDir` (必須), `sourceDir?`, `projectName?` | `mkdir` → `createScript` → 初期pull → `updateSettings`。structured: `{scriptId, projectDir, files[]}` |
| `clone_project` | `projectDir` (必須), `sourceDir?`, `scriptId?` (**実質必須**: 省略時 `Script ID is required.` エラー `server.ts:412-419`) | pull → `updateSettings`。claspはtitle誤り → crspは修正 (§5 #3) |
| `list_projects` | **なし (空オブジェクト `{}`)** | text: `Found N ...` + `{name} ({id})`。structured: `{scripts: [{scriptId, name}]}` |

- パスjail `validateProjectDir`: resolve後、**homedir または cwd 配下のみ許可** (`Security Error: projectDir must be within the user home directory or current working directory. Resolved path "X" is not permitted.`)。
- **`sourceDir` の参照実装における不整合** (claspの現状): ①検証は生パスを `validateProjectDir` (homedir/cwd jail) に通すだけ (`server.ts:315-321`)、②実際のcontentDir解決は `withContentDir` が **`projectRootDir` 相対** で行う (`clasp.ts:129-141`)。検証基準 (cwd) と読み書き基準 (projectDir) が一致していない。
  - **crspの方針 (§5 #5)**: `sourceDir` は **`projectDir` 相対** で解決し、解決後のパスが **`projectDir` 配下** であることをjailとして検証する。ツール説明文もこれに一致させる。
- ツール呼び出しごとに `Clasp` インスタンスを新規構築 (ステートレス)。

## 3. crsp アーキテクチャ

```
crsp/
├── Cargo.toml               # シングルクレート
├── src/
│   ├── main.rs              # エントリ: clapパース → auth/Clasp初期化 (preAction相当) → 実行 → エラー処理
│   ├── cli.rs               # clap derive: グローバルオプション + 29コマンド (正規名/エイリアス完全一致)
│   ├── constants.rs         # PROJECT_NAME="crsp", マニフェスト名, 既定OAuthクライアント
│   ├── i18n.rs              # claspと同一の英語メッセージ定数 (§5差分のみ変更)
│   ├── commands/            # 1コマンド1ファイル (clasp 1:1)
│   │   ├── shared.rs        # assertScriptConfigured / assertGcpProjectConfigured / ellipsize / openUrl / spinner
│   │   └── (29 files)
│   ├── core/                # ドメインロジック (UI非依存)
│   │   ├── clasp.rs         # Clasp: config + credentials + APIクライアントを束ねる実行コンテキスト
│   │   ├── project.rs       # createScript / listScripts / updateSettings (.clasp.json書込)
│   │   ├── files.rs         # push/pullパイプライン + セキュリティ機構
│   │   ├── manifest.rs      # appsscript.json 読書 (serde_json::Value, キー順保持)
│   │   ├── ignore.rs        # micromatch互換マッチャー (match→補集合, dot有効)
│   │   └── pagination.rs    # fetchWithPages (既定100/10, ServiceUsage例外200/10000)
│   ├── auth/
│   │   ├── credential_store.rs  # .clasprc.json 読書 (V1/V3互換, 0600, O_NOFOLLOW直接書き込み)
│   │   ├── oauth_client.rs      # リフレッシュ, クライアント分類
│   │   ├── flow.rs              # PKCE(S256) + state + auth URL
│   │   ├── localhost_flow.rs    # tiny_http ローカルサーバー
│   │   └── serverless_flow.rs   # URLコピペ
│   ├── api/
│   │   ├── script.rs / drive.rs / service_usage.rs / logging.rs / oauth2.rs / discovery.rs
│   │   └── error.rs         # ApiErrorKind + リトライ (§7.1)
│   └── mcp/server.rs        # rmcp stdio, 5ツール
└── tests/                   # 統合テスト (§9)
```

### 3.1 技術選定

| 用途 | クレート | 備考 |
|---|---|---|
| CLI | `clap 4` (derive) | usageエラーも exit 1 に統一 (commander互換) |
| async | `tokio` (full) + `futures` | 並列書き込み・watch・MCP |
| HTTP | `reqwest` + `rustls` | openssl回避の静的リンク |
| JSON | `serde` + `serde_json` | 書き出し。`Value` でキー順保持 (`preserve_order` feature) |
| JSON5読込 | `json5` | `.clasp.json` 読み込みのみ |
| ignore | `globset` + 自前match→補集合 | micromatch互換 (§2.3) |
| ファイル走査 | `walkdir` | skipSubdirectories は depth 0 |
| watch | `notify` | FSEvents/inotify + 500ms debounce |
| 対話UI | `dialoguer` (`fuzzy-select` feature → `fuzzy-matcher`), `indicatif` | spinner は TTY 時のみ |
| ブラウザ | `open` | |
| ホームディレクトリ | `home` | 公式メンテ (cargo使用) |
| OAuth暗号 | `rand` + `sha2` + `base64` | PKCE S256, state 256bit |
| URL/文字列処理 | `url`, `regex`, `inflector` | clone時のURL抽出、create-script既定titleのhumanize (inflection互換) |
| 時刻 | `time` | expiry_date (epoch ms), ログISO表示 |
| ログ | `tracing` + `tracing-subscriber` (env-filter) | claspの `Debug('clasp:*')` 相当 (`CRSP_DEBUG` または `RUST_LOG`) |
| localhost redirect | `tiny_http` | 1ルートのみで十分 (axumより軽量) |
| MCP | `rmcp` | stdio transport |
| エラー | `thiserror` + `anyhow` | mainでの表示用 |
| 表示幅 | `unicode-width` | ellipsize |
| テスト | `wiremock`, `assert_cmd`, `predicates`, `tempfile` | §9 |

### 3.2 実行フロー

1. `main()`: `#[tokio::main]`、clapパース。
2. preAction相当: グローバルオプションから `auth::init()` (credential store読み込み → 必要ならトークンリフレッシュ+再保存 or ADC) → `Clasp::init()` (`-P` または find-up で `.clasp.json` をJSON5パース → contentDir検証 → ignore matcher構築)。
3. コマンドハンドラへ `Clasp` (Arc) を注入して実行。
4. エラーはtyped error → mainで exit code 1 + stderrにmessage。未知コマンド・usageエラーも exit 1。

### 3.3 並行性

- pull の並列書き込み: **無制限並列** (clasp の pMap デフォルトと同一、§2.6)。`futures::stream` の `buffer_unordered(usize::MAX)` 相当。
- watch / MCP / logs polling は tokio 上のasyncタスク。
- ファイルIOは少量なので直接同期実行 (必要時に `spawn_blocking`)。

## 4. 重要な互換性ルール (実装時の契約)

1. `.clasp.json` は JSON5 で読み、プレーン JSON (2スペース) で書く。`updateSettings` は rootDir を常に書き、`filePushOrder` を空配列にリセット。**undefined値のキーは書き込み時に省略** (parentId/projectId は未設定時欠落)。
2. `.clasprc.json` は V3 形式で書き、V1 2形式は `default` ユーザーのみ読む。書き込みは0600 + O_NOFOLLOW 直接書き込み (POSIX)。Windows はベストエフォート。
3. 既定OAuthクライアントは clasp と同一の ID/secret (既存トークンがそのまま動く)。
4. スコープ既定10種は clasp と完全一致。
5. ignore 評価は `micromatch.not` 互換: match (肯定∪ − 否定) の**補集合**が追跡対象。dot有効。
6. ページネーション: 既定 pageSize=100/maxPages=10、ServiceUsageのみ pageSize=200/maxResults=10000。
7. push は差分なし→PUTせず `Script is already up to date.`、差分あり→全ファイル含むフル置換PUT。
8. pull 書き込みは直接書き込み (O_NOFOLLOW/0644、temp→renameではない) + symlink/jail 防御。
9. 非interactive時のフォールバック: push manifest確認=拒否 / pull `-d` 削除=スキップ+警告 / delete=サイレントno-op。
10. `--json` 出力契約は §6.2。open系はJSONモードでも `Opening {url}...` 行を表示 (clasp互換の意図的維持)。
11. ログポーリング6000ms、watch debounce 500ms。
12. マニフェスト名はリモートから `appsscript` (拡張子なし)、ローカルは `appsscript.json`。
13. 書き込み並列は無制限 (claspのpMapデフォルト互換)。

## 5. crsp における意図的な差分 (修正一覧・出力契約付き)

| # | 対象 | clasp の挙動 | crsp の挙動 | 期待される出力/exit code |
|---|---|---|---|---|
| 1 | バイナリ名表示 | メッセージに `clasp` をハードコード (`Unknown command "clasp {command}"`, `program.name`) | `crsp` を表示 (`Unknown command "crsp {command}"`) | 文案のみ変更、exit 1 は同一 |
| 2 | MCPサーバー名 | `Clasp` | `Crsp` | ツール名・スキーマ・annotations は不変 |
| 3 | MCP文言バグ | `pull_files` 成功文が `Pushed project...`、エラー文が `Error cloning project`、`clone_project` のtitle誤り | `pull_files` 成功 `Pulled project in {dir} to local filesystem successfully.`、エラー `Error pulling project`、title `Clone Apps Script project` | text内容のみ変更、structuredContent/エラー形式は不変 |
| 4 | MCP `push_files` の `status: 'success'` | 非標準フィールド付与 | 廃止 (標準JSON-RPC envelopeに準拠) | structuredContent から除外 |
| 5 | MCP `sourceDir` の解決 | 検証が生パスの homedir/cwd jail、実際の解決は projectRootDir 相対 — 2段階で基準不一致 (`server.ts:315-321` + `clasp.ts:129-141`) | **`sourceDir` を `projectDir` 相対で解決し、解決後パスが `projectDir` 配下であることを検証** (仕様・検証・実装を一致) | 絶対パス指定は同等動作。相対パス指定の挙動が変わる (projectDir基準に統一)。エラー時 exit/エラー形式は不変 |
| 6 | tail-logs デバッグ出力 | `console.log('PAST', projectId)` を毎回出力 | 削除 | 出力1行減、exit code 不変 |
| 7 | `status` のJSON | 単一行 | 2スペース pretty (他コマンドと統一) | JSONパース結果は同一 |
| 8 | `delete` の `--json` | 無視 | `{"success": true}` を出力 | 新規出力、exit code 不変 |
| 9 | `pull -d` フラグ定義 | 末尾スペース付き `'-d, --deleteUnusedFiles '` | 修正 | フラグ名は同一のため動作差異なし |
| 10 | `list_projects` の annotations | `destructiveHint: true, readOnlyHint: false` (誤り) | `destructiveHint: false, readOnlyHint: true` | 読み取り専用ツールの正しい宣言 |

上記以外はレイヤA/B契約 (§1.2) に従い clasp と同一。

## 6. エラー処理・出力契約

### 6.1 エラー階層

- `crsp::Error` (thiserror): `Api(ApiErrorKind, message)`, `Config`, `Auth`, `Io`, `Validation`, `Aborted` (ユーザー拒否)。
- APIエラーはHTTP statusから `ApiErrorKind` へ正規化し、コマンドはkindで分岐 (例: run-function → NotAuthorized で「API executable deploy」注意喚起、NotFound で関数不在メッセージ)。
- exit code: 成功0、エラー1、usageエラー1 (clapの既定2を1に上書き — commander互換)。

### 6.2 `--json` 出力マトリクス (コマンド別契約・出所参照付き)

| コマンド | `--json` 出力 | clasp出所 |
|---|---|---|
| login | `{"email": "..."}` (2sp) | `commands/login.ts` |
| logout | `{"success": true}` (トークン有無で同一) | `commands/logout.ts` |
| show-authorized-user | `{"loggedIn", "email", "clientId", "clientType"}` (未ログイン時 `{"loggedIn": false}`) | `commands/show-authorized-user.ts` |
| clone-script | `{"scriptId", "files": [...]}` | `commands/clone-script.ts` |
| create-script | `{"scriptId", "parentId", "files": [...]}` (parentId null 可) | `commands/create-script.ts` |
| push | `[path, ...]` (2sp, watchループ毎に出力) | `commands/push.ts` |
| pull | `{"pulledFiles": [...], "deletedFiles": [...]}` | `commands/pull.ts` |
| create-deployment | `{"deploymentId", "versionNumber", "description"}` | `commands/create-deployment.ts` |
| update-deployment | 同上 (ローカル `--json` も同義) | `commands/update-deployment.ts` |
| delete-deployment | `{"deletedDeploymentIds": [...]}` (なし時 `[]`) | `commands/delete-deployment.ts` |
| create-version | `{"versionNumber": N}` | `commands/create-version.ts` |
| list-versions | `[{"versionNumber", "description"}]` | `commands/list-versions.ts` |
| list-deployments | `[{"deploymentId", "versionNumber", "description"}]` | `commands/list-deployments.ts` |
| list-scripts | `[{"id", "name"}]` | `commands/list-scripts.ts:42-48` |
| run-function | `{"response": <value>, "error": {...}?}` (errorは失敗時のみ) | `commands/run-function.ts` |
| show-file-status | `{"filesToPush": [...], "untrackedFiles": [...]}` (crspでは pretty — §5 #7) | `commands/show-file-status.ts` |
| list-apis | `{"enabledApis": [...], "availableApis": [...]}` | `commands/list-apis.ts` |
| enable-api | `{"success": true}` | `commands/enable-api.ts` |
| disable-api | `{"success": true, "disabledService": "..."}` | `commands/disable-api.ts` |
| setup-logs | `{"success": true}` | `commands/setup-logs.ts` |
| delete-script | **crsp追加: `{"success": true}`** (claspは出力なし — §5 #8) | `commands/delete-script.ts` |
| open-* | `{"url": "..."}` + stdoutに行表示: ブラウザあり `Opening {url} in your browser.` / なし `Open {url} in your browser to continue.` (clasp互換維持 — `commands/utils.ts:180-202`) | `commands/open-*.ts`, `commands/utils.ts` |
| tail-logs | **JSON payload整形行** (clasp同一): `JSON.stringify(entry, null, 2)` をpayload列に埋め込んだ `severity 時刻 関数名 payload` 行 (§純JSONではない — 完全互換を採用)。--json時はpayload不在のエントリも出力可 | `commands/tail-logs.ts:122-149` |
| start-mcp-server | MCPプロトコル (CLI出力なし) | `commands/start-mcp.ts` |

JSON 純度の例外 (open系) は「レイヤA: 出力文言互換」を優先し、quirk として保守する。

## 7. API失敗・再試行・認証同時実行契約

### 7.1 再試行 (gaxios 7.3.1 デフォルト互換 — 実パッケージソースで確認済み)

claspは gaxios 7.3.1 (package-lock解決済み) のデフォルトリトライ設定で動作する。crspは同一契約を再現し、§9.2 golden testで固定する:

- **有効化**: デフォルトON (`retry: true`)。
- **対象メソッド**: `GET, HEAD, PUT, OPTIONS, DELETE` — **POST は対象外** (create version/deployment等の非冪等POSTは再試行されない → 重複作成リスクなし)。
- **対象ステータス**: `100–199`, `408`, `429`, `500–599`。ネットワーク層エラー (応答なし: ETIMEDOUT/ENOTFOUND等) は最大 **2回** (`noResponseRetries`)。
- **リトライ回数**: 最大 **3回** (`retry: 3`、初回+3で最大4リクエスト)。
- **遅延**: 初回リトライ `retryDelay` (既定 **100ms**)、以降 `100 + ((2^n − 1)/2) × 1000 ms` (n=currentRetryAttempt、`retryDelayMultiplier: 2`)。上限は `maxRetryDelay` (既定実質無制限) と totalTimeout 残時間 (`totalTimeout` 既定実質無制限)。`Retry-After` ヘッダは **考慮されない**。
- 中断 (abort/signal): 再試行なし。

### 7.2 タイムアウト

- リクエストタイムアウト 30s、接続タイムアウト 10s (reqwest設定)。タイムアウトはネットワークエラー扱い (応答なしなら再試行対象、§7.1)。
- localhost認証サーバーはタイムアウトなし (ユーザー操作待ち、Ctrl+Cまで)。

### 7.3 ページング失敗と部分結果 (`core/utils.ts:186-214` 準拠)

- **limit到達による部分結果** (正常系): `nextPageToken` が残ったまま `maxPages` (既定10) または `maxResults` に到達 → 取得済み結果を `partialResults: true` で返し、警告表示。**exit 0**。
- **HTTP失敗**: 途中ページの取得例外は捕捉**しない** → エラーとして伝播し、**exit 1**。部分結果は返さない。
- **maxResults超過トリミング**: 取得件数が `maxResults` を超えた場合は切り詰めて `partialResults: true` (正常系)。

### 7.4 トークンリフレッシュ

- 401/`invalid_grant` 時: リフレッシュ → 1回だけ再試行。
- リフレッシュ失敗: `Auth` エラー (`NOT_AUTHENTICATED`)、トークンを**上書きしない**、`crsp login` を促すメッセージ。
- リフレッシュ成功: `.clasprc.json` 再保存 (0600/O_NOFOLLOW)。並行プロセスでの同時リフレッシュは **last-write-wins、lockなし** (clasp互換)。POSIXでのchmod失敗時は警告を出し保存は続行。

## 8. パス・ディレクトリモデル (jail判定の基準)

絶対パス例で固定する。例: cwd=`/home/alice/work/app`

```
/home/alice/work/app/.clasp.json   ← プロジェクトルート (find-up または -P で発見)
/home/alice/work/app/src/          ← contentDir (rootDir/srcDir = "src" の場合)
/home/alice/.clasprc.json          ← 認証情報 (既定)
```

- **プロジェクトルート** = `.clasp.json` を発見したディレクトリ。`.claspignore` はここに置く。
- **contentDir** = プロジェクトルート + `srcDir|rootDir` (未設定・空文字列ならルート自身)。**jail判定はcontentDirをrealpath解決した値に対して** 行う (ファイル読書き・削除・リモート由来パスすべて)。CLIの `--rootDir` も同様にプロジェクトルート相対で解決 (`withContentDir`, `clasp.ts:129-141`)。
- **認証 jail**: `.clasprc.json` のパス (または `-A` 指定先) への symlink 防御は `O_NOFOLLOW` + 事前 `symlink_metadata`。
- **MCP jail**: `projectDir` = `resolve(与えられたパス)` が homedir 自身または homedir 配下、もしくは cwd 自身または cwd 配下であること。`sourceDir` = **`projectDir` 相対** で解決し、解決後パスが `projectDir` 配下であること (§5 #5 — claspの2段階基準不一致を統一)。
- **OS差異**: macOS/Windows の大文字小文字非区別 FS 上では、大文字小文字違いの衝突 (`a.js` vs `A.js`) は検出しない (claspと同様、既知の制限として文書化)。Windows の symlink 保護は POSIX 機構が無いためベストエフォート (§2.3)。

## 9. テスト戦略 (golden test 形式 + live test 分離)

### 9.1 レイヤ別

| レイヤ | 手法 |
|---|---|
| core unit | ignoreマッチャー・config発見・manifest・衝突検出・pagination・jail判定を網羅 (純粋関数) |
| APIクライアント | `wiremock` でエンドポイントモック。トークンリフレッシュ・401再試行・リトライ契約 (§7.1: メソッド/ステータス/遅延系列/noResponseRetries) 含む |
| CLI統合 | `assert_cmd` + `tempfile` 仮想プロジェクト |
| MCP | rmcp クライアントで5ツールのschema/出力検証 |

### 9.2 Golden test 形式 (参照実装との同一性検証)

各golden caseは以下を **ファイルとして固定** し、CIで機械比較する:

1. `fixture/` — 入力: `.clasp.json` / `.clasprc.json` / `.claspignore` / ソースファイルツリー (実claspで生成した実物)
2. `mock-transcript.toml` — wiremockに流す HTTP リクエスト/レスポンス ペア (実claspが発するリクエストを記録)。**リトライ挙動もここに固定** (§7.1の gaxios互換契約: 対象メソッド/ステータス/遅延系列を含む)
3. `expected.json` — 期待値: stdout/stderr (§5差分部分はマスク付き)、exit code、書き換え後のファイルツリースナップショット、APIリクエストbody

比較方法: crsp実行結果を expected と突き合わせ。`Unknown command` 等の `crsp` 文字列置換差分 (§5 #1) は expected に crsp 版を直接記載。

### 9.3 Live test (分離)

- 実Google API を使うテストは別バイナリ/別フィルタに分離し、**`CRSP_LIVE_TEST=1` + 実credentials 存在時のみ実行** (CIではskip、ローカルでopt-in)。
- 対象: login E2E (手動確認)、push/pull/deploy 往復、logs 読取。

### 9.4 セキュリティ回帰テスト (重点)

§2.6 の全 skip reason コード (`symlink`, `unsupported_type`, `outside_content_dir`, `parent_symlink`, `target_symlink`, `race_condition`, `symlink_loop`) と、0600/O_NOFOLLOW、SERVER_JS衝突、MCP jail に対し「状態 → 期待動作 (拒否/警告+スキップ/許可) → アサート対象 (exit code, stderr文言, ファイル状態)」を1テストずつ定義する。

## 10. 実装順序 (リスク降順)

1. 基盤: constants, config (.clasp.json JSON5), credential_store (.clasprc.json V1/V3), OAuth flow (login/logout/show-authorized-user)
2. files: ignoreマッチャー, push/pull, セキュリティ機構 ← 最難所
3. script APIコマンド: clone/create/deploy系/list系/run/version/versions/deployments/undeploy/delete
4. 周辺: open系/status/setup-logs/list-apis/enable/disable/logs/setup-logs
5. MCP (rmcp stdio, 5ツール)
6. 仕上げ: watch, fuzzy picker, メッセージ文言のclasp完全一致監査, リリース

## 11. 成功基準 (検証可能な形)

1. **§9.2 golden test** が全パス (実claspのfixture + 実測HTTPトランスクリプト対 crspの実行結果)。
2. 実claspのプロジェクトディレクトリに対し、crspの `pull` / `push` / `deployments` / `run` が同一exit code・同一stdout/stderr (§5差分除く)・同一ファイル状態で動作する (golden testで機械保証)。
3. `.clasprc.json` に clasp で login したトークンがある状態で crsp が再ログインなしで動く (golden fixtureで検証)。
4. `crsp push --watch`・`crsp logs --watch` が clasp と同等の挙動 (タイマー値・文言をunit testで固定)。
5. `crsp mcp` が MCP Inspector で5ツール動作確認できる (MCP統合テスト)。
6. §9.4 セキュリティ回帰テスト全グリーン。
