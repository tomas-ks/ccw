use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use cc_w_velr::IfcSchemaId;
use serde::{Deserialize, Serialize};

use crate::agent_executor::{
    AgentEntityReference, AgentQueryPlaybook, AgentRelationReference, AgentSchemaContext,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentSchemaReferenceAsset {
    schema_id: String,
    schema_slug: Option<String>,
    summary: String,
    #[serde(default)]
    cautions: Vec<String>,
    #[serde(default)]
    query_habits: Vec<String>,
    #[serde(default)]
    query_playbooks: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    entities: BTreeMap<String, AgentEntityReferenceAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentEntityReferenceAsset {
    summary: String,
    #[serde(default)]
    common_relations: Vec<String>,
    #[serde(default)]
    query_patterns: Vec<String>,
    #[serde(default)]
    cautions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentQueryPlaybookAssetFile {
    schema_id: String,
    schema_slug: Option<String>,
    #[serde(default)]
    playbooks: BTreeMap<String, AgentQueryPlaybookAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentQueryPlaybookAsset {
    summary: String,
    #[serde(default)]
    when_to_use: Vec<String>,
    #[serde(default)]
    recommended_patterns: Vec<String>,
    #[serde(default)]
    related_entities: Vec<String>,
    #[serde(default)]
    cautions: Vec<String>,
    #[serde(default)]
    avoid_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentRelationReferenceAssetFile {
    schema_id: String,
    schema_slug: Option<String>,
    #[serde(default)]
    references: BTreeMap<String, AgentRelationReferenceAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AgentRelationReferenceAsset {
    summary: String,
    #[serde(default)]
    common_connections: Vec<String>,
    #[serde(default)]
    query_patterns: Vec<String>,
    #[serde(default)]
    cautions: Vec<String>,
}

pub fn load_schema_context(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
) -> Result<AgentSchemaContext, String> {
    let asset = load_schema_reference_asset(artifacts_root, schema)?;
    Ok(AgentSchemaContext {
        schema_id: asset.schema_id,
        schema_slug: asset.schema_slug,
        summary: asset.summary,
        cautions: asset.cautions,
        query_habits: asset.query_habits,
        query_playbooks: asset.query_playbooks,
    })
}

pub fn load_entity_references(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
    entity_names: &[String],
) -> Result<Vec<AgentEntityReference>, String> {
    let asset = load_schema_reference_asset(artifacts_root, schema)?;
    let mut requested = dedup_entity_names(entity_names);
    if requested.is_empty() {
        return Ok(Vec::new());
    }

    let mut references = Vec::new();
    for name in requested.drain(..) {
        let Some((entity_name, entity)) = find_entity_reference(&asset.entities, &name) else {
            continue;
        };
        references.push(AgentEntityReference {
            entity_name,
            summary: entity.summary.clone(),
            common_relations: entity.common_relations.clone(),
            query_patterns: entity.query_patterns.clone(),
            cautions: entity.cautions.clone(),
        });
    }

    Ok(references)
}

pub fn load_query_playbooks(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
    goal: &str,
    entity_names: &[String],
) -> Result<Vec<AgentQueryPlaybook>, String> {
    let asset = load_query_playbook_asset_file(artifacts_root, schema)?;
    let normalized_goal = normalize_goal_key(goal);
    let requested_entities = dedup_entity_names(entity_names);
    let requested_entity_keys = requested_entities
        .iter()
        .map(|name| normalize_entity_name(name))
        .collect::<Vec<_>>();

    let mut scored = asset
        .playbooks
        .iter()
        .filter_map(|(name, playbook)| {
            let score = query_playbook_match_score(
                name,
                playbook,
                &normalized_goal,
                &requested_entity_keys,
            );
            (score > 0).then(|| {
                (
                    score,
                    AgentQueryPlaybook {
                        playbook_name: name.clone(),
                        summary: playbook.summary.clone(),
                        when_to_use: playbook.when_to_use.clone(),
                        recommended_patterns: playbook.recommended_patterns.clone(),
                        related_entities: playbook.related_entities.clone(),
                        cautions: playbook.cautions.clone(),
                        avoid_patterns: playbook.avoid_patterns.clone(),
                    },
                )
            })
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.playbook_name.cmp(&right.1.playbook_name))
    });
    scored.truncate(4);
    Ok(scored.into_iter().map(|(_, playbook)| playbook).collect())
}

pub fn load_relation_references(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
    relation_names: &[String],
) -> Result<Vec<AgentRelationReference>, String> {
    let asset = load_relation_reference_asset_file(artifacts_root, schema)?;
    let mut requested = dedup_entity_names(relation_names);
    if requested.is_empty() {
        return Ok(Vec::new());
    }

    let mut references = Vec::new();
    for name in requested.drain(..) {
        let Some((relation_name, relation)) = find_relation_reference(&asset.references, &name)
        else {
            continue;
        };
        references.push(AgentRelationReference {
            relation_name,
            summary: relation.summary.clone(),
            common_connections: relation.common_connections.clone(),
            query_patterns: relation.query_patterns.clone(),
            cautions: relation.cautions.clone(),
        });
    }

    Ok(references)
}

fn load_schema_reference_asset(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
) -> Result<AgentSchemaReferenceAsset, String> {
    let path = schema_reference_path(artifacts_root, schema);
    if path.is_file() {
        let raw = fs::read_to_string(&path).map_err(|error| {
            format!(
                "failed to read schema reference `{}`: {error}",
                path.display()
            )
        })?;
        let mut asset: AgentSchemaReferenceAsset = serde_json::from_str(&raw).map_err(|error| {
            format!(
                "failed to parse schema reference `{}`: {error}",
                path.display()
            )
        })?;
        if asset.schema_id.trim().is_empty() {
            asset.schema_id = schema.canonical_name().to_owned();
        }
        if asset.schema_slug.is_none() {
            asset.schema_slug = schema.generated_artifact_stem().map(ToOwned::to_owned);
        }
        return Ok(asset);
    }

    Ok(default_schema_reference_asset(schema))
}

fn load_query_playbook_asset_file(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
) -> Result<AgentQueryPlaybookAssetFile, String> {
    let path = query_playbook_path(artifacts_root, schema);
    if path.is_file() {
        let raw = fs::read_to_string(&path).map_err(|error| {
            format!(
                "failed to read query playbooks `{}`: {error}",
                path.display()
            )
        })?;
        let mut asset: AgentQueryPlaybookAssetFile =
            serde_json::from_str(&raw).map_err(|error| {
                format!(
                    "failed to parse query playbooks `{}`: {error}",
                    path.display()
                )
            })?;
        if asset.schema_id.trim().is_empty() {
            asset.schema_id = schema.canonical_name().to_owned();
        }
        if asset.schema_slug.is_none() {
            asset.schema_slug = schema.generated_artifact_stem().map(ToOwned::to_owned);
        }
        return Ok(asset);
    }

    Ok(default_query_playbook_asset_file(schema))
}

fn load_relation_reference_asset_file(
    artifacts_root: &Path,
    schema: &IfcSchemaId,
) -> Result<AgentRelationReferenceAssetFile, String> {
    let path = relation_reference_path(artifacts_root, schema);
    if path.is_file() {
        let raw = fs::read_to_string(&path).map_err(|error| {
            format!(
                "failed to read relation references `{}`: {error}",
                path.display()
            )
        })?;
        let mut asset: AgentRelationReferenceAssetFile =
            serde_json::from_str(&raw).map_err(|error| {
                format!(
                    "failed to parse relation references `{}`: {error}",
                    path.display()
                )
            })?;
        if asset.schema_id.trim().is_empty() {
            asset.schema_id = schema.canonical_name().to_owned();
        }
        if asset.schema_slug.is_none() {
            asset.schema_slug = schema.generated_artifact_stem().map(ToOwned::to_owned);
        }
        return Ok(asset);
    }

    Ok(default_relation_reference_asset_file(schema))
}

fn schema_reference_path(artifacts_root: &Path, schema: &IfcSchemaId) -> PathBuf {
    let stem = schema.generated_artifact_stem().unwrap_or("other");
    artifacts_root
        .join("_graphql")
        .join(stem)
        .join("agent-reference.json")
}

fn query_playbook_path(artifacts_root: &Path, schema: &IfcSchemaId) -> PathBuf {
    let stem = schema.generated_artifact_stem().unwrap_or("other");
    artifacts_root
        .join("_graphql")
        .join(stem)
        .join("agent-query-playbook.json")
}

fn relation_reference_path(artifacts_root: &Path, schema: &IfcSchemaId) -> PathBuf {
    let stem = schema.generated_artifact_stem().unwrap_or("other");
    artifacts_root
        .join("_graphql")
        .join(stem)
        .join("agent-relation-reference.json")
}

fn find_entity_reference<'a>(
    entities: &'a BTreeMap<String, AgentEntityReferenceAsset>,
    entity_name: &str,
) -> Option<(String, &'a AgentEntityReferenceAsset)> {
    if let Some(reference) = entities.get(entity_name) {
        return Some((entity_name.to_owned(), reference));
    }

    let normalized = normalize_entity_name(entity_name);
    entities.iter().find_map(|(name, reference)| {
        if normalize_entity_name(name) == normalized {
            Some((name.clone(), reference))
        } else {
            None
        }
    })
}

fn find_relation_reference<'a>(
    references: &'a BTreeMap<String, AgentRelationReferenceAsset>,
    relation_name: &str,
) -> Option<(String, &'a AgentRelationReferenceAsset)> {
    if let Some(reference) = references.get(relation_name) {
        return Some((relation_name.to_owned(), reference));
    }

    let normalized = normalize_entity_name(relation_name);
    references.iter().find_map(|(name, reference)| {
        if normalize_entity_name(name) == normalized {
            Some((name.clone(), reference))
        } else {
            None
        }
    })
}

fn dedup_entity_names(entity_names: &[String]) -> Vec<String> {
    let mut seen = BTreeMap::<String, String>::new();
    for name in entity_names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        seen.entry(normalize_entity_name(trimmed))
            .or_insert_with(|| trimmed.to_owned());
    }
    seen.into_values().collect()
}

fn normalize_entity_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn normalize_goal_key(value: &str) -> String {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn goal_tokens(value: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "as", "at", "by", "for", "from", "in", "into", "is", "it", "of", "on",
        "or", "the", "to", "with",
    ];

    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            (!token.is_empty() && !STOPWORDS.contains(&token.as_str())).then_some(token)
        })
        .collect()
}

