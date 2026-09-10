//! Path utilities and jail checks (spec §8, §2.6, §5 #5).
//!
//! Containment checks are component-aware ([`Path::strip_prefix`] after
//! lexical normalization) — never raw string prefixes, so `/app_fake` is
//! correctly rejected as a sibling of `/app`. Relative-path computation and
//! lexical resolution mirror Node's `path.resolve`/`path.relative` semantics;
//! Windows `\` separators are normalized to `/` lazily via [`Cow`].

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::CrspError;
use crate::i18n;

/// Jail validation for project/content/remote paths.
pub struct PathJail;

impl PathJail {
    /// Component-aware containment (clasp `isInside`, `files.ts:74`): the
    /// candidate must be strictly inside `base` (equal paths are not inside).
    /// Both paths are lexically normalized first, so traversal that lands
    /// back inside is accepted and `/app_fake` is rejected.
    pub fn is_inside(base: &Path, candidate: &Path) -> bool {
        let base = normalize_lexical(base);
        let candidate = normalize_lexical(candidate);
        candidate != base && candidate.strip_prefix(&base).is_ok()
    }

    /// Pure-lexical resolution of `raw` against an absolute `base`
    /// (Node `path.resolve(base, raw)` semantics without touching the
    /// filesystem): `.` segments are dropped, `..` pops a component (never
    /// past the root), and absolute candidates (or Windows drive-prefixed
    /// ones) replace the base entirely.
    pub fn resolve_lexical(base: &Path, raw: &str) -> PathBuf {
        let normalized = normalize_slashes(raw);
        let raw_path = Path::new(normalized.as_ref());
        if let Some(Component::Prefix(_)) | Some(Component::RootDir) = raw_path.components().next()
        {
            return normalize_lexical(raw_path);
        }
        let mut base = normalize_lexical(base);
        for component in raw_path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    base.pop();
                }
                Component::Normal(value) => base.push(value),
                Component::Prefix(_) | Component::RootDir => unreachable!("checked above"),
            }
        }
        base
    }

    /// Resolves a content directory override (`clasp withContentDir`,
    /// `clasp.ts:129-142`, and the unified MCP `sourceDir` rule per spec
    /// §5 #5): relative candidates resolve against `base`, and the resolved
    /// path must be `base` itself or strictly inside it.
    pub fn resolve_content_dir(base: &Path, raw: &str) -> Result<PathBuf, CrspError> {
        let base = normalize_lexical(base);
        let resolved = Self::resolve_lexical(&base, raw);
        if resolved != base && !Self::is_inside(&base, &resolved) {
            return Err(CrspError::Validation(
                i18n::content_dir_resolves_outside_project_root(
                    &resolved.to_string_lossy(),
                    &base.to_string_lossy(),
                ),
            ));
        }
        Ok(resolved)
    }

    /// MCP `projectDir` jail (`server.ts:40-51`): the resolved path must be
    /// the home directory itself, the cwd itself, or inside either.
    /// Relative candidates resolve against `cwd` (Node `path.resolve`).
    pub fn validate_project_dir(
        candidate: &Path,
        home_dir: &Path,
        cwd: &Path,
    ) -> Result<PathBuf, CrspError> {
        let resolved = normalize_lexical(candidate);
        let resolved = if resolved.is_absolute() {
            resolved
        } else {
            Self::resolve_lexical(cwd, &resolved.to_string_lossy())
        };
        let home_dir = normalize_lexical(home_dir);
        let cwd = normalize_lexical(cwd);
        let allowed = [&home_dir, &cwd]
            .iter()
            .any(|base| resolved == **base || Self::is_inside(base, &resolved));
        if !allowed {
            return Err(CrspError::Validation(i18n::project_dir_not_permitted(
                &resolved.to_string_lossy(),
            )));
        }
        Ok(resolved)
    }

    /// Remote-name validation for the pull side (`files.ts:706-713`): joins
    /// a remote file name against the (realpath'd) content directory and
    /// rejects names that would resolve outside it. Windows `\` separators
    /// are normalized first (spec §8). `None` maps to the
    /// `outside_content_dir` skip reason in the file pipeline.
    pub fn remote_target_path(content_dir: &Path, name: &str) -> Option<PathBuf> {
        let resolved = Self::resolve_lexical(content_dir, name);
        if Self::is_inside(content_dir, &resolved) {
            Some(resolved)
        } else {
            None
        }
    }
}

/// Normalizes Windows `\` path separators to `/`, borrowing the input when
/// no conversion is needed (spec §3.1: `Cow` avoids allocation on Unix).
pub fn normalize_slashes(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Lexical normalization (Node `path.resolve` without a cwd base): drops `.`
/// segments, applies `..` (never past the root), and preserves the root/prefix.
pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut prefix: Option<OsString> = None;
    let mut absolute = false;
    let mut components: Vec<OsString> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_os_string()),
            Component::RootDir => {
                absolute = true;
                components.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                } else if !absolute {
                    components.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => components.push(value.to_os_string()),
        }
    }

    let mut out = PathBuf::new();
    if let Some(prefix) = prefix {
        out.push(prefix);
    }
    if absolute {
        out.push(Component::RootDir.as_os_str());
    }
    for component in components {
        out.push(component);
    }
    out
}

