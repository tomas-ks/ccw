export type PostViewerJson = (
  apiBase: string | undefined,
  path: string,
  body: Record<string, unknown>,
) => Promise<string>;

export type IfcElementSearchInput = {
  resource: string;
  text?: string;
  keywords?: string | readonly string[];
  entityNames?: readonly string[];
  renderableOnly?: boolean;
  bridgePartNodeIds?: readonly number[];
  materialNames?: readonly string[];
  limit?: number;
  allMatches?: boolean;
  resourceFilter?: readonly string[];
  apiBase?: string;
};

export type IfcBridgeStructureSummaryInput = {
  resource: string;
  limit?: number;
  maxParts?: number;
  resourceFilter?: readonly string[];
  apiBase?: string;
};

type CypherResponse = {
  resource?: string;
  columns?: unknown;
  rows?: unknown;
  resourceErrors?: unknown;
  resource_errors?: unknown;
  ok?: unknown;
  error?: unknown;
  path?: unknown;
  status?: unknown;
};

type Diagnostic = {
  severity: "warning" | "error";
  query_key?: string | undefined;
  resource?: string | undefined;
  message: string;
};

type QueryPlan = {
  key: string;
  kind: string;
  cypher: string;
  resourceFilter?: string[];
  meta?: Record<string, unknown>;
};

type QueryResult = QueryPlan & {
  columns: string[];
  rows: Array<Record<string, string>>;
  raw_row_count: number;
};

type PartRecord = {
  key: string;
  source_resource?: string | undefined;
  bridge_key?: string | undefined;
  parent_part_key?: string | undefined;
  part_node_id: string;
  part_global_id?: string | undefined;
  part_declared_entity?: string | undefined;
  part_name?: string | undefined;
  part_object_type?: string | undefined;
  part_predefined_type?: string | undefined;
  contained_product_entity_counts: Array<{ entity: string; count: number | string }>;
  child_part_keys: string[];
  diagnostics: Diagnostic[];
};

type BridgeRecord = {
  key: string;
  source_resource?: string | undefined;
  bridge_node_id: string;
  bridge_global_id?: string | undefined;
  bridge_declared_entity?: string | undefined;
  bridge_name?: string | undefined;
  bridge_object_type?: string | undefined;
  bridge_predefined_type?: string | undefined;
  part_keys: string[];
};

const DEFAULT_ELEMENT_LIMIT = 24;
const MAX_ELEMENT_LIMIT = 80;
const MAX_ENTITY_QUERIES = 8;
const MAX_BRIDGE_PART_SEARCH_QUERIES = 10;
const MAX_MATERIAL_NAMES = 6;
const DEFAULT_BRIDGE_LIMIT = 24;
const DEFAULT_MAX_PARTS = 40;
const MAX_BRIDGE_PARTS = 120;

const DEFAULT_TEXT_SEARCH_LABELS = [
  "IfcElementAssembly",
  "IfcBuildingElementProxy",
  "IfcWall",
  "IfcSlab",
  "IfcBeam",
  "IfcColumn",
  "IfcSpace",
  "IfcFurniture",
];

const TEXT_PROPS = ["Name", "ObjectType", "Description", "Tag", "GlobalId"];