fn token_overlap_score(left: &str, right: &str) -> usize {
    let left_tokens = goal_tokens(left);
    let right_tokens = goal_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0;
    }

    let overlap = left_tokens.intersection(&right_tokens).count();
    match overlap {
        0 => 0,
        1 => 1,
        2 => 3,
        _ => 5 + overlap,
    }
}

fn query_playbook_match_score(
    playbook_name: &str,
    playbook: &AgentQueryPlaybookAsset,
    normalized_goal: &str,
    requested_entity_keys: &[String],
) -> usize {
    let mut score = 0usize;
    let playbook_key = normalize_goal_key(playbook_name);
    if !normalized_goal.is_empty() {
        if playbook_key == normalized_goal {
            score += 8;
        }
        if playbook_key.contains(normalized_goal) || normalized_goal.contains(&playbook_key) {
            score += 4;
        }
        score += token_overlap_score(&playbook_key, normalized_goal);
        for phrase in &playbook.when_to_use {
            let phrase_key = normalize_goal_key(phrase);
            if phrase_key == normalized_goal {
                score += 6;
            } else if !phrase_key.is_empty()
                && (phrase_key.contains(normalized_goal) || normalized_goal.contains(&phrase_key))
            {
                score += 3;
            }
            score += token_overlap_score(&phrase_key, normalized_goal);
        }
    }

    if !requested_entity_keys.is_empty() {
        for entity in &playbook.related_entities {
            let entity_key = normalize_entity_name(entity);
            if requested_entity_keys
                .iter()
                .any(|requested| requested == &entity_key)
            {
                score += 5;
            }
        }
    }

    score
}

