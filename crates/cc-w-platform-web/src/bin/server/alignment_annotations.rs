use std::{collections::BTreeMap, net::TcpStream, time::Instant};

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use super::{
    ConsoleLogKind, CypherQueryResult, CypherResourceTarget, HttpRequest, ServerState, console_log,
    cypher_target_from_ifc_resource, execute_cypher_worker, write_json_response,
};

const DEFAULT_MAX_SAMPLES: usize = 500;
const HARD_MAX_SAMPLES: usize = 5_000;
const WORLD_UP: DVec3 = DVec3::Z;
const CURVE_ID_PREFIX: &str = "curve:";
const ALIGNMENT_ID_PREFIX: &str = "alignment:";
const ALIGNMENT_SCENE_DEPTH_MODE: &str = "xray";
const ALIGNMENT_TEXT_DEPTH_MODE: &str = "overlay";
const ALIGNMENT_PRIMARY_COLOR: [f32; 3] = [1.0, 0.72, 0.08];
const ALIGNMENT_LABEL_COLOR: [f32; 3] = [1.0, 0.92, 0.35];
const ALIGNMENT_LABEL_OUTLINE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const ALIGNMENT_LABEL_LEADER_HEIGHT: f64 = 1.8;

pub(super) fn serve_alignment_annotations_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: AlignmentAnnotationsApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/annotations/path JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let execute_started = Instant::now();
    let response = compile_alignment_annotations(&api_request, state);
    let execute_ms = execute_started.elapsed().as_millis();
    let status = if response.layer.is_some() {
        "200 OK"
    } else {
        "400 Bad Request"
    };
    let write_started = Instant::now();
    let write_result = write_json_response(stream, status, &response);
    let write_ms = write_started.elapsed().as_millis();
    println!(
        "[w web timing] path_annotations resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} layer={} diagnostics={}",
        api_request.resource,
        parse_ms,
        execute_ms,
        write_ms,
        request_started.elapsed().as_millis(),
        response.layer.is_some(),
        response.diagnostics.len(),
    );
    if response.layer.is_none() {
        let diagnostic_codes = response
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        console_log(
            ConsoleLogKind::Warn,
            format!(
                "path_annotations resource={} path_id={} failed with {} diagnostic(s): {}",
                api_request.resource,
                api_request.path.id.as_deref().unwrap_or(""),
                response.diagnostics.len(),
                diagnostic_codes
            ),
        );
    }
    write_result
}

