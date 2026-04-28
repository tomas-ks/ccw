import { tryGetFirst } from "../util/object.js";
import {
  graphNodeEntity,
  graphNodeText,
  graphRelationshipDirectionIsKnown,
  isKnownResource,
  relationshipEndpointDirection,
  relationshipEndpointRole,
} from "./graph-helpers.js";

export function graphRelationGroupKey(edge) {
  const relationDbNodeId = tryGetFirst(edge, ["relationNodeDbNodeId", "relationNodeId"]);
  if (relationDbNodeId === null || relationDbNodeId === undefined) {
    return null;
  }
  const text = String(relationDbNodeId).trim();
  return text ? `rel:${text}` : null;
}

export function mapGraphSubgraphResponse(payload, options = {}) {
  const payloadResource = String(payload?.resource || "").trim();
  const sourceResource = isKnownResource(payloadResource) ? payloadResource : null;
  const rawEdges = Array.isArray(payload?.edges) ? payload.edges : [];
  const relationBuckets = new Map();
  for (const edge of rawEdges) {
    const source = String(edge.sourceDbNodeId);
    const target = String(edge.targetDbNodeId);
    const type = edge.relationshipType || edge.type || "RELATION";
    const targetLabel = target;
    if (!relationBuckets.has(source)) {
      relationBuckets.set(source, []);
    }
    if (!relationBuckets.has(target)) {
      relationBuckets.set(target, []);
    }
    relationBuckets.get(source).push({
      type,
      target,
      targetLabel,
    });
    relationBuckets.get(target).push({
      type,
      target: source,
      targetLabel: source,
    });
  }

  const relationGroups = new Map();
  const passthroughEdges = [];
  const registerRelationEndpoint = (group, edge, endpoint, endpointNodeId) => {
    const direction = relationshipEndpointDirection(edge, endpoint);
    if (!graphRelationshipDirectionIsKnown(direction)) {
      return;
    }
    const key = String(endpointNodeId);
    const role = relationshipEndpointRole(edge, endpoint);
    const previous = group.endpoints.get(key);
    if (!previous) {
      group.endpoints.set(key, {
        nodeId: key,
        direction,
        roles: role ? new Set([role]) : new Set(),
      });
      return;
    }
    if (previous.direction !== direction) {
      previous.direction = "";
    }
    if (role) {
      previous.roles.add(role);
    }
  };
  for (const edge of rawEdges) {
    const relationshipType = String(edge.relationshipType || edge.type || "RELATION");
    const source = String(edge.sourceDbNodeId);
    const target = String(edge.targetDbNodeId);
    const groupKey = relationshipType.startsWith("IfcRel") ? graphRelationGroupKey(edge) : null;
    if (!groupKey) {
      passthroughEdges.push(edge);
      continue;
    }
    const relationDbNodeId = tryGetFirst(edge, ["relationNodeDbNodeId", "relationNodeId"]);
    if (!relationGroups.has(groupKey)) {
      relationGroups.set(groupKey, {
        nodeId: groupKey,
        relationDbNodeId,
        relationshipType,
        edgeIds: new Set(),
        endpoints: new Map(),
      });
    }
    const group = relationGroups.get(groupKey);
    group.edgeIds.add(edge.edgeId || groupKey);
    registerRelationEndpoint(group, edge, "source", source);
    registerRelationEndpoint(group, edge, "target", target);
  }

  const nodes = (Array.isArray(payload?.nodes) ? payload.nodes : []).map((node) => {
    const rawNodeResource = String(
      tryGetFirst(node, ["sourceResource", "source_resource", "resource"]) || ""
    ).trim();
    const nodeSourceResource = isKnownResource(rawNodeResource)
      ? rawNodeResource
      : sourceResource;
    return {
      id: String(node.dbNodeId),
      dbNodeId: node.dbNodeId,
      sourceResource: nodeSourceResource,
      label: node.declaredEntity || "IfcEntity",
      name: node.name || null,
      displayName: node.displayLabel || node.name || null,
      entity: node.declaredEntity,
      globalId: node.globalId || null,
      semanticElementId: node.globalId || null,
      degree: relationBuckets.get(String(node.dbNodeId))?.length || 0,
      properties: {
        hopDistance: node.hopDistance,
        isSeed: Boolean(node.isSeed),
        sourceResource: nodeSourceResource,
      },
      relations: relationBuckets.get(String(node.dbNodeId)) || [],
    };
  });
  const nodesById = new Map(nodes.map((node) => [String(node.dbNodeId), node]));

  const labelsById = new Map(nodes.map((node) => [String(node.dbNodeId), graphNodeText(node)]));
  for (const node of nodes) {
    if (!Array.isArray(node.relations)) {
      continue;
    }
    for (const relation of node.relations) {
      relation.targetLabel = labelsById.get(String(relation.target)) || relation.targetLabel;
    }
  }

  const edges = [];
  const relationPairCounts = new Map();
  const relationPairIndexes = new Map();
  const relationGroupEntries = Array.from(relationGroups.values()).map((group) => ({
    ...group,
    endpoints: Array.from(group.endpoints.values())
      .filter((endpoint) => graphRelationshipDirectionIsKnown(endpoint.direction))
      .sort((left, right) => left.nodeId.localeCompare(right.nodeId)),
  }));
  for (const group of relationGroupEntries) {
    if (group.endpoints.length !== 2) {
      continue;
    }
    const pairKey = `${group.endpoints[0].nodeId}->${group.endpoints[1].nodeId}`;
    relationPairCounts.set(pairKey, (relationPairCounts.get(pairKey) || 0) + 1);
  }

  for (const group of relationGroupEntries) {
    if (group.endpoints.length < 2) {
      continue;
    }
    const endpointIds = group.endpoints.map((endpoint) => endpoint.nodeId);
    const [primarySource, primaryTarget] = endpointIds;
    const selectedCandidate = String(
      options.selectedNodeId ??
        (Array.isArray(payload?.seedNodeIds) ? payload.seedNodeIds[0] : "") ??
        ""
    );
    const displaySource =
      endpointIds.includes(selectedCandidate) ? selectedCandidate : primarySource;
    const materialEndpoint = group.endpoints.find(
      (endpoint) => graphNodeEntity(nodesById.get(endpoint.nodeId)) === "IfcMaterial"
    );
    const displayTarget =
      materialEndpoint && materialEndpoint.nodeId !== displaySource
        ? materialEndpoint.nodeId
        : endpointIds.find((nodeId) => nodeId !== displaySource) || primaryTarget;
    const pairKey = group.endpoints.length === 2 ? `${primarySource}->${primaryTarget}` : "";
    const relationSiblingIndex = pairKey ? relationPairIndexes.get(pairKey) || 0 : 0;
    if (pairKey) {
      relationPairIndexes.set(pairKey, relationSiblingIndex + 1);
    }
    const relationSiblingCount = pairKey ? relationPairCounts.get(pairKey) || 1 : 1;
    const endpointHops = group.endpoints
      .map((endpoint) => Number(tryGetFirst(nodesById.get(endpoint.nodeId), ["properties"])?.hopDistance))
      .filter(Number.isFinite);
    nodes.push({
      id: group.nodeId,
      sourceResource,
      label: group.relationshipType,
      name: null,
      displayName: group.relationshipType,
      entity: group.relationshipType,
      globalId: null,
      semanticElementId: null,
      degree: group.endpoints.length,
      properties: {
        hopDistance: (endpointHops.length ? Math.min(...endpointHops) : 0) + 0.5,
        isSeed: false,
        isRelationshipDot: true,
        relationshipType: group.relationshipType,
        relationDbNodeId: group.relationDbNodeId,
        edgeId: group.nodeId,
        endpoints: endpointIds,
        source: displaySource,
        target: displayTarget,
        relationSiblingIndex,
        relationSiblingCount,
        sourceResource,
      },
      relations: group.endpoints.map((endpoint) => ({
        type: group.relationshipType,
        target: endpoint.nodeId,
        targetLabel: labelsById.get(endpoint.nodeId) || endpoint.nodeId,
      })),
    });
    for (const endpoint of group.endpoints) {
      const fromRelation = endpoint.direction === "from_relation";
      const segment =
        endpoint.nodeId === primarySource
          ? "source"
          : endpoint.nodeId === primaryTarget
            ? "target"
            : "endpoint";
      const role = Array.from(endpoint.roles).filter(Boolean).join(", ");
      const edgeId = `${group.nodeId}:${endpoint.nodeId}:${endpoint.direction}`;
      edges.push({
        id: edgeId,
        edgeId,
        source: fromRelation ? group.nodeId : endpoint.nodeId,
        target: fromRelation ? endpoint.nodeId : group.nodeId,
        type: "",
        label: "",
        role,
        relationshipType: group.relationshipType,
        isRelationshipSegment: true,
        relationshipSegment: segment,
        relationshipEdgeId: group.nodeId,
        relationNodeId: group.nodeId,
        relationshipSource: group.endpoints.length === 2 ? primarySource : "",
        relationshipTarget: group.endpoints.length === 2 ? primaryTarget : "",
        relationSiblingIndex,
        relationSiblingCount,
        sourceResource,
      });
    }
  }

  for (const edge of passthroughEdges) {
    const source = String(edge.sourceDbNodeId);
    const target = String(edge.targetDbNodeId);
    const relationshipType = String(edge.relationshipType || edge.type || "RELATION");
    edges.push({
      id: edge.edgeId,
      edgeId: edge.edgeId,
      source,
      target,
      sourceResource,
      type: relationshipType,
      label: relationshipType,
    });
  }

  return {
    resource: sourceResource,
    nodes,
    edges,
    seedNodeIds: Array.isArray(payload?.seedNodeIds) ? payload.seedNodeIds : [],
    selectedNodeId:
      options.selectedNodeId ??
      (Array.isArray(payload?.seedNodeIds) && payload.seedNodeIds[0] !== undefined
        ? String(payload.seedNodeIds[0])
        : null),
    truncated: Boolean(payload?.truncated),
    status:
      options.status ||
      `Graph loaded: ${nodes.length} node${nodes.length === 1 ? "" : "s"}, ${edges.length} edge${edges.length === 1 ? "" : "s"}${payload?.truncated ? " (truncated)" : ""}.`,
  };
}
