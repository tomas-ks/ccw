use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    net::TcpStream,
    time::Instant,
};

use cc_w_velr::CypherQueryResult;
use serde::Serialize;

use super::{
    ConsoleLogKind, CypherResourceScope, CypherResourceTarget, HttpRequest, ServerState,
    console_log, execute_cypher_worker, find_column_index, is_project_resource_id,
    parse_optional_string_cell, required_column_index, resolve_cypher_resource_scope,
    summarize_query_for_log, validate_agent_readonly_cypher, write_json_error, write_json_response,
    write_response,
};

const LAYERS_FACET_ID: &str = "layers";
const CLASSES_FACET_ID: &str = "classes";
const SPATIAL_FACILITY_FACET_ID: &str = "spatial";
const MATERIALS_STYLES_FACET_ID: &str = "materials";
const CONSTRUCTION_STATE_HINTS_FACET_ID: &str = "construction";

const IFC_GRAPH_PROVENANCE: &str = "ifc_graph";

const LAYER_PRODUCTS_QUERY: &str = "\
MATCH (layer:IfcPresentationLayerAssignment)-[:ASSIGNED_ITEMS]->(rep:IfcShapeRepresentation)<-[:REPRESENTATIONS]-(:IfcProductDefinitionShape)<-[:REPRESENTATION]-(product:IfcProduct)
RETURN DISTINCT layer.Name AS group_name, layer.Identifier AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

const CLASS_PRODUCTS_QUERY: &str = "\
MATCH (product:IfcProduct)
RETURN DISTINCT product.declared_entity AS group_name, id(product) AS product_node_id, product.GlobalId AS global_id, product.Name AS product_name
ORDER BY group_name, global_id";

const CLASS_CONTAINED_PRODUCTS_QUERY: &str = "\
MATCH (container:IfcProduct)<-[:RELATING_STRUCTURE]-(:IfcRelContainedInSpatialStructure)-[:RELATED_ELEMENTS]->(product:IfcProduct)
RETURN DISTINCT container.declared_entity AS group_name, container.GlobalId AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

const CLASS_AGGREGATED_PRODUCTS_QUERY: &str = "\
MATCH (container:IfcProduct)<-[:RELATING_OBJECT]-(:IfcRelAggregates)-[:RELATED_OBJECTS]->(product:IfcProduct)
RETURN DISTINCT container.declared_entity AS group_name, container.GlobalId AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

const CLASS_AGGREGATED_CONTAINED_PRODUCTS_QUERY: &str = "\
MATCH (container:IfcProduct)<-[:RELATING_OBJECT]-(:IfcRelAggregates)-[:RELATED_OBJECTS]->(spatial:IfcProduct)
MATCH (spatial)<-[:RELATING_STRUCTURE]-(:IfcRelContainedInSpatialStructure)-[:RELATED_ELEMENTS]->(product:IfcProduct)
RETURN DISTINCT container.declared_entity AS group_name, container.GlobalId AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

const SPATIAL_PRODUCTS_QUERY: &str = "\
MATCH (container:IfcProduct)<-[:RELATING_STRUCTURE]-(:IfcRelContainedInSpatialStructure)-[:RELATED_ELEMENTS]->(product:IfcProduct)
RETURN DISTINCT container.Name AS group_name, container.GlobalId AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

const MATERIAL_PRODUCTS_QUERY: &str = "\
MATCH (product:IfcProduct)<-[:RELATED_OBJECTS]-(:IfcRelAssociatesMaterial)-[:RELATING_MATERIAL]->(material)
RETURN DISTINCT material.Name AS group_name, material.declared_entity AS identifier, id(product) AS product_node_id, product.GlobalId AS global_id, product.declared_entity AS declared_entity, product.Name AS product_name
ORDER BY group_name, global_id";

