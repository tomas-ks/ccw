export type IfcScopePostViewerJson = (
  apiBase: string | undefined,
  path: string,
  body: Record<string, unknown>,
) => Promise<string>;

export type IfcInspectionMode = "replace" | "add" | "remove";

export type IfcScopeSummaryArgs = {
  resource?: string;
  semanticIds?: string[];
  dbNodeIds?: number[];
  includeMaterials?: boolean;
  includeGeometry?: boolean;
  limit?: number;
  apiBase?: string;
  postViewerJson: IfcScopePostViewerJson;
};

export type IfcScopeInspectArgs = {
  resource?: string;
  semanticIds?: string[];
  mode?: string;
  select?: boolean;
  includeSelect?: boolean;
  frameVisible?: boolean;
  frame?: boolean;
  why?: string;
};

type CypherResponse = {
  resource?: string;
  columns?: string[];
  rows?: unknown[][];
  semanticElementIds?: string[];
  resourceErrors?: Array<{ resource?: string; error?: string }>;
  ok?: boolean;
  error?: unknown;
};

type GeometryCatalogResponse = {
  resource?: string;
  catalog?: {
    definitions?: GeometryDefinitionEntry[];
    elements?: GeometryElementEntry[];
    instances?: GeometryInstanceEntry[];
  };
  ok?: boolean;
  error?: unknown;
};

type GeometryDefinitionEntry = {
  id?: number | string;
  vertex_count?: number;
  vertexCount?: number;
  triangle_count?: number;
  triangleCount?: number;
  bounds_min?: number[];
  boundsMin?: number[];
  bounds_max?: number[];
  boundsMax?: number[];
};

type GeometryElementEntry = {
  id?: string;
  label?: string;
  declared_entity?: string;
  declaredEntity?: string;
  default_render_class?: string;
  defaultRenderClass?: string;
  bounds_min?: number[];
  boundsMin?: number[];
  bounds_max?: number[];
  boundsMax?: number[];
};

type GeometryInstanceEntry = {
  id?: number | string;
  element_id?: string;
  elementId?: string;
  definition_id?: number | string;
  definitionId?: number | string;
  external_id?: string;
  externalId?: string;
  label?: string;
};

type ScopedSemanticId = {
  original: string;
  globalId: string;
  sourceResource?: string;
};

type IdentityFact = {
  resource: string;
  nodeId: string;
  globalId: string;
  semanticId: string;
  entity: string;
  name: string;
  objectType: string;
  predefinedType: string;
};

type MaterialFact = {
  resource: string;
  nodeId: string;
  globalId: string;
  semanticId: string;
  materialEntity: string;
  materialName: string;
  materialCategory: string;
};

type CypherRecord = Record<string, string>;

const DEFAULT_LIMIT = 40;
const MAX_LIMIT = 200;

