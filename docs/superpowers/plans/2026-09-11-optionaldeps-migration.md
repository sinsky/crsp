# optionalDependencies Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** postinstallダウンロード方式を廃止し、platform package＋optionalDependencies方式へ移行する。

**Architecture:** `npm/<platform>/`配下に5 platform packageを同リポジトリ管理し、ルートはoptionalDependenciesで参照する。postinstallとscripts/install.jsを廃止する。

**Tech Stack:** npm optionalDependencies (os/cpu制約), Node.js >= 18, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-11-optionaldeps-migration-design.md`

## Global Constraints

- Node.js >= 18 (package.json enginesに準拠)
- ルートとplatform5のversionは常に同期する
- Linuxはmusl static target継続 (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`)
- 初回のみplatform5件は新規パッケージのため通常publishが必要

---

### Task 1: Platform package雛形5件

**Files:**
- Create: `npm/darwin-arm64/package.json`
- Create: `npm/darwin-x64/package.json`
- Create: `npm/linux-x64/package.json`
- Create: `npm/linux-arm64/package.json`
- Create: `npm/win32-x64/package.json`

**Interfaces:**
- Consumes: ルート`package.json`のversion (0.1.1想定、実装時に確認して同期)
- Produces: 5件のplatform package定義 (Task 2のoptionalDependencies参照先、Task 3の配置先)

- [ ] **Step 1: Create `npm/darwin-arm64/package.json`**

```json
{
  "name": "@sinsky-gh/crsp-darwin-arm64",
  "version": "0.1.1",
  "description": "crsp prebuilt binary for darwin-arm64",
  "os": ["darwin"],
  "cpu": ["arm64"],
  "files": ["bin"],
  "publishConfig": {
    "access": "public"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "license": "Apache-2.0"
}
```

- [ ] **Step 2: Create `npm/darwin-x64/package.json`**

```json
{
  "name": "@sinsky-gh/crsp-darwin-x64",
  "version": "0.1.1",
  "description": "crsp prebuilt binary for darwin-x64",
  "os": ["darwin"],
  "cpu": ["x64"],
  "files": ["bin"],
  "publishConfig": {
    "access": "public"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "license": "Apache-2.0"
}
```

- [ ] **Step 3: Create `npm/linux-x64/package.json`**

```json
{
  "name": "@sinsky-gh/crsp-linux-x64",
  "version": "0.1.1",
  "description": "crsp prebuilt binary for linux-x64",
  "os": ["linux"],
  "cpu": ["x64"],
  "files": ["bin"],
  "publishConfig": {
    "access": "public"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "license": "Apache-2.0"
}
```

- [ ] **Step 4: Create `npm/linux-arm64/package.json`**

```json
{
  "name": "@sinsky-gh/crsp-linux-arm64",
  "version": "0.1.1",
  "description": "crsp prebuilt binary for linux-arm64",
  "os": ["linux"],
  "cpu": ["arm64"],
  "files": ["bin"],
  "publishConfig": {
    "access": "public"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "license": "Apache-2.0"
}
```

- [ ] **Step 5: Create `npm/win32-x64/package.json`**

```json
{
  "name": "@sinsky-gh/crsp-win32-x64",
  "version": "0.1.1",
  "description": "crsp prebuilt binary for win32-x64",
  "os": ["win32"],
  "cpu": ["x64"],
  "files": ["bin"],
  "publishConfig": {
    "access": "public"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "license": "Apache-2.0"
}
```

- [ ] **Step 6: Verify versions match root**

Run: `node -e "const v=require('./package.json').version; const fs=require('fs'); for (const d of ['darwin-arm64','darwin-x64','linux-x64','linux-arm64','win32-x64']) { const p=JSON.parse(fs.readFileSync('npm/'+d+'/package.json','utf8')); if (p.version!==v) { console.error(d+': '+p.version+' != '+v); process.exit(1); } } console.log('versions OK: '+v)"`
Expected: `versions OK: <root version>`

- [ ] **Step 7: Commit**

```bash
git add npm/darwin-arm64/package.json npm/darwin-x64/package.json npm/linux-x64/package.json npm/linux-arm64/package.json npm/win32-x64/package.json
git commit -m "feat(npm): add platform package manifests"
```

