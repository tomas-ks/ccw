export type PostViewerJson = (
  apiBase: string | undefined,
  path: string,
  body: Record<string, unknown>,
) => Promise<string>;

export type IfcQuantityTakeoffGroupBy =
  | "entity"
  | "material"
  | "bridge_part"
  | "source_resource";

export type IfcQuantityTakeoffSource = "ifc_quantities" | "geometry" | "count_only";

export type IfcSectionOrientation =
  | string
  | {
      axis?: string;
      angle?: number;
      normal?: NumericTuple;
      up?: NumericTuple;
    };

export type NumericTuple = [number, number] | [number, number, number];

export type IfcSectionPoint =
  | NumericTuple
  | {
      x?: number;
      y?: number;
      z?: number;
    };

export type IfcQuantityTakeoffArgs = {
  resource?: string;
  groupBy?: IfcQuantityTakeoffGroupBy | string;
  group_by?: IfcQuantityTakeoffGroupBy | string;
  entityNames?: string[];
  entity_names?: string[];
  semanticIds?: string[];
  semantic_ids?: string[];
  source?: IfcQuantityTakeoffSource | string;
  limit?: number;
  apiBase?: string;
  api_base?: string;
};

export type IfcSectionAtPointOrStationArgs = {
  resource?: string;
  station?: number | string;
  point?: IfcSectionPoint;
  orientation?: IfcSectionOrientation;
  width?: number;
  depth?: number;
  semanticIds?: string[];
  semantic_ids?: string[];
  limit?: number;
  apiBase?: string;
  api_base?: string;
};

export type IfcAlignmentCatalogArgs = {
  resource?: string;
  limit?: number;
  apiBase?: string;
  api_base?: string;
};

export type IfcStationResolveArgs = {
  resource?: string;
  alignmentId?: string | number;
  alignment_id?: string | number;
  station?: number | string;
  width?: number;
  height?: number;
  thickness?: number;
  clip?: string;
  limit?: number;
  apiBase?: string;
  api_base?: string;
};

type DiagnosticSeverity = "info" | "warning" | "unsupported" | "error";

type Diagnostic = {
  code: string;
  severity: DiagnosticSeverity;
  message: string;
  inputs_needed?: string[];
  details?: Record<string, unknown>;
};

type ProvenanceQuery = {
  purpose: string;
  path: "/api/cypher";
  resource: string;
  resourceFilter?: string[];
  cypher: string;
};

type CypherPayload = {
  resource?: string;
  columns?: unknown;
  rows?: unknown;
  semantic_element_ids?: unknown;
  resource_errors?: unknown;
  ok?: unknown;
  error?: unknown;
};

type CypherResult = {
  text: string;
  payload: CypherPayload | null;
  columns: string[];
  rows: Record<string, unknown>[];
  resourceErrors: unknown[];
};

type SemanticIdSpec = {
  raw: string;
  localId: string;
  sourceResource?: string;
};

type NormalizedRequest = {
  resource: string;
  apiBase?: string;
  limit: number;
  entityNames: string[];
  semanticIds: SemanticIdSpec[];
};

type NormalizedStationRequest = NormalizedRequest & {
  alignmentId: string | null;
  station: number | string | null;
  width: number | null;
  height: number | null;
  thickness: number | null;
  clip: string | null;
};

type Vec2 = [number, number];
type Vec3 = [number, number, number];

type GradientCurveSegmentKind =
  | { kind: "line" }
  | { kind: "circular"; radius: number; turnSign: number }
  | { kind: "clothoid" };

type GradientCurveSegment = {
  segmentId: number;
  segmentOrdinal: number;
  startStation: number;
  length: number;
  signedLength: number;
  startPoint: Vec2;
  direction: Vec2;
  endPoint: Vec2 | null;
  endDirection: Vec2 | null;
  segmentKind: GradientCurveSegmentKind;
};

const DEFAULT_LIMIT = 50;
const MAX_LIMIT = 500;
const DEFAULT_SECTION_WIDTH = 20;
const DEFAULT_SECTION_HEIGHT = 20;
const DEFAULT_SECTION_THICKNESS = 0.1;
const DEFAULT_SECTION_CLIP = "clip-positive-normal";
const WORLD_UP: Vec3 = [0, 0, 1];
const SECTION_INPUTS_NEEDED = [
  "a resolved alignment or an explicit world-space section plane",
  "a trusted station-to-world transform when station is used",
  "world-space product bounds or a mesh/section intersection endpoint",
  "section extents derived from width/depth after the plane is known",
];

export async function ifcQuantityTakeoff(
  args: IfcQuantityTakeoffArgs,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const request = normalizeQuantityRequest(args);
  if (!request.resource) {
    return jsonResult({
      ok: false,
      tool: "ifc_quantity_takeoff",
      provenance: {
        source_requested: normalizeSource(args.source),
        source_used: null,
        queries: [],
      },
      rows: [],
      diagnostics: [
        diagnostic(
          "missing_resource",
          "error",
          "Quantity takeoff requires the selected IFC resource.",
        ),
      ],
    });
  }

  const source = normalizeSource(args.source);
  const groupBy = normalizeGroupBy(args.groupBy ?? args.group_by);
  const provenance: Record<string, unknown> = {
    resource: request.resource,
    group_by: groupBy,
    source_requested: source,
    source_used: null,
    queries: [],
  };

  if (source === "geometry") {
    provenance.source_used = null;
    return jsonResult({
      ok: false,
      tool: "ifc_quantity_takeoff",
      provenance,
      rows: [],
      diagnostics: [
        diagnostic(
          "geometry_quantities_unsupported",
          "unsupported",
          [
            "Geometry-derived quantity takeoff is not implemented by this helper.",
            "No lengths, areas, volumes, weights, or section quantities were estimated.",
          ].join(" "),
          {
            inputs_needed: [
              "an explicit geometry quantity endpoint or trusted per-element measured quantities",
              "unit metadata for the measured quantity values",
              "a provenance path from each quantity back to the IFC product or quantity set",
            ],
          },
        ),
      ],
    });
  }

  if (
    source === "ifc_quantities" &&
    (groupBy === "entity" || groupBy === "source_resource")
  ) {
    return runIfcQuantityFactsTakeoff(request, groupBy, postViewerJson, provenance);
  }

  if (groupBy === "material") {
    provenance.source_used = "material_association_count";
    return runSingleCypherTakeoff(
      request,
      postViewerJson,
      provenance,
      buildMaterialAssociationQuery(request),
      "count products grouped by explicit IfcRelAssociatesMaterial targets",
      materialDiagnostics(source),
      source === "ifc_quantities"
        ? [
            diagnostic(
              "ifc_quantities_not_used_for_material_grouping",
              "warning",
              "Material grouping uses explicit IfcRelAssociatesMaterial graph facts, not IFC quantity values.",
            ),
          ]
        : [],
    );
  }

  if (groupBy === "bridge_part") {
    provenance.source_used = "bridge_part_containment_count";
    return runBridgePartTakeoff(
      request,
      postViewerJson,
      provenance,
      source === "ifc_quantities"
        ? [
            diagnostic(
              "ifc_quantities_not_used_for_bridge_part_grouping",
              "warning",
              "Bridge-part grouping is count-based over explicit IfcBridgePart containment; IFC quantity values are not aggregated here.",
            ),
          ]
        : [],
    );
  }

  provenance.source_used = "count_only";
  return runSingleCypherTakeoff(
    request,
    postViewerJson,
    provenance,
    groupBy === "source_resource"
      ? buildSourceResourceCountQuery(request)
      : buildEntityCountQuery(request),
    groupBy === "source_resource"
      ? "count matching products per IFC source resource"
      : "count matching products per declared IFC entity",
    [
      diagnostic(
        "count_only_takeoff",
        "info",
        "This first quantity-takeoff helper reports element counts only; it does not estimate measured geometric quantities.",
      ),
    ],
  );
}

