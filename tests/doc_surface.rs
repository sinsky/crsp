const COMMANDS: &[&str] = &[
    "login",
    "logout",
    "show-authorized-user",
    "clone-script",
    "create-script",
    "push",
    "pull",
    "create-deployment",
    "update-deployment",
    "delete-deployment",
    "delete-script",
    "create-version",
    "list-versions",
    "list-deployments",
    "list-scripts",
    "run-function",
    "tail-logs",
    "setup-logs",
    "show-file-status",
    "list-apis",
    "enable-api",
    "disable-api",
    "open-script",
    "open-container",
    "open-web-app",
    "open-logs",
    "open-api-console",
    "open-credentials-setup",
    "start-mcp-server",
];

#[test]
fn readme_documents_all_canonical_commands() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    assert_eq!(COMMANDS.len(), 29);
    for command in COMMANDS {
        let exact = format!("`{command}`");
        let bullet = format!("- `{command}`");
        assert!(
            readme.lines().any(|line| {
                let trimmed = line.trim();
                trimmed == exact || trimmed.starts_with(&bullet)
            }),
            "missing `{command}`"
        );
    }
}

#[test]
fn readme_has_required_sections_in_order() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    let headings = [
        "## Install",
        "## Quick start",
        "## Commands",
        "## Global options",
        "## Compatibility",
        "## Versioning",
        "## Development",
        "## License",
    ];
    let mut cursor = 0;
    for heading in headings {
        let found = readme[cursor..]
            .find(heading)
            .unwrap_or_else(|| panic!("README is missing `{heading}`"));
        cursor += found + heading.len();
    }
}

#[test]
fn required_docs_have_required_sections() {
    let sections = [
        (
            "docs/config-files.md",
            &[
                "## Overview",
                "## .clasp.json",
                "## .clasprc.json",
                "## appsscript.json",
                "## .claspignore",
                "## Project discovery",
                "## Security",
            ] as &[&str],
        ),
        (
            "docs/run.md",
            &[
                "## Install",
                "## Login",
                "## Create or clone",
                "## Push and pull",
                "## Deployments",
                "## Logs and functions",
                "## API management",
                "## JSON output",
                "## MCP server",
                "## Troubleshooting",
            ],
        ),
        (
            "docs/migration-clasp-to-crsp.md",
            &[
                "## Compatibility",
                "## Install",
                "## Command mapping",
                "## Configuration",
                "## Authentication",
                "## Output and exit codes",
                "## MCP",
                "## Known intentional differences",
            ],
        ),
        ("CHANGELOG.md", &["# Changelog", "## 0.1.0", "### Added"]),
    ];
    for (path, required) in sections {
        let body = std::fs::read_to_string(path).unwrap();
        for section in required {
            assert!(body.contains(section), "{path} is missing {section}");
        }
    }
}