export function compactJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export async function ifcElementSearch(
  input: IfcElementSearchInput,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const resource = normalizeResource(input.resource);
  const resourceFilter = normalizeStringList(input.resourceFilter);
  const terms = normalizeSearchTerms(input.text, input.keywords);
  const entityLabels = normalizeEntityLabels(input.entityNames);
  const materialNames = normalizeStringList(input.materialNames).slice(0, MAX_MATERIAL_NAMES);
  const bridgePartNodeIds = normalizeNodeIds(input.bridgePartNodeIds).slice(
    0,
    MAX_BRIDGE_PART_SEARCH_QUERIES,
  );
  const allMatches = Boolean(input.allMatches);
  const hasFocusedAnchor =
    entityLabels.length > 0 || materialNames.length > 0 || bridgePartNodeIds.length > 0;
  const limit = allMatches ? null : clampInteger(input.limit, DEFAULT_ELEMENT_LIMIT, 1, MAX_ELEMENT_LIMIT);
  const diagnostics: Diagnostic[] = [];

  if (!resource) {
    return compactJson({
      ok: false,
      tool: "ifc_element_search",
      diagnostics: [
        {
          severity: "error",
          message: "resource is required",
        },
      ],
    });
  }

  if (allMatches && !hasFocusedAnchor) {
    return compactJson({
      ok: false,
      tool: "ifc_element_search",
      resource,
      resource_filter: resourceFilter,
      criteria: {
        text: input.text?.trim() || null,
        keywords: terms,
        entity_names: entityLabels,
        renderable_only: Boolean(input.renderableOnly),
        bridge_part_node_ids: bridgePartNodeIds,
        material_names: materialNames,
        all_matches: allMatches,
        limit,
      },
      queries: [],
      rows: [],
      diagnostics: [
        ...diagnostics,
        {
          severity: "error",
          message:
            "allMatches requires a focused anchor such as entityNames, bridgePartNodeIds, or materialNames; use a bounded preview query first for broad text search",
        },
      ],
    });
  }

  if (isProjectResource(resource) && bridgePartNodeIds.length > 0 && resourceFilter.length !== 1) {
    diagnostics.push({
      severity: "warning",
      message:
        "bridgePartNodeIds are local to each IFC resource; provide one resourceFilter entry to avoid matching the same node id in multiple project members",
    });
  }

  const queries = buildElementSearchQueries({
    terms,
    entityLabels,
    renderableOnly: Boolean(input.renderableOnly),
    bridgePartNodeIds,
    materialNames,
    limit,
    allMatches,
  }).map((query) => ({
    ...query,
    resourceFilter: query.resourceFilter ?? resourceFilter,
  }));

  if (queries.length === 0) {
    return compactJson({
      ok: false,
      tool: "ifc_element_search",
      resource,
      resource_filter: resourceFilter,
      criteria: {
        text: input.text?.trim() || null,
        keywords: terms,
        entity_names: entityLabels,
        renderable_only: Boolean(input.renderableOnly),
        bridge_part_node_ids: bridgePartNodeIds,
        material_names: materialNames,
        all_matches: allMatches,
        limit,
      },
      queries: [],
      rows: [],
      diagnostics: [
        ...diagnostics,
        {
          severity: "error",
          message:
            "no focused search query could be built; provide text, keywords, entityNames, bridgePartNodeIds, or materialNames",
        },
      ],
    });
  }

  const queryResults: QueryResult[] = [];
  for (const query of queries) {
    const result = await runCypherQuery(resource, input.apiBase, query, postViewerJson);
    diagnostics.push(...result.diagnostics);
    if (result.ok) {
      queryResults.push(result.result);
    }
  }

  const rows = queryResults.flatMap((result) =>
    result.rows.map((row) => ({
      query_key: result.key,
      query_kind: result.kind,
      ...row,
    })),
  );

  return compactJson({
    ok: diagnostics.every((diagnostic) => diagnostic.severity !== "error"),
    tool: "ifc_element_search",
    resource,
    resource_filter: resourceFilter,
    criteria: {
      text: input.text?.trim() || null,
      keywords: terms,
      entity_names: entityLabels,
      renderable_only: Boolean(input.renderableOnly),
      bridge_part_node_ids: bridgePartNodeIds,
      material_names: materialNames,
      all_matches: allMatches,
      limit,
    },
    query_count: queries.length,
    row_count: rows.length,
    rows,
    queries: queryResults.map((result) => ({
      key: result.key,
      kind: result.kind,
      meta: result.meta ?? {},
      resource_filter: result.resourceFilter ?? [],
      columns: result.columns,
      row_count: result.rows.length,
      rows: result.rows,
      cypher: result.cypher,
    })),
    diagnostics,
  });
}