export async function ifcSectionAtPointOrStation(
  args: IfcSectionAtPointOrStationArgs,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const request = normalizeSectionRequest(args);
  const provenance: Record<string, unknown> = {
    resource: request.resource,
    station: normalizeStation(args.station),
    point: normalizePoint(args.point),
    orientation: args.orientation ?? null,
    width: finiteNumber(args.width),
    depth: finiteNumber(args.depth),
    section_geometry: "not_implemented",
    queries: [],
  };
  const baseDiagnostics = [sectionOverlayDiagnostic(args)];

  if (!request.resource) {
    return jsonResult({
      ok: false,
      tool: "ifc_section_at_point_or_station",
      provenance,
      rows: [],
      diagnostics: [
        diagnostic(
          "missing_resource",
          "error",
          "Section lookup requires the selected IFC resource.",
        ),
        ...baseDiagnostics,
      ],
    });
  }

  if (request.semanticIds.length > 0) {
    const query = buildSemanticIdCandidateQuery(request);
    const result = await runCypher(
      request,
      postViewerJson,
      query,
      "load explicitly requested section candidate elements by semantic id",
      semanticResourceFilter(request.semanticIds),
    );
    addQuery(provenance, result.query);
    if (result.error) {
      return jsonResult({
        ok: false,
        tool: "ifc_section_at_point_or_station",
        provenance,
        rows: [],
        diagnostics: [result.error, ...baseDiagnostics],
      });
    }

    const rows = result.result.rows;
    return jsonResult({
      ok: true,
      tool: "ifc_section_at_point_or_station",
      provenance,
      rows,
      diagnostics: [
        diagnostic(
          "explicit_semantic_id_candidates_only",
          "info",
          "Returned only the explicit semantic-id candidates. No section intersection or clipping geometry was computed.",
        ),
        ...resourceErrorDiagnostics(result.result),
        ...baseDiagnostics,
      ],
    });
  }

  const station = normalizeStation(args.station);
  if (station !== null) {
    const query = buildStationCandidateQuery(station, request.limit);
    const result = await runCypher(
      request,
      postViewerJson,
      query,
      "load products with explicit IfcLinearPlacement distance matching the requested station",
    );
    addQuery(provenance, result.query);
    if (result.error) {
      return jsonResult({
        ok: false,
        tool: "ifc_section_at_point_or_station",
        provenance,
        rows: [],
        diagnostics: [result.error, ...baseDiagnostics],
      });
    }

    return jsonResult({
      ok: true,
      tool: "ifc_section_at_point_or_station",
      provenance,
      rows: result.result.rows,
      diagnostics: [
        diagnostic(
          "station_candidates_are_exact_linear_placement_matches",
          "info",
          "Rows are products whose explicit IfcLinearPlacement DistanceAlong value matched the requested station. This is not a computed section intersection.",
        ),
        ...resourceErrorDiagnostics(result.result),
        ...baseDiagnostics,
      ],
    });
  }

  return jsonResult({
    ok: false,
    tool: "ifc_section_at_point_or_station",
    provenance,
    rows: [],
    diagnostics: [
      diagnostic(
        "no_safe_candidate_anchor",
        "unsupported",
        [
          "No candidate elements were gathered because the request did not include explicit semantic ids or a station that can be matched to explicit IfcLinearPlacement facts.",
          "A point alone is not enough here because this helper has no trusted world-space section/intersection endpoint.",
        ].join(" "),
      ),
      ...baseDiagnostics,
    ],
  });
}

export async function ifcAlignmentCatalog(
  args: IfcAlignmentCatalogArgs,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const request = normalizeAlignmentCatalogRequest(args);
  const provenance: Record<string, unknown> = {
    resource: request.resource,
    queries: [],
  };
  if (!request.resource) {
    return jsonResult({
      ok: false,
      tool: "ifc_alignment_catalog",
      provenance,
      candidates: [],
      diagnostics: [
        diagnostic(
          "missing_resource",
          "error",
          "Alignment catalog requires the selected IFC resource.",
        ),
      ],
    });
  }

  const querySpecs = [
    {
      query: buildAlignmentRootCatalogQuery(request.limit),
      why: "catalog explicit IfcAlignment roots",
      kind: "alignment_root",
    },
    {
      query: buildLinearPlacementCatalogQuery(request.limit),
      why: "catalog explicit IfcLinearPlacement station facts",
      kind: "linear_placement_station",
    },
    {
      query: buildReferentCatalogQuery(request.limit),
      why: "catalog explicit IfcReferent station facts through IfcLinearPlacement",
      kind: "referent_station",
    },
    {
      query: buildAlignmentCurveCatalogQuery(request.limit),
      why: "catalog explicit alignment curve segment facts",
      kind: "alignment_curve_segment",
    },
  ];

  const candidates: Record<string, unknown>[] = [];
  const diagnostics: Diagnostic[] = [
    diagnostic(
      "explicit_stationing_catalog_only",
      "info",
      "Catalog rows are limited to explicit IFC alignment, linear-placement, referent, and curve facts. No model bounds, names, or visible geometry were used.",
    ),
  ];

  for (const spec of querySpecs) {
    const result = await runCypher(request, postViewerJson, spec.query, spec.why);
    addQuery(provenance, result.query);
    if (result.error) {
      diagnostics.push(result.error);
      continue;
    }
    candidates.push(
      ...ensureSourceResourceRows(result.result.rows, request.resource).map((row) => ({
        candidate_kind: spec.kind,
        resolver_alignment_id: resolverAlignmentIdForRow(spec.kind, row),
        ...row,
      })),
    );
    diagnostics.push(...resourceErrorDiagnostics(result.result));
  }

  if (candidates.length === 0) {
    diagnostics.push(
      diagnostic(
        "no_explicit_alignment_stationing_candidates",
        "unsupported",
        "No explicit IfcAlignment, IfcLinearPlacement station, IfcReferent station, or alignment curve segment facts were found. No inferred alignment was created.",
      ),
    );
  }

  return jsonResult({
    ok: diagnostics.every((entry) => entry.severity !== "error"),
    tool: "ifc_alignment_catalog",
    provenance,
    candidates,
    diagnostics,
  });
}

export async function ifcStationResolve(
  args: IfcStationResolveArgs,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const request = normalizeStationResolveRequest(args);
  const provenance: Record<string, unknown> = {
    resource: request.resource,
    alignment_id: request.alignmentId,
    station: request.station,
    width: request.width,
    height: request.height,
    thickness: request.thickness,
    clip: request.clip,
    queries: [],
  };

  const initialDiagnostics: Diagnostic[] = [];
  if (!request.resource) {
    initialDiagnostics.push(
      diagnostic(
        "missing_resource",
        "error",
        "Station resolution requires the selected IFC resource.",
      ),
    );
  }
  if (!request.alignmentId) {
    initialDiagnostics.push(
      diagnostic(
        "missing_alignment_id",
        "error",
        "Station resolution requires an alignmentId/alignment_id from ifc_alignment_catalog.",
      ),
    );
  }
  if (request.station === null) {
    initialDiagnostics.push(
      diagnostic(
        "missing_station",
        "error",
        "Station resolution requires a station value.",
      ),
    );
  }
  if (initialDiagnostics.length > 0) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: initialDiagnostics,
    });
  }

  const linearPlacementNodeId = parseResolverNodeId(request.alignmentId, "linear_placement");
  if (linearPlacementNodeId !== null) {
    return resolveExactLinearPlacementStation(
      request,
      provenance,
      postViewerJson,
      linearPlacementNodeId,
    );
  }

  const curveNodeId = parseResolverNodeId(request.alignmentId, "curve");
  if (curveNodeId !== null) {
    return resolveGradientCurveStation(
      request,
      provenance,
      postViewerJson,
      curveNodeId,
    );
  }

  const alignmentNodeId = parseResolverNodeId(request.alignmentId, "alignment");
  if (alignmentNodeId !== null) {
    return resolveAlignmentRootGradientCurveStation(
      request,
      provenance,
      postViewerJson,
      alignmentNodeId,
    );
  }

  {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [
        diagnostic(
          "unsupported_alignment_identifier",
          "unsupported",
          [
            "This resolver currently supports resolver_alignment_id values for explicit IfcGradientCurve facts (`curve:<db-node-id>`) and exact IfcLinearPlacement station facts (`linear_placement:<db-node-id>`).",
            "It also accepts an IfcAlignment root id (`alignment:<db-node-id>`) only when that root resolves through explicit Axis representation facts to exactly one IfcGradientCurve.",
            "Use the resolver_alignment_id values from ifc_alignment_catalog.",
          ].join(" "),
          { details: { alignment_id: request.alignmentId } },
        ),
      ],
    });
  }
}

async function resolveAlignmentRootGradientCurveStation(
  request: NormalizedStationRequest,
  provenance: Record<string, unknown>,
  postViewerJson: PostViewerJson,
  alignmentNodeId: number,
): Promise<string> {
  const query = buildAlignmentRootGradientCurveQuery(alignmentNodeId);
  const result = await runCypher(
    request,
    postViewerJson,
    query,
    "resolve an IfcAlignment root to its explicit Axis IfcGradientCurve without inferring alignment geometry",
  );
  addQuery(provenance, result.query);
  if (result.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [result.error],
    });
  }

  const curveIds = Array.from(
    new Set(
      result.result.rows
        .map((row) => parseInteger(row.curve_node_id))
        .filter((id): id is number => id !== null),
    ),
  ).sort((left, right) => left - right);
  if (curveIds.length !== 1) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      rows: result.result.rows,
      diagnostics: [
        diagnostic(
          curveIds.length === 0
            ? "alignment_root_has_no_explicit_axis_curve"
            : "alignment_root_has_multiple_axis_curves",
          "unsupported",
          curveIds.length === 0
            ? "The IfcAlignment root did not expose exactly one explicit IfcGradientCurve through its Axis representation. No section pose was inferred."
            : "The IfcAlignment root exposed multiple explicit IfcGradientCurve axis items, so the section curve cannot be chosen without guessing.",
          {
            details: {
              alignment_id: request.alignmentId,
              curve_node_ids: curveIds,
            },
          },
        ),
        ...resourceErrorDiagnostics(result.result),
      ],
    });
  }

  provenance.alignment_root_id = request.alignmentId;
  provenance.basis_curve_alignment_id = `curve:${curveIds[0]}`;
  return resolveGradientCurveStation(
    {
      ...request,
      alignmentId: `curve:${curveIds[0]}`,
    },
    provenance,
    postViewerJson,
    curveIds[0],
    [
      diagnostic(
        "alignment_root_axis_curve_resolution",
        "info",
        "The IfcAlignment root was resolved through its explicit Axis representation item to one IfcGradientCurve. No alignment geometry was inferred.",
      ),
      ...resourceErrorDiagnostics(result.result),
    ],
    [`alignment:${alignmentNodeId}`, `basis_curve:curve:${curveIds[0]}`],
  );
}

