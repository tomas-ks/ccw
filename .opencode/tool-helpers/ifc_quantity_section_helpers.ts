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

const DEFAULT_LIMIT = 50;
const MAX_LIMIT = 500;
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

export const executeIfcQuantityTakeoff = ifcQuantityTakeoff;
export const executeIfcSectionAtPointOrStation = ifcSectionAtPointOrStation;

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
