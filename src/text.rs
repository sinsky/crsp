//! Text helpers shared by command modules (clasp-compatible).

/// Equivalent of clasp's `inflection.humanize` (inflection@3.0.2) for default
/// project titles: lowercases the full string, removes a trailing `_ids`/`_id`,
/// replaces underscores with spaces, then uppercases only the first character.
/// Hyphens are preserved (`my-project-folder` -> `My-project-folder`).
pub fn humanize_title(name: &str) -> String {
    let lowered = name.to_lowercase();
    let base = lowered
        .strip_suffix("_ids")
        .or_else(|| lowered.strip_suffix("_id"))
        .unwrap_or(&lowered);
    let spaced = base.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => {
            let mut title: String = first.to_uppercase().collect();
            title.push_str(chars.as_str());
            title
        }
        None => spaced,
    }
}

#[cfg(test)]
mod tests {
    use super::humanize_title;

    #[test]
    fn strips_id_suffixes_and_keeps_hyphens() {
        assert_eq!(humanize_title("my-project-folder"), "My-project-folder");
        assert_eq!(humanize_title("My_App"), "My app");
        assert_eq!(humanize_title("blog_id"), "Blog");
        assert_eq!(humanize_title("blog_ids"), "Blog");
        assert_eq!(humanize_title("hello_world_id"), "Hello world");
        assert_eq!(humanize_title("ABC_def"), "Abc def");
        assert_eq!(humanize_title("plain"), "Plain");
        assert_eq!(humanize_title("_id"), "");
        assert_eq!(humanize_title(""), "");
    }
}