export async function ifcScopeSummary(args: IfcScopeSummaryArgs): Promise<string> {
  const diagnostics: string[] = [];
  const resource = cleanText(args.resource);
  const limit = normalizeLimit(args.limit);
  const semanticIds = uniqueStrings(args.semanticIds ?? []);
  const dbNodeIds = uniqueDbNodeIds(args.dbNodeIds ?? []);

  if ((args.dbNodeIds?.length ?? 0) !== dbNodeIds.length) {
    diagnostics.push("Ignored duplicate, non-finite, or negative dbNodeIds.");
  }

  if ((args.semanticIds?.length ?? 0) !== semanticIds.length) {
    diagnostics.push("Ignored duplicate or blank semanticIds.");
  }

  if (!resource && dbNodeIds.length > 0) {
    diagnostics.push("dbNodeIds require a concrete IFC resource; no dbNodeIds were queried.");
  }

  if (!resource && semanticIds.length === 0) {
    return asJson({
      ok: false,
      error: "ifc_scope_summary requires resource, source-scoped semanticIds, or dbNodeIds with resource.",
      diagnostics,
    });
  }

  if (semanticIds.length === 0 && dbNodeIds.length === 0) {
    return asJson({
      ok: false,
      resource: resource || null,
      error: "ifc_scope_summary requires at least one semanticId or dbNodeId.",
      diagnostics,
    });
  }

  const scopedSemanticIds = semanticIds.map(parseScopedSemanticId);
  const unscopedSemanticIds = scopedSemanticIds.filter((id) => !id.sourceResource);
  const scopedOnly =
    dbNodeIds.length === 0 &&
    scopedSemanticIds.length > 0 &&
    scopedSemanticIds.every((id) => Boolean(id.sourceResource));

  if (scopedOnly) {
    diagnostics.push("Using source resources from source-scoped semanticIds.");
  }

  if (unscopedSemanticIds.length > 0 && !resource) {
    diagnostics.push("Unscoped semanticIds require resource; those ids were not queried.");
  }

  const identityFacts: IdentityFact[] = [];
  const materialFacts: MaterialFact[] = [];
  const sourceGroups = groupSemanticIdsByResource(scopedSemanticIds, resource);
  const queries: Array<Record<string, unknown>> = [];

  for (const group of sourceGroups) {
    if (!group.resource) {
      continue;
    }
    const ids = group.ids.slice(0, limit);
    if (group.ids.length > ids.length) {
      diagnostics.push(
        `Truncated semanticId query for ${group.resource} from ${group.ids.length} to ${ids.length} ids.`,
      );
    }
    const identityQuery = semanticIdentityQuery(ids.map((id) => id.globalId), limit);
    queries.push({ resource: group.resource, kind: "semantic_identity", count: ids.length });
    const identity = await runCypher(
      args.postViewerJson,
      args.apiBase,
      group.resource,
      identityQuery,
      "summarize explicitly requested IFC semantic ids",
      diagnostics,
    );
    const records = cypherRecords(identity);
    const scopedByGlobalId = new Map(ids.map((id) => [id.globalId, id]));
    identityFacts.push(...records.map((row) => identityFactFromRecord(group.resource, row, scopedByGlobalId)));
    addMissingSemanticDiagnostics(group.resource, ids, records, diagnostics);

    if (args.includeMaterials) {
      const materialQuery = semanticMaterialQuery(ids.map((id) => id.globalId), limit);
      queries.push({ resource: group.resource, kind: "semantic_material", count: ids.length });
      const materials = await runCypher(
        args.postViewerJson,
        args.apiBase,
        group.resource,
        materialQuery,
        "summarize material associations for explicitly requested IFC semantic ids",
        diagnostics,
      );
      materialFacts.push(
        ...cypherRecords(materials).map((row) =>
          materialFactFromRecord(group.resource, row, scopedByGlobalId),
        ),
      );
    }
  }

  if (dbNodeIds.length > 0 && resource) {
    if (isProjectResource(resource)) {
      diagnostics.push(
        "Skipped dbNodeIds because project resources cannot safely disambiguate per-model database node ids.",
      );
    } else {
      const ids = dbNodeIds.slice(0, limit);
      if (dbNodeIds.length > ids.length) {
        diagnostics.push(`Truncated dbNodeId query from ${dbNodeIds.length} to ${ids.length} ids.`);
      }
      const identityQuery = dbIdentityQuery(ids, limit);
      queries.push({ resource, kind: "db_identity", count: ids.length });
      const identity = await runCypher(
        args.postViewerJson,
        args.apiBase,
        resource,
        identityQuery,
        "summarize explicitly requested IFC database node ids",
        diagnostics,
      );
      const records = cypherRecords(identity);
      identityFacts.push(...records.map((row) => identityFactFromRecord(resource, row)));
      addMissingDbNodeDiagnostics(resource, ids, records, diagnostics);

      if (args.includeMaterials) {
        const materialQuery = dbMaterialQuery(ids, limit);
        queries.push({ resource, kind: "db_material", count: ids.length });
        const materials = await runCypher(
          args.postViewerJson,
          args.apiBase,
          resource,
          materialQuery,
          "summarize material associations for explicitly requested IFC database node ids",
          diagnostics,
        );
        materialFacts.push(
          ...cypherRecords(materials).map((row) => materialFactFromRecord(resource, row)),
        );
      }
    }
  }

  if (args.includeMaterials && materialFacts.length === 0) {
    diagnostics.push(
      "No direct IfcRelAssociatesMaterial facts were returned for the requested ids; no material was inferred.",
    );
  }

  const geometry = args.includeGeometry
    ? await summarizeGeometry(
        args.postViewerJson,
        args.apiBase,
        resource,
        sourceGroups,
        identityFacts,
        limit,
        diagnostics,
      )
    : null;

  return asJson({
    ok: true,
    resource: resource || null,
    requested: {
      semanticIds,
      dbNodeIds,
      includeMaterials: Boolean(args.includeMaterials),
      includeGeometry: Boolean(args.includeGeometry),
      limit,
    },
    counts: {
      identityFacts: identityFacts.length,
      materialFacts: materialFacts.length,
    },
    summaries: {
      byEntity: summarize(identityFacts.map((fact) => fact.entity)),
      byName: summarize(identityFacts.map((fact) => fact.name)),
      byObjectType: summarize(identityFacts.map((fact) => fact.objectType)),
      byMaterial: summarize(materialFacts.map((fact) => fact.materialName || fact.materialEntity)),
    },
    facts: identityFacts,
    materialFacts: args.includeMaterials ? materialFacts : undefined,
    geometry,
    diagnostics,
    queries,
  });
}