### Task 2: ルートpackage.json切替＋postinstall廃止

**Files:**
- Modify: `package.json`
- Delete: `scripts/install.js`

**Interfaces:**
- Consumes: Task 1のplatform package名5件
- Produces: optionalDependencies参照つきルート定義 (Task 4の検証対象)

- [ ] **Step 1: Edit root package.json**

`package.json`の`scripts.postinstall`を削除し、`optionalDependencies`を追加し、`files`から`scripts`を除外する。`version`はTask 1と同一にする。

```json
{
  "name": "@sinsky-gh/crsp",
  "version": "0.1.1",
  "description": "crsp - The Apps Script CLI (Rust port of google/clasp)",
  "publishConfig": {
    "access": "public"
  },
  "bin": {
    "crsp": "bin/crsp.js"
  },
  "optionalDependencies": {
    "@sinsky-gh/crsp-darwin-arm64": "0.1.1",
    "@sinsky-gh/crsp-darwin-x64": "0.1.1",
    "@sinsky-gh/crsp-linux-x64": "0.1.1",
    "@sinsky-gh/crsp-linux-arm64": "0.1.1",
    "@sinsky-gh/crsp-win32-x64": "0.1.1"
  },
  "files": [
    "bin",
    "README.md",
    "LICENSE"
  ],
  "engines": {
    "node": ">=18"
  },
  "repository": {
    "type": "git",
    "url": "git+https://github.com/sinsky/crsp.git"
  },
  "keywords": [
    "google-apps-script",
    "apps-script",
    "clasp",
    "crsp",
    "cli"
  ],
  "author": "sinsky",
  "license": "Apache-2.0",
  "bugs": {
    "url": "https://github.com/sinsky/crsp/issues"
  },
  "homepage": "https://github.com/sinsky/crsp#readme"
}
```

- [ ] **Step 2: Delete scripts/install.js**

```bash
git rm scripts/install.js
```

- [ ] **Step 3: Verify postinstall is gone**

Run: `node -e "const p=require('./package.json'); if (p.scripts && p.scripts.postinstall) { process.exit(1); } console.log('postinstall removed OK')"`
Expected: `postinstall removed OK`

- [ ] **Step 4: Commit**

```bash
git add package.json
git commit -m "feat(npm): switch to optionalDependencies, drop postinstall"
```

### Task 3: binラッパー解決先変更

**Files:**
- Modify: `bin/crsp.js`

**Interfaces:**
- Consumes: Task 1のplatform package名規則 (`@sinsky-gh/crsp-<platform>/bin/crsp-bin`)
- Produces: optionalDependencies解決対応のラッパー (Task 4の動作確認対象)

- [ ] **Step 1: Rewrite bin/crsp.js**

```js
#!/usr/bin/env node

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const PLATFORM_PACKAGES = [
  'darwin-arm64',
  'darwin-x64',
  'linux-x64',
  'linux-arm64',
  'win32-x64',
];

const exeName = process.platform === 'win32' ? 'crsp-bin.exe' : 'crsp-bin';

function resolveBinary() {
  for (const platform of PLATFORM_PACKAGES) {
    const pkgName = `@sinsky-gh/crsp-${platform}`;
    try {
      const pkgPath = path.dirname(require.resolve(`${pkgName}/package.json`));
      const binPath = path.join(pkgPath, 'bin', exeName);
      if (fs.existsSync(binPath)) {
        return binPath;
      }
    } catch {
      // not installed for this platform, try next
    }
  }
  return null;
}

const binPath = resolveBinary();

if (!binPath) {
  console.error(`Error: crsp binary was not found for ${process.platform}-${process.arch}`);
  console.error(`Installed platform packages: ${PLATFORM_PACKAGES.join(', ')} (only matching os/cpu is installed)`);
  console.error(`Please reinstall with "npm install -g @sinsky-gh/crsp"`);
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), {
  stdio: 'inherit'
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);
```

- [ ] **Step 2: Syntax check**

Run: `node --check bin/crsp.js && echo "syntax OK"`
Expected: `syntax OK`

- [ ] **Step 3: Commit**