pub(super) fn serve_semantic_outliner_api(
    stream: &mut TcpStream,
    head_only: bool,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let resource = parse_semantic_outliner_resource(&request.target);
    let parse_ms = parse_started.elapsed().as_millis();

    if head_only {
        return write_response(
            stream,
            if resource.is_ok() {
                "200 OK"
            } else {
                "400 Bad Request"
            },
            "application/json; charset=utf-8",
            b"",
            true,
        );
    }

    let resource = match resource {
        Ok(resource) => resource,
        Err(error) => return write_json_error(stream, "400 Bad Request", &error),
    };

    let execute_started = Instant::now();
    match build_semantic_outliner_response(&resource, state) {
        Ok((response, metrics)) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] semantic_outliner resource={} targets={} parse_ms={} open_ms={} query_ms={} exec_ms={} write_ms={} total_ms={} layer_groups={} class_groups={} diagnostics={}",
                response.resource,
                metrics.targets,
                parse_ms,
                metrics.open_ms,
                metrics.query_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                response.group_count(LAYERS_FACET_ID),
                response.group_count(CLASSES_FACET_ID),
                response.diagnostic_count(),
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            console_log(
                ConsoleLogKind::Error,
                format!(
                    "semantic_outliner resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} error={}",
                    resource,
                    parse_ms,
                    execute_ms,
                    write_ms,
                    request_started.elapsed().as_millis(),
                    error,
                ),
            );
            write_result
        }
    }
}

fn build_semantic_outliner_response(
    resource: &str,
    state: &ServerState,
) -> Result<(SemanticOutlinerApiResponse, SemanticOutlinerMetrics), String> {
    let scope = resolve_cypher_resource_scope(
        &state.project_registry,
        resource,
        &[],
        &state.ifc_artifacts_root,
    )?;
    let targets = scope_targets(&scope);
    let mut metrics = SemanticOutlinerMetrics {
        targets: targets.len(),
        ..SemanticOutlinerMetrics::default()
    };

    let layers_facet = build_layers_facet(resource, state, &targets, &mut metrics);
    let classes_facet = build_classes_facet(resource, state, &targets, &mut metrics);
    let spatial_facet = build_spatial_facet(resource, state, &targets, &mut metrics);
    let materials_facet = build_materials_facet(resource, state, &targets, &mut metrics);
    let construction_facet = build_construction_state_hints_facet(&layers_facet);
    let facets = vec![
        layers_facet,
        classes_facet,
        spatial_facet,
        materials_facet,
        construction_facet,
    ];

    Ok((
        SemanticOutlinerApiResponse {
            resource: resource.to_owned(),
            facets,
            diagnostics: Vec::new(),
        },
        metrics,
    ))
}

fn build_layers_facet(
    response_resource: &str,
    state: &ServerState,
    targets: &[CypherResourceTarget],
    metrics: &mut SemanticOutlinerMetrics,
) -> SemanticOutlinerFacet {
    let (results, query_metrics, mut diagnostics) =
        execute_outliner_query(state, targets, LAYERS_FACET_ID, LAYER_PRODUCTS_QUERY);
    metrics.open_ms += query_metrics.open_ms;
    metrics.query_ms += query_metrics.query_ms;

    let mut group_rows = Vec::new();
    for result in results {
        let rows = result.result.rows.len();
        metrics.layer_rows += rows;
        match parse_group_rows(&result.source_resource, &result.result, GroupRowKind::Layer) {
            Ok(mut parsed) => group_rows.append(&mut parsed),
            Err(error) => diagnostics.push(SemanticOutlinerDiagnostic::warning(
                "layers_parse_failed",
                error,
                Some(result.source_resource),
            )),
        }
    }

    let groups = aggregate_product_groups(
        response_resource,
        LAYERS_FACET_ID,
        "layer",
        "Unnamed presentation layer",
        group_rows,
    );
    if groups.is_empty() && diagnostics.is_empty() {
        diagnostics.push(SemanticOutlinerDiagnostic::info(
            "no_layer_groups",
            "No IfcPresentationLayerAssignment to IfcProduct groups were found in the source graph.",
            None,
        ));
    }

    SemanticOutlinerFacet {
        id: LAYERS_FACET_ID.to_owned(),
        label: "Layers".to_owned(),
        provenance: IFC_GRAPH_PROVENANCE.to_owned(),
        groups,
        diagnostics,
    }
}