fn compile_alignment_annotations(
    request: &AlignmentAnnotationsApiRequest,
    state: &ServerState,
) -> AlignmentAnnotationsApiResponse {
    let mut diagnostics = validate_request(request);
    if diagnostics.iter().any(AnnotationDiagnostic::is_blocking) {
        return AlignmentAnnotationsApiResponse {
            layer: None,
            diagnostics,
        };
    }

    let target = match cypher_target_from_ifc_resource(&request.resource) {
        Ok(target) => target,
        Err(error) => {
            diagnostics.push(AnnotationDiagnostic::error(
                "invalid_resource",
                error,
                BTreeMap::new(),
            ));
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };

    let max_samples = request.max_samples.unwrap_or(DEFAULT_MAX_SAMPLES);
    let path_id = match request.path_id() {
        Some(path_id) => path_id,
        None => {
            diagnostics.push(AnnotationDiagnostic::error(
                "missing_path_id",
                "Path annotation compilation requires path.id for an ifc_alignment path.",
                BTreeMap::new(),
            ));
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };
    let curve_node_id = match resolve_curve_node_id_for_alignment_path(path_id, state, &target) {
        Ok(id) => id,
        Err(mut issues) => {
            diagnostics.append(&mut issues);
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };

    let horizontal_query = build_gradient_curve_horizontal_segments_query(curve_node_id);
    let vertical_query = build_gradient_curve_vertical_segments_query(curve_node_id);
    let horizontal = execute_cypher_worker(
        state,
        &target,
        &horizontal_query,
        state.cypher_worker.timeout,
    );
    let vertical =
        execute_cypher_worker(state, &target, &vertical_query, state.cypher_worker.timeout);

    let horizontal = match horizontal {
        Ok(run) => run.result,
        Err(error) => {
            diagnostics.push(AnnotationDiagnostic::error(
                "horizontal_curve_query_failed",
                format!("Could not read explicit horizontal curve segment facts: {error}"),
                BTreeMap::new(),
            ));
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };
    let vertical = match vertical {
        Ok(run) => run.result,
        Err(error) => {
            diagnostics.push(AnnotationDiagnostic::error(
                "vertical_curve_query_failed",
                format!("Could not read explicit vertical/elevation curve segment facts: {error}"),
                BTreeMap::new(),
            ));
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };

    let horizontal_segments = match gradient_curve_segments(&horizontal, false, "horizontal") {
        Ok(segments) => segments,
        Err(mut issues) => {
            diagnostics.append(&mut issues);
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };
    let vertical_segments = match gradient_curve_segments(&vertical, true, "vertical") {
        Ok(segments) => segments,
        Err(mut issues) => {
            diagnostics.append(&mut issues);
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };

    compile_alignment_annotations_from_explicit_path(
        request,
        path_id,
        curve_node_id,
        &horizontal_segments,
        &vertical_segments,
        SourceSegmentCounts {
            horizontal: horizontal.rows.len(),
            vertical: vertical.rows.len(),
        },
        max_samples,
        diagnostics,
    )
}

fn compile_alignment_annotations_from_explicit_path(
    request: &AlignmentAnnotationsApiRequest,
    path_id: &str,
    curve_node_id: i64,
    horizontal_segments: &[GradientCurveSegment],
    vertical_segments: &[GradientCurveSegment],
    source_counts: SourceSegmentCounts,
    max_samples: usize,
    mut diagnostics: Vec<AnnotationDiagnostic>,
) -> AlignmentAnnotationsApiResponse {
    let Some(explicit_station_range) =
        overlapping_station_range(horizontal_segments, vertical_segments)
    else {
        diagnostics.push(AnnotationDiagnostic::unsupported(
            "missing_evaluable_station_range",
            "The explicit curve facts did not produce an overlapping horizontal and vertical station range. No annotation layer was compiled.",
            BTreeMap::new(),
        ));
        return AlignmentAnnotationsApiResponse {
            layer: None,
            diagnostics,
        };
    };
    let plan = match compile_path_annotation_plan(explicit_station_range, request, max_samples) {
        Ok(plan) => plan,
        Err(error) => {
            diagnostics.push(error);
            return AlignmentAnnotationsApiResponse {
                layer: None,
                diagnostics,
            };
        }
    };

    let mut primitives = Vec::new();
    for line_range in &plan.line_ranges {
        let polyline_stations = polyline_sample_stations(line_range.measure_range, max_samples);
        match sampled_points(horizontal_segments, vertical_segments, &polyline_stations) {
            Ok(points) if points.len() >= 2 => {
                primitives.push(AnnotationPrimitiveJson::Polyline(PolylineJson {
                    id: format!("path-line-{}", line_range.id_fragment),
                    points,
                    color: ALIGNMENT_PRIMARY_COLOR,
                    alpha: 1.0,
                    width_px: 6.0,
                    depth_mode: ALIGNMENT_SCENE_DEPTH_MODE,
                }));
            }
            Ok(_) => diagnostics.push(AnnotationDiagnostic::unsupported(
                "not_enough_polyline_samples",
                "The explicit station range yielded fewer than two polyline samples.",
                BTreeMap::from([(
                    "range".to_owned(),
                    serde_json::Value::String(line_range.id_fragment.clone()),
                )]),
            )),
            Err(error) => {
                diagnostics.push(error);
                return AlignmentAnnotationsApiResponse {
                    layer: None,
                    diagnostics,
                };
            }
        }
    }

    let station_tolerance = station_range_tolerance(plan.explicit_station_range);
    let mut emitted_tick_stations = Vec::new();
    for group in &plan.marker_groups {
        for station in &group.measures {
            if !push_unique_station(&mut emitted_tick_stations, *station, station_tolerance) {
                continue;
            }
            match station_pose(horizontal_segments, vertical_segments, *station) {
                Ok(pose) => primitives.push(AnnotationPrimitiveJson::Marker(MarkerJson {
                    id: format!(
                        "path-marker-{}-{}",
                        group.id_fragment,
                        station_id_fragment(*station)
                    ),
                    position: pose.origin,
                    direction: Some(pose.normal),
                    normal: Some(WORLD_UP.to_array()),
                    color: ALIGNMENT_PRIMARY_COLOR,
                    alpha: 0.96,
                    size_px: 13.0,
                    marker_kind: "tick",
                    depth_mode: ALIGNMENT_SCENE_DEPTH_MODE,
                })),
                Err(error) => {
                    diagnostics.push(error);
                    return AlignmentAnnotationsApiResponse {
                        layer: None,
                        diagnostics,
                    };
                }
            }
        }
    }

    let mut emitted_label_stations = Vec::new();
    for group in &plan.marker_groups {
        if group.label != PathMarkerLabel::Measure {
            continue;
        }
        for station in &group.measures {
            if !push_unique_station(&mut emitted_label_stations, *station, station_tolerance) {
                continue;
            }
            match station_pose(horizontal_segments, vertical_segments, *station) {
                Ok(pose) => {
                    let label_anchor = station_label_anchor(pose.origin);
                    let station_fragment = station_id_fragment(*station);
                    primitives.push(AnnotationPrimitiveJson::Polyline(PolylineJson {
                        id: format!(
                            "path-marker-label-leader-{}-{station_fragment}",
                            group.id_fragment
                        ),
                        points: vec![pose.origin, label_anchor],
                        color: ALIGNMENT_PRIMARY_COLOR,
                        alpha: 0.9,
                        width_px: 2.0,
                        depth_mode: ALIGNMENT_SCENE_DEPTH_MODE,
                    }));
                    primitives.push(AnnotationPrimitiveJson::Text(TextLabelJson {
                        id: format!("path-marker-label-{}-{station_fragment}", group.id_fragment),
                        text: format_station_label(*station),
                        anchor: label_anchor,
                        screen_offset_px: [0.0, -14.0],
                        horizontal_align: "center",
                        vertical_align: "bottom",
                        depth_mode: ALIGNMENT_TEXT_DEPTH_MODE,
                        style: TextStyleJson {
                            color: ALIGNMENT_LABEL_COLOR,
                            background_color: None,
                            outline_color: Some(ALIGNMENT_LABEL_OUTLINE_COLOR),
                            size_px: 30.0,
                            embolden_px: 0.35,
                            padding_px: 0.0,
                        },
                    }));
                }
                Err(error) => {
                    diagnostics.push(error);
                    return AlignmentAnnotationsApiResponse {
                        layer: None,
                        diagnostics,
                    };
                }
            }
        }
    }

    if primitives.is_empty() || diagnostics.iter().any(AnnotationDiagnostic::is_blocking) {
        if primitives.is_empty() {
            diagnostics.push(AnnotationDiagnostic::unsupported(
                "empty_annotation_layer",
                "No annotation primitives could be compiled from the explicit station facts.",
                BTreeMap::new(),
            ));
        }
        return AlignmentAnnotationsApiResponse {
            layer: None,
            diagnostics,
        };
    }

    diagnostics.push(AnnotationDiagnostic::info(
        "explicit_curve_facts_only",
        "The annotation layer was compiled from explicit alignment curve segment and elevation facts. No model bounds, visible geometry, names, or guessed placement fallback was used.",
        BTreeMap::from([
            (
                "horizontal_segment_rows".to_owned(),
                serde_json::json!(source_counts.horizontal),
            ),
            (
                "vertical_segment_rows".to_owned(),
                serde_json::json!(source_counts.vertical),
            ),
            (
                "explicit_station_start".to_owned(),
                serde_json::json!(plan.explicit_station_range.0),
            ),
            (
                "explicit_station_end".to_owned(),
                serde_json::json!(plan.explicit_station_range.1),
            ),
            (
                "line_range_count".to_owned(),
                serde_json::json!(plan.line_ranges.len()),
            ),
            (
                "marker_group_count".to_owned(),
                serde_json::json!(plan.marker_groups.len()),
            ),
        ]),
    ));

    let layer = AnnotationLayerJson {
        id: annotation_layer_id(request, path_id, curve_node_id, plan.part),
        source: Some(annotation_layer_source(request, path_id, plan.part)),
        visible: true,
        lifecycle: "temporary",
        primitives,
        provenance: annotation_layer_provenance(request, path_id, plan.part),
    };

    AlignmentAnnotationsApiResponse {
        layer: Some(layer),
        diagnostics,
    }
}

fn annotation_layer_id(
    request: &AlignmentAnnotationsApiRequest,
    path_id: &str,
    curve_node_id: i64,
    part: Option<PathAnnotationPart>,
) -> String {
    if let Some(layer_id) = request.valid_layer_id() {
        return layer_id.to_owned();
    }
    let Some(part) = part else {
        return format!(
            "path-annotations-{}",
            station_id_fragment(curve_node_id as f64)
        );
    };
    format!(
        "path-annotations-{}-{}-{}-{}",
        annotation_id_fragment(&request.resource),
        annotation_id_fragment(&request.path.kind),
        annotation_id_fragment(path_id),
        part.layer_id_fragment()
    )
}

fn annotation_layer_source(
    request: &AlignmentAnnotationsApiRequest,
    path_id: &str,
    part: Option<PathAnnotationPart>,
) -> String {
    match part {
        Some(part) => format!(
            "path:{}:{}:part={}",
            request.path.kind,
            path_id,
            part.name()
        ),
        None => format!("path:{}:{}", request.path.kind, path_id),
    }
}

fn annotation_layer_provenance(
    request: &AlignmentAnnotationsApiRequest,
    path_id: &str,
    part: Option<PathAnnotationPart>,
) -> Vec<String> {
    let mut provenance = vec![
        "endpoint=/api/annotations/path".to_owned(),
        format!("resource={}", request.resource),
        format!("path_kind={}", request.path.kind),
        format!("path_id={path_id}"),
        "basis=explicit_ifc_gradient_curve".to_owned(),
    ];
    if let Some(part) = part {
        provenance.push(format!("part={}", part.name()));
    }
    provenance
}

fn annotation_id_fragment(value: &str) -> String {
    let mut fragment = String::new();
    let mut pushed_separator = false;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            fragment.push(character.to_ascii_lowercase());
            pushed_separator = false;
        } else if !fragment.is_empty() && !pushed_separator {
            fragment.push('-');
            pushed_separator = true;
        }
    }
    while fragment.ends_with('-') {
        fragment.pop();
    }
    if fragment.is_empty() {
        "path".to_owned()
    } else {
        fragment
    }
}

fn compile_path_annotation_plan(
    explicit_station_range: (f64, f64),
    request: &AlignmentAnnotationsApiRequest,
    max_samples: usize,
) -> Result<CompiledPathAnnotationPlan, AnnotationDiagnostic> {
    let part = request.annotation_part()?;
    let line_ranges = if part.is_none_or(PathAnnotationPart::includes_line) {
        requested_path_line_ranges_for_part(explicit_station_range, request, part)?
    } else {
        Vec::new()
    };
    let marker_groups = if part.is_none_or(PathAnnotationPart::includes_stations) {
        requested_path_marker_groups(explicit_station_range, request, max_samples)?
    } else {
        Vec::new()
    };
    Ok(CompiledPathAnnotationPlan {
        explicit_station_range,
        part,
        line_ranges,
        marker_groups,
    })
}

fn resolve_curve_node_id_for_alignment_path(
    path_id: &str,
    state: &ServerState,
    target: &CypherResourceTarget,
) -> Result<i64, Vec<AnnotationDiagnostic>> {
    if let Some(curve_node_id) = parse_curve_alignment_id(path_id) {
        return Ok(curve_node_id);
    }

    if let Some(alignment_node_id) = parse_alignment_root_id(path_id) {
        let query = build_alignment_root_curve_query(alignment_node_id);
        let run = execute_cypher_worker(state, target, &query, state.cypher_worker.timeout)
            .map_err(|error| {
                vec![AnnotationDiagnostic::error(
                    "alignment_root_curve_query_failed",
                    format!(
                        "Could not resolve the IfcAlignment root to an explicit axis curve: {error}"
                    ),
                    BTreeMap::from([(
                        "path_id".to_owned(),
                        serde_json::Value::String(path_id.to_owned()),
                    )]),
                )]
            })?;
        let mut curve_ids = run
            .result
            .rows
            .iter()
            .filter_map(|row| {
                let values = cypher_row_map(&run.result.columns, row);
                values
                    .get("curve_node_id")
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
            .collect::<Vec<_>>();
        curve_ids.sort_unstable();
        curve_ids.dedup();
        return match curve_ids.as_slice() {
            [curve_node_id] => Ok(*curve_node_id),
            [] => Err(vec![AnnotationDiagnostic::unsupported(
                "alignment_root_has_no_explicit_axis_curve",
                "The IfcAlignment root did not expose exactly one explicit IfcGradientCurve through its Axis representation. No alignment geometry was inferred.",
                BTreeMap::from([(
                    "path_id".to_owned(),
                    serde_json::Value::String(path_id.to_owned()),
                )]),
            )]),
            _ => Err(vec![AnnotationDiagnostic::unsupported(
                "alignment_root_has_multiple_axis_curves",
                "The IfcAlignment root exposed multiple explicit IfcGradientCurve axis items, so the annotation curve cannot be chosen without guessing.",
                BTreeMap::from([
                    (
                        "path_id".to_owned(),
                        serde_json::Value::String(path_id.to_owned()),
                    ),
                    ("curve_node_ids".to_owned(), serde_json::json!(curve_ids)),
                ]),
            )]),
        };
    }

    Err(vec![AnnotationDiagnostic::unsupported(
        "unsupported_path_identifier",
        "IFC alignment path annotation compilation requires an explicit catalog resolver id of the form `curve:<db-node-id>` or an IfcAlignment root id of the form `alignment:<db-node-id>` that resolves to exactly one explicit axis curve.",
        BTreeMap::from([(
            "path_id".to_owned(),
            serde_json::Value::String(path_id.to_owned()),
        )]),
    )])
}

fn build_alignment_root_curve_query(alignment_node_id: i64) -> String {
    [
        "MATCH (alignment:IfcAlignment)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(representation:IfcShapeRepresentation)-[:ITEMS]->(curve:IfcGradientCurve)",
        &format!("WHERE id(alignment) = {alignment_node_id}"),
        "RETURN id(alignment) AS alignment_node_id, id(curve) AS curve_node_id, representation.RepresentationIdentifier AS representation_identifier, representation.RepresentationType AS representation_type",
        "ORDER BY curve_node_id",
        "LIMIT 4",
    ]
    .join("\n")
}

fn validate_request(request: &AlignmentAnnotationsApiRequest) -> Vec<AnnotationDiagnostic> {
    let mut diagnostics = Vec::new();
    if request.resource.trim().is_empty() {
        diagnostics.push(AnnotationDiagnostic::error(
            "missing_resource",
            "Path annotation compilation requires a selected resource.",
            BTreeMap::new(),
        ));
    }
    let path_kind = request.path.kind.trim();
    if path_kind.is_empty() {
        diagnostics.push(AnnotationDiagnostic::error(
            "missing_path_kind",
            "Path annotation compilation requires path.kind.",
            BTreeMap::new(),
        ));
    }
    if !path_kind.is_empty() && path_kind != "ifc_alignment" {
        diagnostics.push(AnnotationDiagnostic::error(
            "unsupported_path_kind",
            "Path annotation compilation currently supports path.kind `ifc_alignment`.",
            BTreeMap::from([(
                "path_kind".to_owned(),
                serde_json::Value::String(path_kind.to_owned()),
            )]),
        ));
    }
    if request
        .path
        .id
        .as_deref()
        .is_none_or(|id| id.trim().is_empty())
    {
        diagnostics.push(AnnotationDiagnostic::error(
            "missing_path_id",
            "Path annotation compilation requires path.id.",
            BTreeMap::new(),
        ));
    }
    let measure = request.path.measure.as_deref().unwrap_or("station").trim();
    if !["station", "chainage", "measure"].contains(&measure) {
        diagnostics.push(AnnotationDiagnostic::error(
            "unsupported_path_measure",
            "IFC alignment path annotations currently support station/chainage measure coordinates.",
            BTreeMap::from([(
                "measure".to_owned(),
                serde_json::Value::String(measure.to_owned()),
            )]),
        ));
    }
    if let Some(max_samples) = request.max_samples {
        if max_samples == 0 || max_samples > HARD_MAX_SAMPLES {
            diagnostics.push(AnnotationDiagnostic::error(
                "invalid_max_samples",
                format!("max_samples must be between 1 and {HARD_MAX_SAMPLES}."),
                BTreeMap::new(),
            ));
        }
    }
    let part = match request.annotation_part() {
        Ok(part) => part,
        Err(error) => {
            diagnostics.push(error);
            None
        }
    };
    let needs_line = part.is_none_or(PathAnnotationPart::includes_line);
    let needs_stations = part.is_none_or(PathAnnotationPart::includes_stations);
    if part.is_none() && request.line.is_none() && request.markers.is_empty() {
        diagnostics.push(AnnotationDiagnostic::error(
            "missing_path_annotation_primitives",
            "Path annotation compilation requires at least one line range or marker group.",
            BTreeMap::new(),
        ));
    }
    if part == Some(PathAnnotationPart::Stations) && request.markers.is_empty() {
        diagnostics.push(AnnotationDiagnostic::error(
            "missing_path_station_markers",
            "Station-only path annotation compilation requires explicit marker request data.",
            BTreeMap::new(),
        ));
    }
    if needs_line {
        if let Some(line) = &request.line {
            for (index, range) in line.ranges.iter().enumerate() {
                validate_measure_range(format!("line.ranges[{index}]"), range, &mut diagnostics);
            }
        }
    }
    if needs_stations {
        for (index, marker) in request.markers.iter().enumerate() {
            let prefix = format!("markers[{index}]");
            if !marker
                .every
                .is_some_and(|every| every.is_finite() && every > 0.0)
            {
                diagnostics.push(AnnotationDiagnostic::error(
                    "invalid_marker_interval",
                    format!("{prefix}.every must be a finite positive number."),
                    BTreeMap::new(),
                ));
            }
            if let Some(range) = &marker.range {
                validate_measure_range(format!("{prefix}.range"), range, &mut diagnostics);
            }
        }
    }
    diagnostics
}

fn validate_measure_range(
    field: String,
    range: &MeasureRangeRequest,
    diagnostics: &mut Vec<AnnotationDiagnostic>,
) {
    for (name, value) in [
        ("from", range.from),
        ("to", range.to),
        ("from_offset", range.from_offset),
        ("to_offset", range.to_offset),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            diagnostics.push(AnnotationDiagnostic::error(
                "invalid_measure_range",
                format!("{field}.{name} must be finite when provided."),
                BTreeMap::new(),
            ));
        }
    }
    if range.from.is_some() && range.from_offset.is_some() {
        diagnostics.push(AnnotationDiagnostic::error(
            "invalid_measure_range",
            format!("{field} cannot provide both from and from_offset."),
            BTreeMap::new(),
        ));
    }
    if range.to.is_some() && range.to_offset.is_some() {
        diagnostics.push(AnnotationDiagnostic::error(
            "invalid_measure_range",
            format!("{field} cannot provide both to and to_offset."),
            BTreeMap::new(),
        ));
    }
    if range.to_end && (range.to.is_some() || range.to_offset.is_some()) {
        diagnostics.push(AnnotationDiagnostic::error(
            "invalid_measure_range",
            format!("{field} cannot provide to_end together with to or to_offset."),
            BTreeMap::new(),
        ));
    }
}

fn build_gradient_curve_horizontal_segments_query(curve_node_id: i64) -> String {
    [
        "MATCH (curve:IfcGradientCurve)-[:BASE_CURVE]->(base_curve:IfcCompositeCurve)-[segment_edge:SEGMENTS]->(segment:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)",
        &format!("WHERE id(curve) = {curve_node_id}"),
        "MATCH (place)-[:REF_DIRECTION]->(direction:IfcDirection)",
        "MATCH (segment)-[:SEGMENT_LENGTH]->(length)",
        "OPTIONAL MATCH (segment)-[:PARENT_CURVE]->(parent_curve)",
        "RETURN id(curve) AS curve_node_id, id(segment) AS segment_node_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, direction.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius",
        "ORDER BY segment_ordinal, segment_node_id",
    ]
    .join("\n")
}

fn build_gradient_curve_vertical_segments_query(curve_node_id: i64) -> String {
    [
        "MATCH (curve:IfcGradientCurve)-[segment_edge:SEGMENTS]->(segment:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)",
        &format!("WHERE id(curve) = {curve_node_id}"),
        "MATCH (place)-[:REF_DIRECTION]->(direction:IfcDirection)",
        "MATCH (segment)-[:SEGMENT_LENGTH]->(length)",
        "OPTIONAL MATCH (segment)-[:PARENT_CURVE]->(parent_curve)",
        "RETURN id(curve) AS curve_node_id, id(segment) AS segment_node_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, direction.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius",
        "ORDER BY segment_ordinal, segment_node_id",
    ]
    .join("\n")
}

fn gradient_curve_segments(
    result: &CypherQueryResult,
    use_explicit_station: bool,
    label: &str,
) -> Result<Vec<GradientCurveSegment>, Vec<AnnotationDiagnostic>> {
    if result.rows.is_empty() {
        return Err(vec![AnnotationDiagnostic::unsupported(
            format!("missing_{label}_curve_segments"),
            format!(
                "No explicit {label} curve segment facts were found for the requested alignment id."
            ),
            BTreeMap::new(),
        )]);
    }
    let mut issues = Vec::new();
    let mut rows = Vec::new();
    for row in &result.rows {
        match SegmentRow::from_cypher_row(&result.columns, row) {
            Ok(row) => rows.push(row),
            Err(error) => issues.push(error),
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    rows.sort_by_key(|row| (row.ordinal, row.segment_id));
    if let Some([left, right]) = rows
        .windows(2)
        .find(|window| window[0].ordinal == window[1].ordinal)
    {
        return Err(vec![AnnotationDiagnostic::unsupported(
            format!("duplicate_{label}_segment_ordinal"),
            "The IFC SEGMENTS list contains duplicate ordinals, so the alignment segment order cannot be resolved without guessing.",
            BTreeMap::from([
                (
                    "segment_ordinal".to_owned(),
                    serde_json::json!(left.ordinal),
                ),
                (
                    "segment_node_ids".to_owned(),
                    serde_json::json!([left.segment_id, right.segment_id]),
                ),
            ]),
        )]);
    }

    let mut cumulative_station = 0.0;
    let mut segments = Vec::new();
    for index in 0..rows.len() {
        let row = &rows[index];
        let next = rows.get(index + 1);
        let length = row.signed_length.abs();
        let kind = match segment_kind(row, next, length) {
            Ok(kind) => kind,
            Err(error) => return Err(vec![error]),
        };
        segments.push(GradientCurveSegment {
            start_station: if use_explicit_station {
                row.start_point.x
            } else {
                cumulative_station
            },
            length,
            start_point: row.start_point,
            direction: row.direction,
            end_point: next.map(|next| next.start_point),
            end_direction: next.map(|next| next.direction),
            kind,
        });
        cumulative_station += length;
    }

    Ok(segments)
}

fn segment_kind(
    row: &SegmentRow,
    next: Option<&SegmentRow>,
    length: f64,
) -> Result<SegmentKind, AnnotationDiagnostic> {
    match row.parent_curve_entity.as_deref() {
        Some("IfcCircle") => {
            let Some(radius) = row.radius.filter(|radius| radius.abs() > 1.0e-9) else {
                return Err(AnnotationDiagnostic::unsupported(
                    "invalid_circular_segment_radius",
                    "A circular curve segment did not expose a finite non-zero radius.",
                    BTreeMap::from([(
                        "segment_node_id".to_owned(),
                        serde_json::json!(row.segment_id),
                    )]),
                ));
            };
            Ok(SegmentKind::Circular {
                radius: radius.abs(),
                turn_sign: choose_circular_segment_turn_sign(row, next, radius.abs(), length),
            })
        }
        Some("IfcClothoid") => {
            if next.is_none() {
                return Err(AnnotationDiagnostic::unsupported(
                    "clothoid_segment_missing_successor",
                    "A clothoid curve segment did not have a following explicit segment row, so it cannot be evaluated without extrapolating an end point.",
                    BTreeMap::from([(
                        "segment_node_id".to_owned(),
                        serde_json::json!(row.segment_id),
                    )]),
                ));
            }
            Ok(SegmentKind::Clothoid)
        }
        _ => Ok(SegmentKind::Line),
    }
}

fn choose_circular_segment_turn_sign(
    row: &SegmentRow,
    next: Option<&SegmentRow>,
    radius: f64,
    length: f64,
) -> f64 {
    let explicit_length_sign = if row.signed_length < 0.0 { -1.0 } else { 1.0 };
    let Some(next) = next else {
        return explicit_length_sign;
    };
    let left = circular_segment_point(row.start_point, row.direction, radius, -1.0, length);
    let right = circular_segment_point(row.start_point, row.direction, radius, 1.0, length);
    if left.distance_squared(next.start_point) <= right.distance_squared(next.start_point) {
        -1.0
    } else {
        1.0
    }
}

fn sampled_points(
    horizontal: &[GradientCurveSegment],
    vertical: &[GradientCurveSegment],
    stations: &[f64],
) -> Result<Vec<[f64; 3]>, AnnotationDiagnostic> {
    stations
        .iter()
        .map(|station| station_pose(horizontal, vertical, *station).map(|pose| pose.origin))
        .collect()
}

fn station_pose(
    horizontal: &[GradientCurveSegment],
    vertical: &[GradientCurveSegment],
    station: f64,
) -> Result<StationPoseJson, AnnotationDiagnostic> {
    let horizontal = evaluate_segments(horizontal, station).ok_or_else(|| {
        AnnotationDiagnostic::unsupported(
            "station_outside_horizontal_range",
            "A requested station could not be evaluated from the explicit horizontal curve segments.",
            BTreeMap::from([("station".to_owned(), serde_json::json!(station))]),
        )
    })?;
    let vertical = evaluate_segments(vertical, station).ok_or_else(|| {
        AnnotationDiagnostic::unsupported(
            "station_outside_vertical_range",
            "A requested station could not be evaluated from the explicit vertical/elevation curve segments.",
            BTreeMap::from([("station".to_owned(), serde_json::json!(station))]),
        )
    })?;
    let normal = DVec3::new(horizontal.tangent.x, horizontal.tangent.y, 0.0)
        .try_normalize()
        .ok_or_else(|| {
            AnnotationDiagnostic::unsupported(
                "station_tangent_degenerate",
                "The resolved station tangent is degenerate, so tick orientation cannot be formed without guessing.",
                BTreeMap::from([("station".to_owned(), serde_json::json!(station))]),
            )
        })?;
    Ok(StationPoseJson {
        origin: [horizontal.point.x, horizontal.point.y, vertical.point.y],
        normal: normal.to_array(),
    })
}

fn evaluate_segments(segments: &[GradientCurveSegment], station: f64) -> Option<CurveEvaluation> {
    let first = segments.first()?;
    let last = segments.last().unwrap_or(first);
    let segment = segments
        .iter()
        .find(|entry| station <= entry.start_station + entry.length || entry.length <= 1.0e-12)
        .unwrap_or(last);
    let along = if segment.length <= 1.0e-12 {
        0.0
    } else {
        (station - segment.start_station).clamp(0.0, segment.length)
    };
    evaluate_segment(segment, along)
}

fn evaluate_segment(segment: &GradientCurveSegment, along: f64) -> Option<CurveEvaluation> {
    match segment.kind {
        SegmentKind::Line => Some(CurveEvaluation {
            point: segment.start_point + segment.direction * along,
            tangent: segment.direction,
        }),
        SegmentKind::Circular { radius, turn_sign } => Some(evaluate_circular_segment(
            segment.start_point,
            segment.direction,
            radius,
            turn_sign,
            along,
        )),
        SegmentKind::Clothoid => {
            let end_point = segment.end_point?;
            let end_direction = segment.end_direction?;
            evaluate_hermite_segment(
                segment.start_point,
                segment.direction * segment.length,
                end_point,
                end_direction * segment.length,
                if segment.length <= 1.0e-12 {
                    0.0
                } else {
                    (along / segment.length).clamp(0.0, 1.0)
                },
            )
        }
    }
}

fn evaluate_circular_segment(
    start_point: DVec2,
    direction: DVec2,
    radius: f64,
    turn_sign: f64,
    along: f64,
) -> CurveEvaluation {
    if radius <= 1.0e-9 {
        return CurveEvaluation {
            point: start_point + direction * along,
            tangent: direction,
        };
    }
    let sign = if turn_sign < 0.0 { -1.0 } else { 1.0 };
    let left_normal = DVec2::new(-direction.y, direction.x);
    let center = start_point + left_normal * radius * sign;
    let radial = start_point - center;
    let angle = sign * along / radius;
    CurveEvaluation {
        point: center + rotate_vec2(radial, angle),
        tangent: rotate_vec2(direction, angle)
            .try_normalize()
            .unwrap_or(direction),
    }
}

fn circular_segment_point(
    start_point: DVec2,
    direction: DVec2,
    radius: f64,
    turn_sign: f64,
    along: f64,
) -> DVec2 {
    evaluate_circular_segment(start_point, direction, radius, turn_sign, along).point
}

fn evaluate_hermite_segment(
    p0: DVec2,
    m0: DVec2,
    p1: DVec2,
    m1: DVec2,
    t: f64,
) -> Option<CurveEvaluation> {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    let point = p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11;

    let dh00 = 6.0 * t2 - 6.0 * t;
    let dh10 = 3.0 * t2 - 4.0 * t + 1.0;
    let dh01 = -6.0 * t2 + 6.0 * t;
    let dh11 = 3.0 * t2 - 2.0 * t;
    let tangent = (p0 * dh00 + m0 * dh10 + p1 * dh01 + m1 * dh11).try_normalize()?;
    Some(CurveEvaluation { point, tangent })
}

fn rotate_vec2(value: DVec2, angle: f64) -> DVec2 {
    let (sin, cos) = angle.sin_cos();
    DVec2::new(value.x * cos - value.y * sin, value.x * sin + value.y * cos)
}

fn overlapping_station_range(
    horizontal: &[GradientCurveSegment],
    vertical: &[GradientCurveSegment],
) -> Option<(f64, f64)> {
    let horizontal = station_range(horizontal)?;
    let vertical = station_range(vertical)?;
    let start = horizontal.0.max(vertical.0);
    let end = horizontal.1.min(vertical.1);
    (end >= start).then_some((start, end))
}

fn station_range(segments: &[GradientCurveSegment]) -> Option<(f64, f64)> {
    let first = segments.first()?;
    let last = segments.last()?;
    Some((first.start_station, last.start_station + last.length))
}

fn requested_path_line_ranges(
    explicit_range: (f64, f64),
    request: &AlignmentAnnotationsApiRequest,
) -> Result<Vec<CompiledPathLineRange>, AnnotationDiagnostic> {
    let Some(line) = &request.line else {
        return Ok(Vec::new());
    };
    let ranges = if line.ranges.is_empty() {
        vec![(0, explicit_range)]
    } else {
        line.ranges
            .iter()
            .enumerate()
            .map(|(index, range)| {
                resolve_measure_range(explicit_range, Some(range), index, "line.ranges")
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    Ok(ranges
        .into_iter()
        .map(|(index, measure_range)| CompiledPathLineRange {
            id_fragment: measure_range_id_fragment("line", index, measure_range),
            measure_range,
        })
        .collect())
}

fn requested_path_line_ranges_for_part(
    explicit_range: (f64, f64),
    request: &AlignmentAnnotationsApiRequest,
    part: Option<PathAnnotationPart>,
) -> Result<Vec<CompiledPathLineRange>, AnnotationDiagnostic> {
    if part == Some(PathAnnotationPart::Line) && request.line.is_none() {
        return Ok(vec![CompiledPathLineRange {
            id_fragment: measure_range_id_fragment("line", 0, explicit_range),
            measure_range: explicit_range,
        }]);
    }
    requested_path_line_ranges(explicit_range, request)
}

fn requested_path_marker_groups(
    explicit_range: (f64, f64),
    request: &AlignmentAnnotationsApiRequest,
    max_samples: usize,
) -> Result<Vec<CompiledPathMarkerGroup>, AnnotationDiagnostic> {
    request
        .markers
        .iter()
        .enumerate()
        .map(|(index, marker)| {
            let (_, measure_range) =
                resolve_measure_range(explicit_range, marker.range.as_ref(), index, "markers")?;
            let every = marker.every.ok_or_else(|| {
                AnnotationDiagnostic::error(
                    "invalid_marker_interval",
                    format!("markers[{index}].every must be a finite positive number."),
                    BTreeMap::new(),
                )
            })?;
            let measures =
                station_values_in_range(measure_range, every, max_samples).map_err(|error| {
                    AnnotationDiagnostic::unsupported(
                        "invalid_marker_sampling",
                        error,
                        BTreeMap::from([
                            ("marker_index".to_owned(), serde_json::json!(index)),
                            ("every".to_owned(), serde_json::json!(every)),
                            ("max_samples".to_owned(), serde_json::json!(max_samples)),
                        ]),
                    )
                })?;
            Ok(CompiledPathMarkerGroup {
                id_fragment: measure_range_id_fragment("marker", index, measure_range),
                measure_range,
                measures,
                label: marker.label_kind(),
            })
        })
        .collect()
}

fn resolve_measure_range(
    explicit_range: (f64, f64),
    range: Option<&MeasureRangeRequest>,
    index: usize,
    field: &'static str,
) -> Result<(usize, (f64, f64)), AnnotationDiagnostic> {
    let start = range
        .and_then(|range| range.from)
        .or_else(|| {
            range
                .and_then(|range| range.from_offset)
                .map(|offset| explicit_range.0 + offset)
        })
        .unwrap_or(explicit_range.0);
    let tolerance = station_range_tolerance(explicit_range);
    let zero_end_offset_after_start = range
        .and_then(|range| range.to_offset)
        .is_some_and(|offset| offset.abs() <= tolerance)
        && range.and_then(|range| range.to).is_none()
        && start > explicit_range.0 + tolerance;
    let uses_explicit_path_end =
        range.is_some_and(|range| range.to_end) || zero_end_offset_after_start;
    let end = if uses_explicit_path_end {
        explicit_range.1
    } else {
        range
            .and_then(|range| range.to)
            .or_else(|| {
                range
                    .and_then(|range| range.to_offset)
                    .map(|offset| explicit_range.0 + offset)
            })
            .unwrap_or(explicit_range.1)
    };

    if !start.is_finite() || !end.is_finite() {
        return Err(AnnotationDiagnostic::error(
            "invalid_measure_range",
            format!("{field}[{index}] endpoints must be finite numbers."),
            BTreeMap::new(),
        ));
    }
    if end < start - tolerance {
        return Err(AnnotationDiagnostic::unsupported(
            "reversed_measure_range",
            format!("{field}[{index}].from must be less than or equal to .to."),
            BTreeMap::from([
                ("from".to_owned(), serde_json::json!(start)),
                ("to".to_owned(), serde_json::json!(end)),
            ]),
        ));
    }
    if start < explicit_range.0 - tolerance || end > explicit_range.1 + tolerance {
        return Err(AnnotationDiagnostic::unsupported(
            "measure_range_outside_explicit_path",
            "Requested measure range falls outside the explicit IFC alignment station range.",
            BTreeMap::from([
                (
                    "explicit_measure_start".to_owned(),
                    serde_json::json!(explicit_range.0),
                ),
                (
                    "explicit_measure_end".to_owned(),
                    serde_json::json!(explicit_range.1),
                ),
                ("from".to_owned(), serde_json::json!(start)),
                ("to".to_owned(), serde_json::json!(end)),
            ]),
        ));
    }

    Ok((
        index,
        (
            clean_station(start.max(explicit_range.0)),
            clean_station(end.min(explicit_range.1)),
        ),
    ))
}

fn station_values_in_range(
    range: (f64, f64),
    interval: f64,
    max_samples: usize,
) -> Result<Vec<f64>, String> {
    let tolerance = interval.abs() * 1.0e-9;
    let mut station = (range.0 / interval).ceil() * interval;
    if station < range.0 - tolerance {
        station += interval;
    }
    let mut values = Vec::new();
    while station <= range.1 + tolerance {
        if values.len() >= max_samples {
            return Err(format!(
                "The requested interval would produce more than {max_samples} markers."
            ));
        }
        values.push(clean_station(station));
        station += interval;
    }
    Ok(values)
}

fn measure_range_id_fragment(prefix: &str, index: usize, measure_range: (f64, f64)) -> String {
    format!(
        "{prefix}{index}-{}-{}",
        station_id_fragment(measure_range.0),
        station_id_fragment(measure_range.1)
    )
}

fn push_unique_station(stations: &mut Vec<f64>, station: f64, tolerance: f64) -> bool {
    if stations
        .iter()
        .any(|existing| (*existing - station).abs() <= tolerance)
    {
        return false;
    }
    stations.push(station);
    true
}

fn station_range_tolerance(range: (f64, f64)) -> f64 {
    (range.1 - range.0).abs().max(1.0) * 1.0e-9
}

fn polyline_sample_stations(range: (f64, f64), max_samples: usize) -> Vec<f64> {
    let count = max_samples.clamp(2, HARD_MAX_SAMPLES);
    if (range.1 - range.0).abs() <= f64::EPSILON {
        return vec![range.0];
    }
    (0..count)
        .map(|index| {
            let t = index as f64 / (count - 1) as f64;
            clean_station(range.0 + (range.1 - range.0) * t)
        })
        .collect()
}

fn clean_station(value: f64) -> f64 {
    if value.abs() < 1.0e-9 { 0.0 } else { value }
}

fn parse_curve_alignment_id(value: &str) -> Option<i64> {
    value
        .trim()
        .strip_prefix(CURVE_ID_PREFIX)?
        .trim()
        .parse::<i64>()
        .ok()
}

fn parse_alignment_root_id(value: &str) -> Option<i64> {
    value
        .trim()
        .strip_prefix(ALIGNMENT_ID_PREFIX)?
        .trim()
        .parse::<i64>()
        .ok()
}

fn format_station_label(station: f64) -> String {
    let station = clean_station(station);
    if (station - station.round()).abs() <= 1.0e-9 {
        format!("{station:.0}")
    } else {
        let mut value = format!("{station:.3}");
        while value.contains('.') && value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
        value
    }
}

fn station_label_anchor(origin: [f64; 3]) -> [f64; 3] {
    [
        origin[0],
        origin[1],
        origin[2] + ALIGNMENT_LABEL_LEADER_HEIGHT,
    ]
}

fn station_id_fragment(station: f64) -> String {
    format_station_label(station)
        .replace('-', "m")
        .replace('.', "p")
}

impl SegmentRow {
    fn from_cypher_row(columns: &[String], row: &[String]) -> Result<Self, AnnotationDiagnostic> {
        let values = cypher_row_map(columns, row);
        let segment_id = required_i64(&values, "segment_node_id")?;
        let ordinal = required_i64(&values, "segment_ordinal")?;
        let start_point = required_vec2(&values, "start_point")?;
        let direction = required_vec2(&values, "direction")?
            .try_normalize()
            .ok_or_else(|| {
                AnnotationDiagnostic::unsupported(
                    "invalid_curve_segment_direction",
                    "A curve segment row had a degenerate direction vector.",
                    BTreeMap::from([("segment_node_id".to_owned(), serde_json::json!(segment_id))]),
                )
            })?;
        let signed_length = required_f64(&values, "segment_length")?;
        if !signed_length.is_finite() {
            return Err(AnnotationDiagnostic::unsupported(
                "invalid_curve_segment_length",
                "A curve segment row had a non-finite segment length.",
                BTreeMap::from([("segment_node_id".to_owned(), serde_json::json!(segment_id))]),
            ));
        }
        Ok(Self {
            segment_id,
            ordinal,
            start_point,
            direction,
            signed_length,
            parent_curve_entity: optional_text(&values, "parent_curve_entity"),
            radius: optional_f64(&values, "radius"),
        })
    }
}

fn cypher_row_map<'a>(columns: &'a [String], row: &'a [String]) -> BTreeMap<&'a str, &'a str> {
    columns
        .iter()
        .zip(row.iter())
        .map(|(column, value)| (column.as_str(), value.as_str()))
        .collect()
}

fn required_i64(values: &BTreeMap<&str, &str>, column: &str) -> Result<i64, AnnotationDiagnostic> {
    let value = required_cell(values, column)?;
    value.trim().parse::<i64>().map_err(|error| {
        AnnotationDiagnostic::unsupported(
            "invalid_integer_cell",
            format!("Column `{column}` did not contain an integer: {error}"),
            BTreeMap::from([(
                "value".to_owned(),
                serde_json::Value::String(value.to_owned()),
            )]),
        )
    })
}

fn required_f64(values: &BTreeMap<&str, &str>, column: &str) -> Result<f64, AnnotationDiagnostic> {
    let value = required_cell(values, column)?;
    parse_f64_cell(value).ok_or_else(|| {
        AnnotationDiagnostic::unsupported(
            "invalid_numeric_cell",
            format!("Column `{column}` did not contain a finite number."),
            BTreeMap::from([(
                "value".to_owned(),
                serde_json::Value::String(value.to_owned()),
            )]),
        )
    })
}

fn optional_f64(values: &BTreeMap<&str, &str>, column: &str) -> Option<f64> {
    values.get(column).and_then(|value| parse_f64_cell(value))
}

fn required_vec2(
    values: &BTreeMap<&str, &str>,
    column: &str,
) -> Result<DVec2, AnnotationDiagnostic> {
    let value = required_cell(values, column)?;
    let numbers = parse_numeric_array(value).ok_or_else(|| {
        AnnotationDiagnostic::unsupported(
            "invalid_vector_cell",
            format!("Column `{column}` did not contain an explicit numeric vector."),
            BTreeMap::from([(
                "value".to_owned(),
                serde_json::Value::String(value.to_owned()),
            )]),
        )
    })?;
    if numbers.len() < 2 || !numbers[0].is_finite() || !numbers[1].is_finite() {
        return Err(AnnotationDiagnostic::unsupported(
            "invalid_vector_cell",
            format!("Column `{column}` did not contain at least two finite coordinates."),
            BTreeMap::from([(
                "value".to_owned(),
                serde_json::Value::String(value.to_owned()),
            )]),
        ));
    }
    Ok(DVec2::new(numbers[0], numbers[1]))
}

fn required_cell<'a>(
    values: &'a BTreeMap<&str, &str>,
    column: &str,
) -> Result<&'a str, AnnotationDiagnostic> {
    values
        .get(column)
        .copied()
        .filter(|value| !value.trim().is_empty() && value.trim() != "null")
        .ok_or_else(|| {
            AnnotationDiagnostic::unsupported(
                "missing_required_column_value",
                format!("The curve segment result did not include `{column}`."),
                BTreeMap::new(),
            )
        })
}

fn optional_text(values: &BTreeMap<&str, &str>, column: &str) -> Option<String> {
    values.get(column).and_then(|value| {
        let value = value.trim();
        (!value.is_empty() && value != "null").then(|| value.to_owned())
    })
}

fn parse_f64_cell(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    if let Ok(number) = trimmed.parse::<f64>() {
        return number.is_finite().then_some(number);
    }
    let parsed = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    match parsed {
        serde_json::Value::Number(number) => number.as_f64().filter(|value| value.is_finite()),
        serde_json::Value::String(text) => {
            text.parse::<f64>().ok().filter(|value| value.is_finite())
        }
        _ => None,
    }
}

fn parse_numeric_array(value: &str) -> Option<Vec<f64>> {
    let parsed = serde_json::from_str::<serde_json::Value>(value.trim()).ok()?;
    let array = parsed.as_array()?;
    let mut numbers = Vec::with_capacity(array.len());
    for entry in array {
        let number = match entry {
            serde_json::Value::Number(number) => number.as_f64()?,
            serde_json::Value::String(text) => text.parse::<f64>().ok()?,
            _ => return None,
        };
        if !number.is_finite() {
            return None;
        }
        numbers.push(number);
    }
    Some(numbers)
}

#[derive(Debug, Deserialize)]
struct AlignmentAnnotationsApiRequest {
    resource: String,
    path: PathSourceRequest,
    #[serde(default)]
    part: Option<String>,
    #[serde(default, alias = "layerId")]
    layer_id: Option<String>,
    #[serde(default)]
    line: Option<PathLineRequest>,
    #[serde(default)]
    markers: Vec<PathMarkerRequest>,
    #[serde(default, alias = "maxSamples")]
    max_samples: Option<usize>,
}

impl AlignmentAnnotationsApiRequest {
    fn path_id(&self) -> Option<&str> {
        self.path
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }

    fn annotation_part(&self) -> Result<Option<PathAnnotationPart>, AnnotationDiagnostic> {
        let Some(part) = self
            .part
            .as_deref()
            .map(str::trim)
            .filter(|part| !part.is_empty())
        else {
            return Ok(None);
        };
        PathAnnotationPart::parse(part).map(Some).ok_or_else(|| {
            AnnotationDiagnostic::error(
                "unsupported_path_annotation_part",
                "Path annotation part must be `line` or `stations` when provided.",
                BTreeMap::from([(
                    "part".to_owned(),
                    serde_json::Value::String(part.to_owned()),
                )]),
            )
        })
    }

    fn valid_layer_id(&self) -> Option<&str> {
        self.layer_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathSourceRequest {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    measure: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathLineRequest {
    #[serde(default)]
    ranges: Vec<MeasureRangeRequest>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathMarkerRequest {
    #[serde(default)]
    range: Option<MeasureRangeRequest>,
    #[serde(default, alias = "interval", alias = "step")]
    every: Option<f64>,
    #[serde(default)]
    label: Option<serde_json::Value>,
}

impl PathMarkerRequest {
    fn label_kind(&self) -> PathMarkerLabel {
        match self.label.as_ref() {
            Some(serde_json::Value::Bool(false)) => PathMarkerLabel::None,
            Some(serde_json::Value::Null) => PathMarkerLabel::None,
            Some(serde_json::Value::String(value))
                if matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "none" | "off" | "false"
                ) =>
            {
                PathMarkerLabel::None
            }
            _ => PathMarkerLabel::Measure,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeasureRangeRequest {
    #[serde(
        default,
        alias = "start",
        alias = "from_measure",
        alias = "fromMeasure"
    )]
    from: Option<f64>,
    #[serde(default, alias = "end", alias = "to_measure", alias = "toMeasure")]
    to: Option<f64>,
    #[serde(
        default,
        alias = "from_offset",
        alias = "fromOffset",
        alias = "start_offset",
        alias = "startOffset"
    )]
    from_offset: Option<f64>,
    #[serde(
        default,
        alias = "to_offset",
        alias = "toOffset",
        alias = "end_offset",
        alias = "endOffset"
    )]
    to_offset: Option<f64>,
    #[serde(
        default,
        alias = "to_end",
        alias = "toEnd",
        alias = "path_end",
        alias = "pathEnd"
    )]
    to_end: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlignmentAnnotationsApiResponse {
    layer: Option<AnnotationLayerJson>,
    diagnostics: Vec<AnnotationDiagnostic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnnotationDiagnostic {
    code: String,
    severity: DiagnosticSeverity,
    message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    details: BTreeMap<String, serde_json::Value>,
}

impl AnnotationDiagnostic {
    fn info(
        code: impl Into<String>,
        message: impl Into<String>,
        details: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Info,
            message: message.into(),
            details,
        }
    }

    fn unsupported(
        code: impl Into<String>,
        message: impl Into<String>,
        details: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Unsupported,
            message: message.into(),
            details,
        }
    }

    fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        details: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            details,
        }
    }

    fn is_blocking(&self) -> bool {
        matches!(
            self.severity,
            DiagnosticSeverity::Unsupported | DiagnosticSeverity::Error
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticSeverity {
    Info,
    Unsupported,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnnotationLayerJson {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    visible: bool,
    lifecycle: &'static str,
    primitives: Vec<AnnotationPrimitiveJson>,
    provenance: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum AnnotationPrimitiveJson {
    Polyline(PolylineJson),
    Marker(MarkerJson),
    Text(TextLabelJson),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolylineJson {
    id: String,
    points: Vec<[f64; 3]>,
    color: [f32; 3],
    alpha: f32,
    width_px: f32,
    depth_mode: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkerJson {
    id: String,
    position: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    direction: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    normal: Option<[f64; 3]>,
    color: [f32; 3],
    alpha: f32,
    size_px: f32,
    marker_kind: &'static str,
    depth_mode: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TextLabelJson {
    id: String,
    text: String,
    anchor: [f64; 3],
    screen_offset_px: [f32; 2],
    horizontal_align: &'static str,
    vertical_align: &'static str,
    depth_mode: &'static str,
    style: TextStyleJson,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TextStyleJson {
    color: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    background_color: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outline_color: Option<[f32; 4]>,
    size_px: f32,
    embolden_px: f32,
    padding_px: f32,
}

#[derive(Debug, Clone)]
struct SegmentRow {
    segment_id: i64,
    ordinal: i64,
    start_point: DVec2,
    direction: DVec2,
    signed_length: f64,
    parent_curve_entity: Option<String>,
    radius: Option<f64>,
}

#[derive(Debug, Clone)]
struct GradientCurveSegment {
    start_station: f64,
    length: f64,
    start_point: DVec2,
    direction: DVec2,
    end_point: Option<DVec2>,
    end_direction: Option<DVec2>,
    kind: SegmentKind,
}

#[derive(Debug, Clone, Copy)]
enum SegmentKind {
    Line,
    Circular { radius: f64, turn_sign: f64 },
    Clothoid,
}

#[derive(Debug, Clone, Copy)]
struct CurveEvaluation {
    point: DVec2,
    tangent: DVec2,
}

#[derive(Debug, Clone, Copy)]
struct StationPoseJson {
    origin: [f64; 3],
    normal: [f64; 3],
}

#[derive(Debug, Clone, Copy)]
struct SourceSegmentCounts {
    horizontal: usize,
    vertical: usize,
}

#[derive(Debug, Clone)]
struct CompiledPathAnnotationPlan {
    explicit_station_range: (f64, f64),
    part: Option<PathAnnotationPart>,
    line_ranges: Vec<CompiledPathLineRange>,
    marker_groups: Vec<CompiledPathMarkerGroup>,
}

#[derive(Debug, Clone)]
struct CompiledPathLineRange {
    id_fragment: String,
    measure_range: (f64, f64),
}

#[derive(Debug, Clone)]
struct CompiledPathMarkerGroup {
    id_fragment: String,
    #[allow(dead_code)]
    measure_range: (f64, f64),
    measures: Vec<f64>,
    label: PathMarkerLabel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathMarkerLabel {
    None,
    Measure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathAnnotationPart {
    Line,
    Stations,
}

impl PathAnnotationPart {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "line" => Some(Self::Line),
            "stations" => Some(Self::Stations),
            _ => None,
        }
    }

    fn includes_line(self) -> bool {
        self == Self::Line
    }

    fn includes_stations(self) -> bool {
        self == Self::Stations
    }

    fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Stations => "stations",
        }
    }

    fn layer_id_fragment(self) -> &'static str {
        self.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct CompiledPathSummary {
        explicit_station_range: (f64, f64),
        line_ranges: Vec<(f64, f64)>,
        marker_ranges: Vec<(f64, f64)>,
        marker_values: Vec<Vec<f64>>,
        diagnostic_codes: Vec<String>,
    }

    fn annotation_request(value: serde_json::Value) -> AlignmentAnnotationsApiRequest {
        serde_json::from_value(value).expect("annotation request")
    }

    fn request_with_path(value: serde_json::Value) -> AlignmentAnnotationsApiRequest {
        let mut object = value.as_object().cloned().unwrap_or_default();
        object.insert(
            "resource".to_owned(),
            serde_json::json!("ifc/bridge-for-minnd"),
        );
        object.insert(
            "path".to_owned(),
            serde_json::json!({ "kind": "ifc_alignment", "id": "curve:215711", "measure": "station" }),
        );
        annotation_request(serde_json::Value::Object(object))
    }

    fn summarize_plan(
        explicit_station_range: (f64, f64),
        request: &AlignmentAnnotationsApiRequest,
        max_samples: usize,
    ) -> CompiledPathSummary {
        let plan = compile_path_annotation_plan(explicit_station_range, request, max_samples)
            .expect("path annotation plan");
        CompiledPathSummary {
            explicit_station_range: plan.explicit_station_range,
            line_ranges: plan
                .line_ranges
                .iter()
                .map(|range| range.measure_range)
                .collect(),
            marker_ranges: plan
                .marker_groups
                .iter()
                .map(|group| group.measure_range)
                .collect(),
            marker_values: plan
                .marker_groups
                .iter()
                .map(|group| group.measures.clone())
                .collect(),
            diagnostic_codes: Vec::new(),
        }
    }

    fn summarize_plan_error(
        explicit_station_range: (f64, f64),
        request: &AlignmentAnnotationsApiRequest,
        max_samples: usize,
    ) -> CompiledPathSummary {
        let diagnostic = compile_path_annotation_plan(explicit_station_range, request, max_samples)
            .expect_err("path annotation plan should fail");
        CompiledPathSummary {
            explicit_station_range,
            line_ranges: Vec::new(),
            marker_ranges: Vec::new(),
            marker_values: Vec::new(),
            diagnostic_codes: vec![diagnostic.code],
        }
    }

    fn straight_path_segments(
        end_station: f64,
    ) -> (Vec<GradientCurveSegment>, Vec<GradientCurveSegment>) {
        (
            vec![GradientCurveSegment {
                start_station: 0.0,
                length: end_station,
                start_point: DVec2::new(0.0, 0.0),
                direction: DVec2::X,
                end_point: None,
                end_direction: None,
                kind: SegmentKind::Line,
            }],
            vec![GradientCurveSegment {
                start_station: 0.0,
                length: end_station,
                start_point: DVec2::new(0.0, 10.0),
                direction: DVec2::X,
                end_point: None,
                end_direction: None,
                kind: SegmentKind::Line,
            }],
        )
    }

    fn compile_fixture_layer(request: &AlignmentAnnotationsApiRequest) -> AnnotationLayerJson {
        let (horizontal, vertical) = straight_path_segments(500.0);
        let response = compile_alignment_annotations_from_explicit_path(
            request,
            "curve:215711",
            215711,
            &horizontal,
            &vertical,
            SourceSegmentCounts {
                horizontal: 1,
                vertical: 1,
            },
            8,
            validate_request(request),
        );
        assert!(
            response
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.is_blocking()),
            "unexpected diagnostics: {:?}",
            response.diagnostics
        );
        response.layer.expect("annotation layer")
    }

    fn primitive_ids(layer: &AnnotationLayerJson) -> Vec<String> {
        layer
            .primitives
            .iter()
            .map(|primitive| match primitive {
                AnnotationPrimitiveJson::Polyline(polyline) => polyline.id.clone(),
                AnnotationPrimitiveJson::Marker(marker) => marker.id.clone(),
                AnnotationPrimitiveJson::Text(label) => label.id.clone(),
            })
            .collect()
    }

    fn line_primitive_ids(layer: &AnnotationLayerJson) -> Vec<String> {
        layer
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                AnnotationPrimitiveJson::Polyline(polyline)
                    if polyline.id.starts_with("path-line-") =>
                {
                    Some(polyline.id.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn marker_label_texts(layer: &AnnotationLayerJson) -> Vec<String> {
        layer
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                AnnotationPrimitiveJson::Text(label) => Some(label.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn straight_cypher_segments() -> CypherQueryResult {
        CypherQueryResult {
            columns: vec![
                "curve_node_id".to_owned(),
                "segment_node_id".to_owned(),
                "segment_ordinal".to_owned(),
                "start_point".to_owned(),
                "direction".to_owned(),
                "segment_length".to_owned(),
                "parent_curve_entity".to_owned(),
                "radius".to_owned(),
            ],
            rows: vec![
                vec!["215711", "1", "0", "[0,0]", "[1,0]", "200", "null", "null"],
                vec![
                    "215711", "2", "1", "[200,0]", "[1,0]", "300", "null", "null",
                ],
            ]
            .into_iter()
            .map(|row| row.into_iter().map(str::to_owned).collect())
            .collect(),
        }
    }

    #[test]
    fn station_values_are_capped() {
        let error = station_values_in_range((0.0, 100.0), 1.0, 10).unwrap_err();
        assert!(error.contains("more than 10"));
    }

    #[test]
    fn pure_plan_line_100_to_200_emits_one_line_range() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 100.0, "to": 200.0 }] }
        }));

        assert_eq!(
            summarize_plan((0.0, 500.0), &request, 64),
            CompiledPathSummary {
                explicit_station_range: (0.0, 500.0),
                line_ranges: vec![(100.0, 200.0)],
                marker_ranges: Vec::new(),
                marker_values: Vec::new(),
                diagnostic_codes: Vec::new(),
            }
        );
    }

    #[test]
    fn pure_plan_line_100_to_200_has_no_markers_when_none_requested() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 100.0, "to": 200.0 }] }
        }));
        let summary = summarize_plan((0.0, 500.0), &request, 64);

        assert_eq!(summary.line_ranges, vec![(100.0, 200.0)]);
        assert!(summary.marker_ranges.is_empty());
        assert!(summary.marker_values.is_empty());
    }

    #[test]
    fn pure_plan_marker_only_add_does_not_emit_line() {
        let request = request_with_path(serde_json::json!({
            "markers": [
                { "range": { "from": 120.0, "to_end": true }, "every": 50.0, "label": "measure" }
            ]
        }));

        assert_eq!(
            summarize_plan((0.0, 300.0), &request, 64),
            CompiledPathSummary {
                explicit_station_range: (0.0, 300.0),
                line_ranges: Vec::new(),
                marker_ranges: vec![(120.0, 300.0)],
                marker_values: vec![vec![150.0, 200.0, 250.0, 300.0]],
                diagnostic_codes: Vec::new(),
            }
        );
    }

    #[test]
    fn pure_plan_empty_line_means_whole_explicit_path() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [] }
        }));

        assert_eq!(
            summarize_plan((0.0, 500.0), &request, 64).line_ranges,
            vec![(0.0, 500.0)]
        );
    }

    #[test]
    fn pure_plan_line_outside_explicit_path_fails_with_details() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 400.0, "to": 600.0 }] }
        }));

        assert_eq!(
            summarize_plan_error((0.0, 500.0), &request, 64).diagnostic_codes,
            vec!["measure_range_outside_explicit_path"]
        );
    }

    #[test]
    fn path_line_ranges_crop_alignment_without_guessing() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "line": { "ranges": [{ "to": 140.0 }] }
        }))
        .expect("request");

        assert_eq!(
            requested_path_line_ranges((0.0, 280.0), &request).expect("ranges")[0].measure_range,
            (0.0, 140.0)
        );
        let error = requested_path_line_ranges((0.0, 100.0), &request).unwrap_err();
        assert_eq!(error.code, "measure_range_outside_explicit_path");
    }

    #[test]
    fn path_markers_sample_independent_ranges() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "line": { "ranges": [{ "from": 0.0, "to": 120.0 }, { "from": 120.0 }] },
            "markers": [
                { "range": { "from": 0.0, "to": 120.0 }, "every": 20.0, "label": "measure" },
                { "range": { "from": 120.0 }, "every": 50.0, "label": "measure" }
            ]
        }))
        .expect("request");

        let lines = requested_path_line_ranges((0.0, 300.0), &request).expect("line ranges");
        let groups =
            requested_path_marker_groups((0.0, 300.0), &request, 64).expect("marker groups");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].measure_range, (0.0, 120.0));
        assert_eq!(lines[1].measure_range, (120.0, 300.0));
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].measures,
            vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0, 120.0]
        );
        assert_eq!(groups[1].measures, vec![150.0, 200.0, 250.0, 300.0]);
    }

    #[test]
    fn path_marker_followup_can_emit_rest_only() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "markers": [
                { "range": { "from": 120.0 }, "every": 50.0, "label": "measure" }
            ]
        }))
        .expect("request");

        let lines = requested_path_line_ranges((0.0, 300.0), &request).expect("line ranges");
        let groups =
            requested_path_marker_groups((0.0, 300.0), &request, 64).expect("marker groups");

        assert!(lines.is_empty());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].measures, vec![150.0, 200.0, 250.0, 300.0]);
    }

    #[test]
    fn path_marker_followup_tolerates_zero_to_offset_as_path_end() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "line": { "ranges": [{ "from": 120.0, "to_offset": 0.0 }] },
            "markers": [
                { "range": { "from": 120.0, "to_offset": 0.0 }, "every": 50.0, "label": "measure" }
            ]
        }))
        .expect("request");

        let lines = requested_path_line_ranges((0.0, 300.0), &request).expect("line ranges");
        let groups =
            requested_path_marker_groups((0.0, 300.0), &request, 64).expect("marker groups");

        assert_eq!(lines[0].measure_range, (120.0, 300.0));
        assert_eq!(groups[0].measure_range, (120.0, 300.0));
        assert_eq!(groups[0].measures, vec![150.0, 200.0, 250.0, 300.0]);
    }

    #[test]
    fn path_range_to_end_uses_explicit_path_end() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "line": { "ranges": [{ "from": 400.0, "to_end": true }] },
            "markers": [
                { "range": { "from": 400.0, "to_end": true }, "every": 50.0, "label": "measure" }
            ]
        }))
        .expect("request");

        let lines = requested_path_line_ranges((0.0, 500.0), &request).expect("line ranges");
        let groups =
            requested_path_marker_groups((0.0, 500.0), &request, 64).expect("marker groups");

        assert_eq!(lines[0].measure_range, (400.0, 500.0));
        assert_eq!(groups[0].measure_range, (400.0, 500.0));
        assert_eq!(groups[0].measures, vec![400.0, 450.0, 500.0]);
    }

    #[test]
    fn path_range_rejects_to_end_with_numeric_end() {
        let request: AlignmentAnnotationsApiRequest = serde_json::from_value(serde_json::json!({
            "resource": "ifc/infra-road",
            "path": { "kind": "ifc_alignment", "id": "curve:42", "measure": "station" },
            "line": { "ranges": [{ "from": 400.0, "to": 500.0, "to_end": true }] }
        }))
        .expect("request");

        let diagnostics = validate_request(&request);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cannot provide to_end"))
        );
    }

    #[test]
    fn cypher_rows_extract_ordered_explicit_path_segments() {
        let rows = straight_cypher_segments();
        let horizontal = gradient_curve_segments(&rows, false, "horizontal").expect("horizontal");
        let vertical = gradient_curve_segments(&rows, true, "vertical").expect("vertical");

        assert_eq!(station_range(&horizontal), Some((0.0, 500.0)));
        assert_eq!(station_range(&vertical), Some((0.0, 500.0)));

        let station_100 = station_pose(&horizontal, &vertical, 100.0).expect("station 100");
        let station_200 = station_pose(&horizontal, &vertical, 200.0).expect("station 200");
        assert_eq!(station_100.origin, [100.0, 0.0, 0.0]);
        assert_eq!(station_200.origin, [200.0, 0.0, 0.0]);

        let samples =
            sampled_points(&horizontal, &vertical, &[100.0, 150.0, 200.0]).expect("samples");
        assert_eq!(
            samples,
            vec![[100.0, 0.0, 0.0], [150.0, 0.0, 0.0], [200.0, 0.0, 0.0]]
        );
    }

    #[test]
    fn endpoint_replay_line_100_to_200_compiles_one_line_primitive() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 100.0, "to": 200.0 }] }
        }));

        let layer = compile_fixture_layer(&request);

        assert_eq!(line_primitive_ids(&layer), vec!["path-line-line0-100-200"]);
        assert_eq!(layer.primitives.len(), 1);
        let AnnotationPrimitiveJson::Polyline(polyline) = &layer.primitives[0] else {
            panic!("expected line primitive");
        };
        assert_eq!(polyline.width_px, 6.0);
        assert_eq!(polyline.depth_mode, ALIGNMENT_SCENE_DEPTH_MODE);
        assert_eq!(polyline.points.first().copied(), Some([100.0, 0.0, 10.0]));
        assert_eq!(polyline.points.last().copied(), Some([200.0, 0.0, 10.0]));
    }

    #[test]
    fn endpoint_replay_composes_line_and_marker_ranges_without_full_path_line() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 300.0, "to": 400.0 }] },
            "markers": [
                { "range": { "from": 300.0, "to": 400.0 }, "every": 20.0, "label": "measure" }
            ]
        }));

        let layer = compile_fixture_layer(&request);
        let ids = primitive_ids(&layer);

        assert_eq!(line_primitive_ids(&layer), vec!["path-line-line0-300-400"]);
        assert!(ids.iter().any(|id| id == "path-marker-marker0-300-400-300"));
        assert!(ids.iter().any(|id| id == "path-marker-marker0-300-400-400"));
        assert_eq!(
            marker_label_texts(&layer),
            vec!["300", "320", "340", "360", "380", "400"]
        );
        assert!(!ids.iter().any(|id| id == "path-line-line0-0-500"));
    }

    #[test]
    fn endpoint_replay_to_end_line_and_markers_use_explicit_end() {
        let request = request_with_path(serde_json::json!({
            "line": { "ranges": [{ "from": 400.0, "to_end": true }] },
            "markers": [
                { "range": { "from": 400.0, "to_end": true }, "every": 50.0, "label": "measure" }
            ]
        }));

        let layer = compile_fixture_layer(&request);

        assert_eq!(line_primitive_ids(&layer), vec!["path-line-line0-400-500"]);
        assert_eq!(marker_label_texts(&layer), vec!["400", "450", "500"]);
    }

    #[test]
    fn endpoint_replay_marker_only_payload_emits_no_line_primitive() {
        let request = request_with_path(serde_json::json!({
            "markers": [
                { "range": { "from": 400.0, "to_end": true }, "every": 50.0, "label": "measure" }
            ]
        }));

        let layer = compile_fixture_layer(&request);

        assert!(line_primitive_ids(&layer).is_empty());
        assert_eq!(marker_label_texts(&layer), vec!["400", "450", "500"]);
    }

    #[test]
    fn endpoint_replay_part_line_defaults_to_whole_path_and_excludes_stations() {
        let request = request_with_path(serde_json::json!({
            "part": "line",
            "markers": [
                { "range": { "from": 100.0, "to": 140.0 }, "every": 20.0, "label": "measure" }
            ]
        }));

        let layer = compile_fixture_layer(&request);

        assert_eq!(primitive_ids(&layer), vec!["path-line-line0-0-500"]);
        assert_eq!(line_primitive_ids(&layer), vec!["path-line-line0-0-500"]);
        assert!(marker_label_texts(&layer).is_empty());
    }

    #[test]
    fn endpoint_replay_part_stations_excludes_line_and_keeps_marker_labels() {
        let request = request_with_path(serde_json::json!({
            "part": "stations",
            "line": { "ranges": [{ "from": 0.0, "to": 500.0 }] },
            "markers": [
                { "range": { "from": 100.0, "to": 140.0 }, "every": 20.0, "label": "measure" }
            ]
        }));

        let layer = compile_fixture_layer(&request);
        let ids = primitive_ids(&layer);

        assert!(line_primitive_ids(&layer).is_empty());
        assert!(ids.iter().any(|id| id == "path-marker-marker0-100-140-100"));
        assert!(
            ids.iter()
                .any(|id| id == "path-marker-label-leader-marker0-100-140-100")
        );
        assert_eq!(marker_label_texts(&layer), vec!["100", "120", "140"]);
    }

    #[test]
    fn endpoint_replay_part_layer_ids_are_deterministic_and_independent() {
        let line_request = request_with_path(serde_json::json!({
            "part": "line"
        }));
        let stations_request = request_with_path(serde_json::json!({
            "part": "stations",
            "markers": [
                { "range": { "from": 100.0, "to": 140.0 }, "every": 20.0, "label": "measure" }
            ]
        }));

        let line_layer = compile_fixture_layer(&line_request);
        let line_layer_again = compile_fixture_layer(&line_request);
        let stations_layer = compile_fixture_layer(&stations_request);

        assert_eq!(line_layer.id, line_layer_again.id);
        assert_eq!(
            line_layer.id,
            "path-annotations-ifc-bridge-for-minnd-ifc-alignment-curve-215711-line"
        );
        assert_eq!(
            stations_layer.id,
            "path-annotations-ifc-bridge-for-minnd-ifc-alignment-curve-215711-stations"
        );
        assert_ne!(line_layer.id, stations_layer.id);
    }

    #[test]
    fn endpoint_replay_layer_id_alias_overrides_part_layer_id_when_valid() {
        let request = request_with_path(serde_json::json!({
            "part": "line",
            "layerId": "  alignment-line-custom  "
        }));
        let fallback_request = request_with_path(serde_json::json!({
            "part": "line",
            "layer_id": "   "
        }));

        let layer = compile_fixture_layer(&request);
        let fallback_layer = compile_fixture_layer(&fallback_request);

        assert_eq!(layer.id, "alignment-line-custom");
        assert_eq!(
            fallback_layer.id,
            "path-annotations-ifc-bridge-for-minnd-ifc-alignment-curve-215711-line"
        );
    }

    #[test]
    fn line_segments_evaluate_station_positions() {
        let horizontal = vec![GradientCurveSegment {
            start_station: 0.0,
            length: 10.0,
            start_point: DVec2::new(2.0, 3.0),
            direction: DVec2::X,
            end_point: None,
            end_direction: None,
            kind: SegmentKind::Line,
        }];
        let vertical = vec![GradientCurveSegment {
            start_station: 0.0,
            length: 10.0,
            start_point: DVec2::new(0.0, 5.0),
            direction: DVec2::X,
            end_point: None,
            end_direction: None,
            kind: SegmentKind::Line,
        }];
        let pose = station_pose(&horizontal, &vertical, 4.0).unwrap();
        assert_eq!(pose.origin, [6.0, 3.0, 5.0]);
        assert_eq!(pose.normal, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn curve_filter_is_scoped_before_optional_parent_curve_match() {
        let horizontal = build_gradient_curve_horizontal_segments_query(215711);
        let vertical = build_gradient_curve_vertical_segments_query(215711);

        for query in [horizontal, vertical] {
            let where_index = query
                .find("WHERE id(curve) = 215711")
                .expect("query should filter the requested curve");
            let optional_index = query
                .find("OPTIONAL MATCH (segment)-[:PARENT_CURVE]->(parent_curve)")
                .expect("query should include parent curve metadata");
            assert!(
                where_index < optional_index,
                "the curve filter must constrain the main MATCH, not the optional parent curve match"
            );
        }
    }

    #[test]
    fn alignment_root_curve_query_uses_explicit_axis_representation() {
        let query = build_alignment_root_curve_query(193656);
        assert!(query.contains("MATCH (alignment:IfcAlignment)"));
        assert!(query.contains("[:REPRESENTATION]"));
        assert!(query.contains("[:REPRESENTATIONS]"));
        assert!(query.contains("[:ITEMS]->(curve:IfcGradientCurve)"));
        assert!(query.contains("WHERE id(alignment) = 193656"));
        assert!(
            query.contains("RETURN id(alignment) AS alignment_node_id, id(curve) AS curve_node_id")
        );
    }

    #[test]
    fn alignment_scene_annotations_draw_through_model_surfaces() {
        assert_eq!(ALIGNMENT_SCENE_DEPTH_MODE, "xray");
        assert_eq!(ALIGNMENT_TEXT_DEPTH_MODE, "overlay");
    }

    #[test]
    fn station_label_anchor_is_visual_leader_offset_only() {
        assert_eq!(station_label_anchor([10.0, 20.0, 30.0]), [10.0, 20.0, 31.8]);
    }
}