export function ifcScopeInspect(args: IfcScopeInspectArgs): string {
  const semanticIds = uniqueStrings(args.semanticIds ?? []);
  const mode = normalizeInspectionMode(args.mode);
  const reason = cleanText(args.why);
  const count = semanticIds.length;
  const diagnostics: string[] = [];

  if (count === 0) {
    return [
      "Prepared elements.inspect replace for 0 elements.",
      "Diagnostic: ifc_scope_inspect requires at least one renderable semantic id; no viewer action should be emitted.",
    ].join("\n");
  }

  if (args.mode && !modeMatches(args.mode, mode)) {
    diagnostics.push(`Unsupported inspection mode \`${args.mode}\`; using \`${mode}\`.`);
  }

  const lines = [
    `Prepared elements.inspect ${mode} for ${count} element${count === 1 ? "" : "s"}.`,
  ];
  if (args.select || args.includeSelect) {
    lines.push(`Prepared elements.select for ${count} element${count === 1 ? "" : "s"}.`);
  }
  if (args.frameVisible || args.frame) {
    lines.push("Prepared viewer.frame_visible.");
  }
  if (reason) {
    lines.push(`Why: ${reason}`);
  }
  lines.push(...diagnostics.map((diagnostic) => `Diagnostic: ${diagnostic}`));
  return lines.join("\n");
}

export const ifc_scope_summary = ifcScopeSummary;
export const ifc_scope_inspect = ifcScopeInspect;

function semanticIdentityQuery(globalIds: string[], limit: number): string {
  return [
    "MATCH (n)",
    `WHERE n.GlobalId IN ${cypherStringList(globalIds)}`,
    "RETURN id(n) AS node_id, n.GlobalId AS global_id, n.declared_entity AS entity, n.Name AS name, n.ObjectType AS object_type, n.PredefinedType AS predefined_type",
    `LIMIT ${limit}`,
  ].join("\n");
}

function dbIdentityQuery(dbNodeIds: number[], limit: number): string {
  return [
    "MATCH (n)",
    `WHERE id(n) IN ${cypherNumberList(dbNodeIds)}`,
    "RETURN id(n) AS node_id, n.GlobalId AS global_id, n.declared_entity AS entity, n.Name AS name, n.ObjectType AS object_type, n.PredefinedType AS predefined_type",
    `LIMIT ${limit}`,
  ].join("\n");
}

function semanticMaterialQuery(globalIds: string[], limit: number): string {
  return [
    "MATCH (n)--(:IfcRelAssociatesMaterial)--(material:IfcMaterial)",
    `WHERE n.GlobalId IN ${cypherStringList(globalIds)}`,
    "RETURN id(n) AS node_id, n.GlobalId AS global_id, material.declared_entity AS material_entity, material.Name AS material_name, material.Category AS material_category",
    `LIMIT ${limit}`,
  ].join("\n");
}

function dbMaterialQuery(dbNodeIds: number[], limit: number): string {
  return [
    "MATCH (n)--(:IfcRelAssociatesMaterial)--(material:IfcMaterial)",
    `WHERE id(n) IN ${cypherNumberList(dbNodeIds)}`,
    "RETURN id(n) AS node_id, n.GlobalId AS global_id, material.declared_entity AS material_entity, material.Name AS material_name, material.Category AS material_category",
    `LIMIT ${limit}`,
  ].join("\n");
}

async function runCypher(
  postViewerJson: IfcScopePostViewerJson,
  apiBase: string | undefined,
  resource: string,
  cypher: string,
  why: string,
  diagnostics: string[],
): Promise<CypherResponse | null> {
  const text = await postViewerJson(apiBase, "/api/cypher", { resource, cypher, why });
  const parsed = parseJson<CypherResponse>(text);
  if (!parsed) {
    diagnostics.push(`Cypher response from ${resource} was not valid JSON.`);
    return null;
  }
  if (parsed.ok === false) {
    diagnostics.push(`Cypher query failed for ${resource}: ${stringifyError(parsed.error)}`);
    return null;
  }
  for (const error of parsed.resourceErrors ?? []) {
    diagnostics.push(
      `Cypher resource error for ${error.resource ?? resource}: ${error.error ?? "unknown error"}.`,
    );
  }
  return parsed;
}