/// Lexical Windows drive-letter detection (`C:` / `C:/…`): Rust only parses
/// this as `Component::Prefix` on Windows, but Node's `path.relative` treats
/// cross-drive inputs as different roots on every platform (spec §8).
fn drive_letter(path: &str) -> Option<char> {
    let mut chars = path.chars();
    let drive = chars.next()?.to_ascii_uppercase();
    if drive.is_ascii_alphabetic() && chars.next() == Some(':') {
        Some(drive)
    } else {
        None
    }
}

/// Node `path.relative(from, to)` equivalent on lexically normalized paths.
/// Returns an OS-native relative path string; equal paths yield `""`, and
/// candidates on a different root (Windows drives) yield the absolute target.
pub fn relative_path(from: &Path, to: &Path) -> String {
    let from = normalize_lexical(from);
    let to = normalize_lexical(to);
    if from == to {
        return String::new();
    }

    let mut from_components = from.components();
    let mut to_components = to.components();
    // Rust parses `C:/…` as a drive prefix only on Windows; on POSIX it is
    // just two normal segments. Detect the drive letter lexically so the
    // cross-drive rule holds on every platform (spec §8).
    if drive_letter(&from.to_string_lossy()) != drive_letter(&to.to_string_lossy())
        && (drive_letter(&from.to_string_lossy()).is_some()
            || drive_letter(&to.to_string_lossy()).is_some())
    {
        return to.to_string_lossy().into_owned();
    }
    let (from_first, to_first) = (from_components.next(), to_components.next());
    // Node only treats a Windows drive/prefix (or a POSIX root) as the "root"
    // for mismatch purposes; two relative paths with different first normal
    // segments (e.g. `dirA/f1` vs `dirB/f2`) still share an empty common
    // prefix and resolve to `../dirB/f2`, they do not return the target.
    if from_first != to_first {
        let roots_differ = match (from_first, to_first) {
            (
                Some(Component::Prefix(_) | Component::RootDir),
                Some(Component::Prefix(_) | Component::RootDir),
            )
            | (None, Some(Component::Prefix(_) | Component::RootDir))
            | (Some(Component::Prefix(_) | Component::RootDir), None) => true,
            // One side exhausted (a strict prefix of the other), or both
            // sides start with a normal/curdir component: not a root
            // mismatch, just a differing tail.
            _ => false,
        };
        if roots_differ {
            return to.to_string_lossy().into_owned();
        }
    }
    let mut from_rest: Vec<OsString> = from_first
        .into_iter()
        .chain(from_components)
        .map(|component| component.as_os_str().to_os_string())
        .collect();
    let mut to_rest: Vec<OsString> = to_first
        .into_iter()
        .chain(to_components)
        .map(|component| component.as_os_str().to_os_string())
        .collect();
    // The common root/prefix component consumed above is not part of the
    // relative tail; drop it. When the heads differed (relative paths with
    // different first segments), both heads stay so they count as divergent
    // segments.
    if from_first == to_first && !from_rest.is_empty() {
        from_rest.remove(0);
        to_rest.remove(0);
    }

    let common = from_rest
        .iter()
        .zip(to_rest.iter())
        .take_while(|(from, to)| from == to)
        .count();

    let mut out = PathBuf::new();
    for _ in common..from_rest.len() {
        out.push("..");
    }
    for component in &to_rest[common..] {
        out.push(component);
    }
    out.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lexical_handles_empty_and_root() {
        assert_eq!(normalize_lexical(Path::new("/")), PathBuf::from("/"));
        assert_eq!(
            normalize_lexical(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            normalize_lexical(Path::new("/a/../../b")),
            PathBuf::from("/b")
        );
    }

    #[test]
    fn resolve_lexical_pops_never_past_root() {
        assert_eq!(
            PathJail::resolve_lexical(Path::new("/app"), "../../../x"),
            PathBuf::from("/x")
        );
    }

    #[test]
    fn relative_path_across_drives_returns_absolute_target() {
        // Windows drive change: Node returns the absolute target.
        assert_eq!(relative_path(Path::new("C:/a"), Path::new("D:/b")), "D:/b");
    }

    #[test]
    fn relative_paths_with_different_heads_resolve_like_node() {
        // Node treats both sides as directory paths (`path.relative` has no
        // file/dir distinction): `relative('dirA/f1', 'dirB/f2')` climbs out
        // of `f1` as well. The key parity point is that the differing first
        // segments are NOT consumed as a drive/root name (which would yield
        // just `dirB/f2`); verified against `node -e` above.
        assert_eq!(
            relative_path(Path::new("dirA/f1"), Path::new("dirB/f2")),
            "../../dirB/f2"
        );
        assert_eq!(
            relative_path(Path::new("a/b/c"), Path::new("a/d")),
            "../../d"
        );
    }
}