fn default_schema_reference_asset(schema: &IfcSchemaId) -> AgentSchemaReferenceAsset {
    let schema_slug = schema.generated_artifact_stem().map(ToOwned::to_owned);
    let mut entities = common_entity_reference_assets();
    let (summary, cautions) = match schema {
        IfcSchemaId::Ifc2x3Tc1 => (
            "IFC2X3_TC1 schema. Expect older buildingSMART modeling patterns and fewer modern IFC4/IFC4X3 entity refinements.".to_owned(),
            vec![
                "Do not assume IFC4 or IFC4X3-only entities are present.".to_owned(),
                "Prefer checking actual neighboring relations and names before inferring semantics from type names alone.".to_owned(),
            ],
        ),
        IfcSchemaId::Ifc4 => (
            "IFC4 schema. General building product/spatial structure patterns apply, but IFC4X3 infrastructure additions may be absent.".to_owned(),
            vec![
                "Use IFC4 entity names and relation patterns; do not assume IFC4X3-specific domain coverage.".to_owned(),
            ],
        ),
        IfcSchemaId::Ifc4x3Add2 => (
            "IFC4X3_ADD2 schema. Modern IFC4X3 buildingSMART schema with broader domain coverage; building product and relation patterns still apply for viewer queries.".to_owned(),
            vec![
                "Broader schema coverage does not mean the current model uses every newer entity; inspect the actual graph before concluding.".to_owned(),
            ],
        ),
        IfcSchemaId::Other(token) => (
            format!(
                "Schema `{}`. Use the current model graph as ground truth and inspect relations before leaning on assumptions.",
                token
            ),
            vec![
                "This schema is not one of the curated IFC2X3/IFC4/IFC4X3 families, so use conservative exploration habits.".to_owned(),
            ],
        ),
    };

    if matches!(schema, IfcSchemaId::Ifc2x3Tc1) {
        entities.insert(
            "IfcProject".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Top-level IFC project root. Useful as a stable graph seed when you want the broad semantic structure of the model.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelDeclares".to_owned(),
                ],
                query_patterns: vec!["MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1".to_owned()],
                cautions: vec![],
            },
        );
    }

    AgentSchemaReferenceAsset {
        schema_id: schema.canonical_name().to_owned(),
        schema_slug,
        summary,
        cautions,
        query_habits: vec![
            "Return `id(...) AS node_id` when you want graph seeds.".to_owned(),
            "Return `...GlobalId AS global_id` when you want viewer element actions.".to_owned(),
            "Use `LIMIT` unless the user explicitly needs a full scan.".to_owned(),
            "Inspect neighboring relations before treating opaque enum values as the human answer."
                .to_owned(),
        ],
        query_playbooks: common_query_playbooks(),
        entities,
    }
}