function cypherRecords(response: CypherResponse | null): CypherRecord[] {
  if (!response || !Array.isArray(response.columns) || !Array.isArray(response.rows)) {
    return [];
  }
  return response.rows.filter(Array.isArray).map((row) => {
    const record: CypherRecord = {};
    response.columns?.forEach((column, index) => {
      record[normalizeColumnName(column)] = cleanText(row[index]);
    });
    return record;
  });
}

function identityFactFromRecord(
  resource: string,
  record: CypherRecord,
  scopedByGlobalId?: Map<string, ScopedSemanticId>,
): IdentityFact {
  const globalId = recordValue(record, ["global_id", "globalId"]);
  const scoped = scopedByGlobalId?.get(globalId);
  const sourceResource =
    scoped?.sourceResource || recordValue(record, ["source_resource", "sourceResource"]) || resource;
  const semanticId = semanticIdFor(resource, sourceResource, globalId, scoped);
  return {
    resource: sourceResource,
    nodeId: recordValue(record, ["node_id", "nodeId"]),
    globalId,
    semanticId,
    entity: recordValue(record, ["entity"]),
    name: recordValue(record, ["name"]),
    objectType: recordValue(record, ["object_type", "objectType"]),
    predefinedType: recordValue(record, ["predefined_type", "predefinedType"]),
  };
}

function materialFactFromRecord(
  resource: string,
  record: CypherRecord,
  scopedByGlobalId?: Map<string, ScopedSemanticId>,
): MaterialFact {
  const globalId = recordValue(record, ["global_id", "globalId"]);
  const scoped = scopedByGlobalId?.get(globalId);
  const sourceResource =
    scoped?.sourceResource || recordValue(record, ["source_resource", "sourceResource"]) || resource;
  const semanticId = semanticIdFor(resource, sourceResource, globalId, scoped);
  return {
    resource: sourceResource,
    nodeId: recordValue(record, ["node_id", "nodeId"]),
    globalId,
    semanticId,
    materialEntity: recordValue(record, ["material_entity", "materialEntity"]),
    materialName: recordValue(record, ["material_name", "materialName"]),
    materialCategory: recordValue(record, ["material_category", "materialCategory"]),
  };
}

function semanticIdFor(
  queryResource: string,
  sourceResource: string,
  globalId: string,
  scoped?: ScopedSemanticId,
): string {
  if (!globalId) {
    return "";
  }
  if (scoped?.sourceResource) {
    return scoped.original;
  }
  if (sourceResource && sourceResource !== queryResource) {
    return `${sourceResource}::${globalId}`;
  }
  return scoped?.original ?? globalId;
}

function addMissingSemanticDiagnostics(
  resource: string,
  requestedIds: ScopedSemanticId[],
  records: CypherRecord[],
  diagnostics: string[],
): void {
  const returned = new Set(records.map((record) => recordValue(record, ["global_id", "globalId"])));
  const missing = requestedIds.filter((id) => !returned.has(id.globalId));
  if (missing.length > 0) {
    diagnostics.push(
      `Cypher summary for ${resource} did not return ${missing.length} requested semantic id${missing.length === 1 ? "" : "s"}; no facts were inferred for them.`,
    );
  }
}

function addMissingDbNodeDiagnostics(
  resource: string,
  requestedIds: number[],
  records: CypherRecord[],
  diagnostics: string[],
): void {
  const returned = new Set(records.map((record) => recordValue(record, ["node_id", "nodeId"])));
  const missing = requestedIds.filter((id) => !returned.has(String(id)));
  if (missing.length > 0) {
    diagnostics.push(
      `Cypher summary for ${resource} did not return ${missing.length} requested dbNodeId${missing.length === 1 ? "" : "s"}; no facts were inferred for them.`,
    );
  }
}