fn build_classes_facet(
    response_resource: &str,
    state: &ServerState,
    targets: &[CypherResourceTarget],
    metrics: &mut SemanticOutlinerMetrics,
) -> SemanticOutlinerFacet {
    let mut group_rows = Vec::new();
    let mut diagnostics = Vec::new();
    for (query_name, cypher) in [
        ("direct", CLASS_PRODUCTS_QUERY),
        ("contained_descendants", CLASS_CONTAINED_PRODUCTS_QUERY),
        ("aggregated_descendants", CLASS_AGGREGATED_PRODUCTS_QUERY),
        (
            "aggregated_contained_descendants",
            CLASS_AGGREGATED_CONTAINED_PRODUCTS_QUERY,
        ),
    ] {
        let (results, query_metrics, mut query_diagnostics) =
            execute_outliner_query(state, targets, CLASSES_FACET_ID, cypher);
        metrics.open_ms += query_metrics.open_ms;
        metrics.query_ms += query_metrics.query_ms;
        diagnostics.append(&mut query_diagnostics);

        for result in results {
            let rows = result.result.rows.len();
            metrics.class_rows += rows;
            match parse_group_rows(&result.source_resource, &result.result, GroupRowKind::Class) {
                Ok(mut parsed) => {
                    for row in &mut parsed {
                        row.metadata
                            .entry("classMembership".to_owned())
                            .or_insert_with(|| query_name.to_owned());
                    }
                    group_rows.append(&mut parsed);
                }
                Err(error) => diagnostics.push(SemanticOutlinerDiagnostic::warning(
                    "classes_parse_failed",
                    error,
                    Some(result.source_resource),
                )),
            }
        }
    }

    let groups = aggregate_product_groups(
        response_resource,
        CLASSES_FACET_ID,
        "class",
        "IfcProduct (declared entity missing)",
        group_rows,
    );
    if groups.is_empty() && diagnostics.is_empty() {
        diagnostics.push(SemanticOutlinerDiagnostic::info(
            "no_class_groups",
            "No IfcProduct class groups or explicit contained/aggregated class descendants were found in the source graph.",
            None,
        ));
    }

    SemanticOutlinerFacet {
        id: CLASSES_FACET_ID.to_owned(),
        label: "Classes".to_owned(),
        provenance: IFC_GRAPH_PROVENANCE.to_owned(),
        groups,
        diagnostics,
    }
}

fn build_spatial_facet(
    response_resource: &str,
    state: &ServerState,
    targets: &[CypherResourceTarget],
    metrics: &mut SemanticOutlinerMetrics,
) -> SemanticOutlinerFacet {
    let (results, query_metrics, mut diagnostics) = execute_outliner_query(
        state,
        targets,
        SPATIAL_FACILITY_FACET_ID,
        SPATIAL_PRODUCTS_QUERY,
    );
    metrics.open_ms += query_metrics.open_ms;
    metrics.query_ms += query_metrics.query_ms;

    let mut group_rows = Vec::new();
    for result in results {
        match parse_group_rows(
            &result.source_resource,
            &result.result,
            GroupRowKind::Spatial,
        ) {
            Ok(mut parsed) => group_rows.append(&mut parsed),
            Err(error) => diagnostics.push(SemanticOutlinerDiagnostic::warning(
                "spatial_parse_failed",
                error,
                Some(result.source_resource),
            )),
        }
    }

    let groups = aggregate_product_groups(
        response_resource,
        SPATIAL_FACILITY_FACET_ID,
        "spatial",
        "Uncontained facility/spatial group",
        group_rows,
    );
    if groups.is_empty() && diagnostics.is_empty() {
        diagnostics.push(SemanticOutlinerDiagnostic::info(
            "no_spatial_groups",
            "No IfcRelContainedInSpatialStructure product groups were found in the source graph.",
            None,
        ));
    }

    SemanticOutlinerFacet {
        id: SPATIAL_FACILITY_FACET_ID.to_owned(),
        label: "Spatial".to_owned(),
        provenance: IFC_GRAPH_PROVENANCE.to_owned(),
        groups,
        diagnostics,
    }
}