export async function ifcBridgeStructureSummary(
  input: IfcBridgeStructureSummaryInput,
  postViewerJson: PostViewerJson,
): Promise<string> {
  const resource = normalizeResource(input.resource);
  const resourceFilter = normalizeStringList(input.resourceFilter);
  const limit = clampInteger(input.limit, DEFAULT_BRIDGE_LIMIT, 1, MAX_ELEMENT_LIMIT);
  const maxParts = clampInteger(input.maxParts, DEFAULT_MAX_PARTS, 1, MAX_BRIDGE_PARTS);
  const diagnostics: Diagnostic[] = [];

  if (!resource) {
    return compactJson({
      ok: false,
      tool: "ifc_bridge_structure_summary",
      diagnostics: [
        {
          severity: "error",
          message: "resource is required",
        },
      ],
    });
  }

  const bridgeQuery = buildBridgeRootQuery(limit);
  const bridgePartQuery = buildDirectBridgePartQuery(limit * 2);
  const rootResult = await runCypherQuery(
    resource,
    input.apiBase,
    { ...bridgeQuery, resourceFilter },
    postViewerJson,
  );
  diagnostics.push(...rootResult.diagnostics);

  const directPartResult = await runCypherQuery(
    resource,
    input.apiBase,
    { ...bridgePartQuery, resourceFilter },
    postViewerJson,
  );
  diagnostics.push(...directPartResult.diagnostics);

  const bridges = new Map<string, BridgeRecord>();
  const parts = new Map<string, PartRecord>();
  const partQueue: PartRecord[] = [];
  const inspectedPartKeys = new Set<string>();

  if (rootResult.ok) {
    for (const row of rootResult.result.rows) {
      upsertBridge(bridges, row);
    }
  }

  if (directPartResult.ok) {
    for (const row of directPartResult.result.rows) {
      const bridge = upsertBridge(bridges, row);
      const part = upsertPart(parts, row, bridge.key);
      appendUnique(bridge.part_keys, part.key);
      partQueue.push(part);
    }
  }

  while (partQueue.length > 0 && inspectedPartKeys.size < maxParts) {
    const part = partQueue.shift();
    if (!part || inspectedPartKeys.has(part.key)) {
      continue;
    }
    inspectedPartKeys.add(part.key);
    const partFilter = projectPartResourceFilter(resource, part.source_resource, resourceFilter);
    const countQuery = buildPartProductCountQuery(part.part_node_id);
    const childQuery = buildChildBridgePartQuery(part.part_node_id, limit);

    const countResult = await runCypherQuery(
      resource,
      input.apiBase,
      { ...countQuery, resourceFilter: partFilter },
      postViewerJson,
    );
    diagnostics.push(...countResult.diagnostics);
    part.diagnostics.push(...countResult.diagnostics.filter((item) => item.severity === "error"));
    if (countResult.ok) {
      part.contained_product_entity_counts = countResult.result.rows.map((row) => ({
        entity: row.entity || "",
        count: parseCount(row.entity_count),
      }));
    }

    const childResult = await runCypherQuery(
      resource,
      input.apiBase,
      { ...childQuery, resourceFilter: partFilter },
      postViewerJson,
    );
    diagnostics.push(...childResult.diagnostics);
    part.diagnostics.push(...childResult.diagnostics.filter((item) => item.severity === "error"));
    if (childResult.ok) {
      for (const row of childResult.result.rows) {
        const child = upsertPart(parts, row, part.bridge_key, part.key);
        appendUnique(part.child_part_keys, child.key);
        if (part.bridge_key) {
          const bridge = bridges.get(part.bridge_key);
          if (bridge) {
            appendUnique(bridge.part_keys, child.key);
          }
        }
        if (!inspectedPartKeys.has(child.key) && inspectedPartKeys.size + partQueue.length < maxParts) {
          partQueue.push(child);
        }
      }
    }
  }

  if (partQueue.length > 0) {
    diagnostics.push({
      severity: "warning",
      message: `stopped after inspecting ${maxParts} bridge parts; ${partQueue.length} queued part(s) were not queried`,
    });
  }

  const partList = Array.from(parts.values());
  const bridgeList = Array.from(bridges.values()).map((bridge) => ({
    ...bridge,
    parts: bridge.part_keys.map((key) => parts.get(key)).filter((part): part is PartRecord => Boolean(part)),
  }));

  return compactJson({
    ok: diagnostics.every((diagnostic) => diagnostic.severity !== "error"),
    tool: "ifc_bridge_structure_summary",
    resource,
    resource_filter: resourceFilter,
    bridge_count: bridgeList.length,
    part_count: partList.length,
    inspected_part_count: inspectedPartKeys.size,
    bridges: bridgeList,
    parts: partList,
    queries: [
      rootResult.ok ? summarizeQueryResult(rootResult.result) : summarizeQueryPlan(bridgeQuery),
      directPartResult.ok
        ? summarizeQueryResult(directPartResult.result)
        : summarizeQueryPlan(bridgePartQuery),
    ],
    diagnostics,
  });
}

export const ifc_element_search = ifcElementSearch;
export const ifc_bridge_structure_summary = ifcBridgeStructureSummary;

