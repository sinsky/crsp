//! Ignore matching with `micromatch.not(files, patterns, {dot: true})`
//! semantics (spec §2.3, §4 rule 5, clasp `files.ts:129`).
//!
//! micromatch processes patterns **in order**, building two sets per file:
//! positive patterns add a file to the match set *and rescue* it from a
//! previous negation; `!`-negated patterns add it to the omitted set. A file
//! is in the match set when it was kept by some positive pattern and not
//! omitted afterwards (or, when every pattern is negated, when it matches no
//! negation). `tracked` is the complement of the match set over the full
//! file list. Dot files are always considered (`dot: true`), and every
//! evaluated path is normalized to `/` separators before glob evaluation
//! (Windows `\`), with the original strings returned unchanged.

use std::borrow::Cow;
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};

use crate::constants::PROJECT_IGNORE_FILENAME;
use crate::error::CrspError;

/// Default patterns applied when no `.claspignore` exists (clasp
/// `DEFAULT_CLASP_IGNORE`, `clasp.ts:37-46`). `!**/*.ts` keeps `.ts` files
/// tracked; the unsupported-type skip for `.ts` happens later in the file
/// pipeline, not here.
pub const DEFAULT_IGNORE_PATTERNS: [&str; 8] = [
    "**/**",
    "!**/appsscript.json",
    "!**/*.gs",
    "!**/*.js",
    "!**/*.ts",
    "!**/*.html",
    ".git/**",
    "node_modules/**",
];

/// A single parsed ignore rule.
struct Rule {
    negated: bool,
    matcher: Option<GlobMatcher>,
}

impl Rule {
    fn matches(&self, file: &str) -> bool {
        self.matcher.as_ref().is_some_and(|m| m.is_match(file))
    }
}

/// Ordered-pattern ignore matcher replicating `micromatch.not`.
pub struct IgnoreMatcher {
    rules: Vec<Rule>,
    all_negated: bool,
}

impl IgnoreMatcher {
    /// Builds a matcher from ordered patterns. Patterns starting with `!`
    /// are negations. Patterns globset cannot compile are treated as
    /// never-matching, mirroring picomatch's `/^$/` fallback for invalid
    /// globs (clasp behavior).
    pub fn from_patterns<I, S>(patterns: I) -> Result<Self, CrspError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rules = Vec::new();
        let mut negated_count = 0usize;
        for pattern in patterns {
            let pattern = pattern.as_ref();
            let negated = pattern.starts_with('!');
            let body = if negated { &pattern[1..] } else { pattern };
            let matcher = GlobBuilder::new(body)
                .literal_separator(true)
                .build()
                .ok()
                .map(|glob| glob.compile_matcher());
            if negated {
                negated_count += 1;
            }
            rules.push(Rule { negated, matcher });
        }
        let all_negated = negated_count == rules.len() && !rules.is_empty();
        Ok(Self { rules, all_negated })
    }

    /// Loads a `.claspignore` file (clasp `loadIgnoreFileOrDefaults`):
    /// strips a UTF-8 BOM, splits on `\r?\n`, and drops empty lines. Line
    /// content is otherwise kept verbatim (no trimming, matching clasp).
    pub async fn from_file(path: &Path) -> Result<Self, CrspError> {
        let content = tokio::fs::read_to_string(path).await?;
        Self::from_patterns(split_ignore_lines(&content))
    }

    /// The file name of the ignore file at a project root.
    pub fn file_name() -> &'static str {
        PROJECT_IGNORE_FILENAME
    }

    /// Default patterns for projects without a `.claspignore`.
    pub fn default_patterns() -> &'static [&'static str] {
        &DEFAULT_IGNORE_PATTERNS
    }

    /// `micromatch.not(files, patterns, {dot: true})`: returns the files NOT
    /// in the match set — the tracked files — preserving input order and the
    /// original (unnormalized) strings.
    pub fn tracked<'a, I>(&self, files: I) -> Vec<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
    {
        files
            .into_iter()
            .filter(|file| self.is_tracked(file))
            .collect()
    }

    /// Whether a single file is tracked (not in the micromatch match set).
    pub fn is_tracked(&self, file: &str) -> bool {
        !self.is_matched(file)
    }

    /// micromatch's inner match-set membership for one file, in pattern
    /// order: positives keep (and rescue from previous omission), negatives
    /// omit.
    fn is_matched(&self, file: &str) -> bool {
        let normalized = normalize_evaluated_path(file);
        let mut kept = false;
        let mut omitted = false;
        for rule in &self.rules {
            if rule.matches(&normalized) {
                if rule.negated {
                    omitted = true;
                } else {
                    omitted = false;
                    kept = true;
                }
            }
        }
        if self.all_negated {
            !omitted
        } else {
            kept && !omitted
        }
    }
}

/// Normalizes a path to `/` separators before glob evaluation, borrowing
/// when no backslash is present (spec §8, §3.1).
fn normalize_evaluated_path(file: &str) -> Cow<'_, str> {
    if file.contains('\\') {
        Cow::Owned(file.replace('\\', "/"))
    } else {
        Cow::Borrowed(file)
    }
}

/// Strips a UTF-8 BOM, splits on `\r?\n`, and drops empty lines.
fn split_ignore_lines(content: &str) -> Vec<String> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);
    stripped
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_strips_bom_and_crlf_but_keeps_spaces() {
        // CRLF is split; a lone \r inside a line is preserved (Node
        // `split(/\r?\n/)`), and line content is not trimmed.
        assert_eq!(split_ignore_lines("\u{feff}a\r\nb\rc\n"), vec!["a", "b\rc"]);
        assert_eq!(split_ignore_lines(""), Vec::<String>::new());
    }

    #[test]
    fn invalid_patterns_never_match_like_picomatch() {
        // picomatch falls back to /^$/ for uncompilable globs, so an invalid
        // pattern contributes nothing: alone it never matches, and mixed in,
        // valid patterns still decide the outcome.
        let only_invalid = IgnoreMatcher::from_patterns(["[a-"]).unwrap();
        assert!(!only_invalid.is_matched("x.js"));
        assert!(only_invalid.is_tracked("x.js"));

        let mixed = IgnoreMatcher::from_patterns(["[a-", "*.js"]).unwrap();
        assert!(mixed.is_matched("x.js"));
        assert!(!mixed.is_matched("y.txt"));

        // An invalid negation never omits anything either.
        let invalid_negation = IgnoreMatcher::from_patterns(["![a-", "**/**"]).unwrap();
        assert!(invalid_negation.is_matched("x.js"));
        assert!(invalid_negation.is_matched("y.txt"));
    }
}