fn build_materials_facet(
    response_resource: &str,
    state: &ServerState,
    targets: &[CypherResourceTarget],
    metrics: &mut SemanticOutlinerMetrics,
) -> SemanticOutlinerFacet {
    let (results, query_metrics, mut diagnostics) = execute_outliner_query(
        state,
        targets,
        MATERIALS_STYLES_FACET_ID,
        MATERIAL_PRODUCTS_QUERY,
    );
    metrics.open_ms += query_metrics.open_ms;
    metrics.query_ms += query_metrics.query_ms;

    let mut group_rows = Vec::new();
    for result in results {
        match parse_group_rows(
            &result.source_resource,
            &result.result,
            GroupRowKind::Material,
        ) {
            Ok(mut parsed) => group_rows.append(&mut parsed),
            Err(error) => diagnostics.push(SemanticOutlinerDiagnostic::warning(
                "materials_parse_failed",
                error,
                Some(result.source_resource),
            )),
        }
    }

    let groups = aggregate_product_groups(
        response_resource,
        MATERIALS_STYLES_FACET_ID,
        "material",
        "Material association without name",
        group_rows,
    );
    if groups.is_empty() && diagnostics.is_empty() {
        diagnostics.push(SemanticOutlinerDiagnostic::info(
            "no_material_groups",
            "No IfcRelAssociatesMaterial product groups were found in the source graph.",
            None,
        ));
    }

    SemanticOutlinerFacet {
        id: MATERIALS_STYLES_FACET_ID.to_owned(),
        label: "Materials".to_owned(),
        provenance: IFC_GRAPH_PROVENANCE.to_owned(),
        groups,
        diagnostics,
    }
}

fn build_construction_state_hints_facet(
    layers_facet: &SemanticOutlinerFacet,
) -> SemanticOutlinerFacet {
    let mut groups = Vec::new();
    for layer_group in &layers_facet.groups {
        let Some((state_label, detail)) = construction_state_hint_for_layer(&layer_group.label)
        else {
            continue;
        };
        let mut metadata = BTreeMap::new();
        metadata.insert("layer".to_owned(), layer_group.label.clone());
        metadata.insert("stateHint".to_owned(), state_label.to_owned());
        groups.push(SemanticOutlinerGroup {
            id: format!(
                "{}:{}",
                CONSTRUCTION_STATE_HINTS_FACET_ID,
                slugify(&layer_group.label)
            ),
            label: format!("{state_label}: {}", layer_group.label),
            kind: "construction_state_hint".to_owned(),
            provenance: "viewer_inference".to_owned(),
            source_resources: layer_group.source_resources.clone(),
            element_count: layer_group.element_count,
            semantic_ids: layer_group.semantic_ids.clone(),
            metadata,
            diagnostics: vec![SemanticOutlinerDiagnostic::info(
                "construction_state_from_layer_name",
                detail,
                None,
            )],
        });
    }

    let diagnostics = if groups.is_empty() {
        vec![SemanticOutlinerDiagnostic::info(
            "no_construction_state_hints",
            "No construction/state groups were inferred. This facet only uses explicit process facts or clearly paired layer naming conventions; none were found in the current layer groups.",
            None,
        )]
    } else {
        vec![SemanticOutlinerDiagnostic::info(
            "construction_state_hints_are_inferred",
            "Construction/state groups in this facet are viewer inferences from authored presentation layer names, not IFC source facts.",
            None,
        )]
    };

    SemanticOutlinerFacet {
        id: CONSTRUCTION_STATE_HINTS_FACET_ID.to_owned(),
        label: "State".to_owned(),
        provenance: "viewer_inference".to_owned(),
        groups,
        diagnostics,
    }
}

