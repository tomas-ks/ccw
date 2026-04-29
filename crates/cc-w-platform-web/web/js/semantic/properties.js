import { tryGetFirst } from "../util/object.js";
import {
  isIfcResource,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
} from "../viewer/resource.js";

const PICKED_PROPERTY_HIDDEN_KEYS = new Set([
  "declared_entity",
  "declaredEntity",
  "GlobalId",
  "globalId",
  "semanticId",
  "semantic_id",
  "instanceId",
  "definitionId",
  "pickAnchor",
  "worldAnchor",
  "worldCentroid",
  "Tag",
  "tag",
]);

const GRAPH_PROPERTY_HIDDEN_KEYS = new Set([
  "declared_entity",
  "declaredEntity",
  "GlobalId",
  "globalId",
  "Name",
  "name",
]);

export function cypherStringLiteral(value) {
  return `'${String(value).replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
}

export function normalizedColumnName(value) {
  return String(value || "")
    .replace(/[^a-zA-Z0-9]/g, "")
    .toLowerCase();
}

export function findCypherColumnIndex(columns, candidates) {
  const normalizedCandidates = candidates.map(normalizedColumnName);
  return columns.findIndex((column) =>
    normalizedCandidates.includes(normalizedColumnName(column))
  );
}

export function extractDbNodeIdsFromCypherPayload(payload) {
  const columns = Array.isArray(payload?.columns) ? payload.columns : [];
  const rows = Array.isArray(payload?.rows) ? payload.rows : [];
  let nodeIdColumn = findCypherColumnIndex(columns, ["node_id", "db_node_id", "id"]);
  if (nodeIdColumn === -1 && columns.length === 1) {
    const normalized = normalizedColumnName(columns[0]);
    if (
      normalized === "id" ||
      normalized.startsWith("id") ||
      normalized.startsWith("dbnodeid") ||
      normalized.startsWith("nodeid")
    ) {
      nodeIdColumn = 0;
    }
  }
  if (nodeIdColumn === -1) {
    throw new Error(
      "Graph seed query must return a node id column, ideally `id(n) AS node_id`."
    );
  }

  const ids = [];
  for (const row of rows) {
    const parsed = normalizeDbNodeId(row?.[nodeIdColumn]);
    if (parsed !== null) {
      ids.push(parsed);
    }
  }

  return Array.from(new Set(ids));
}

export function normalizeDbNodeId(value) {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  const parsed = Number.parseInt(String(value).trim(), 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function firstNonNull(...values) {
  for (const value of values) {
    if (value !== null && value !== undefined) {
      return value;
    }
  }
  return null;
}

function fetchImplementation(fetchImpl = globalThis.fetch) {
  if (typeof fetchImpl !== "function") {
    throw new Error("A fetch implementation is required for semantic property requests.");
  }
  return fetchImpl;
}

async function postJson(url, payload, { fetchImpl = globalThis.fetch, errorPrefix = url } = {}) {
  const fetchFn = fetchImplementation(fetchImpl);
  const response = await fetchFn(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(body.error || `${errorPrefix} failed (${response.status})`);
  }
  return body;
}

export async function queryCypher(
  cypher,
  {
    resource,
    viewer = null,
    fetchImpl = globalThis.fetch,
  } = {}
) {
  if (viewer && typeof viewer.queryCypher === "function") {
    return viewer.queryCypher(cypher, resource || undefined);
  }
  if (!resource) {
    throw new Error("Cypher query needs an IFC resource.");
  }
  return postJson(
    "/api/cypher",
    { resource, cypher },
    { fetchImpl, errorPrefix: "Cypher query" }
  );
}

export async function queryGraphNodeProperties(
  dbNodeId,
  {
    resource,
    maxRelations = undefined,
    viewer = null,
    fetchImpl = globalThis.fetch,
  } = {}
) {
  const normalizedDbNodeId = normalizeDbNodeId(dbNodeId);
  if (normalizedDbNodeId === null) {
    throw new Error("Node property query needs a DB node id.");
  }
  if (viewer && typeof viewer.queryGraphNodeProperties === "function") {
    return viewer.queryGraphNodeProperties(
      normalizedDbNodeId,
      { maxRelations },
      resource || undefined
    );
  }
  if (!resource) {
    throw new Error("Node property query needs an IFC resource.");
  }
  return postJson(
    "/api/graph/node-properties",
    {
      resource,
      dbNodeId: normalizedDbNodeId,
      maxRelations,
    },
    { fetchImpl, errorPrefix: "Node property query" }
  );
}

export function pickedElementLookupTarget(
  semanticId,
  {
    resource = null,
    viewer = null,
  } = {}
) {
  const scoped = parseSourceScopedSemanticId(semanticId);
  if (scoped) {
    return {
      resource: scoped.sourceResource,
      semanticId: scoped.semanticId,
    };
  }

  const currentResource = resource || safeViewerCurrentResource(viewer);
  if (!semanticId || !isIfcResource(currentResource)) {
    return null;
  }
  return {
    resource: currentResource,
    semanticId: String(semanticId),
  };
}

export async function findPickedElementNode(
  semanticId,
  {
    resource = null,
    viewer = null,
    fetchImpl = globalThis.fetch,
  } = {}
) {
  const target = pickedElementLookupTarget(semanticId, { resource, viewer });
  if (!target) {
    return null;
  }
  const cypher = [
    `MATCH (n) WHERE n.GlobalId = ${cypherStringLiteral(target.semanticId)}`,
    "RETURN id(n) AS node_id LIMIT 1",
  ].join(" ");
  const payload = await queryCypher(cypher, {
    resource: target.resource,
    viewer,
    fetchImpl,
  });
  const dbNodeIds = extractDbNodeIdsFromCypherPayload(payload);
  return {
    ...target,
    dbNodeId: dbNodeIds[0] ?? null,
  };
}

export async function loadPickedElementDetails(
  hit,
  {
    resource = null,
    viewer = null,
    fetchImpl = globalThis.fetch,
    maxRelations = 1,
  } = {}
) {
  const lookup = await findPickedElementNode(hit?.elementId, {
    resource,
    viewer,
    fetchImpl,
  });
  if (!lookup?.dbNodeId) {
    return {
      hit,
      lookup,
      details: null,
    };
  }
  const details = await queryGraphNodeProperties(lookup.dbNodeId, {
    resource: lookup.resource,
    maxRelations,
    viewer,
    fetchImpl,
  });
  return {
    hit,
    lookup,
    details,
  };
}

function nodeEntity(node) {
  return (
    tryGetFirst(node, ["declaredEntity", "declared_entity", "entity", "type", "label"]) ||
    "IfcEntity"
  );
}

function nodeName(node) {
  const raw = tryGetFirst(node, ["name", "Name", "displayName", "displayLabel", "title"]);
  if (typeof raw !== "string") {
    return null;
  }
  const trimmed = raw.trim();
  return trimmed ? trimmed : null;
}

function nodeGlobalId(node) {
  return tryGetFirst(node, ["globalId", "global_id", "GlobalId", "semanticId", "semantic_id"]);
}

function nodeDbNodeId(node) {
  return normalizeDbNodeId(
    tryGetFirst(node, ["dbNodeId", "db_node_id", "nodeId", "id", "key"])
  );
}

function nodeDegree(node) {
  return tryGetFirst(node, ["degree"]);
}

function nodeResource(node) {
  const raw = tryGetFirst(node, ["sourceResource", "source_resource", "resource"]);
  const resource = raw === null ? "" : String(raw).trim();
  return resource || null;
}

function nodeText(node) {
  const entity = nodeEntity(node);
  const name = nodeName(node);
  return name && name !== entity ? `${entity} \u00b7 ${name}` : entity;
}

function scalarPropertyRows(properties, hiddenKeys) {
  const rows = [];
  if (!properties || typeof properties !== "object") {
    return rows;
  }
  for (const [key, value] of Object.entries(properties)) {
    if (hiddenKeys.has(key) || value === null || value === undefined || value === "") {
      continue;
    }
    rows.push([key, value]);
  }
  return rows;
}

function mergePropertyObjects(...values) {
  const merged = {};
  for (const value of values) {
    if (value && typeof value === "object") {
      Object.assign(merged, value);
    }
  }
  return merged;
}

function relationKey(relation) {
  const title =
    relation?.title ||
    relation?.type ||
    relation?.label ||
    relation?.name ||
    relation?.relationshipType ||
    "";
  const detail =
    relation?.detail ||
    relation?.targetLabel ||
    relation?.target ||
    relation?.description ||
    relation?.to ||
    relation?.other?.dbNodeId ||
    "";
  return `${title}::${detail}`;
}

export function mergeBalloonRelations(left = [], right = []) {
  const merged = new Map();
  for (const relation of [...left, ...right]) {
    if (!relation) {
      continue;
    }
    merged.set(relationKey(relation), {
      ...(merged.get(relationKey(relation)) || {}),
      ...relation,
    });
  }
  return Array.from(merged.values());
}

function graphRelationView(relation) {
  return {
    title: tryGetFirst(relation, ["type", "label", "name"]) || "Relation",
    detail: tryGetFirst(relation, ["targetLabel", "target", "description", "to"]) || "",
  };
}

function backendRelationView(relation) {
  const other = relation?.other || {};
  const otherLabel =
    tryGetFirst(other, ["displayLabel", "name", "globalId", "semanticId", "declaredEntity"]) ||
    "";
  const direction = relation?.direction ? `${relation.direction}: ` : "";
  return {
    title: relation?.relationshipType || "Relation",
    detail: `${direction}${otherLabel}`,
  };
}

function relationViews(node, details) {
  const graphRelations = Array.isArray(node?.relations)
    ? node.relations.map(graphRelationView)
    : [];
  const backendRelations = Array.isArray(details?.relations)
    ? details.relations.map(backendRelationView)
    : [];
  return mergeBalloonRelations(graphRelations, backendRelations);
}

export function buildPickLoadingBalloonView(hit = {}) {
  return {
    title: "Loading IFC properties",
    subtitle: hit?.elementId ? `id: ${hit.elementId}` : "id unavailable",
    emptyVisible: false,
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: {
      semanticId: hit?.elementId || null,
      dbNodeId: null,
      resource: null,
      source: "pick",
    },
  };
}

export function buildNoPickBalloonView() {
  return {
    title: "No element picked",
    subtitle: "No visible instance was found at that pixel.",
    emptyVisible: true,
    emptyText: "No visible IFC element was found at that pixel.",
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: null,
  };
}

export function buildPickedElementMissingView(hit = {}, lookup = null) {
  return {
    title: "IFC element",
    subtitle: hit?.elementId ? `id: ${hit.elementId}` : "id unavailable",
    emptyVisible: true,
    emptyText: "No semantic IFC node was found for this pick.",
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: {
      semanticId: hit?.elementId || lookup?.semanticId || null,
      dbNodeId: null,
      resource: lookup?.resource || null,
      source: "pick",
    },
  };
}

export function buildPickedElementErrorView(hit = {}, error = null) {
  return {
    title: "IFC element",
    subtitle: hit?.elementId ? `id: ${hit.elementId}` : "id unavailable",
    emptyVisible: true,
    emptyText: `Could not load IFC properties: ${error?.message || error}`,
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: {
      semanticId: hit?.elementId || null,
      dbNodeId: null,
      resource: null,
      source: "pick",
    },
  };
}

export function buildPickedElementBalloonView({
  hit = {},
  details = null,
  lookup = null,
  resource = null,
} = {}) {
  const resolvedResource = details?.resource || lookup?.resource || resource || null;
  const node = details?.node || {};
  const entity = nodeEntity(node) || "IFC element";
  const ifcId = nodeGlobalId(node) || lookup?.semanticId || hit?.elementId || "";
  const properties = details?.properties && typeof details.properties === "object"
    ? details.properties
    : {};
  const coreRows = [];
  const name = nodeName(node);
  if (name) {
    coreRows.push(["Name", name]);
  }
  coreRows.push(...scalarPropertyRows(properties, PICKED_PROPERTY_HIDDEN_KEYS));
  const hasRows = coreRows.length > 0;

  return {
    title: entity,
    subtitle: ifcId
      ? `id: ${ifcId}${resolvedResource ? ` in ${resolvedResource}` : ""}`
      : "id unavailable",
    emptyVisible: !hasRows,
    emptyText: hasRows ? "" : `No scalar properties found for ${entity}.`,
    graphButtonVisible: firstNonNull(lookup?.dbNodeId, nodeDbNodeId(node)) !== null,
    coreRows,
    extraRows: [],
    relations: [],
    target: {
      semanticId: ifcId || hit?.elementId || null,
      dbNodeId: firstNonNull(lookup?.dbNodeId, nodeDbNodeId(node)),
      resource: resolvedResource,
      source: "pick",
    },
  };
}

export function buildNoGraphNodeBalloonView() {
  return {
    title: "No graph node selected",
    subtitle: "Pick an object or select a graph node to inspect IFC properties.",
    emptyVisible: true,
    emptyText: "Pick an object in the model to inspect its IFC properties.",
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: null,
  };
}

export function buildGraphNodeLoadingBalloonView({
  dbNodeId = null,
  resource = null,
} = {}) {
  return {
    title: "Loading IFC properties",
    subtitle: dbNodeId ? `DB node id: ${dbNodeId}` : "DB node id unavailable",
    emptyVisible: false,
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: {
      semanticId: null,
      dbNodeId,
      resource,
      source: "graph",
    },
  };
}

export function buildGraphNodeErrorBalloonView({
  node = null,
  dbNodeId = null,
  resource = null,
  error = null,
} = {}) {
  const entity = node ? nodeEntity(node) : "IFC graph node";
  return {
    title: node ? nodeText(node) : entity,
    subtitle: dbNodeId ? `DB node id: ${dbNodeId}` : "DB node id unavailable",
    emptyVisible: true,
    emptyText: `Could not load IFC properties: ${error?.message || error}`,
    graphButtonVisible: false,
    coreRows: [],
    extraRows: [],
    relations: [],
    target: {
      semanticId: node ? nodeGlobalId(node) : null,
      dbNodeId,
      resource,
      source: "graph",
    },
  };
}

export function buildGraphNodeBalloonView({
  node = null,
  details = null,
  focus = null,
  resource = null,
} = {}) {
  const detailsNode = details?.node || null;
  const mergedNode = {
    ...(detailsNode || {}),
    ...(node || {}),
  };
  const dbNodeId = firstNonNull(
    normalizeDbNodeId(focus?.dbNodeId),
    nodeDbNodeId(mergedNode),
    nodeDbNodeId(detailsNode)
  );
  if (!node && !detailsNode && dbNodeId === null) {
    return buildNoGraphNodeBalloonView();
  }

  const resolvedResource =
    resource ||
    focus?.resource ||
    details?.resource ||
    nodeResource(mergedNode) ||
    null;
  const entity = nodeEntity(mergedNode);
  const globalId = nodeGlobalId(mergedNode);
  const degree = nodeDegree(mergedNode);
  const coreRows = [
    ["Entity", entity],
    ["DB node id", dbNodeId],
  ];
  if (globalId) {
    coreRows.push(["GlobalId", String(globalId)]);
  }
  if (degree !== null) {
    coreRows.push(["Degree", String(degree)]);
  }

  const graphProperties = tryGetFirst(node, ["properties", "extraProperties", "attrs"]);
  const extraProperties = mergePropertyObjects(graphProperties, details?.properties);
  const extraRows = scalarPropertyRows(extraProperties, GRAPH_PROPERTY_HIDDEN_KEYS);

  return {
    title: nodeText(mergedNode),
    subtitle: resolvedResource
      ? `${entity} in ${resolvedResource}`
      : `${entity} while the viewer is still starting`,
    emptyVisible: false,
    emptyText: "",
    graphButtonVisible: false,
    coreRows,
    extraRows,
    relations: relationViews(node, details),
    target: {
      semanticId: globalId || focus?.semanticId || null,
      dbNodeId,
      resource: resolvedResource,
      source: "graph",
    },
  };
}
