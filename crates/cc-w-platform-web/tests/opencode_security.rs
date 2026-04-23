use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_root().join(relative_path))
        .unwrap_or_else(|error| panic!("failed to read `{relative_path}`: {error}"))
}

fn split_frontmatter(markdown: &str) -> (&str, &str) {
    let mut sections = markdown.splitn(3, "---\n");
    let prefix = sections
        .next()
        .expect("frontmatter split should always yield a prefix section");
    assert!(
        prefix.is_empty(),
        "expected the markdown file to start with frontmatter"
    );
    let frontmatter = sections
        .next()
        .expect("expected a YAML frontmatter section");
    let body = sections.next().expect("expected a markdown body section");
    (frontmatter, body)
}

fn parse_permission_map(frontmatter: &str) -> BTreeMap<String, String> {
    let mut permissions = BTreeMap::new();
    let mut in_permission_block = false;

    for line in frontmatter.lines() {
        if !in_permission_block {
            if line.trim() == "permission:" {
                in_permission_block = true;
            }
            continue;
        }

        let Some(entry) = line.strip_prefix("  ") else {
            if !line.trim().is_empty() {
                break;
            }
            continue;
        };

        let (key, value) = entry
            .split_once(':')
            .expect("permission entry should contain a key and a value");
        permissions.insert(
            key.trim().trim_matches('"').to_owned(),
            value.trim().trim_matches('"').to_owned(),
        );
    }

    permissions
}

fn backticked_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut remaining = text;

    while let Some(start) = remaining.find('`') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('`') else {
            break;
        };
        tokens.insert(remaining[..end].to_owned());
        remaining = &remaining[end + 1..];
    }

    tokens
}

#[test]
fn opencode_config_is_deny_by_default_and_seeded_with_gpt_5_4() {
    let config: Value =
        serde_json::from_str(&read_repo_file("tools/opencode/opencode.json"))
            .expect("opencode config should be valid json");

    assert_eq!(
        config.get("model").and_then(Value::as_str),
        Some("openai/gpt-5.4"),
        "the repo-local OpenCode config should keep GPT as the default seed"
    );

    let permission = config
        .get("permission")
        .and_then(Value::as_object)
        .expect("opencode config should include a permission map");

    assert_eq!(
        permission.get("*").and_then(Value::as_str),
        Some("deny"),
        "OpenCode should stay deny-by-default"
    );
    assert_eq!(
        permission.get("ifc_*").and_then(Value::as_str),
        Some("allow"),
        "the IFC tool family should remain the only allow-listed family"
    );

    let allow_entries: Vec<&str> = permission
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_str()
                .filter(|value| *value == "allow")
                .map(|_| key.as_str())
        })
        .collect();

    assert_eq!(
        allow_entries,
        vec!["ifc_*"],
        "only the IFC-prefixed tool family should be allowed"
    );

    assert!(
        permission
            .iter()
            .all(|(key, value)| key == "ifc_*" || value.as_str() == Some("deny")),
        "every non-IFC permission should stay denied"
    );
}

#[test]
fn ifc_agent_config_is_deny_by_default_and_mentions_only_ifc_tools() {
    let agent = read_repo_file(".opencode/agents/ifc-explorer.md");
    let (frontmatter, body) = split_frontmatter(&agent);
    let permissions = parse_permission_map(frontmatter);

    assert_eq!(
        permissions.get("*").map(String::as_str),
        Some("deny"),
        "the IFC agent should stay deny-by-default"
    );
    assert_eq!(
        permissions.get("ifc_*").map(String::as_str),
        Some("allow"),
        "the IFC agent should only allow the IFC tool family"
    );

    assert!(
        permissions
            .iter()
            .all(|(key, value)| key == "*" || key == "ifc_*" || value == "deny"),
        "the IFC agent should not grant any extra permissions"
    );

    let backticked_tokens = backticked_tokens(body);
    assert_eq!(
        backticked_tokens,
        BTreeSet::from([String::from("ifc_*")]),
        "the agent body should only refer to the IFC tool family"
    );
}