fn construction_state_hint_for_layer(layer: &str) -> Option<(&'static str, &'static str)> {
    let normalized = layer.to_ascii_lowercase();
    if normalized.contains("ante operam")
        || normalized.contains("existing")
        || normalized.contains("before")
    {
        return Some((
            "Existing / before",
            "Layer name suggests an existing or before-works state. This is an authored naming convention, not a formal IFC visibility flag.",
        ));
    }
    if normalized.contains("post operam")
        || normalized.contains("proposed")
        || normalized.contains("after")
    {
        return Some((
            "Proposed / after",
            "Layer name suggests a proposed or after-works state. This is an authored naming convention, not a formal IFC visibility flag.",
        ));
    }
    None
}

fn execute_outliner_query(
    state: &ServerState,
    targets: &[CypherResourceTarget],
    facet_id: &str,
    cypher: &str,
) -> (
    Vec<ScopedCypherQueryResult>,
    SemanticOutlinerQueryMetrics,
    Vec<SemanticOutlinerDiagnostic>,
) {
    let mut results = Vec::new();
    let mut metrics = SemanticOutlinerQueryMetrics::default();
    let mut diagnostics = Vec::new();
    let cypher = match validate_agent_readonly_cypher(cypher) {
        Ok(cypher) => cypher,
        Err(error) => {
            diagnostics.push(SemanticOutlinerDiagnostic::error(
                "outliner_query_not_readonly",
                error,
                None,
            ));
            return (results, metrics, diagnostics);
        }
    };

    for target in targets {
        match execute_cypher_worker(state, target, &cypher, state.cypher_worker.timeout) {
            Ok(run) => {
                metrics.open_ms += run.open_ms;
                metrics.query_ms += run.query_ms;
                results.push(ScopedCypherQueryResult {
                    source_resource: target.resource.clone(),
                    result: run.result,
                });
            }
            Err(error) => {
                let mut details = BTreeMap::new();
                details.insert("facetId".to_owned(), facet_id.to_owned());
                details.insert("query".to_owned(), summarize_query_for_log(&cypher));
                diagnostics.push(SemanticOutlinerDiagnostic::warning_with_details(
                    "outliner_query_failed",
                    format!(
                        "Could not load `{facet_id}` groups for `{}`: {error}",
                        target.resource
                    ),
                    Some(target.resource.clone()),
                    details,
                ));
            }
        }
    }

    (results, metrics, diagnostics)
}

