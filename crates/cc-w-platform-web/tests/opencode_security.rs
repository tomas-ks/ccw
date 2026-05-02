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

fn ifc_tool_export_suffixes() -> BTreeSet<String> {
    read_repo_file(".opencode/tools/ifc.ts")
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("export const ")?;
            let name = rest.split_whitespace().next()?.trim_end_matches('=');
            (!name.is_empty()).then_some(name.to_owned())
        })
        .collect()
}

fn public_ifc_tool_names() -> BTreeSet<String> {
    ifc_tool_export_suffixes()
        .into_iter()
        .map(|suffix| format!("ifc_{suffix}"))
        .collect()
}

fn assert_no_bare_ifc_export_names(body: &str, context: &str) {
    let backticked_tokens = backticked_tokens(body);
    for suffix in ifc_tool_export_suffixes() {
        assert!(
            !backticked_tokens.contains(&suffix),
            "{context} should not mention bare OpenCode export suffix `{suffix}`; use `ifc_{suffix}`"
        );
    }
}

#[test]
fn opencode_ifc_tools_have_one_public_prefix_convention() {
    let tool_files: BTreeSet<String> = fs::read_dir(repo_root().join(".opencode/tools"))
        .expect("runtime OpenCode tools directory should exist")
        .map(|entry| {
            entry
                .expect("tool directory entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".ts"))
        .collect();

    assert_eq!(
        tool_files,
        BTreeSet::from(["ifc.ts".to_owned()]),
        "runtime OpenCode tools should live under the single `ifc` namespace"
    );

    let suffixes = ifc_tool_export_suffixes();
    assert!(
        suffixes.contains("alignment_catalog"),
        "the IFC tool module should define the alignment catalog suffix"
    );
    assert!(
        suffixes.contains("station_resolve"),
        "the IFC tool module should define the station resolve suffix"
    );
    assert!(
        suffixes.contains("viewer_section_set"),
        "the IFC tool module should define the viewer section suffix"
    );
    assert!(
        suffixes.contains("viewer_annotations_show_path"),
        "the IFC tool module should define the path annotation viewer suffix"
    );
    assert!(
        suffixes.iter().all(|suffix| !suffix.starts_with("ifc_")),
        "exports in `.opencode/tools/ifc.ts` are namespace suffixes; do not double-prefix them"
    );

    let public_tools = public_ifc_tool_names();
    for tool in [
        "ifc_alignment_catalog",
        "ifc_station_resolve",
        "ifc_section_intersections",
        "ifc_viewer_section_set",
        "ifc_viewer_annotations_show_path",
    ] {
        assert!(
            public_tools.contains(tool),
            "`{tool}` should be a public OpenCode tool name derived from the `ifc` module"
        );
    }
}

#[test]
fn opencode_config_is_deny_by_default_and_seeded_with_gpt_5_4() {
    let config: Value = serde_json::from_str(&read_repo_file("tools/opencode/opencode.json"))
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
fn ifc_agent_config_is_deny_by_default_and_allows_only_canonical_ifc_tools() {
    let agent = read_repo_file("agent/agents/ifc-explorer.md");
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
        "the IFC agent should allow the IFC tool family"
    );
    assert!(
        !permissions.contains_key("entity_search") && !permissions.contains_key("properties"),
        "the IFC agent should not expose unprefixed tool names"
    );

    assert!(
        permissions
            .iter()
            .all(|(key, value)| key == "*" || key == "ifc_*" || value == "deny"),
        "the IFC agent should not grant any extra permissions beyond the canonical IFC tools"
    );

    let backticked_tokens = backticked_tokens(body);
    for token in [
        "ifc_*",
        "ifc_entity_reference",
        "ifc_query_playbook",
        "ifc_readonly_cypher",
        "ifc_node_relations",
        "ifc_properties_show_node",
    ] {
        assert!(
            backticked_tokens.contains(token),
            "the agent body should mention `{token}`"
        );
    }
    assert!(
        !backticked_tokens.contains("entity_search") && !backticked_tokens.contains("properties"),
        "the IFC agent body should not mention unprefixed tool names"
    );
    assert_no_bare_ifc_export_names(body, "the IFC agent body");
}

#[test]
fn strict_ifc_agent_config_is_deny_by_default_and_mentions_only_canonical_ifc_tools() {
    let agent = read_repo_file("agent/agents/ifc-explorer-strict.md");
    let (frontmatter, body) = split_frontmatter(&agent);
    let permissions = parse_permission_map(frontmatter);

    assert_eq!(
        permissions.get("*").map(String::as_str),
        Some("deny"),
        "the strict IFC agent should stay deny-by-default"
    );
    assert_eq!(
        permissions.get("ifc_*").map(String::as_str),
        Some("allow"),
        "the strict IFC agent should allow the IFC tool family"
    );
    assert!(
        !permissions.contains_key("entity_search"),
        "the strict IFC agent should not expose unprefixed tool names"
    );
    assert!(
        !permissions.contains_key("properties"),
        "the strict IFC agent should not expose unprefixed tool names"
    );
    assert!(
        permissions
            .iter()
            .all(|(key, value)| key == "*" || key == "ifc_*" || value == "deny"),
        "the strict IFC agent should not grant any extra permissions"
    );

    let backticked_tokens = backticked_tokens(body);
    for token in [
        "ifc_*",
        "ifc_entity_reference",
        "ifc_relation_reference",
        "ifc_query_playbook",
        "ifc_readonly_cypher",
        "ifc_node_relations",
        "ifc_properties_show_node",
        "ifc_graph_set_seeds",
        "ifc_elements_hide",
        "ifc_elements_show",
        "ifc_elements_select",
        "ifc_elements_inspect",
        "ifc_viewer_frame_visible",
        "ifc_viewer_clear_inspection",
    ] {
        assert!(
            backticked_tokens.contains(token),
            "the strict agent body should mention `{token}`"
        );
    }
    assert!(
        !backticked_tokens.contains("entity_search") && !backticked_tokens.contains("properties"),
        "the strict agent body should not mention unprefixed tool names"
    );
    assert_no_bare_ifc_export_names(body, "the strict IFC agent body");
    assert!(
        body.contains("What schema are we using?"),
        "the strict agent body should include the schema example"
    );
    assert!(
        body.contains("We are using IFC4X3_ADD2."),
        "the strict agent body should include the direct-answer example"
    );
    assert!(
        body.contains("Keep direct factual replies short."),
        "the strict agent body should tell the agent to answer direct factual questions directly"
    );
}

#[test]
fn answer_42_debug_agent_is_deny_by_default_and_forces_literal_42() {
    let agent = read_repo_file("agent/agents/ifc-answer-42.md");
    let (frontmatter, body) = split_frontmatter(&agent);
    let permissions = parse_permission_map(frontmatter);

    assert_eq!(
        permissions.get("*").map(String::as_str),
        Some("deny"),
        "the 42 debug agent should stay deny-by-default"
    );
    assert!(
        permissions
            .iter()
            .all(|(key, value)| key == "*" || value == "deny"),
        "the 42 debug agent should not grant any extra permissions"
    );

    let backticked_tokens = backticked_tokens(body);
    assert!(
        body.contains("respond with exactly `42` and nothing else"),
        "the 42 debug agent should force a literal 42 response"
    );
    assert!(
        body.contains("Do not call any tools."),
        "the 42 debug agent should forbid tool use"
    );
    assert!(
        !backticked_tokens.contains("ifc_*")
            && !backticked_tokens.contains("entity_search")
            && !backticked_tokens.contains("properties"),
        "the 42 debug agent should not mention any IFC tool names"
    );
}

#[test]
fn readonly_cypher_only_debug_agent_is_deny_by_default_and_uses_one_tool() {
    let agent = read_repo_file("agent/agents/ifc-readonly-cypher-only.md");
    let (frontmatter, body) = split_frontmatter(&agent);
    let permissions = parse_permission_map(frontmatter);

    assert_eq!(
        permissions.get("*").map(String::as_str),
        Some("deny"),
        "the one-tool debug agent should stay deny-by-default"
    );
    assert_eq!(
        permissions.get("ifc_readonly_cypher").map(String::as_str),
        Some("allow"),
        "the one-tool debug agent should allow read-only Cypher"
    );
    assert!(
        permissions
            .iter()
            .all(|(key, value)| key == "*" || key == "ifc_readonly_cypher" || value == "deny"),
        "the one-tool debug agent should not grant any extra permissions"
    );

    let backticked_tokens = backticked_tokens(body);
    assert!(
        body.contains("Use only `ifc_readonly_cypher`."),
        "the one-tool debug agent should require the single canonical tool"
    );
    assert!(
        body.contains("Do not call any other tool."),
        "the one-tool debug agent should forbid all other tools"
    );
    assert!(
        backticked_tokens.contains("ifc_readonly_cypher"),
        "the one-tool debug agent should mention the only allowed tool"
    );
    assert!(
        !backticked_tokens.contains("ifc_query_playbook")
            && !backticked_tokens.contains("ifc_entity_reference")
            && !backticked_tokens.contains("ifc_relation_reference")
            && !backticked_tokens.contains("ifc_node_relations")
            && !backticked_tokens.contains("ifc_properties_show_node"),
        "the one-tool debug agent should not mention any additional IFC tools"
    );
}

#[test]
fn playbook_and_cypher_debug_agent_is_deny_by_default_and_uses_two_tools() {
    let agent = read_repo_file("agent/agents/ifc-playbook-cypher-only.md");
    let (frontmatter, body) = split_frontmatter(&agent);
    let permissions = parse_permission_map(frontmatter);

    assert_eq!(
        permissions.get("*").map(String::as_str),
        Some("deny"),
        "the two-tool debug agent should stay deny-by-default"
    );
    assert_eq!(
        permissions.get("ifc_query_playbook").map(String::as_str),
        Some("allow"),
        "the two-tool debug agent should allow query playbooks"
    );
    assert_eq!(
        permissions.get("ifc_readonly_cypher").map(String::as_str),
        Some("allow"),
        "the two-tool debug agent should allow read-only Cypher"
    );
    assert!(
        permissions.iter().all(|(key, value)| key == "*"
            || key == "ifc_query_playbook"
            || key == "ifc_readonly_cypher"
            || value == "deny"),
        "the two-tool debug agent should not grant any extra permissions"
    );

    let backticked_tokens = backticked_tokens(body);
    assert!(
        body.contains("Use only `ifc_query_playbook` and `ifc_readonly_cypher`."),
        "the two-tool debug agent should require the exact tool pair"
    );
    assert!(
        body.contains("For any question about the model, you may call `ifc_query_playbook` once"),
        "the two-tool debug agent should prescribe the tool order"
    );
    assert!(
        body.contains("For material questions like \"What are the walls made of?\""),
        "the two-tool debug agent should include a concrete material-query example"
    );
    assert!(
        body.contains("The user question is already complete. Never ask the user to provide their question again."),
        "the two-tool debug agent should forbid clarification follow-ups"
    );
    assert!(
        body.contains("Never call `ifc_query_playbook` more than once for the same user question."),
        "the two-tool debug agent should forbid repeated playbook lookups"
    );
    assert!(
        body.contains("Do not respond to the playbook result with a clarification request."),
        "the two-tool debug agent should treat the playbook result as a query-shape hint, not a prompt for more user input"
    );
    assert!(
        body.contains("MATCH (wall:IfcWall)--(:IfcRelAssociatesMaterial)--(material:IfcMaterial)"),
        "the two-tool debug agent should include the exact wall-material traversal"
    );
    assert!(
        body.contains("Treat `IfcRelAssociatesMaterial` as the middle node label in the graph shape, not as a relationship type."),
        "the two-tool debug agent should explain the graph shape clearly"
    );
    assert!(
        body.contains("Do not use `IFC_REL_ASSOCIATES_MATERIAL`, `HAS_MATERIAL`"),
        "the two-tool debug agent should forbid invented relationship labels"
    );
    assert!(
        backticked_tokens.contains("ifc_query_playbook")
            && backticked_tokens.contains("ifc_readonly_cypher"),
        "the two-tool debug agent should mention both allowed tools"
    );
    assert!(
        !backticked_tokens.contains("ifc_entity_reference")
            && !backticked_tokens.contains("ifc_relation_reference")
            && !backticked_tokens.contains("ifc_node_relations")
            && !backticked_tokens.contains("ifc_properties_show_node")
            && !backticked_tokens.contains("ifc_graph_set_seeds")
            && !backticked_tokens.contains("ifc_elements_hide")
            && !backticked_tokens.contains("ifc_elements_show")
            && !backticked_tokens.contains("ifc_elements_select")
            && !backticked_tokens.contains("ifc_elements_inspect")
            && !backticked_tokens.contains("ifc_viewer_frame_visible")
            && !backticked_tokens.contains("ifc_viewer_clear_inspection"),
        "the two-tool debug agent should not mention any other IFC tools"
    );
}