fn default_query_playbook_asset_file(schema: &IfcSchemaId) -> AgentQueryPlaybookAssetFile {
    AgentQueryPlaybookAssetFile {
        schema_id: schema.canonical_name().to_owned(),
        schema_slug: schema.generated_artifact_stem().map(ToOwned::to_owned),
        playbooks: common_query_playbook_assets(),
    }
}

fn default_relation_reference_asset_file(schema: &IfcSchemaId) -> AgentRelationReferenceAssetFile {
    AgentRelationReferenceAssetFile {
        schema_id: schema.canonical_name().to_owned(),
        schema_slug: schema.generated_artifact_stem().map(ToOwned::to_owned),
        references: common_relation_reference_assets(),
    }
}

fn common_query_playbooks() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        (
            "model_overview".to_owned(),
            vec![
                "MATCH (p:IfcProject) RETURN id(p) AS node_id, p.Name AS project_name, p.GlobalId AS global_id LIMIT 1".to_owned(),
                "MATCH (b:IfcBuilding) RETURN count(b) AS building_count".to_owned(),
                "MATCH (bridge:IfcBridge) RETURN count(bridge) AS bridge_count".to_owned(),
                "MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 12".to_owned(),
            ],
        ),
        (
            "project_root".to_owned(),
            vec!["MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1".to_owned()],
        ),
        (
            "roof_slabs".to_owned(),
            vec![
                "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT slab.GlobalId AS global_id".to_owned(),
                "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT id(slab) AS node_id".to_owned(),
            ],
        ),
        (
            "bridge_to_products".to_owned(),
            vec![
                "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200".to_owned(),
                "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)--(:IfcRelAggregates)-->(subpart:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200".to_owned(),
            ],
        ),
        (
            "storey_containment".to_owned(),
            vec![
                "MATCH (:IfcBuildingStorey)<--(:IfcRelContainedInSpatialStructure)-->(element) RETURN DISTINCT id(element) AS node_id LIMIT 24".to_owned(),
            ],
        ),
    ])
}