fn parse_group_rows(
    source_resource: &str,
    result: &CypherQueryResult,
    kind: GroupRowKind,
) -> Result<Vec<ProductGroupRow>, String> {
    let group_index = required_column_index(&result.columns, &["groupname"], "group_name")?;
    let product_node_index =
        required_column_index(&result.columns, &["productnodeid"], "product_node_id")?;
    let global_id_index = find_column_index(&result.columns, &["globalid"]);
    let identifier_index = find_column_index(&result.columns, &["identifier"]);
    let declared_entity_index = find_column_index(&result.columns, &["declaredentity"]);
    let product_name_index = find_column_index(&result.columns, &["productname"]);

    let mut rows = Vec::new();
    for row in &result.rows {
        let group_name = parse_optional_string_cell(row.get(group_index));
        let product_node_id = row
            .get(product_node_index)
            .map(String::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let global_id =
            global_id_index.and_then(|index| parse_optional_string_cell(row.get(index)));
        let identifier =
            identifier_index.and_then(|index| parse_optional_string_cell(row.get(index)));
        let declared_entity =
            declared_entity_index.and_then(|index| parse_optional_string_cell(row.get(index)));
        let product_name =
            product_name_index.and_then(|index| parse_optional_string_cell(row.get(index)));
        let mut metadata = BTreeMap::new();
        if let Some(identifier) = identifier {
            metadata.insert("identifier".to_owned(), identifier);
        }
        if let Some(declared_entity) = declared_entity {
            metadata.insert("declaredEntitySample".to_owned(), declared_entity);
        }
        if let Some(product_name) = product_name {
            metadata.insert("productNameSample".to_owned(), product_name);
        }
        rows.push(ProductGroupRow {
            source_resource: source_resource.to_owned(),
            group_name,
            product_node_id,
            global_id,
            metadata,
            kind,
        });
    }
    Ok(rows)
}

fn aggregate_product_groups(
    response_resource: &str,
    facet_id: &str,
    kind: &str,
    fallback_label: &str,
    rows: Vec<ProductGroupRow>,
) -> Vec<SemanticOutlinerGroup> {
    let mut groups = BTreeMap::<String, ProductGroupAccumulator>::new();
    for row in rows {
        let label = row
            .group_name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback_label.to_owned());
        let key = format!("{facet_id}:{}", slugify(&label));
        let accumulator = groups
            .entry(key.clone())
            .or_insert_with(|| ProductGroupAccumulator {
                id: key,
                label,
                kind: kind.to_owned(),
                provenance: IFC_GRAPH_PROVENANCE.to_owned(),
                source_resources: BTreeSet::new(),
                node_keys: HashSet::new(),
                semantic_ids: Vec::new(),
                seen_semantic_ids: HashSet::new(),
                metadata: BTreeMap::new(),
                diagnostics: Vec::new(),
            });
        accumulator
            .source_resources
            .insert(row.source_resource.clone());
        for (key, value) in row.metadata {
            accumulator.metadata.entry(key).or_insert(value);
        }
        if !row.product_node_id.is_empty() {
            accumulator
                .node_keys
                .insert(format!("{}:{}", row.source_resource, row.product_node_id));
        }
        if let Some(global_id) = row.global_id.as_deref() {
            let semantic_id =
                scoped_semantic_id(response_resource, &row.source_resource, global_id);
            if accumulator.seen_semantic_ids.insert(semantic_id.clone()) {
                accumulator.semantic_ids.push(semantic_id);
            }
        }
        if row.global_id.is_none() && row.kind == GroupRowKind::Layer {
            accumulator.diagnostics.push(SemanticOutlinerDiagnostic::info(
                "layer_product_missing_global_id",
                "A product assigned to this presentation layer has no GlobalId, so it is counted but not added to semanticIds.",
                Some(row.source_resource),
            ));
        }
    }

    groups
        .into_values()
        .map(ProductGroupAccumulator::into_group)
        .collect()
}

fn scope_targets(scope: &CypherResourceScope) -> Vec<CypherResourceTarget> {
    match scope {
        CypherResourceScope::Single(target) => vec![target.clone()],
        CypherResourceScope::Project { targets, .. } => targets.clone(),
    }
}

fn parse_semantic_outliner_resource(target: &str) -> Result<String, String> {
    let resource = query_param(target, "resource")?
        .ok_or_else(|| "semantic outliner requires a `resource` query parameter".to_owned())?;
    let resource = resource.trim();
    if resource.is_empty() {
        return Err("semantic outliner `resource` query parameter must not be empty".to_owned());
    }
    Ok(resource.to_owned())
}

fn query_param(target: &str, key: &str) -> Result<Option<String>, String> {
    let Some((_, query)) = target.split_once('?') else {
        return Ok(None);
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_query_component(raw_name)?;
        if name == key {
            return percent_decode_query_component(raw_value).map(Some);
        }
    }
    Ok(None)
}