function buildElementSearchQueries(input: {
  terms: string[];
  entityLabels: string[];
  renderableOnly: boolean;
  bridgePartNodeIds: number[];
  materialNames: string[];
  limit: number | null;
  allMatches: boolean;
}): QueryPlan[] {
  if (
    input.terms.length === 0 &&
    input.entityLabels.length === 0 &&
    input.bridgePartNodeIds.length === 0 &&
    input.materialNames.length === 0
  ) {
    return [];
  }

  if (input.bridgePartNodeIds.length > 0) {
    return buildBridgePartElementQueries(input);
  }

  if (input.materialNames.length > 0) {
    return buildMaterialElementQueries(input);
  }

  const labels = input.entityLabels.length > 0 ? input.entityLabels : inferTextSearchLabels(input.terms);
  return labels.slice(0, MAX_ENTITY_QUERIES).map((label, index) => {
    const where = buildNodeWhere("n", input.terms, input.renderableOnly);
    return {
      key: `entity_${index + 1}_${label}`,
      kind: "entity_label",
      meta: { entity_label: label, all_matches: input.allMatches },
      cypher: [
        `MATCH (n:${label})`,
        where ? `WHERE ${where}` : null,
        buildElementReturnClause(),
        buildLimitClause(input.limit),
      ]
        .filter((line): line is string => Boolean(line))
        .join("\n"),
    };
  });
}

function buildBridgePartElementQueries(input: {
  terms: string[];
  entityLabels: string[];
  renderableOnly: boolean;
  bridgePartNodeIds: number[];
  materialNames: string[];
  limit: number | null;
  allMatches: boolean;
}): QueryPlan[] {
  const labels = input.entityLabels.length > 0 ? input.entityLabels.slice(0, MAX_ENTITY_QUERIES) : [""];
  const queries: QueryPlan[] = [];
  for (const partNodeId of input.bridgePartNodeIds) {
    for (const label of labels) {
      const alias = label ? `n:${label}` : "n";
      const whereParts = buildWhereParts("n", input.terms, input.renderableOnly);
      const materialWhere = parenthesize(buildMaterialWhere("material", input.materialNames));
      queries.push({
        key: `bridge_part_${partNodeId}_${label || "any"}`,
        kind: "bridge_part_containment",
        meta: {
          bridge_part_node_id: partNodeId,
          entity_label: label || null,
          material_names: input.materialNames,
          all_matches: input.allMatches,
        },
        cypher: [
          "MATCH (part:IfcBridgePart)",
          `WHERE id(part) = ${partNodeId}`,
          "MATCH (rel:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(part)",
          `MATCH (rel)-[:RELATED_ELEMENTS]->(${alias})`,
          input.materialNames.length > 0
            ? "MATCH (matrel:IfcRelAssociatesMaterial)-[:RELATED_OBJECTS]->(n)"
            : null,
          input.materialNames.length > 0
            ? "MATCH (matrel)-[:RELATING_MATERIAL]->(material:IfcMaterial)"
            : null,
          whereParts.length > 0 || materialWhere
            ? `WHERE ${[...whereParts, materialWhere].filter(Boolean).join(" AND ")}`
            : null,
          buildElementReturnClause([
            "id(part) AS bridge_part_node_id",
            "part.GlobalId AS bridge_part_global_id",
            "part.Name AS bridge_part_name",
            input.materialNames.length > 0 ? "material.Name AS material_name" : null,
          ]),
          buildLimitClause(input.limit),
        ]
          .filter((line): line is string => Boolean(line))
          .join("\n"),
      });
    }
  }
  return queries.slice(0, MAX_BRIDGE_PART_SEARCH_QUERIES);
}

function buildMaterialElementQueries(input: {
  terms: string[];
  entityLabels: string[];
  renderableOnly: boolean;
  materialNames: string[];
  limit: number | null;
  allMatches: boolean;
}): QueryPlan[] {
  const labels = input.entityLabels.length > 0 ? input.entityLabels.slice(0, MAX_ENTITY_QUERIES) : [""];
  return labels.map((label, index) => {
    const alias = label ? `n:${label}` : "n";
    const whereParts = buildWhereParts("n", input.terms, input.renderableOnly);
    const materialWhere = buildMaterialWhere("material", input.materialNames);
    return {
      key: `material_${index + 1}_${label || "any"}`,
      kind: "material_association",
      meta: {
        entity_label: label || null,
        material_names: input.materialNames,
        all_matches: input.allMatches,
      },
      cypher: [
        "MATCH (material:IfcMaterial)",
        materialWhere ? `WHERE ${materialWhere}` : null,
        "MATCH (rel:IfcRelAssociatesMaterial)-[:RELATING_MATERIAL]->(material)",
        `MATCH (rel)-[:RELATED_OBJECTS]->(${alias})`,
        whereParts.length > 0 ? `WHERE ${whereParts.join(" AND ")}` : null,
        buildElementReturnClause(["material.Name AS material_name"]),
        buildLimitClause(input.limit),
      ]
        .filter((line): line is string => Boolean(line))
        .join("\n"),
    };
  });
}

