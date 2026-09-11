# Configuration files

## Overview

`crsp` uses the same local files as `clasp`, so an existing clasp project works without conversion:

- `.clasp.json` — project settings (script id, content directory, push order).
- `.clasprc.json` — OAuth credentials, keyed by user name.
- `appsscript.json` — the Apps Script manifest, stored in the content directory.
- `.claspignore` — ignore patterns that decide which files are tracked.

## .clasp.json

Read with **JSON5** (comments and single quotes are accepted); written as plain **JSON with two-space indentation**.

Keys:

- `scriptId` — the target Apps Script project.
- `projectId` — the associated Google Cloud project id.
- `parentId` — the container file id; if it is an array, the first element is used.
- `srcDir` / `rootDir` — the content directory, resolved relative to the project root. An empty string (`""`) is treated as unset, so the content directory becomes the project root itself.
- `filePushOrder` — explicit push order, applied before alphabetical ordering.
- `scriptExtensions` — defaults to `[".js", ".gs"]`.
- `htmlExtensions` — defaults to `[".html"]`.
- `jsonExtensions` — defaults to `[".json"]`.
- `fileExtension` — legacy single extension.
- `skipSubdirectories` — when true, only the top level of the content directory is scanned.
- `allowSymlinks` — when true, symlinks are followed instead of being skipped.

When `crsp` writes project settings (during `clone-script`, `create-script`, and the MCP create/clone tools), it stores the **recomputed** root directory, not the input value: the value written to `rootDir` is the relative path from the project root to the content directory. `filePushOrder` is reset to an empty array. Keys with `undefined` values are omitted, so `parentId` and `projectId` are absent when unset.

## .clasprc.json

Credentials live in `.clasprc.json` in your home directory by default. The V3 shape is:

```json
{
  "tokens": {
    "<user>": {
      "client_id": "...",
      "client_secret": "...",
      "refresh_token": "...",
      "access_token": "...",
      "expiry_date": 0,
      "type": "authorized_user"
    }
  }
}
```

Multiple named users can coexist. Select one with the global `--auth <user>` flag (or `-A`); `logout` removes only the selected user's token. Two legacy V1 layouts (a local token plus `oauth2ClientSettings`, and a top-level access token) are still read, but only for the `default` user.

Access tokens refresh automatically and the refreshed token is written back. On POSIX systems the file is written directly with mode `0600` and `O_NOFOLLOW` (no temp-then-rename), which protects against a symlinked credential file. On Windows those POSIX protections are best effort: the chmod is a no-op and symlink defense relies on a pre-write `symlink_metadata` check.

The default OAuth client is the same public Google desktop client that `clasp` uses, so existing clasp tokens keep working. A client you supply through `--creds` is classified as user-provided.

## appsscript.json

`appsscript.json` is the Apps Script manifest. It lives in the content directory. During `push` and `pull` it is transferred as a JSON file whose remote name is `appsscript` (no extension). `enable-api` and `disable-api` read and write `dependencies.enabledAdvancedServices` entries (`userSymbol`, `serviceId`, `version`), preserving key order.

## .claspignore

`.claspignore` lives in the project root (or is supplied with `-I`). A UTF-8 BOM is stripped before the file is split into lines.

Pattern evaluation matches clasp: a file is *ignored* when it matches at least one positive pattern and no negated (`!`) pattern; the set of tracked files is the **complement** of that ignored set, with dot files included. For example, with the defaults, `x.txt`, `.git/config.js`, and `node_modules/f.js` are ignored while `a.js`, `appsscript.json`, and `sub/Code.gs` are tracked.

When `.claspignore` is absent, the default patterns are:

```
**/**
!**/appsscript.json
!**/*.gs
!**/*.js
!**/*.ts
!**/*.html
.git/**
node_modules/**
```

Note that `.ts` files are tracked by the default ignore rules but are not in the default `scriptExtensions`, so they are skipped during push as an unsupported type.

## Project discovery

`crsp` finds the project by searching from the current working directory upward for `.clasp.json`. The `-P, --project <path>` global flag overrides discovery and names the project directory directly. Ignore patterns are read from the discovered project root, or from the file given by `-I, --ignore`.

The **content directory** is the project root joined with `srcDir`/`rootDir` (or the project root itself when that setting is unset or empty). It must stay inside the project root.

## Security

- The content directory is resolved with `realpath` before any file is read, written, or deleted.
- Path jails are checked by path components, not string prefixes, so `/app_fake` is not treated as inside `/app`.
- `push` skips symlinks unless `allowSymlinks` is enabled; `pull` writes files directly with `O_NOFOLLOW` and rejects symlinked parents, symlinked targets, and paths that escape the content directory.
- Credentials are secrets. Never commit `.clasprc.json`, and avoid pasting token contents into logs, issues, or transcripts. `crsp` does not print token values.