async function resolveExactLinearPlacementStation(
  request: NormalizedStationRequest,
  provenance: Record<string, unknown>,
  postViewerJson: PostViewerJson,
  linearPlacementNodeId: number,
): Promise<string> {
  const query = buildLinearPlacementStationResolveQuery(
    linearPlacementNodeId,
    request.station as string | number,
  );
  const result = await runCypher(
    request,
    postViewerJson,
    query,
    "resolve exact IfcLinearPlacement station facts without curve interpolation or geometry fallback",
  );
  addQuery(provenance, result.query);
  if (result.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [result.error],
    });
  }

  const rows = result.result.rows;
  const diagnostics = [
    diagnostic(
      "exact_linear_placement_station_only",
      "info",
      "Station resolution was anchored to one explicit IfcLinearPlacement and an exact DistanceAlong match. No bbox, name, visible-geometry, or guessed-axis fallback was used.",
    ),
    ...resourceErrorDiagnostics(result.result),
  ];
  if (rows.length === 0) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "station_fact_not_found",
          "unsupported",
          "No explicit IfcLinearPlacement DistanceAlong fact matched the requested station for the requested alignment id.",
          {
            inputs_needed: [
              "an IfcLinearPlacement with a matching IfcPointByDistanceExpression DistanceAlong value",
              "the resolver_alignment_id returned by ifc_alignment_catalog for that IfcLinearPlacement",
            ],
          },
        ),
      ],
    });
  }

  const row = rows[0];
  const pose = explicitSectionPoseFromRow(row);
  if (!pose) {
    const curveNodeId = parseInteger(row.curve_node_id);
    const curveEntity = textValue(row.curve_entity);
    if (curveNodeId !== null && curveEntity === "IfcGradientCurve") {
      provenance.linear_placement_station_row = row;
      provenance.basis_curve_alignment_id = `curve:${curveNodeId}`;
      return resolveGradientCurveStation(
        {
          ...request,
          alignmentId: `curve:${curveNodeId}`,
        },
        provenance,
        postViewerJson,
        curveNodeId,
        [
          ...diagnostics,
          diagnostic(
            "linear_placement_basis_curve_resolution",
            "info",
            [
              "The station fact was found on an explicit IfcLinearPlacement.",
              "Its IfcPointByDistanceExpression BASIS_CURVE points to an IfcGradientCurve, so the section pose was resolved from that explicit curve instead of requiring precomputed pose columns.",
            ].join(" "),
          ),
        ],
        [
          `linear_placement:${linearPlacementNodeId}`,
          `basis_curve:curve:${curveNodeId}`,
        ],
      );
    }
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      rows,
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "explicit_section_pose_missing",
          "unsupported",
          [
            "The station fact exists, but the graph row did not contain explicit world-space section pose vectors.",
            "This helper will not evaluate curves, infer world placement, choose +Z up, or guess a lateral axis.",
          ].join(" "),
          {
            inputs_needed: [
              "explicit origin vector for the station plane",
              "explicit tangent vector for the in-plane width direction",
              "explicit normal vector for the section plane",
              "explicit up vector for the in-plane height direction",
            ],
            details: {
              supported_pose_columns: [
                "section_origin / SectionOrigin",
                "section_tangent / SectionTangent",
                "section_normal / SectionNormal",
                "section_up / SectionUp",
              ],
              matched_station_row: row,
            },
          },
        ),
      ],
    });
  }

  const section = {
    resource: request.resource,
    alignmentId: request.alignmentId,
    station: finiteNumber(request.station) ?? request.station,
    pose,
    width: request.width ?? DEFAULT_SECTION_WIDTH,
    height: request.height ?? DEFAULT_SECTION_HEIGHT,
    thickness: request.thickness ?? DEFAULT_SECTION_THICKNESS,
    mode: "3d-overlay",
    clip: request.clip ?? DEFAULT_SECTION_CLIP,
    provenance: [
      "ifc_station_resolve",
      `resource=${request.resource}`,
      `alignment_id=${request.alignmentId}`,
      `station=${String(request.station)}`,
      "pose=explicit_graph_vectors",
    ],
  };

  return jsonResult({
    ok: true,
    tool: "ifc_station_resolve",
    provenance,
    section,
    rows,
    diagnostics,
  });
}

async function resolveGradientCurveStation(
  request: NormalizedStationRequest,
  provenance: Record<string, unknown>,
  postViewerJson: PostViewerJson,
  curveNodeId: number,
  extraDiagnostics: Diagnostic[] = [],
  extraSectionProvenance: string[] = [],
): Promise<string> {
  const station = finiteNumber(request.station);
  if (station === null) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [
        diagnostic(
          "station_must_be_numeric_for_curve_resolution",
          "unsupported",
          "Station resolution on an IfcGradientCurve requires a numeric station value.",
          { details: { station: request.station } },
        ),
      ],
    });
  }

  const horizontal = await runCypher(
    request,
    postViewerJson,
    buildGradientCurveHorizontalSegmentsQuery(curveNodeId),
    "read explicit IfcGradientCurve BASE_CURVE segments for station resolution",
  );
  addQuery(provenance, horizontal.query);
  if (horizontal.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [horizontal.error],
    });
  }

  const vertical = await runCypher(
    request,
    postViewerJson,
    buildGradientCurveVerticalSegmentsQuery(curveNodeId),
    "read explicit IfcGradientCurve vertical/elevation segments for station resolution",
  );
  addQuery(provenance, vertical.query);
  if (vertical.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [vertical.error],
    });
  }

  const diagnostics = [
    ...extraDiagnostics,
    diagnostic(
      "explicit_gradient_curve_station_resolution",
      "info",
      [
        "Station resolution evaluated explicit IfcGradientCurve BASE_CURVE and elevation segment facts.",
        "No model bounds, names, visible geometry, or guessed alignment fallback was used.",
      ].join(" "),
    ),
    ...resourceErrorDiagnostics(horizontal.result),
    ...resourceErrorDiagnostics(vertical.result),
  ];

  const horizontalSegments = buildGradientCurveSegments(horizontal.result.rows, false);
  const verticalSegments = buildGradientCurveSegments(vertical.result.rows, true);
  const segmentDiagnostics = [
    ...gradientCurveSegmentDiagnostics("horizontal", horizontalSegments),
    ...gradientCurveSegmentDiagnostics("vertical", verticalSegments),
  ];
  if (segmentDiagnostics.some((entry) => entry.severity === "error" || entry.severity === "unsupported")) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      rows: {
        horizontal_segments: horizontal.result.rows,
        vertical_segments: vertical.result.rows,
      },
      diagnostics: [...diagnostics, ...segmentDiagnostics],
    });
  }

  const horizontalRange = stationRange(horizontalSegments.segments);
  const verticalRange = stationRange(verticalSegments.segments);
  const stationInsideRange =
    stationIsInRange(station, horizontalRange) && stationIsInRange(station, verticalRange);
  const horizontalEvaluation = stationInsideRange
    ? evaluateGradientCurveSegments(horizontalSegments.segments, station)
    : null;
  const verticalEvaluation = stationInsideRange
    ? evaluateGradientCurveSegments(verticalSegments.segments, station)
    : null;
  if (!horizontalEvaluation || !verticalEvaluation) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      rows: {
        horizontal_segments: horizontal.result.rows,
        vertical_segments: vertical.result.rows,
      },
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "station_outside_evaluable_curve_range",
          "unsupported",
          "The requested station could not be evaluated from the explicit curve segments.",
          {
            details: {
              station,
              horizontal_range: horizontalRange,
              vertical_range: verticalRange,
            },
          },
        ),
      ],
    });
  }

  const planeNormal = normalizeVec3([
    horizontalEvaluation.tangent[0],
    horizontalEvaluation.tangent[1],
    0,
  ]);
  if (!planeNormal) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "station_tangent_degenerate",
          "unsupported",
          "The resolved alignment tangent is degenerate, so a section plane normal cannot be formed without guessing.",
        ),
      ],
    });
  }
  const widthDirection = normalizeVec3(crossVec3(WORLD_UP, planeNormal));
  if (!widthDirection) {
    return jsonResult({
      ok: false,
      tool: "ifc_station_resolve",
      provenance,
      section: null,
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "station_lateral_axis_degenerate",
          "unsupported",
          "The resolved alignment tangent is parallel to the renderer up axis, so an in-plane width direction cannot be formed without guessing.",
        ),
      ],
    });
  }

  const origin: Vec3 = [
    horizontalEvaluation.point[0],
    horizontalEvaluation.point[1],
    verticalEvaluation.point[1],
  ];
  const section = {
    resource: request.resource,
    alignmentId: request.alignmentId,
    station,
    pose: {
      origin,
      tangent: widthDirection,
      normal: planeNormal,
      up: WORLD_UP,
    },
    width: request.width ?? DEFAULT_SECTION_WIDTH,
    height: request.height ?? DEFAULT_SECTION_HEIGHT,
    thickness: request.thickness ?? DEFAULT_SECTION_THICKNESS,
    mode: "3d-overlay",
    clip: request.clip ?? DEFAULT_SECTION_CLIP,
    provenance: [
      "ifc_station_resolve",
      `resource=${request.resource}`,
      `alignment_id=${request.alignmentId}`,
      `station=${String(request.station)}`,
      "pose=explicit_ifc_gradient_curve",
      ...extraSectionProvenance,
    ],
  };

  return jsonResult({
    ok: true,
    tool: "ifc_station_resolve",
    provenance,
    section,
    diagnostics: [...diagnostics, ...gradientCurveApproximationDiagnostics(horizontalSegments, verticalSegments)],
  });
}