fn percent_decode_query_component(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(format!("invalid percent-encoded query component `{value}`"));
                }
                let high = hex_value(bytes[index + 1])
                    .ok_or_else(|| format!("invalid percent-encoded query component `{value}`"))?;
                let low = hex_value(bytes[index + 2])
                    .ok_or_else(|| format!("invalid percent-encoded query component `{value}`"))?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded)
        .map_err(|error| format!("invalid UTF-8 in query component `{value}`: {error}"))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn scoped_semantic_id(response_resource: &str, source_resource: &str, local_id: &str) -> String {
    if is_project_resource_id(response_resource) {
        format!("{source_resource}::{local_id}")
    } else {
        local_id.to_owned()
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    if slug.is_empty() {
        "unnamed".to_owned()
    } else {
        slug
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SemanticOutlinerApiResponse {
    resource: String,
    facets: Vec<SemanticOutlinerFacet>,
    diagnostics: Vec<SemanticOutlinerDiagnostic>,
}

impl SemanticOutlinerApiResponse {
    fn group_count(&self, facet_id: &str) -> usize {
        self.facets
            .iter()
            .find(|facet| facet.id == facet_id)
            .map(|facet| facet.groups.len())
            .unwrap_or(0)
    }

    fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
            + self
                .facets
                .iter()
                .map(|facet| facet.diagnostics.len())
                .sum::<usize>()
            + self
                .facets
                .iter()
                .flat_map(|facet| facet.groups.iter())
                .map(|group| group.diagnostics.len())
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SemanticOutlinerFacet {
    id: String,
    label: String,
    provenance: String,
    groups: Vec<SemanticOutlinerGroup>,
    diagnostics: Vec<SemanticOutlinerDiagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SemanticOutlinerGroup {
    id: String,
    label: String,
    kind: String,
    provenance: String,
    source_resources: Vec<String>,
    element_count: usize,
    semantic_ids: Vec<String>,
    metadata: BTreeMap<String, String>,
    diagnostics: Vec<SemanticOutlinerDiagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SemanticOutlinerDiagnostic {
    severity: String,
    code: String,
    message: String,
    resource: Option<String>,
    details: BTreeMap<String, String>,
}

impl SemanticOutlinerDiagnostic {
    fn info(code: impl Into<String>, message: impl Into<String>, resource: Option<String>) -> Self {
        Self {
            severity: "info".to_owned(),
            code: code.into(),
            message: message.into(),
            resource,
            details: BTreeMap::new(),
        }
    }

    fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        resource: Option<String>,
    ) -> Self {
        Self {
            severity: "warning".to_owned(),
            code: code.into(),
            message: message.into(),
            resource,
            details: BTreeMap::new(),
        }
    }

    fn warning_with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        resource: Option<String>,
        details: BTreeMap<String, String>,
    ) -> Self {
        Self {
            severity: "warning".to_owned(),
            code: code.into(),
            message: message.into(),
            resource,
            details,
        }
    }

    fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        resource: Option<String>,
    ) -> Self {
        Self {
            severity: "error".to_owned(),
            code: code.into(),
            message: message.into(),
            resource,
            details: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default)]
struct SemanticOutlinerMetrics {
    targets: usize,
    open_ms: u128,
    query_ms: u128,
    layer_rows: usize,
    class_rows: usize,
}

#[derive(Debug, Default)]
struct SemanticOutlinerQueryMetrics {
    open_ms: u128,
    query_ms: u128,
}

#[derive(Debug)]
struct ScopedCypherQueryResult {
    source_resource: String,
    result: CypherQueryResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupRowKind {
    Layer,
    Class,
    Spatial,
    Material,
}

#[derive(Debug)]
struct ProductGroupRow {
    source_resource: String,
    group_name: Option<String>,
    product_node_id: String,
    global_id: Option<String>,
    metadata: BTreeMap<String, String>,
    kind: GroupRowKind,
}

struct ProductGroupAccumulator {
    id: String,
    label: String,
    kind: String,
    provenance: String,
    source_resources: BTreeSet<String>,
    node_keys: HashSet<String>,
    semantic_ids: Vec<String>,
    seen_semantic_ids: HashSet<String>,
    metadata: BTreeMap<String, String>,
    diagnostics: Vec<SemanticOutlinerDiagnostic>,
}

impl ProductGroupAccumulator {
    fn into_group(self) -> SemanticOutlinerGroup {
        SemanticOutlinerGroup {
            id: self.id,
            label: self.label,
            kind: self.kind,
            provenance: self.provenance,
            source_resources: self.source_resources.into_iter().collect(),
            element_count: self.node_keys.len(),
            semantic_ids: self.semantic_ids,
            metadata: self.metadata,
            diagnostics: self.diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resource_query_parameter_with_percent_decoding() {
        assert_eq!(
            parse_semantic_outliner_resource("/api/semantic/outliner?resource=ifc%2Finfra-road")
                .unwrap(),
            "ifc/infra-road"
        );
    }

    #[test]
    fn requires_resource_query_parameter() {
        let error = parse_semantic_outliner_resource("/api/semantic/outliner").unwrap_err();
        assert!(error.contains("resource"));
    }

    #[test]
    fn project_resource_scopes_group_semantic_ids_by_source_resource() {
        let groups = aggregate_product_groups(
            "project/infra",
            LAYERS_FACET_ID,
            "layer",
            "Unnamed presentation layer",
            vec![
                ProductGroupRow {
                    source_resource: "ifc/a".to_owned(),
                    group_name: Some("TRIANGOLI - post operam".to_owned()),
                    product_node_id: "1".to_owned(),
                    global_id: Some("same".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Layer,
                },
                ProductGroupRow {
                    source_resource: "ifc/b".to_owned(),
                    group_name: Some("TRIANGOLI - post operam".to_owned()),
                    product_node_id: "1".to_owned(),
                    global_id: Some("same".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Layer,
                },
            ],
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].semantic_ids,
            vec!["ifc/a::same".to_owned(), "ifc/b::same".to_owned()]
        );
        assert_eq!(groups[0].element_count, 2);
    }

    #[test]
    fn single_ifc_resource_keeps_local_semantic_ids() {
        let groups = aggregate_product_groups(
            "ifc/a",
            CLASSES_FACET_ID,
            "class",
            "IfcProduct (declared entity missing)",
            vec![ProductGroupRow {
                source_resource: "ifc/a".to_owned(),
                group_name: Some("IfcWall".to_owned()),
                product_node_id: "7".to_owned(),
                global_id: Some("wall-guid".to_owned()),
                metadata: BTreeMap::new(),
                kind: GroupRowKind::Class,
            }],
        );

        assert_eq!(groups[0].semantic_ids, vec!["wall-guid".to_owned()]);
    }

    #[test]
    fn product_group_aggregation_deduplicates_nodes_and_ids() {
        let groups = aggregate_product_groups(
            "ifc/a",
            LAYERS_FACET_ID,
            "layer",
            "Unnamed presentation layer",
            vec![
                ProductGroupRow {
                    source_resource: "ifc/a".to_owned(),
                    group_name: Some("S_PROGETTO".to_owned()),
                    product_node_id: "11".to_owned(),
                    global_id: Some("guid-11".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Layer,
                },
                ProductGroupRow {
                    source_resource: "ifc/a".to_owned(),
                    group_name: Some("S_PROGETTO".to_owned()),
                    product_node_id: "11".to_owned(),
                    global_id: Some("guid-11".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Layer,
                },
            ],
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].element_count, 1);
        assert_eq!(groups[0].semantic_ids, vec!["guid-11".to_owned()]);
    }

    #[test]
    fn class_group_can_accumulate_container_descendant_ids() {
        let groups = aggregate_product_groups(
            "ifc/building-architecture",
            CLASSES_FACET_ID,
            "class",
            "IfcProduct (declared entity missing)",
            vec![
                ProductGroupRow {
                    source_resource: "ifc/building-architecture".to_owned(),
                    group_name: Some("IfcBuilding".to_owned()),
                    product_node_id: "building-node".to_owned(),
                    global_id: Some("building-global-id".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Class,
                },
                ProductGroupRow {
                    source_resource: "ifc/building-architecture".to_owned(),
                    group_name: Some("IfcBuilding".to_owned()),
                    product_node_id: "wall-node".to_owned(),
                    global_id: Some("wall-global-id".to_owned()),
                    metadata: BTreeMap::new(),
                    kind: GroupRowKind::Class,
                },
            ],
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].element_count, 2);
        assert_eq!(
            groups[0].semantic_ids,
            vec!["building-global-id".to_owned(), "wall-global-id".to_owned()]
        );
    }

    #[test]
    fn construction_state_hint_uses_layer_names_as_inference_only() {
        let (label, detail) = construction_state_hint_for_layer("TRIANGOLI - ante operam").unwrap();
        assert_eq!(label, "Existing / before");
        assert!(detail.contains("not a formal IFC visibility flag"));
        assert!(construction_state_hint_for_layer("S_PROGETTO").is_none());
    }
}