async function summarizeGeometry(
  postViewerJson: IfcScopePostViewerJson,
  apiBase: string | undefined,
  defaultResource: string,
  sourceGroups: Array<{ resource: string; ids: ScopedSemanticId[] }>,
  identityFacts: IdentityFact[],
  limit: number,
  diagnostics: string[],
): Promise<Record<string, unknown> | null> {
  const idsByResource = new Map<string, Set<string>>();
  for (const group of sourceGroups) {
    if (!group.resource || isProjectResource(group.resource)) {
      continue;
    }
    const set = idsByResource.get(group.resource) ?? new Set<string>();
    for (const id of group.ids) {
      set.add(id.globalId);
    }
    idsByResource.set(group.resource, set);
  }
  for (const fact of identityFacts) {
    if (!fact.resource || isProjectResource(fact.resource) || !fact.globalId) {
      continue;
    }
    const set = idsByResource.get(fact.resource) ?? new Set<string>();
    set.add(fact.globalId);
    idsByResource.set(fact.resource, set);
  }

  const factsWithoutGlobalId = identityFacts.filter(
    (fact) => fact.resource && !isProjectResource(fact.resource) && !fact.globalId,
  ).length;
  if (factsWithoutGlobalId > 0) {
    diagnostics.push(
      `Geometry catalog matching skipped ${factsWithoutGlobalId} summarized node${factsWithoutGlobalId === 1 ? "" : "s"} without GlobalId; no geometry was inferred for them.`,
    );
  }

  if (idsByResource.size === 0 && defaultResource && !isProjectResource(defaultResource)) {
    idsByResource.set(defaultResource, new Set<string>());
  }

  if (idsByResource.size === 0) {
    diagnostics.push(
      "Geometry catalog was not queried because no concrete IFC resource could be resolved for the requested ids.",
    );
    return null;
  }

  const resources: Record<string, unknown> = {};
  for (const [resource, ids] of idsByResource) {
    const text = await postViewerJson(apiBase, "/api/geometry/catalog", { resource });
    const parsed = parseJson<GeometryCatalogResponse>(text);
    if (!parsed || parsed.ok === false || !parsed.catalog) {
      diagnostics.push(
        `Geometry catalog query failed for ${resource}: ${stringifyError(parsed?.error ?? text)}`,
      );
      continue;
    }

    const definitions = parsed.catalog.definitions ?? [];
    const elements = parsed.catalog.elements ?? [];
    const instances = parsed.catalog.instances ?? [];
    const definitionById = new Map(definitions.map((definition) => [String(definition.id), definition]));
    const requestedIds = Array.from(ids).slice(0, limit);
    const elementById = new Map(elements.map((element) => [element.id ?? "", element]));
    const matchingElements = requestedIds
      .map((id) => elementById.get(id))
      .filter((element): element is GeometryElementEntry => Boolean(element));
    const missingElementIds = requestedIds.filter((id) => !elementById.has(id));
    if (missingElementIds.length > 0) {
      diagnostics.push(
        `Geometry catalog for ${resource} did not contain ${missingElementIds.length} requested semantic id${missingElementIds.length === 1 ? "" : "s"}; no geometry was inferred for them.`,
      );
    }

    resources[resource] = {
      catalogCounts: {
        definitions: definitions.length,
        elements: elements.length,
        instances: instances.length,
        vertices: sumNumbers(definitions.map((definition) => definition.vertex_count ?? definition.vertexCount)),
        triangles: sumNumbers(
          definitions.map((definition) => definition.triangle_count ?? definition.triangleCount),
        ),
      },
      requested: requestedIds.length > 0
        ? {
            matchedElements: matchingElements.length,
            missingElementIds,
            elements: matchingElements.map((element) =>
              summarizeGeometryElement(element, instances, definitionById),
            ),
          }
        : {
            matchedElements: 0,
            missingElementIds: [],
            diagnostic:
              "No requested semantic ids were available for per-element geometry matching; only catalog-level counts are reported.",
          },
    };
  }

  return { resources };
}

function summarizeGeometryElement(
  element: GeometryElementEntry,
  instances: GeometryInstanceEntry[],
  definitionById: Map<string, GeometryDefinitionEntry>,
): Record<string, unknown> {
  const elementId = element.id ?? "";
  const elementInstances = instances.filter((instance) => (instance.element_id ?? instance.elementId) === elementId);
  const definitionIds = uniqueStrings(
    elementInstances.map((instance) => cleanText(instance.definition_id ?? instance.definitionId)),
  );
  const definitions = definitionIds
    .map((id) => definitionById.get(id))
    .filter((definition): definition is GeometryDefinitionEntry => Boolean(definition));
  return {
    id: elementId,
    label: element.label ?? "",
    declaredEntity: element.declared_entity ?? element.declaredEntity ?? "",
    defaultRenderClass: element.default_render_class ?? element.defaultRenderClass ?? "",
    boundsMin: element.bounds_min ?? element.boundsMin ?? null,
    boundsMax: element.bounds_max ?? element.boundsMax ?? null,
    instanceCount: elementInstances.length,
    definitionCount: definitionIds.length,
    vertices: sumNumbers(definitions.map((definition) => definition.vertex_count ?? definition.vertexCount)),
    triangles: sumNumbers(definitions.map((definition) => definition.triangle_count ?? definition.triangleCount)),
  };
}