fn common_query_playbook_assets() -> BTreeMap<String, AgentQueryPlaybookAsset> {
    BTreeMap::from([
        (
            "model_overview".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "For broad questions about what the current model is, start with a tiny overview: identify the project root, check one or two domain signals, and sample a small material vocabulary.".to_owned(),
                when_to_use: vec![
                    "what can you tell me about the model".to_owned(),
                    "what is in this model".to_owned(),
                    "give me an overview of the model".to_owned(),
                    "what kind of model is this".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (p:IfcProject) RETURN id(p) AS node_id, p.Name AS project_name, p.GlobalId AS global_id LIMIT 1".to_owned(),
                    "MATCH (b:IfcBuilding) RETURN count(b) AS building_count".to_owned(),
                    "MATCH (bridge:IfcBridge) RETURN count(bridge) AS bridge_count".to_owned(),
                    "MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 12".to_owned(),
                ],
                related_entities: vec![
                    "IfcProject".to_owned(),
                    "IfcBuilding".to_owned(),
                    "IfcBridge".to_owned(),
                    "IfcMaterial".to_owned(),
                ],
                cautions: vec![
                    "Keep the first pass tiny. Do not start an overview answer with a broad unlabeled scan or deep multi-join query.".to_owned(),
                    "Use one or two domain signals only, then explain what they imply instead of enumerating the whole model.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Broad unlabeled exploration before checking IfcProject.".to_owned(),
                    "Large product-family scans or graph walks as the first overview step.".to_owned(),
                ],
            },
        ),
        (
            "project_seed".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Start from a stable semantic root when the user wants a broad graph foothold or when the domain is still unclear.".to_owned(),
                when_to_use: vec![
                    "seed the graph from the project".to_owned(),
                    "find a stable root".to_owned(),
                    "start from the project".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1".to_owned(),
                ],
                related_entities: vec!["IfcProject".to_owned()],
                cautions: vec![
                    "Project roots are semantic anchors, not renderable elements.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Broad unlabeled scans before checking IfcProject.".to_owned(),
                ],
            },
        ),
        (
            "named_furniture_discovery".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Find candidate furnishing objects first, then inspect names and object types before writing a filtered discovery query.".to_owned(),
                when_to_use: vec![
                    "is there a kitchen unit".to_owned(),
                    "find furniture by name".to_owned(),
                    "find a furnishing candidate".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (n:IfcFurniture) RETURN id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type LIMIT 25".to_owned(),
                    "MATCH (n:IfcSystemFurnitureElement) RETURN id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type LIMIT 25".to_owned(),
                ],
                related_entities: vec![
                    "IfcFurniture".to_owned(),
                    "IfcSystemFurnitureElement".to_owned(),
                ],
                cautions: vec![
                    "Do not begin with `any(...)`, `coalesce(...)`, or multi-property text filters.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Dynamic property-list predicates over multiple text fields.".to_owned(),
                ],
            },
        ),
        (
            "relation_summary".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Summarize what a product family connects to using a tiny local pattern before trying filtered relation logic.".to_owned(),
                when_to_use: vec![
                    "what relations are slabs connected to".to_owned(),
                    "how is this connected".to_owned(),
                    "show the relation types".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (slab:IfcSlab)-[r]-(other) RETURN type(r) AS relation, count(*) AS connections ORDER BY connections DESC LIMIT 24".to_owned(),
                    "MATCH (wall:IfcWall)-[r]-(other) RETURN type(r) AS relation, count(*) AS connections ORDER BY connections DESC LIMIT 24".to_owned(),
                ],
                related_entities: vec!["IfcSlab".to_owned(), "IfcWall".to_owned()],
                cautions: vec![
                    "Start without WHERE filters. Add narrowing only if the plain summary is too broad.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Complex WHERE clauses before a first relation sample.".to_owned(),
                ],
            },
        ),
        (
            "roof_renderable_descendants".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Hide or show the visible roof by resolving the renderable descendants, usually slabs, instead of acting on the high-level IfcRoof node.".to_owned(),
                when_to_use: vec![
                    "hide the roof".to_owned(),
                    "show the roof".to_owned(),
                    "find roof slabs".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT slab.GlobalId AS global_id LIMIT 200".to_owned(),
                    "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT id(slab) AS node_id LIMIT 24".to_owned(),
                ],
                related_entities: vec!["IfcRoof".to_owned(), "IfcSlab".to_owned(), "IfcRelAggregates".to_owned()],
                cautions: vec![
                    "A roof node is often semantic only; viewer element actions need descendant GlobalId values.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Trying to hide an IfcRoof directly with element actions.".to_owned(),
                ],
            },
        ),
        (
            "material_scan".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "For broad composition questions, first list the materials actually present before attempting product-to-material joins.".to_owned(),
                when_to_use: vec![
                    "what is the house built of".to_owned(),
                    "what materials are in the model".to_owned(),
                    "material overview".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 24".to_owned(),
                ],
                related_entities: vec!["IfcRelAssociatesMaterial".to_owned(), "IfcMaterial".to_owned()],
                cautions: vec![
                    "If direct product-to-IfcMaterial joins return zero, say the evidence is indirect before presenting a full parts/material table.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Starting with a speculative multi-join material table query.".to_owned(),
                ],
            },
        ),
        (
            "bounded_descendant_discovery".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Use a small, relation-constrained variable-length traversal to discover candidate descendants when the exact one-hop structure is unclear.".to_owned(),
                when_to_use: vec![
                    "find renderable descendants".to_owned(),
                    "explore descendants".to_owned(),
                    "walk the local structure".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (root)-[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]-(n) RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name LIMIT 40".to_owned(),
                    "MATCH (root)-[:RELATING_OBJECT|RELATED_OBJECTS|RELATING_STRUCTURE|RELATED_ELEMENTS*1..3]-(n) RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name LIMIT 40".to_owned(),
                ],
                related_entities: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                cautions: vec![
                    "Keep the range bounded and the relation family as narrow as you can.".to_owned(),
                    "Use the varlen walk to discover candidates, then follow up with a simpler query on the concrete products you found.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Bare `[*]` or `[*1..3]` walks when the relation family is already known.".to_owned(),
                    "Open-ended `*0..` traversal as a first exploration step.".to_owned(),
                ],
            },
        ),
        (
            "bridge_renderable_descendants".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "For bridge viewer actions, descend from IfcBridge into bridge parts, then use containment to reach the actual visible products instead of hiding IfcBridgePart containers.".to_owned(),
                when_to_use: vec![
                    "hide the bridge".to_owned(),
                    "hide the rail bridge".to_owned(),
                    "show the bridge".to_owned(),
                    "find renderable bridge products".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200".to_owned(),
                    "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)--(:IfcRelAggregates)-->(subpart:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200".to_owned(),
                    "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT id(prod) AS node_id LIMIT 24".to_owned(),
                ],
                related_entities: vec![
                    "IfcBridge".to_owned(),
                    "IfcBridgePart".to_owned(),
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                cautions: vec![
                    "IfcBridge and IfcBridgePart are often semantic containers; viewer element actions usually need the contained product descendants' GlobalId values.".to_owned(),
                    "If the first bridge-part containment query only covers some of the visible structure, check one more aggregate hop for nested parts such as piers.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Trying to hide IfcBridgePart ids directly with element actions.".to_owned(),
                ],
            },
        ),
        (
            "storey_containment".to_owned(),
            AgentQueryPlaybookAsset {
                summary: "Use storey containment to explain where building elements sit in the spatial hierarchy.".to_owned(),
                when_to_use: vec![
                    "which storey contains this wall".to_owned(),
                    "show building storey structure".to_owned(),
                    "explain containment".to_owned(),
                ],
                recommended_patterns: vec![
                    "MATCH (:IfcBuildingStorey)<--(:IfcRelContainedInSpatialStructure)-->(element) RETURN DISTINCT id(element) AS node_id LIMIT 24".to_owned(),
                    "MATCH (storey:IfcBuildingStorey) RETURN id(storey) AS node_id LIMIT 8".to_owned(),
                ],
                related_entities: vec!["IfcBuildingStorey".to_owned(), "IfcRelContainedInSpatialStructure".to_owned()],
                cautions: vec![
                    "Containment explains hierarchy, not direct visibility.".to_owned(),
                ],
                avoid_patterns: vec![
                    "Assuming a storey query is useful before checking the model is building-centric.".to_owned(),
                ],
            },
        ),
    ])
}