function buildBridgeRootQuery(limit: number): QueryPlan {
  return {
    key: "bridge_roots",
    kind: "bridge_roots",
    cypher: [
      "MATCH (bridge:IfcBridge)",
      [
        "RETURN DISTINCT",
        "id(bridge) AS bridge_node_id,",
        "bridge.GlobalId AS bridge_global_id,",
        "bridge.declared_entity AS bridge_declared_entity,",
        "bridge.Name AS bridge_name,",
        "bridge.ObjectType AS bridge_object_type,",
        "bridge.PredefinedType AS bridge_predefined_type",
      ].join(" "),
      `LIMIT ${limit}`,
    ].join("\n"),
  };
}

function buildDirectBridgePartQuery(limit: number): QueryPlan {
  return {
    key: "direct_bridge_parts",
    kind: "direct_bridge_parts",
    cypher: [
      "MATCH (rel:IfcRelAggregates)-[:RELATING_OBJECT]->(bridge:IfcBridge)",
      "MATCH (rel)-[:RELATED_OBJECTS]->(part:IfcBridgePart)",
      [
        "RETURN DISTINCT",
        "id(bridge) AS bridge_node_id,",
        "bridge.GlobalId AS bridge_global_id,",
        "bridge.declared_entity AS bridge_declared_entity,",
        "bridge.Name AS bridge_name,",
        "bridge.ObjectType AS bridge_object_type,",
        "bridge.PredefinedType AS bridge_predefined_type,",
        "id(part) AS part_node_id,",
        "part.GlobalId AS part_global_id,",
        "part.declared_entity AS part_declared_entity,",
        "part.Name AS part_name,",
        "part.ObjectType AS part_object_type,",
        "part.PredefinedType AS part_predefined_type",
      ].join(" "),
      `LIMIT ${limit}`,
    ].join("\n"),
  };
}

function buildPartProductCountQuery(partNodeId: string): QueryPlan {
  return {
    key: `part_${partNodeId}_contained_counts`,
    kind: "part_contained_product_counts",
    meta: { part_node_id: partNodeId },
    cypher: [
      "MATCH (part:IfcBridgePart)",
      `WHERE id(part) = ${safeIntegerLiteral(partNodeId)}`,
      "MATCH (rel:IfcRelContainedInSpatialStructure)-[:RELATING_STRUCTURE]->(part)",
      "MATCH (rel)-[:RELATED_ELEMENTS]->(prod)",
      "WHERE prod.declared_entity IS NOT NULL",
      "RETURN prod.declared_entity AS entity, count(*) AS entity_count",
      "ORDER BY entity_count DESC",
      "LIMIT 32",
    ].join("\n"),
  };
}

function buildChildBridgePartQuery(partNodeId: string, limit: number): QueryPlan {
  return {
    key: `part_${partNodeId}_child_parts`,
    kind: "child_bridge_parts",
    meta: { part_node_id: partNodeId },
    cypher: [
      "MATCH (part:IfcBridgePart)",
      `WHERE id(part) = ${safeIntegerLiteral(partNodeId)}`,
      "MATCH (rel:IfcRelAggregates)-[:RELATING_OBJECT]->(part)",
      "MATCH (rel)-[:RELATED_OBJECTS]->(child:IfcBridgePart)",
      [
        "RETURN DISTINCT",
        "id(child) AS part_node_id,",
        "child.GlobalId AS part_global_id,",
        "child.declared_entity AS part_declared_entity,",
        "child.Name AS part_name,",
        "child.ObjectType AS part_object_type,",
        "child.PredefinedType AS part_predefined_type",
      ].join(" "),
      `LIMIT ${limit}`,
    ].join("\n"),
  };
}

function buildElementReturnClause(extraColumns: Array<string | null> = []): string {
  const columns = [
    "id(n) AS node_id",
    "n.GlobalId AS global_id",
    "n.declared_entity AS declared_entity",
    "n.Name AS name",
    "n.ObjectType AS object_type",
    "n.PredefinedType AS predefined_type",
    "n.Description AS description",
    "n.Tag AS tag",
    ...extraColumns.filter((column): column is string => Boolean(column)),
  ];
  return `RETURN DISTINCT ${columns.join(", ")}`;
}