function groupSemanticIdsByResource(
  ids: ScopedSemanticId[],
  defaultResource: string,
): Array<{ resource: string; ids: ScopedSemanticId[] }> {
  const groups = new Map<string, ScopedSemanticId[]>();
  for (const id of ids) {
    const resource = id.sourceResource ?? defaultResource;
    if (!resource) {
      continue;
    }
    const group = groups.get(resource) ?? [];
    group.push(id);
    groups.set(resource, group);
  }
  return Array.from(groups, ([groupResource, groupIds]) => ({
    resource: groupResource,
    ids: groupIds,
  }));
}

function parseScopedSemanticId(value: string): ScopedSemanticId {
  const trimmed = value.trim();
  const marker = trimmed.indexOf("::");
  if (marker > 0) {
    const sourceResource = trimmed.slice(0, marker).trim();
    const globalId = trimmed.slice(marker + 2).trim();
    if (sourceResource.startsWith("ifc/") && globalId) {
      return { original: trimmed, globalId, sourceResource };
    }
  }
  return { original: trimmed, globalId: trimmed };
}

function normalizeInspectionMode(value: string | undefined): IfcInspectionMode {
  const normalized = cleanText(value).toLowerCase();
  if (normalized === "add" || normalized === "append" || normalized === "include" || normalized === "plus") {
    return "add";
  }
  if (
    normalized === "remove" ||
    normalized === "subtract" ||
    normalized === "exclude" ||
    normalized === "drop"
  ) {
    return "remove";
  }
  return "replace";
}

function modeMatches(input: string, mode: IfcInspectionMode): boolean {
  return cleanText(input).toLowerCase() === mode;
}

function recordValue(record: CypherRecord, names: string[]): string {
  for (const name of names) {
    const value = record[normalizeColumnName(name)];
    if (value) {
      return value;
    }
  }
  return "";
}

function normalizeColumnName(value: string): string {
  return value.replace(/[^a-zA-Z0-9]/g, "").toLowerCase();
}

function normalizeLimit(value: number | undefined): number {
  if (!Number.isFinite(value)) {
    return DEFAULT_LIMIT;
  }
  return Math.max(1, Math.min(Math.trunc(value as number), MAX_LIMIT));
}

function summarize(values: string[]): Array<{ value: string; count: number }> {
  const counts = new Map<string, number>();
  for (const value of values) {
    const key = cleanText(value) || "(blank)";
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return Array.from(counts, ([value, count]) => ({ value, count })).sort((left, right) => {
    if (right.count !== left.count) {
      return right.count - left.count;
    }
    return left.value.localeCompare(right.value);
  });
}

function sumNumbers(values: Array<number | undefined>): number {
  let sum = 0;
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) {
      sum += value;
    }
  }
  return sum;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const text = cleanText(value);
    if (text && !seen.has(text)) {
      seen.add(text);
      result.push(text);
    }
  }
  return result;
}

function uniqueDbNodeIds(values: number[]): number[] {
  const seen = new Set<number>();
  const result: number[] = [];
  for (const value of values) {
    if (!Number.isFinite(value)) {
      continue;
    }
    const id = Math.trunc(value);
    if (id < 0 || seen.has(id)) {
      continue;
    }
    seen.add(id);
    result.push(id);
  }
  return result;
}

function cypherStringList(values: string[]): string {
  return `[${values.map((value) => JSON.stringify(value)).join(", ")}]`;
}

function cypherNumberList(values: number[]): string {
  return `[${values.map((value) => Math.trunc(value)).join(", ")}]`;
}

function isProjectResource(resource: string): boolean {
  return resource.startsWith("project/");
}

function cleanText(value: unknown): string {
  return String(value ?? "").trim();
}

function parseJson<T>(text: string): T | null {
  try {
    return JSON.parse(text) as T;
  } catch {
    return null;
  }
}

function stringifyError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}

function asJson(value: unknown): string {
  return JSON.stringify(value, (_key, entry) => (entry === undefined ? undefined : entry), 2);
}