export const executeIfcQuantityTakeoff = ifcQuantityTakeoff;
export const executeIfcSectionAtPointOrStation = ifcSectionAtPointOrStation;
export const executeIfcAlignmentCatalog = ifcAlignmentCatalog;
export const executeIfcStationResolve = ifcStationResolve;

async function runIfcQuantityFactsTakeoff(
  request: NormalizedRequest,
  groupBy: IfcQuantityTakeoffGroupBy,
  postViewerJson: PostViewerJson,
  provenance: Record<string, unknown>,
): Promise<string> {
  provenance.source_used = "ifc_quantities";
  const query =
    groupBy === "source_resource"
      ? buildIfcQuantitySourceResourceQuery(request)
      : buildIfcQuantityEntityQuery(request);
  const result = await runCypher(
    request,
    postViewerJson,
    query,
    "summarize explicit IfcElementQuantity facts without estimating geometry",
    semanticResourceFilter(request.semanticIds),
  );
  addQuery(provenance, result.query);
  if (result.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_quantity_takeoff",
      provenance,
      rows: [],
      diagnostics: [result.error],
    });
  }

  const diagnostics = [
    diagnostic(
      "explicit_ifc_quantities_only",
      "info",
      "This result only counts explicit IfcElementQuantity/IfcQuantity* facts found in the graph; it does not derive missing quantities from geometry.",
    ),
  ];
  if (result.result.rows.length === 0) {
    diagnostics.push(
      diagnostic(
        "no_explicit_ifc_quantities_found",
        "warning",
        "No explicit IfcElementQuantity facts matched the request pattern. No geometric quantities were estimated as a fallback.",
      ),
    );
  }
  diagnostics.push(...resourceErrorDiagnostics(result.result));

  return jsonResult({
    ok: true,
    tool: "ifc_quantity_takeoff",
    provenance,
    rows: ensureSourceResourceRows(result.result.rows, request.resource),
    diagnostics,
  });
}

async function runSingleCypherTakeoff(
  request: NormalizedRequest,
  postViewerJson: PostViewerJson,
  provenance: Record<string, unknown>,
  cypher: string,
  why: string,
  diagnostics: Diagnostic[],
  extraDiagnostics: Diagnostic[] = [],
): Promise<string> {
  const result = await runCypher(
    request,
    postViewerJson,
    cypher,
    why,
    semanticResourceFilter(request.semanticIds),
  );
  addQuery(provenance, result.query);
  if (result.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_quantity_takeoff",
      provenance,
      rows: [],
      diagnostics: [result.error, ...extraDiagnostics],
    });
  }

  return jsonResult({
    ok: true,
    tool: "ifc_quantity_takeoff",
    provenance,
    rows: ensureSourceResourceRows(result.result.rows, request.resource),
    diagnostics: [
      ...diagnostics,
      ...extraDiagnostics,
      ...resourceErrorDiagnostics(result.result),
    ],
  });
}

async function runBridgePartTakeoff(
  request: NormalizedRequest,
  postViewerJson: PostViewerJson,
  provenance: Record<string, unknown>,
  extraDiagnostics: Diagnostic[],
): Promise<string> {
  if (request.semanticIds.length > 0) {
    const result = await runCypher(
      request,
      postViewerJson,
      buildBridgePartSemanticIdQuery(request),
      "count explicitly requested semantic ids by containing IfcBridgePart",
      semanticResourceFilter(request.semanticIds),
    );
    addQuery(provenance, result.query);
    if (result.error) {
      return jsonResult({
        ok: false,
        tool: "ifc_quantity_takeoff",
        provenance,
        rows: [],
        diagnostics: [result.error, ...extraDiagnostics],
      });
    }
    return jsonResult({
      ok: true,
      tool: "ifc_quantity_takeoff",
      provenance,
      rows: result.result.rows,
      diagnostics: [
        diagnostic(
          "bridge_part_semantic_id_count",
          "info",
          "Bridge-part counts are based on explicit containment of the requested products under IfcBridgePart nodes.",
        ),
        ...extraDiagnostics,
        ...resourceErrorDiagnostics(result.result),
      ],
    });
  }

  const partResult = await runCypher(
    request,
    postViewerJson,
    buildBridgePartDiscoveryQuery(request.limit),
    "list candidate IfcBridgePart nodes before counting contained products one part at a time",
  );
  addQuery(provenance, partResult.query);
  if (partResult.error) {
    return jsonResult({
      ok: false,
      tool: "ifc_quantity_takeoff",
      provenance,
      rows: [],
      diagnostics: [partResult.error, ...extraDiagnostics],
    });
  }

  const rows: Record<string, unknown>[] = [];
  const diagnostics = [
    diagnostic(
      "bridge_part_count_anchored",
      "info",
      "Bridge-part counts are gathered with one anchored containment query per discovered IfcBridgePart to avoid broad unanchored aggregate traversals.",
    ),
    ...extraDiagnostics,
  ];

  for (const part of partResult.result.rows) {
    const partNodeId = parseInteger(part.bridge_part_node_id);
    if (partNodeId === null) {
      diagnostics.push(
        diagnostic(
          "bridge_part_node_id_unusable",
          "warning",
          "Skipped an IfcBridgePart row because its database node id was not numeric.",
          { details: { row: part } },
        ),
      );
      continue;
    }
    const sourceResource = textValue(part.source_resource);
    const resourceFilter = sourceResource ? [sourceResource] : undefined;
    const countResult = await runCypher(
      request,
      postViewerJson,
      buildBridgePartAnchoredCountQuery(partNodeId, request),
      `count products contained by IfcBridgePart node ${partNodeId}`,
      resourceFilter,
    );
    addQuery(provenance, countResult.query);
    if (countResult.error) {
      diagnostics.push(countResult.error);
      continue;
    }
    rows.push(...countResult.result.rows);
  }

  return jsonResult({
    ok: diagnostics.every((entry) => entry.severity !== "error"),
    tool: "ifc_quantity_takeoff",
    provenance,
    rows,
    diagnostics,
  });
}

async function runCypher(
  request: NormalizedRequest,
  postViewerJson: PostViewerJson,
  cypher: string,
  why: string,
  resourceFilter?: string[],
): Promise<{
  query: ProvenanceQuery;
  result: CypherResult;
  error?: Diagnostic;
}> {
  const cleanResourceFilter = request.resource.startsWith("project/")
    ? uniqueStrings(resourceFilter ?? [])
    : [];
  const body: Record<string, unknown> = {
    resource: request.resource,
    cypher,
    why,
  };
  if (cleanResourceFilter.length > 0) {
    body.resourceFilter = cleanResourceFilter;
  }
  const query: ProvenanceQuery = {
    purpose: why,
    path: "/api/cypher",
    resource: request.resource,
    cypher,
  };
  if (cleanResourceFilter.length > 0) {
    query.resourceFilter = cleanResourceFilter;
  }

  const text = await postViewerJson(request.apiBase, "/api/cypher", body);
  const result = parseCypherResult(text);
  if (!result.payload) {
    return {
      query,
      result,
      error: diagnostic(
        "cypher_response_not_json",
        "error",
        "Viewer Cypher response was not valid JSON.",
        { details: { response: text } },
      ),
    };
  }
  if (result.payload.ok === false || result.payload.error !== undefined) {
    return {
      query,
      result,
      error: diagnostic(
        "cypher_query_failed",
        "error",
        "Viewer Cypher query failed.",
        { details: { response: result.payload } },
      ),
    };
  }
  if (!Array.isArray(result.payload.columns) || !Array.isArray(result.payload.rows)) {
    return {
      query,
      result,
      error: diagnostic(
        "cypher_response_missing_rows",
        "error",
        "Viewer Cypher response did not include columns and rows.",
        { details: { response: result.payload } },
      ),
    };
  }
  return { query, result };
}