function buildLimitClause(limit: number | null): string | null {
  return limit === null ? null : `LIMIT ${limit}`;
}

function buildNodeWhere(alias: string, terms: string[], renderableOnly: boolean): string {
  return buildWhereParts(alias, terms, renderableOnly).join(" AND ");
}

function buildWhereParts(alias: string, terms: string[], renderableOnly: boolean): string[] {
  const parts: string[] = [];
  const textWhere = buildTextWhere(alias, terms);
  if (textWhere) {
    parts.push(`(${textWhere})`);
  }
  if (renderableOnly) {
    parts.push(`${alias}.GlobalId IS NOT NULL`);
  }
  return parts;
}

function buildTextWhere(alias: string, terms: string[]): string {
  const predicates: string[] = [];
  for (const term of expandTermVariants(terms)) {
    const literal = cypherString(term);
    for (const prop of TEXT_PROPS) {
      predicates.push(`${alias}.${prop} CONTAINS ${literal}`);
    }
  }
  return predicates.join(" OR ");
}

function buildMaterialWhere(alias: string, materialNames: readonly string[]): string {
  return expandTermVariants(materialNames)
    .map((name) => `${alias}.Name CONTAINS ${cypherString(name)}`)
    .join(" OR ");
}

function parenthesize(value: string): string {
  return value ? `(${value})` : "";
}

function inferTextSearchLabels(terms: string[]): string[] {
  const labels = new Set<string>();
  const lowered = terms.join(" ").toLowerCase();
  if (/\bbridge|rail|road|girder|arch|pier|abutment|foundation|footing\b/.test(lowered)) {
    [
      "IfcBridge",
      "IfcBridgePart",
      "IfcElementAssembly",
      "IfcSlab",
      "IfcBeam",
      "IfcColumn",
      "IfcFooting",
    ].forEach((label) => labels.add(label));
  }
  if (/\bmanhole|sewer|accessory\b/.test(lowered)) {
    ["IfcElementAssembly", "IfcElementAssemblyType"].forEach((label) => labels.add(label));
  }
  if (/\broof\b/.test(lowered)) {
    ["IfcRoof", "IfcSlab"].forEach((label) => labels.add(label));
  }
  if (/\bwall\b/.test(lowered)) {
    labels.add("IfcWall");
  }
  if (/\bspace|room|storey|story\b/.test(lowered)) {
    ["IfcSpace", "IfcBuildingStorey"].forEach((label) => labels.add(label));
  }
  for (const label of DEFAULT_TEXT_SEARCH_LABELS) {
    labels.add(label);
  }
  return Array.from(labels).slice(0, MAX_ENTITY_QUERIES);
}

async function runCypherQuery(
  resource: string,
  apiBase: string | undefined,
  query: QueryPlan,
  postViewerJson: PostViewerJson,
): Promise<
  | { ok: true; result: QueryResult; diagnostics: Diagnostic[] }
  | { ok: false; diagnostics: Diagnostic[] }
> {
  const body: Record<string, unknown> = {
    resource,
    cypher: query.cypher,
  };
  if (isProjectResource(resource) && query.resourceFilter && query.resourceFilter.length > 0) {
    body.resourceFilter = query.resourceFilter;
  }

  const responseText = await postViewerJson(apiBase, "/api/cypher", body);
  const parsed = parseJsonObject(responseText);
  if (!parsed) {
    return {
      ok: false,
      diagnostics: [
        {
          severity: "error",
          query_key: query.key,
          message: `query returned non-JSON response: ${responseText.slice(0, 400)}`,
        },
      ],
    };
  }

  if (parsed.ok === false || parsed.error !== undefined) {
    return {
      ok: false,
      diagnostics: [
        {
          severity: "error",
          query_key: query.key,
          message: stringifyUnknown(parsed.error ?? parsed),
        },
      ],
    };
  }

  const columns = stringArray(parsed.columns);
  const rows = rowArray(parsed.rows);
  if (!columns || !rows) {
    return {
      ok: false,
      diagnostics: [
        {
          severity: "error",
          query_key: query.key,
          message: "query response did not include columns and rows arrays",
        },
      ],
    };
  }

  const diagnostics = normalizeResourceErrors(parsed).map((error) => ({
    severity: "error" as const,
    query_key: query.key,
    resource: error.resource,
    message: error.error,
  }));

  return {
    ok: true,
    diagnostics,
    result: {
      ...query,
      columns,
      rows: rows.map((row) => rowToRecord(columns, row)),
      raw_row_count: rows.length,
    },
  };
}