fn common_relation_reference_assets() -> BTreeMap<String, AgentRelationReferenceAsset> {
    BTreeMap::from([
        (
            "IfcRelAggregates".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Decomposition relation connecting a semantic parent to aggregated children such as roof slabs, storeys, or project hierarchy nodes.".to_owned(),
                common_connections: vec![
                    "RELATING_OBJECT".to_owned(),
                    "RELATED_OBJECTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (parent)<--(rel:IfcRelAggregates)-->(child) RETURN id(rel) AS node_id, parent.Name AS parent_name, child.Name AS child_name LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "The relation itself is structural, not renderable.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelContainedInSpatialStructure".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Containment relation between elements and their spatial containers such as storeys or buildings.".to_owned(),
                common_connections: vec![
                    "RELATING_STRUCTURE".to_owned(),
                    "RELATED_ELEMENTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (container)<--(rel:IfcRelContainedInSpatialStructure)-->(element) RETURN id(rel) AS node_id, container.Name AS container_name, element.Name AS element_name LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "Good for explaining hierarchy; not by itself evidence of geometry.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelDefinesByType".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Type assignment relation. Useful when an instance name is weak and you need the attached type to explain the object.".to_owned(),
                common_connections: vec![
                    "RELATING_TYPE".to_owned(),
                    "RELATED_OBJECTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (instance)<--(rel:IfcRelDefinesByType)-->(type) RETURN id(rel) AS node_id, instance.Name AS instance_name, type.Name AS type_name LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "Prefer this when instance labels are generic or numeric type codes are unhelpful.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelDefinesByProperties".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Property-set attachment relation. Helpful for turning graph inspection into user-facing explanation.".to_owned(),
                common_connections: vec![
                    "RELATING_PROPERTY_DEFINITION".to_owned(),
                    "RELATED_OBJECTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (instance)<--(rel:IfcRelDefinesByProperties)-->(definition) RETURN id(rel) AS node_id, instance.Name AS instance_name, definition.Name AS definition_name LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "Property definitions may require a second inspection step for details.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelAssociatesMaterial".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Material-association relation between products and materials or intermediate material-definition nodes.".to_owned(),
                common_connections: vec![
                    "RELATING_MATERIAL".to_owned(),
                    "RELATED_OBJECTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (rel:IfcRelAssociatesMaterial)--(other) RETURN DISTINCT labels(other)[0] AS node_label, count(*) AS connections ORDER BY connections DESC LIMIT 24".to_owned(),
                    "MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "A direct IfcMaterial hop may be absent even when indirect material-definition nodes exist.".to_owned(),
                ],
            },
        ),
        (
            "RELATED_OBJECTS".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Common outgoing role from aggregate, type, property, and material relations toward the related product objects.".to_owned(),
                common_connections: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelDefinesByType".to_owned(),
                    "IfcRelDefinesByProperties".to_owned(),
                    "IfcRelAssociatesMaterial".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (rel)-[:RELATED_OBJECTS]->(other) RETURN labels(rel)[0] AS relation_label, labels(other)[0] AS other_label, count(*) AS connections ORDER BY connections DESC LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "The role name alone is not enough; inspect the relation node label too.".to_owned(),
                ],
            },
        ),
        (
            "RELATED_ELEMENTS".to_owned(),
            AgentRelationReferenceAsset {
                summary: "Containment/connectivity role from relation nodes toward the participating product elements.".to_owned(),
                common_connections: vec![
                    "IfcRelContainedInSpatialStructure".to_owned(),
                    "IfcRelConnects".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (rel)-[:RELATED_ELEMENTS]->(other) RETURN labels(rel)[0] AS relation_label, labels(other)[0] AS other_label, count(*) AS connections ORDER BY connections DESC LIMIT 24".to_owned(),
                ],
                cautions: vec![
                    "Use the relation node label to distinguish containment from connectivity.".to_owned(),
                ],
            },
        ),
    ])
}

fn common_entity_reference_assets() -> BTreeMap<String, AgentEntityReferenceAsset> {
    BTreeMap::from([
        (
            "IfcRoof".to_owned(),
            AgentEntityReferenceAsset {
                summary: "High-level roof product or aggregate. In many models it describes semantic structure rather than directly carrying the visible roof geometry.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelDefinesByProperties".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (roof:IfcRoof) RETURN id(roof) AS node_id LIMIT 8".to_owned(),
                    "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT slab.GlobalId AS global_id".to_owned(),
                ],
                cautions: vec![
                    "Do not assume the IfcRoof node itself is directly renderable; inspect aggregated slabs or other descendants.".to_owned(),
                ],
            },
        ),
        (
            "IfcSlab".to_owned(),
            AgentEntityReferenceAsset {
                summary: "General slab-like product. Slabs can represent floors, roofs, landings, or other planar building elements depending on context.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                    "IfcRelDefinesByType".to_owned(),
                    "IfcRelDefinesByProperties".to_owned(),
                    "IfcRelAssociatesMaterial".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (slab:IfcSlab) RETURN id(slab) AS node_id LIMIT 8".to_owned(),
                    "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT id(slab) AS node_id".to_owned(),
                ],
                cautions: vec![
                    "A slab is not necessarily a roof slab; use neighboring relation context, names, and containers before concluding.".to_owned(),
                ],
            },
        ),
        (
            "IfcWall".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Wall product. Often directly renderable and usually a good candidate for element actions when you have its GlobalId.".to_owned(),
                common_relations: vec![
                    "IfcRelContainedInSpatialStructure".to_owned(),
                    "IfcRelDefinesByType".to_owned(),
                    "IfcRelDefinesByProperties".to_owned(),
                    "IfcRelAssociatesMaterial".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (w:IfcWall) RETURN id(w) AS node_id LIMIT 8".to_owned(),
                    "MATCH (w:IfcWall) RETURN w.GlobalId AS global_id LIMIT 8".to_owned(),
                ],
                cautions: vec![],
            },
        ),
        (
            "IfcBuildingStorey".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Spatial container for building elements on a storey. Useful for explaining containment and building hierarchy.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (storey:IfcBuildingStorey) RETURN id(storey) AS node_id LIMIT 8".to_owned(),
                ],
                cautions: vec![
                    "Storeys are usually semantic/spatial nodes rather than directly renderable geometry.".to_owned(),
                ],
            },
        ),
        (
            "IfcBridge".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Infrastructure facility root. It usually organizes bridge semantics, while the visible geometry often hangs off descendant bridge parts or contained products.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (bridge:IfcBridge) RETURN id(bridge) AS node_id LIMIT 8".to_owned(),
                    "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 32".to_owned(),
                ],
                cautions: vec![
                    "Do not assume the IfcBridge node itself is directly renderable; inspect descendant bridge parts and the products contained by those parts.".to_owned(),
                ],
            },
        ),
        (
            "IfcBridgePart".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Bridge subdivision/container node. In infrastructure models it often sits between the bridge root and the visible columns, walls, members, fill, and other products.".to_owned(),
                common_relations: vec![
                    "IfcRelAggregates".to_owned(),
                    "IfcRelContainedInSpatialStructure".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (part:IfcBridgePart) RETURN id(part) AS node_id LIMIT 8".to_owned(),
                    "MATCH (part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 32".to_owned(),
                ],
                cautions: vec![
                    "IfcBridgePart is often still semantic/spatial structure. For viewer actions, prefer the contained descendant products' GlobalId values.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelAggregates".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Aggregation relationship node. Often connects semantic containers like roofs, projects, or storeys to their children.".to_owned(),
                common_relations: vec![
                    "RELATING_OBJECT".to_owned(),
                    "RELATED_OBJECTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (:IfcRoof)<--(rel:IfcRelAggregates)-->() RETURN id(rel) AS node_id LIMIT 8".to_owned(),
                ],
                cautions: vec![
                    "Aggregation nodes describe graph structure, not renderable elements.".to_owned(),
                ],
            },
        ),
        (
            "IfcRelContainedInSpatialStructure".to_owned(),
            AgentEntityReferenceAsset {
                summary: "Containment relationship node linking products to spatial containers such as storeys or buildings.".to_owned(),
                common_relations: vec![
                    "RELATING_STRUCTURE".to_owned(),
                    "RELATED_ELEMENTS".to_owned(),
                ],
                query_patterns: vec![
                    "MATCH (:IfcBuildingStorey)<--(rel:IfcRelContainedInSpatialStructure)-->() RETURN id(rel) AS node_id LIMIT 8".to_owned(),
                ],
                cautions: vec![
                    "Containment explains where an element sits in the building hierarchy, not whether it is renderable.".to_owned(),
                ],
            },
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reference_includes_schema_id_and_playbooks() {
        let reference = load_schema_context(Path::new("/tmp/no-assets-here"), &IfcSchemaId::Ifc4)
            .expect("default schema reference should load");

        assert_eq!(reference.schema_id, "IFC4");
        assert!(reference.query_playbooks.contains_key("roof_slabs"));
    }

    #[test]
    fn entity_lookup_is_case_insensitive() {
        let references = load_entity_references(
            Path::new("/tmp/no-assets-here"),
            &IfcSchemaId::Ifc4x3Add2,
            &[String::from("ifcrelaggregates"), String::from("IfcRoof")],
        )
        .expect("entity references should load");

        assert_eq!(references.len(), 2);
        assert_eq!(references[0].entity_name, "IfcRelAggregates");
        assert_eq!(references[1].entity_name, "IfcRoof");
    }

    #[test]
    fn query_playbooks_match_goal_and_entities() {
        let playbooks = load_query_playbooks(
            Path::new("/tmp/no-assets-here"),
            &IfcSchemaId::Ifc4x3Add2,
            "hide the roof",
            &[String::from("IfcRoof"), String::from("IfcSlab")],
        )
        .expect("query playbooks should load");

        assert!(!playbooks.is_empty());
        assert_eq!(playbooks[0].playbook_name, "roof_renderable_descendants");
    }

    #[test]
    fn query_playbooks_match_bridge_goal_with_loose_wording() {
        let playbooks = load_query_playbooks(
            Path::new("/tmp/no-assets-here"),
            &IfcSchemaId::Ifc4x3Add2,
            "hide the rail bridge by finding renderable related products or parts with semantic ids",
            &[String::from("IfcBridge"), String::from("IfcBridgePart")],
        )
        .expect("query playbooks should load");

        assert!(!playbooks.is_empty());
        assert_eq!(playbooks[0].playbook_name, "bridge_renderable_descendants");
    }

    #[test]
    fn query_playbooks_match_model_overview_goal() {
        let playbooks = load_query_playbooks(
            Path::new("/tmp/no-assets-here"),
            &IfcSchemaId::Ifc4x3Add2,
            "what can you tell me about the model",
            &[],
        )
        .expect("query playbooks should load");

        assert!(!playbooks.is_empty());
        assert_eq!(playbooks[0].playbook_name, "model_overview");
    }

    #[test]
    fn relation_lookup_is_case_insensitive() {
        let references = load_relation_references(
            Path::new("/tmp/no-assets-here"),
            &IfcSchemaId::Ifc4,
            &[
                String::from("related_objects"),
                String::from("ifcrelassociatesmaterial"),
            ],
        )
        .expect("relation references should load");

        assert_eq!(references.len(), 2);
        assert_eq!(references[0].relation_name, "IfcRelAssociatesMaterial");
        assert_eq!(references[1].relation_name, "RELATED_OBJECTS");
    }
}