function buildEntityCountQuery(request: NormalizedRequest): string {
  return [
    "MATCH (n)",
    cypherWhere(productFilterLines("n", request)),
    "RETURN n.declared_entity AS entity, count(DISTINCT n) AS element_count",
    "ORDER BY element_count DESC, entity",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildSourceResourceCountQuery(request: NormalizedRequest): string {
  return [
    "MATCH (n)",
    cypherWhere(productFilterLines("n", request)),
    "RETURN count(DISTINCT n) AS element_count",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildMaterialAssociationQuery(request: NormalizedRequest): string {
  const filters = [
    ...productFilterLines("n", request),
    "material.declared_entity IS NOT NULL",
  ];
  return [
    "MATCH (n)--(:IfcRelAssociatesMaterial)--(material)",
    cypherWhere(filters),
    "RETURN material.declared_entity AS material_entity, material.Name AS material_name, count(DISTINCT n) AS element_count",
    "ORDER BY element_count DESC, material_name",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildIfcQuantityEntityQuery(request: NormalizedRequest): string {
  const filters = [
    ...productFilterLines("prod", request),
    "quantity.declared_entity STARTS WITH 'IfcQuantity'",
  ];
  return [
    "MATCH (prod)--(:IfcRelDefinesByProperties)--(qto:IfcElementQuantity)--(quantity)",
    cypherWhere(filters),
    "RETURN prod.declared_entity AS entity, qto.Name AS quantity_set_name, quantity.declared_entity AS quantity_entity, quantity.Name AS quantity_name, count(DISTINCT prod) AS elements_with_explicit_quantity_count, count(DISTINCT quantity) AS quantity_fact_count",
    "ORDER BY elements_with_explicit_quantity_count DESC, entity, quantity_name",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildIfcQuantitySourceResourceQuery(request: NormalizedRequest): string {
  const filters = [
    ...productFilterLines("prod", request),
    "quantity.declared_entity STARTS WITH 'IfcQuantity'",
  ];
  return [
    "MATCH (prod)--(:IfcRelDefinesByProperties)--(qto:IfcElementQuantity)--(quantity)",
    cypherWhere(filters),
    "RETURN count(DISTINCT prod) AS elements_with_explicit_quantity_count, count(DISTINCT quantity) AS quantity_fact_count",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildBridgePartSemanticIdQuery(request: NormalizedRequest): string {
  const filters = productFilterLines("prod", request);
  return [
    "MATCH (prod)",
    cypherWhere(filters),
    "MATCH (part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod)",
    "RETURN id(part) AS bridge_part_node_id, part.GlobalId AS bridge_part_global_id, part.Name AS bridge_part_name, count(DISTINCT prod) AS element_count",
    "ORDER BY element_count DESC, bridge_part_name",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildBridgePartDiscoveryQuery(limit: number): string {
  return [
    "MATCH (part:IfcBridgePart)",
    "RETURN id(part) AS bridge_part_node_id, part.GlobalId AS bridge_part_global_id, part.Name AS bridge_part_name",
    "ORDER BY bridge_part_name, bridge_part_node_id",
    `LIMIT ${limit}`,
  ].join("\n");
}

function buildBridgePartAnchoredCountQuery(
  partNodeId: number,
  request: NormalizedRequest,
): string {
  const filters = productFilterLines("prod", {
    ...request,
    semanticIds: [],
  });
  return [
    "MATCH (part:IfcBridgePart)",
    `WHERE id(part) = ${partNodeId}`,
    "MATCH (part)<--(:IfcRelContainedInSpatialStructure)-->(prod)",
    cypherWhere(filters),
    "RETURN id(part) AS bridge_part_node_id, part.GlobalId AS bridge_part_global_id, part.Name AS bridge_part_name, count(DISTINCT prod) AS element_count",
    "LIMIT 1",
  ]
    .filter(Boolean)
    .join("\n");
}

function buildSemanticIdCandidateQuery(request: NormalizedRequest): string {
  return [
    "MATCH (n)",
    cypherWhere(productFilterLines("n", request)),
    "RETURN id(n) AS node_id, n.GlobalId AS global_id, n.declared_entity AS entity, n.Name AS name, n.ObjectType AS object_type",
    "ORDER BY entity, name, global_id",
    `LIMIT ${request.limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildStationCandidateQuery(station: string | number, limit: number): string {
  return [
    "MATCH (prod)-[:OBJECT_PLACEMENT]->(lp:IfcLinearPlacement)-[:RELATIVE_PLACEMENT]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(station_point:IfcPointByDistanceExpression)-[:DISTANCE_ALONG]->(distance)",
    cypherWhere(stationFilterLines(station)),
    "RETURN id(prod) AS node_id, prod.GlobalId AS global_id, prod.declared_entity AS entity, prod.Name AS name, prod.ObjectType AS object_type, distance.payload_value AS station, station_point.OffsetLongitudinal AS offset_longitudinal, station_point.OffsetLateral AS offset_lateral, station_point.OffsetVertical AS offset_vertical",
    "ORDER BY entity, name, global_id",
    `LIMIT ${limit}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function buildAlignmentRootCatalogQuery(limit: number): string {
  return [
    "MATCH (alignment:IfcAlignment)",
    "OPTIONAL MATCH (alignment)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(representation:IfcShapeRepresentation)-[:ITEMS]->(curve:IfcGradientCurve)",
    "RETURN id(alignment) AS alignment_node_id, alignment.GlobalId AS global_id, alignment.declared_entity AS entity, alignment.Name AS name, alignment.ObjectType AS object_type, id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, curve.Name AS curve_name, representation.RepresentationIdentifier AS representation_identifier, representation.RepresentationType AS representation_type",
    "ORDER BY name, alignment_node_id, curve_node_id",
    `LIMIT ${limit}`,
  ].join("\n");
}

function buildLinearPlacementCatalogQuery(limit: number): string {
  return [
    "MATCH (lp:IfcLinearPlacement)-[:RELATIVE_PLACEMENT]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(station_point:IfcPointByDistanceExpression)-[:BASIS_CURVE]->(curve)",
    "MATCH (station_point)-[:DISTANCE_ALONG]->(distance)",
    "RETURN id(lp) AS linear_placement_node_id, id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, curve.Name AS curve_name, distance.payload_value AS station, station_point.OffsetLongitudinal AS offset_longitudinal, station_point.OffsetLateral AS offset_lateral, station_point.OffsetVertical AS offset_vertical",
    "ORDER BY linear_placement_node_id",
    `LIMIT ${limit}`,
  ].join("\n");
}

function buildReferentCatalogQuery(limit: number): string {
  return [
    "MATCH (referent:IfcReferent)-[:OBJECT_PLACEMENT]->(lp:IfcLinearPlacement)-[:RELATIVE_PLACEMENT]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(station_point:IfcPointByDistanceExpression)-[:BASIS_CURVE]->(curve)",
    "MATCH (station_point)-[:DISTANCE_ALONG]->(distance)",
    "RETURN id(referent) AS referent_node_id, referent.GlobalId AS global_id, referent.declared_entity AS entity, referent.Name AS name, referent.ObjectType AS object_type, id(lp) AS linear_placement_node_id, id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, distance.payload_value AS station, station_point.OffsetLongitudinal AS offset_longitudinal, station_point.OffsetLateral AS offset_lateral, station_point.OffsetVertical AS offset_vertical",
    "ORDER BY name, referent_node_id",
    `LIMIT ${limit}`,
  ].join("\n");
}

function buildAlignmentCurveCatalogQuery(limit: number): string {
  return [
    "MATCH (curve:IfcGradientCurve)-[:BASE_CURVE]->(:IfcCompositeCurve)-[segment_edge:SEGMENTS]->(segment:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)",
    "MATCH (place)-[:REF_DIRECTION]->(direction:IfcDirection)",
    "MATCH (segment)-[:SEGMENT_LENGTH]->(length)",
    "RETURN id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, curve.Name AS curve_name, id(segment) AS segment_node_id, segment_edge.ordinal AS segment_ordinal, segment.declared_entity AS segment_entity, point.Coordinates AS start_coordinates, direction.DirectionRatios AS direction_ratios, length.payload_value AS segment_length",
    "ORDER BY curve_node_id, segment_ordinal, segment_node_id",
    `LIMIT ${limit}`,
  ].join("\n");
}

function buildAlignmentRootGradientCurveQuery(alignmentNodeId: number): string {
  return [
    "MATCH (alignment:IfcAlignment)-[:REPRESENTATION]->(:IfcProductDefinitionShape)-[:REPRESENTATIONS]->(representation:IfcShapeRepresentation)-[:ITEMS]->(curve:IfcGradientCurve)",
    `WHERE id(alignment) = ${alignmentNodeId}`,
    "RETURN id(alignment) AS alignment_node_id, id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, curve.Name AS curve_name, representation.RepresentationIdentifier AS representation_identifier, representation.RepresentationType AS representation_type",
    "ORDER BY curve_node_id",
    "LIMIT 4",
  ].join("\n");
}

function buildLinearPlacementStationResolveQuery(
  linearPlacementNodeId: number,
  station: string | number,
): string {
  return [
    "MATCH (lp:IfcLinearPlacement)-[:RELATIVE_PLACEMENT]->(:IfcAxis2PlacementLinear)-[:LOCATION]->(station_point:IfcPointByDistanceExpression)-[:BASIS_CURVE]->(curve)",
    "MATCH (station_point)-[:DISTANCE_ALONG]->(distance)",
    cypherWhere([`id(lp) = ${linearPlacementNodeId}`, ...stationFilterLines(station)]),
    "RETURN id(lp) AS linear_placement_node_id, id(curve) AS curve_node_id, curve.declared_entity AS curve_entity, curve.Name AS curve_name, distance.payload_value AS station, station_point.OffsetLongitudinal AS offset_longitudinal, station_point.OffsetLateral AS offset_lateral, station_point.OffsetVertical AS offset_vertical, lp.section_origin AS section_origin, lp.section_tangent AS section_tangent, lp.section_normal AS section_normal, lp.section_up AS section_up, lp.SectionOrigin AS SectionOrigin, lp.SectionTangent AS SectionTangent, lp.SectionNormal AS SectionNormal, lp.SectionUp AS SectionUp",
    "LIMIT 1",
  ]
    .filter(Boolean)
    .join("\n");
}

function buildGradientCurveHorizontalSegmentsQuery(curveNodeId: number): string {
  return [
    "MATCH (curve:IfcGradientCurve)-[:BASE_CURVE]->(base_curve:IfcCompositeCurve)-[segment_edge:SEGMENTS]->(segment:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)",
    `WHERE id(curve) = ${curveNodeId}`,
    "MATCH (place)-[:REF_DIRECTION]->(direction:IfcDirection)",
    "MATCH (segment)-[:SEGMENT_LENGTH]->(length)",
    "OPTIONAL MATCH (segment)-[:PARENT_CURVE]->(parent_curve)",
    "RETURN id(curve) AS curve_node_id, id(segment) AS segment_node_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, direction.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius, parent_curve.ClothoidConstant AS clothoid_constant",
    "ORDER BY segment_ordinal, segment_node_id",
  ].join("\n");
}

function buildGradientCurveVerticalSegmentsQuery(curveNodeId: number): string {
  return [
    "MATCH (curve:IfcGradientCurve)-[segment_edge:SEGMENTS]->(segment:IfcCurveSegment)-[:PLACEMENT]->(place:IfcAxis2Placement2D)-[:LOCATION]->(point:IfcCartesianPoint)",
    `WHERE id(curve) = ${curveNodeId}`,
    "MATCH (place)-[:REF_DIRECTION]->(direction:IfcDirection)",
    "MATCH (segment)-[:SEGMENT_LENGTH]->(length)",
    "OPTIONAL MATCH (segment)-[:PARENT_CURVE]->(parent_curve)",
    "RETURN id(curve) AS curve_node_id, id(segment) AS segment_node_id, segment_edge.ordinal AS segment_ordinal, point.Coordinates AS start_point, direction.DirectionRatios AS direction, length.payload_value AS segment_length, parent_curve.declared_entity AS parent_curve_entity, parent_curve.Radius AS radius, parent_curve.ClothoidConstant AS clothoid_constant",
    "ORDER BY segment_ordinal, segment_node_id",
  ].join("\n");
}

function productFilterLines(alias: string, request: NormalizedRequest): string[] {
  const filters = [
    `${alias}.GlobalId IS NOT NULL`,
    `${alias}.declared_entity IS NOT NULL`,
  ];
  const entityPredicate = equalityPredicate(
    `${alias}.declared_entity`,
    request.entityNames,
  );
  if (entityPredicate) {
    filters.push(entityPredicate);
  }
  const semanticPredicate = equalityPredicate(
    `${alias}.GlobalId`,
    request.semanticIds.map((entry) => entry.localId),
  );
  if (semanticPredicate) {
    filters.push(semanticPredicate);
  }
  return filters;
}

function stationFilterLines(station: string | number): string[] {
  const values = stationLiteralValues(station);
  const predicates = values.map((value) => `distance.payload_value = ${value}`);
  return predicates.length > 0 ? [`(${predicates.join(" OR ")})`] : [];
}

function stationLiteralValues(station: string | number): string[] {
  const values = new Set<string>();
  if (typeof station === "number" && Number.isFinite(station)) {
    values.add(String(station));
    values.add(cypherString(String(station)));
    if (Number.isInteger(station)) {
      values.add(station.toFixed(1));
      values.add(cypherString(station.toFixed(1)));
    }
  } else {
    const text = String(station).trim();
    if (text) {
      values.add(cypherString(text));
      const numeric = Number(text);
      if (Number.isFinite(numeric)) {
        values.add(String(numeric));
        values.add(cypherString(String(numeric)));
        if (Number.isInteger(numeric)) {
          values.add(numeric.toFixed(1));
          values.add(cypherString(numeric.toFixed(1)));
        }
      }
    }
  }
  return [...values];
}

function equalityPredicate(property: string, values: string[]): string | null {
  const unique = uniqueStrings(values);
  if (unique.length === 0) {
    return null;
  }
  return `(${unique.map((value) => `${property} = ${cypherString(value)}`).join(" OR ")})`;
}

function cypherWhere(filters: string[]): string {
  return filters.length > 0 ? `WHERE ${filters.join("\n  AND ")}` : "";
}

function cypherString(value: string): string {
  return `'${value.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
}

function normalizeQuantityRequest(args: IfcQuantityTakeoffArgs): NormalizedRequest {
  return {
    resource: String(args.resource ?? "").trim(),
    apiBase: args.apiBase ?? args.api_base,
    limit: normalizeLimit(args.limit),
    entityNames: uniqueStrings([...(args.entityNames ?? []), ...(args.entity_names ?? [])]),
    semanticIds: parseSemanticIds([...(args.semanticIds ?? []), ...(args.semantic_ids ?? [])]),
  };
}

function normalizeSectionRequest(args: IfcSectionAtPointOrStationArgs): NormalizedRequest {
  return {
    resource: String(args.resource ?? "").trim(),
    apiBase: args.apiBase ?? args.api_base,
    limit: normalizeLimit(args.limit),
    entityNames: [],
    semanticIds: parseSemanticIds([...(args.semanticIds ?? []), ...(args.semantic_ids ?? [])]),
  };
}

function normalizeAlignmentCatalogRequest(args: IfcAlignmentCatalogArgs): NormalizedRequest {
  return {
    resource: String(args.resource ?? "").trim(),
    apiBase: args.apiBase ?? args.api_base,
    limit: normalizeLimit(args.limit),
    entityNames: [],
    semanticIds: [],
  };
}

function normalizeStationResolveRequest(
  args: IfcStationResolveArgs,
): NormalizedStationRequest {
  return {
    resource: String(args.resource ?? "").trim(),
    apiBase: args.apiBase ?? args.api_base,
    limit: normalizeLimit(args.limit),
    entityNames: [],
    semanticIds: [],
    alignmentId: normalizeAlignmentId(args.alignmentId ?? args.alignment_id),
    station: normalizeStation(args.station),
    width: positiveNumber(args.width),
    height: positiveNumber(args.height),
    thickness: positiveNumber(args.thickness),
    clip: normalizeClip(args.clip),
  };
}

function normalizeAlignmentId(value: unknown): string | null {
  const text = String(value ?? "").trim();
  return text ? text : null;
}

function normalizeClip(value: unknown): string | null {
  const text = String(value ?? "").trim();
  if (!text) return null;
  const normalized = text.toLowerCase();
  if (
    normalized === "plane" ||
    normalized === "section-plane" ||
    normalized === "sectionplane" ||
    normalized === "overlay" ||
    normalized === "3d-overlay" ||
    normalized === "3doverlay"
  ) {
    return "none";
  }
  if (
    normalized === "slice" ||
    normalized === "section" ||
    normalized === "cross-section" ||
    normalized === "crosssection" ||
    normalized === "cross_section" ||
    normalized === "cut" ||
    normalized === "cut-plane" ||
    normalized === "cutplane" ||
    normalized === "positive" ||
    normalized === "clip-positive" ||
    normalized === "clippositivenormal"
  ) {
    return "clip-positive-normal";
  }
  if (
    normalized === "negative" ||
    normalized === "clip-negative" ||
    normalized === "clipnegativenormal"
  ) {
    return "clip-negative-normal";
  }
  return text;
}

function normalizeGroupBy(value: unknown): IfcQuantityTakeoffGroupBy {
  const normalized = String(value ?? "entity")
    .trim()
    .toLowerCase();
  if (
    normalized === "entity" ||
    normalized === "material" ||
    normalized === "bridge_part" ||
    normalized === "source_resource"
  ) {
    return normalized;
  }
  return "entity";
}

function normalizeSource(value: unknown): IfcQuantityTakeoffSource {
  const normalized = String(value ?? "count_only")
    .trim()
    .toLowerCase();
  if (
    normalized === "ifc_quantities" ||
    normalized === "geometry" ||
    normalized === "count_only"
  ) {
    return normalized;
  }
  return "count_only";
}

function normalizeLimit(value: unknown): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return DEFAULT_LIMIT;
  }
  return Math.max(1, Math.min(MAX_LIMIT, Math.trunc(numeric)));
}

function normalizeStation(value: unknown): number | string | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  const text = String(value ?? "").trim();
  return text ? text : null;
}

function normalizePoint(value: unknown): Record<string, number> | number[] | null {
  if (Array.isArray(value)) {
    const x = finiteNumber(value[0]);
    const y = finiteNumber(value[1]);
    const z = finiteNumber(value[2]);
    if (x === null || y === null) {
      return null;
    }
    return z === null ? [x, y] : [x, y, z];
  }
  if (value && typeof value === "object") {
    const point = value as Record<string, unknown>;
    const x = finiteNumber(point.x);
    const y = finiteNumber(point.y);
    const z = finiteNumber(point.z);
    if (x !== null && y !== null) {
      return z === null ? { x, y } : { x, y, z };
    }
  }
  return null;
}

function finiteNumber(value: unknown): number | null {
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

function positiveNumber(value: unknown): number | null {
  const numeric = finiteNumber(value);
  return numeric !== null && numeric > 0 ? numeric : null;
}

function parseSemanticIds(values: string[]): SemanticIdSpec[] {
  const seen = new Set<string>();
  const specs: SemanticIdSpec[] = [];
  for (const rawValue of values) {
    const raw = String(rawValue ?? "").trim();
    if (!raw) {
      continue;
    }
    const splitIndex = raw.indexOf("::");
    const sourceResource =
      splitIndex > 0 ? raw.slice(0, splitIndex).trim() || undefined : undefined;
    const localId = (splitIndex > 0 ? raw.slice(splitIndex + 2) : raw).trim();
    if (!localId) {
      continue;
    }
    const key = `${sourceResource ?? ""}::${localId}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    specs.push({ raw, localId, sourceResource });
  }
  return specs;
}

function semanticResourceFilter(specs: SemanticIdSpec[]): string[] | undefined {
  if (specs.length === 0 || specs.some((spec) => !spec.sourceResource)) {
    return undefined;
  }
  return uniqueStrings(specs.map((spec) => spec.sourceResource ?? ""));
}

function parseCypherResult(text: string): CypherResult {
  const payload = parseJson(text) as CypherPayload | null;
  const rawColumns = payload?.columns;
  const columns = Array.isArray(rawColumns)
    ? rawColumns.map((column) => String(column))
    : [];
  const rawRowsValue = payload ? payload.rows : undefined;
  const rawRows = Array.isArray(rawRowsValue) ? rawRowsValue : [];
  const rows = rawRows.map((row) => rowToObject(columns, row));
  const resourceErrors = payload?.resource_errors;
  return {
    text,
    payload,
    columns,
    rows,
    resourceErrors: Array.isArray(resourceErrors) ? resourceErrors : [],
  };
}

function rowToObject(columns: string[], row: unknown): Record<string, unknown> {
  if (row && typeof row === "object" && !Array.isArray(row)) {
    return row as Record<string, unknown>;
  }
  const tuple = Array.isArray(row) ? row : [];
  const object: Record<string, unknown> = {};
  columns.forEach((column, index) => {
    object[column] = tuple[index] ?? null;
  });
  return object;
}

function ensureSourceResourceRows(
  rows: Record<string, unknown>[],
  resource: string,
): Record<string, unknown>[] {
  return rows.map((row) =>
    row.source_resource === undefined ? { source_resource: resource, ...row } : row,
  );
}

function materialDiagnostics(source: IfcQuantityTakeoffSource): Diagnostic[] {
  return [
    diagnostic(
      "material_association_summary",
      "info",
      "Material rows summarize explicit graph associations through IfcRelAssociatesMaterial; they are counts of associated products, not measured material volumes or areas.",
    ),
    ...(source === "count_only"
      ? []
      : [
          diagnostic(
            "no_material_quantity_estimates",
            "info",
            "No material quantities were estimated from geometry.",
          ),
        ]),
  ];
}

function resourceErrorDiagnostics(result: CypherResult): Diagnostic[] {
  if (result.resourceErrors.length === 0) {
    return [];
  }
  return [
    diagnostic(
      "project_resource_partial_errors",
      "warning",
      "The project-wide query returned rows for at least one IFC resource, but one or more project members failed.",
      { details: { resource_errors: result.resourceErrors } },
    ),
  ];
}

function sectionOverlayDiagnostic(args: IfcSectionAtPointOrStationArgs): Diagnostic {
  return diagnostic(
    "section_geometry_overlay_not_implemented",
    "unsupported",
    [
      "Section geometry overlay is not implemented in this helper.",
      "The helper does not guess a section plane from station, point, orientation, width, or depth.",
    ].join(" "),
    {
      inputs_needed: SECTION_INPUTS_NEEDED,
      details: {
        station: normalizeStation(args.station),
        point: normalizePoint(args.point),
        orientation: args.orientation ?? null,
        width: finiteNumber(args.width),
        depth: finiteNumber(args.depth),
      },
    },
  );
}

function resolverAlignmentIdForRow(
  kind: string,
  row: Record<string, unknown>,
): string | null {
  if (
    kind === "linear_placement_station" ||
    kind === "referent_station"
  ) {
    const id = parseInteger(row.linear_placement_node_id);
    return id === null ? null : `linear_placement:${id}`;
  }
  if (kind === "alignment_root") {
    const curveId = parseInteger(row.curve_node_id);
    if (curveId !== null) {
      return `curve:${curveId}`;
    }
    const alignmentId = parseInteger(row.alignment_node_id);
    return alignmentId === null ? null : `alignment:${alignmentId}`;
  }
  if (kind === "alignment_curve_segment") {
    const id = parseInteger(row.curve_node_id);
    return id === null ? null : `curve:${id}`;
  }
  return null;
}

function parseResolverNodeId(value: string | null, prefix: string): number | null {
  if (!value) {
    return null;
  }
  const trimmed = value.trim();
  const prefixed = `${prefix}:`;
  if (trimmed.startsWith(prefixed)) {
    return parseInteger(trimmed.slice(prefixed.length));
  }
  return null;
}

function explicitSectionPoseFromRow(
  row: Record<string, unknown>,
): Record<string, number[]> | null {
  const origin = vector3Value(row.section_origin ?? row.SectionOrigin);
  const tangent = vector3Value(row.section_tangent ?? row.SectionTangent);
  const normal = vector3Value(row.section_normal ?? row.SectionNormal);
  const up = vector3Value(row.section_up ?? row.SectionUp);
  if (!origin || !tangent || !normal || !up) {
    return null;
  }
  if (
    vectorLength(tangent) <= Number.EPSILON ||
    vectorLength(normal) <= Number.EPSILON ||
    vectorLength(up) <= Number.EPSILON
  ) {
    return null;
  }
  return {
    origin,
    tangent,
    normal,
    up,
  };
}

function buildGradientCurveSegments(
  rows: Record<string, unknown>[],
  useExplicitStation: boolean,
): { segments: GradientCurveSegment[]; diagnostics: Diagnostic[]; usedClothoid: boolean } {
  const diagnostics: Diagnostic[] = [];
  const parsedRows = rows
    .map((row) => {
      const segmentId = parseInteger(row.segment_node_id);
      const segmentOrdinal = parseInteger(row.segment_ordinal);
      const startPoint = vector2Value(row.start_point ?? row.start_coordinates);
      const direction = normalizeVec2(vector2Value(row.direction ?? row.direction_ratios));
      const signedLength = finiteNumber(row.segment_length);
      if (
        segmentId === null ||
        segmentOrdinal === null ||
        !startPoint ||
        !direction ||
        signedLength === null
      ) {
        diagnostics.push(
          diagnostic(
            "invalid_gradient_curve_segment_row",
            "unsupported",
            "A curve segment row was missing explicit segment id, segment ordinal, start point, direction, or length.",
            { details: { row } },
          ),
        );
        return null;
      }
      return {
        segmentId,
        segmentOrdinal,
        startPoint,
        direction,
        signedLength,
        parentCurveEntity: textValue(row.parent_curve_entity),
        radius: finiteNumber(row.radius),
      };
    })
    .filter((row): row is NonNullable<typeof row> => row !== null)
    .sort(
      (left, right) =>
        left.segmentOrdinal - right.segmentOrdinal || left.segmentId - right.segmentId,
    );

  const duplicateOrdinal = parsedRows.find(
    (row, index) => index > 0 && row.segmentOrdinal === parsedRows[index - 1].segmentOrdinal,
  );
  if (duplicateOrdinal) {
    return {
      segments: [],
      diagnostics: [
        ...diagnostics,
        diagnostic(
          "duplicate_gradient_curve_segment_ordinal",
          "unsupported",
          "The IFC SEGMENTS list contains duplicate ordinals, so the alignment segment order cannot be resolved without guessing.",
          {
            details: {
              segment_ordinal: duplicateOrdinal.segmentOrdinal,
              segment_node_id: duplicateOrdinal.segmentId,
            },
          },
        ),
      ],
      usedClothoid: false,
    };
  }

  let cumulativeStation = 0;
  let usedClothoid = false;
  const segments: GradientCurveSegment[] = [];
  for (let index = 0; index < parsedRows.length; index += 1) {
    const row = parsedRows[index];
    const next = parsedRows[index + 1] ?? null;
    const length = Math.abs(row.signedLength);
    const segmentKind = gradientCurveSegmentKind(row, next, length);
    if (segmentKind.kind === "clothoid") {
      usedClothoid = true;
    }
    segments.push({
      segmentId: row.segmentId,
      segmentOrdinal: row.segmentOrdinal,
      startStation: useExplicitStation ? row.startPoint[0] : cumulativeStation,
      length,
      signedLength: row.signedLength,
      startPoint: row.startPoint,
      direction: row.direction,
      endPoint: next?.startPoint ?? null,
      endDirection: next?.direction ?? null,
      segmentKind,
    });
    cumulativeStation += length;
  }

  return { segments, diagnostics, usedClothoid };
}

function gradientCurveSegmentKind(
  row: {
    startPoint: Vec2;
    direction: Vec2;
    signedLength: number;
    parentCurveEntity: string | null;
    radius: number | null;
  },
  next: { startPoint: Vec2 } | null,
  length: number,
): GradientCurveSegmentKind {
  if (row.parentCurveEntity === "IfcCircle" && row.radius !== null && Math.abs(row.radius) > 1e-9) {
    return {
      kind: "circular",
      radius: Math.abs(row.radius),
      turnSign: chooseCircularSegmentTurnSign(
        row.startPoint,
        row.direction,
        Math.abs(row.radius),
        length,
        next?.startPoint ?? null,
        row.signedLength,
      ),
    };
  }
  if (row.parentCurveEntity === "IfcClothoid") {
    return { kind: "clothoid" };
  }
  return { kind: "line" };
}

function gradientCurveSegmentDiagnostics(
  label: string,
  result: { segments: GradientCurveSegment[]; diagnostics: Diagnostic[] },
): Diagnostic[] {
  if (result.segments.length === 0) {
    return [
      ...result.diagnostics,
      diagnostic(
        `missing_${label}_gradient_curve_segments`,
        "unsupported",
        `No explicit ${label} IfcGradientCurve segments were found for the requested curve.`,
      ),
    ];
  }
  return result.diagnostics;
}

function gradientCurveApproximationDiagnostics(
  horizontal: { usedClothoid: boolean },
  vertical: { usedClothoid: boolean },
): Diagnostic[] {
  if (!horizontal.usedClothoid && !vertical.usedClothoid) {
    return [];
  }
  return [
    diagnostic(
      "clothoid_segment_render_evaluation",
      "warning",
      "At least one IfcClothoid segment was evaluated with the same cubic Hermite approximation used by the renderer import path.",
    ),
  ];
}

function evaluateGradientCurveSegments(
  segments: GradientCurveSegment[],
  distanceAlong: number,
): { point: Vec2; tangent: Vec2 } | null {
  const first = segments[0];
  if (!first) {
    return null;
  }
  const last = segments[segments.length - 1] ?? first;
  const segment =
    segments.find(
      (entry) =>
        distanceAlong <= entry.startStation + entry.length || entry.length <= 1e-12,
    ) ?? last;
  const along =
    segment.length <= 1e-12
      ? 0
      : clamp(distanceAlong - segment.startStation, 0, segment.length);
  return evaluateGradientCurveSegment(segment, along);
}

function evaluateGradientCurveSegment(
  segment: GradientCurveSegment,
  along: number,
): { point: Vec2; tangent: Vec2 } {
  const startTangent = segment.direction;
  if (segment.segmentKind.kind === "circular") {
    return evaluateCircularSegment(
      segment.startPoint,
      startTangent,
      segment.segmentKind.radius,
      segment.segmentKind.turnSign,
      along,
    );
  }
  if (
    segment.segmentKind.kind === "clothoid" &&
    segment.endPoint &&
    segment.endDirection
  ) {
    return evaluateHermiteSegment(
      segment.startPoint,
      scaleVec2(startTangent, segment.length),
      segment.endPoint,
      scaleVec2(segment.endDirection, segment.length),
      segment.length <= 1e-12 ? 0 : clamp(along / segment.length, 0, 1),
    );
  }
  return {
    point: addVec2(segment.startPoint, scaleVec2(startTangent, along)),
    tangent: startTangent,
  };
}

function chooseCircularSegmentTurnSign(
  startPoint: Vec2,
  direction: Vec2,
  radius: number,
  length: number,
  nextStartPoint: Vec2 | null,
  signedLength: number,
): number {
  const fallback = signedLength < 0 ? -1 : 1;
  if (!nextStartPoint) {
    return fallback;
  }
  const leftPoint = circularSegmentPoint(startPoint, direction, radius, -1, length);
  const rightPoint = circularSegmentPoint(startPoint, direction, radius, 1, length);
  return distanceSquaredVec2(leftPoint, nextStartPoint) <=
    distanceSquaredVec2(rightPoint, nextStartPoint)
    ? -1
    : 1;
}

function evaluateCircularSegment(
  startPoint: Vec2,
  direction: Vec2,
  radius: number,
  turnSign: number,
  along: number,
): { point: Vec2; tangent: Vec2 } {
  if (radius <= 1e-9) {
    return { point: addVec2(startPoint, scaleVec2(direction, along)), tangent: direction };
  }
  const sign = turnSign < 0 ? -1 : 1;
  const leftNormal: Vec2 = [-direction[1], direction[0]];
  const center = addVec2(startPoint, scaleVec2(leftNormal, radius * sign));
  const radial = subVec2(startPoint, center);
  const angle = (sign * along) / radius;
  return {
    point: addVec2(center, rotateVec2(radial, angle)),
    tangent: normalizeVec2(rotateVec2(direction, angle)) ?? direction,
  };
}

function circularSegmentPoint(
  startPoint: Vec2,
  direction: Vec2,
  radius: number,
  turnSign: number,
  along: number,
): Vec2 {
  return evaluateCircularSegment(startPoint, direction, radius, turnSign, along).point;
}

function evaluateHermiteSegment(
  p0: Vec2,
  m0: Vec2,
  p1: Vec2,
  m1: Vec2,
  t: number,
): { point: Vec2; tangent: Vec2 } {
  const t2 = t * t;
  const t3 = t2 * t;
  const h00 = 2 * t3 - 3 * t2 + 1;
  const h10 = t3 - 2 * t2 + t;
  const h01 = -2 * t3 + 3 * t2;
  const h11 = t3 - t2;
  const point = addVec2(
    addVec2(scaleVec2(p0, h00), scaleVec2(m0, h10)),
    addVec2(scaleVec2(p1, h01), scaleVec2(m1, h11)),
  );

  const dh00 = 6 * t2 - 6 * t;
  const dh10 = 3 * t2 - 4 * t + 1;
  const dh01 = -6 * t2 + 6 * t;
  const dh11 = 3 * t2 - 2 * t;
  const tangent =
    normalizeVec2(
      addVec2(
        addVec2(scaleVec2(p0, dh00), scaleVec2(m0, dh10)),
        addVec2(scaleVec2(p1, dh01), scaleVec2(m1, dh11)),
      ),
    ) ?? normalizeVec2(m0) ?? [1, 0];
  return { point, tangent };
}

function stationRange(segments: GradientCurveSegment[]): [number, number] | null {
  const first = segments[0];
  const last = segments[segments.length - 1] ?? null;
  if (!first || !last) {
    return null;
  }
  return [first.startStation, last.startStation + last.length];
}

function stationIsInRange(station: number, range: [number, number] | null): boolean {
  if (!range) {
    return false;
  }
  const tolerance = 1e-9;
  return station >= range[0] - tolerance && station <= range[1] + tolerance;
}

function numericArrayValue(value: unknown): number[] | null {
  const raw = Array.isArray(value) ? value : typeof value === "string" ? parseJson(value) : null;
  if (!Array.isArray(raw)) {
    return null;
  }
  const numbers = raw.map((entry) => finiteNumber(entry));
  return numbers.some((entry) => entry === null) ? null : (numbers as number[]);
}

function vector2Value(value: unknown): Vec2 | null {
  const numeric = numericArrayValue(value);
  if (!numeric || numeric.length < 2) {
    return null;
  }
  return [numeric[0], numeric[1]];
}

function vector3Value(value: unknown): Vec3 | null {
  const numeric = numericArrayValue(value);
  if (!numeric || numeric.length < 3) {
    return null;
  }
  return [numeric[0], numeric[1], numeric[2]];
}

function normalizeVec2(vector: Vec2 | null): Vec2 | null {
  if (!vector) {
    return null;
  }
  const length = Math.hypot(vector[0], vector[1]);
  return length <= Number.EPSILON ? null : [vector[0] / length, vector[1] / length];
}

function normalizeVec3(vector: Vec3): Vec3 | null {
  const length = vectorLength(vector);
  return length <= Number.EPSILON
    ? null
    : [vector[0] / length, vector[1] / length, vector[2] / length];
}

function vectorLength(vector: number[]): number {
  return Math.hypot(vector[0] ?? 0, vector[1] ?? 0, vector[2] ?? 0);
}

function crossVec3(left: Vec3, right: Vec3): Vec3 {
  return [
    left[1] * right[2] - left[2] * right[1],
    left[2] * right[0] - left[0] * right[2],
    left[0] * right[1] - left[1] * right[0],
  ];
}

function addVec2(left: Vec2, right: Vec2): Vec2 {
  return [left[0] + right[0], left[1] + right[1]];
}

function subVec2(left: Vec2, right: Vec2): Vec2 {
  return [left[0] - right[0], left[1] - right[1]];
}

function scaleVec2(vector: Vec2, scale: number): Vec2 {
  return [vector[0] * scale, vector[1] * scale];
}

function rotateVec2(vector: Vec2, angle: number): Vec2 {
  const sin = Math.sin(angle);
  const cos = Math.cos(angle);
  return [vector[0] * cos - vector[1] * sin, vector[0] * sin + vector[1] * cos];
}

function distanceSquaredVec2(left: Vec2, right: Vec2): number {
  const dx = left[0] - right[0];
  const dy = left[1] - right[1];
  return dx * dx + dy * dy;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function addQuery(provenance: Record<string, unknown>, query: ProvenanceQuery): void {
  const queries = Array.isArray(provenance.queries)
    ? (provenance.queries as ProvenanceQuery[])
    : [];
  queries.push(query);
  provenance.queries = queries;
}

function diagnostic(
  code: string,
  severity: DiagnosticSeverity,
  message: string,
  extra: Partial<Diagnostic> = {},
): Diagnostic {
  return { code, severity, message, ...extra };
}

function jsonResult(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function parseJson(text: string): unknown | null {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function parseInteger(value: unknown): number | null {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function textValue(value: unknown): string | null {
  const text = String(value ?? "").trim();
  return text ? text : null;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const text = String(value ?? "").trim();
    if (!text || seen.has(text)) {
      continue;
    }
    seen.add(text);
    result.push(text);
  }
  return result;
}