function summarizeQueryResult(result: QueryResult): Record<string, unknown> {
  return {
    key: result.key,
    kind: result.kind,
    resource_filter: result.resourceFilter ?? [],
    columns: result.columns,
    row_count: result.rows.length,
    cypher: result.cypher,
  };
}

function summarizeQueryPlan(query: QueryPlan): Record<string, unknown> {
  return {
    key: query.key,
    kind: query.kind,
    resource_filter: query.resourceFilter ?? [],
    cypher: query.cypher,
  };
}

function upsertBridge(
  bridges: Map<string, BridgeRecord>,
  row: Record<string, string>,
): BridgeRecord {
  const source = row.source_resource || undefined;
  const bridgeNodeId = row.bridge_node_id || "";
  const key = scopedKey(source, bridgeNodeId);
  const existing = bridges.get(key);
  if (existing) {
    return existing;
  }
  const bridge: BridgeRecord = {
    key,
    source_resource: source,
    bridge_node_id: bridgeNodeId,
    bridge_global_id: blankToUndefined(row.bridge_global_id),
    bridge_declared_entity: blankToUndefined(row.bridge_declared_entity),
    bridge_name: blankToUndefined(row.bridge_name),
    bridge_object_type: blankToUndefined(row.bridge_object_type),
    bridge_predefined_type: blankToUndefined(row.bridge_predefined_type),
    part_keys: [],
  };
  bridges.set(key, bridge);
  return bridge;
}

function upsertPart(
  parts: Map<string, PartRecord>,
  row: Record<string, string>,
  bridgeKey?: string,
  parentPartKey?: string,
): PartRecord {
  const source = row.source_resource || undefined;
  const partNodeId = row.part_node_id || "";
  const key = scopedKey(source, partNodeId);
  const existing = parts.get(key);
  if (existing) {
    if (bridgeKey && !existing.bridge_key) {
      existing.bridge_key = bridgeKey;
    }
    if (parentPartKey && !existing.parent_part_key) {
      existing.parent_part_key = parentPartKey;
    }
    return existing;
  }
  const part: PartRecord = {
    key,
    source_resource: source,
    bridge_key: bridgeKey,
    parent_part_key: parentPartKey,
    part_node_id: partNodeId,
    part_global_id: blankToUndefined(row.part_global_id),
    part_declared_entity: blankToUndefined(row.part_declared_entity),
    part_name: blankToUndefined(row.part_name),
    part_object_type: blankToUndefined(row.part_object_type),
    part_predefined_type: blankToUndefined(row.part_predefined_type),
    contained_product_entity_counts: [],
    child_part_keys: [],
    diagnostics: [],
  };
  parts.set(key, part);
  return part;
}

function projectPartResourceFilter(
  resource: string,
  sourceResource: string | undefined,
  fallbackFilter: string[],
): string[] {
  if (!isProjectResource(resource)) {
    return [];
  }
  if (sourceResource) {
    return [sourceResource];
  }
  return fallbackFilter;
}

function rowToRecord(columns: string[], row: string[]): Record<string, string> {
  const record: Record<string, string> = {};
  columns.forEach((column, index) => {
    record[column] = row[index] ?? "";
  });
  return record;
}

function normalizeResource(value: string): string {
  return String(value ?? "").trim();
}

function isProjectResource(resource: string): boolean {
  return resource.trim().startsWith("project/");
}

function normalizeSearchTerms(text: string | undefined, keywords: string | readonly string[] | undefined): string[] {
  const values: string[] = [];
  if (text) {
    values.push(...splitSearchText(text));
  }
  if (typeof keywords === "string") {
    values.push(...splitSearchText(keywords));
  } else if (Array.isArray(keywords)) {
    for (const keyword of keywords) {
      values.push(...splitSearchText(keyword));
    }
  }
  return uniqueStrings(values).slice(0, 8);
}