```bash
git add bin/crsp.js
git commit -m "feat(npm): resolve binary from platform packages"
```

### Task 4: release.yamlを6パッケージstage publish化

**Files:**
- Modify: `.github/workflows/release.yaml:10-72`

**Interfaces:**
- Consumes: Task 1の`npm/<platform>/bin/`配置先、Task 2のルート定義
- Produces: リリース時に6パッケージをstageするCI

- [ ] **Step 1: Add darwin-x64 matrix entry and npm package dir mapping**

`.github/workflows/release.yaml`のmatrixに以下を追加する。

```yaml
          - os: macos-15-intel
            target: x86_64-apple-darwin
            archive: tar.gz
            npm_dir: npm/darwin-x64
```

既存4エントリにも`npm_dir`を追加する。

```yaml
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
            archive: tar.gz
            npm_dir: npm/linux-x64
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-musl
            archive: tar.gz
            npm_dir: npm/linux-arm64
          - os: macos-latest
            target: aarch64-apple-darwin
            archive: tar.gz
            npm_dir: npm/darwin-arm64
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            archive: zip
            npm_dir: npm/win32-x64
```

- [ ] **Step 2: Copy built binary into npm package dir**

`Stage dist directory`ステップの後に以下を追加する。Windowsは`crsp.exe`を`crsp-bin.exe`にリネームして配置する。

```yaml
      - name: Stage npm platform package
        shell: bash
        run: |
          mkdir -p ${{ matrix.npm_dir }}/bin
          if [ "${{ matrix.os }}" = "windows-latest" ]; then
            cp target/${{ matrix.target }}/release/crsp.exe ${{ matrix.npm_dir }}/bin/crsp-bin.exe
          else
            cp target/${{ matrix.target }}/release/crsp ${{ matrix.npm_dir }}/bin/crsp-bin
            chmod +x ${{ matrix.npm_dir }}/bin/crsp-bin
          fi
```

- [ ] **Step 3: Stage all six npm packages**

`npm-stage`ジョブの`run: npm stage publish`を以下に置き換える。

```yaml
      - run: npm stage publish
      - run: npm stage publish --workspace npm/darwin-arm64 || npm --prefix npm/darwin-arm64 stage publish
```

ではなく、明示的に6件stageする。

```yaml
      - name: Stage npm packages
        run: |
          npm stage publish
          for dir in npm/darwin-arm64 npm/darwin-x64 npm/linux-x64 npm/linux-arm64 npm/win32-x64; do
            npm --prefix "$dir" stage publish
          done
```

- [ ] **Step 4: Verify workflow YAML loads**

Run: `node -e "const fs=require('fs'); const s=fs.readFileSync('.github/workflows/release.yaml','utf8'); if (!s.includes('npm/darwin-arm64') || !s.includes('npm stage publish')) { process.exit(1); } console.log('workflow OK')"`
Expected: `workflow OK`

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yaml
git commit -m "ci(release): stage six npm packages with platform binaries"
```

### Task 5: 公開前検証

**Files:**
- Test: `package.json`, `npm/*/package.json`, `bin/crsp.js`

**Interfaces:**
- Consumes: Task 1〜4の成果物すべて
- Produces: 公開可否の検証証拠

- [ ] **Step 1: Dry-run all six packages**

```bash
npm pack --dry-run 2>&1 | tail -8
for dir in npm/darwin-arm64 npm/darwin-x64 npm/linux-x64 npm/linux-arm64 npm/win32-x64; do npm --prefix "$dir" pack --dry-run 2>&1 | tail -8; done
```

Expected: 6件とも`Tarball Contents`に`bin/`相当と`package.json`が含まれ、エラーなし

- [ ] **Step 2: Local install check**

```bash
rm -rf /tmp/crsp-optdeps-test && mkdir -p /tmp/crsp-optdeps-test
TARBALL="$(npm pack --silent)" && mv "$TARBALL" /tmp/crsp-optdeps-test/ && rm -f ./*.tgz
echo "packed OK"
```

Expected: `packed OK` (バイナリ実物がないため`crsp --help`の動作確認は次回リリース後に実施)

- [ ] **Step 3: Push branch**

```bash
git push
```

Expected: push成功、CIが起動すること