function splitSearchText(value: string): string[] {
  const trimmed = value.trim();
  if (!trimmed) {
    return [];
  }
  const pieces = trimmed
    .split(/[,\n]/)
    .map((piece) => piece.trim())
    .filter(Boolean);
  if (pieces.length > 1) {
    return pieces;
  }
  const words = trimmed
    .split(/\s+/)
    .map((piece) => piece.trim())
    .filter((piece) => piece.length > 2);
  return uniqueStrings([trimmed, ...words]);
}

function normalizeStringList(values: readonly string[] | undefined): string[] {
  if (!Array.isArray(values)) {
    return [];
  }
  return uniqueStrings(values.map((value) => String(value ?? "").trim()).filter(Boolean));
}

function normalizeNodeIds(values: readonly number[] | undefined): number[] {
  if (!Array.isArray(values)) {
    return [];
  }
  const ids = values
    .map((value) => Math.trunc(Number(value)))
    .filter((value) => Number.isFinite(value) && value >= 0);
  return Array.from(new Set(ids));
}

function normalizeEntityLabels(values: readonly string[] | undefined): string[] {
  return normalizeStringList(values)
    .map((value) => {
      const compact = value.replace(/[^A-Za-z0-9_]/g, "");
      if (!compact) {
        return "";
      }
      return compact.startsWith("Ifc") ? compact : `Ifc${compact}`;
    })
    .filter((value) => /^Ifc[A-Za-z0-9_]*$/.test(value))
    .slice(0, MAX_ENTITY_QUERIES);
}

function uniqueStrings(values: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) {
      continue;
    }
    seen.add(trimmed);
    result.push(trimmed);
  }
  return result;
}

function expandTermVariants(values: readonly string[]): string[] {
  const variants: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed) {
      continue;
    }
    variants.push(trimmed);
    variants.push(trimmed.toLowerCase());
    variants.push(capitalizeWords(trimmed.toLowerCase()));
    variants.push(trimmed.toUpperCase());
  }
  return uniqueStrings(variants).slice(0, 16);
}

function capitalizeWords(value: string): string {
  return value.replace(/\b[a-z]/g, (match) => match.toUpperCase());
}

function cypherString(value: string): string {
  return `'${value
    .replace(/\\/g, "\\\\")
    .replace(/'/g, "\\'")
    .replace(/\r/g, "\\r")
    .replace(/\n/g, "\\n")}'`;
}

function safeIntegerLiteral(value: string): string {
  const parsed = Math.trunc(Number(value));
  if (!Number.isFinite(parsed) || parsed < 0) {
    return "0";
  }
  return String(parsed);
}

function clampInteger(
  value: number | undefined,
  fallback: number,
  minimum: number,
  maximum: number,
): number {
  const parsed = Math.trunc(Number(value ?? fallback));
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return Math.min(Math.max(parsed, minimum), maximum);
}

function parseJsonObject(text: string): CypherResponse | null {
  try {
    const parsed = JSON.parse(text);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as CypherResponse)
      : null;
  } catch {
    return null;
  }
}

function stringArray(value: unknown): string[] | null {
  if (!Array.isArray(value)) {
    return null;
  }
  return value.map((entry) => String(entry ?? ""));
}

function rowArray(value: unknown): string[][] | null {
  if (!Array.isArray(value)) {
    return null;
  }
  const rows: string[][] = [];
  for (const row of value) {
    if (!Array.isArray(row)) {
      return null;
    }
    rows.push(row.map((entry) => String(entry ?? "")));
  }
  return rows;
}

function normalizeResourceErrors(payload: CypherResponse): Array<{
  resource: string | undefined;
  error: string;
}> {
  const raw = payload.resourceErrors ?? payload.resource_errors;
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw
    .map((entry) => {
      if (!entry || typeof entry !== "object") {
        return null;
      }
      const record = entry as Record<string, unknown>;
      const error = stringifyUnknown(record.error ?? record.message ?? "");
      if (!error) {
        return null;
      }
      return {
        resource: record.resource ? String(record.resource) : undefined,
        error,
      };
    })
    .filter((entry): entry is { resource: string | undefined; error: string } =>
      Boolean(entry),
    );
}

function stringifyUnknown(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (value === undefined || value === null) {
    return "";
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function parseCount(value: string | undefined): number | string {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) ? parsed : value ?? "";
}

function appendUnique(values: string[], value: string): void {
  if (!value || values.includes(value)) {
    return;
  }
  values.push(value);
}

function scopedKey(sourceResource: string | undefined, nodeId: string): string {
  return sourceResource ? `${sourceResource}::${nodeId}` : nodeId;
}

function blankToUndefined(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
