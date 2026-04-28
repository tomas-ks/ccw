import { DEFAULT_EDGE_CURVATURE } from "@sigma/edge-curve";
import { tryGetFirst } from "../util/object.js";

export const GRAPH_EDGE_LABEL_MAX_RATIO = 0.95;
export const GRAPH_NODE_NAME_MAX_RATIO = 0.58;
export const GRAPH_NODE_FORCE_LABEL_MAX_RATIO = 1.05;
export const GRAPH_RELATIONSHIP_LABEL_MAX_RATIO = 0.34;
export const GRAPH_RELATIONSHIP_FORCE_LABEL_MAX_RATIO = GRAPH_RELATIONSHIP_LABEL_MAX_RATIO;
export const GRAPH_REFERENCE_VIEWPORT = Object.freeze({
  width: 648,
  height: 640,
});
export const GRAPH_CAMERA_BASE_PADDING = 1.18;
export const GRAPH_DEFAULT_EDGE_CURVATURE = DEFAULT_EDGE_CURVATURE;

export function isIfcResource(resource) {
  return typeof resource === "string" && resource.startsWith("ifc/");
}

export function isProjectResource(resource) {
  return typeof resource === "string" && resource.startsWith("project/");
}

export function isKnownResource(resource) {
  return isIfcResource(resource) || isProjectResource(resource);
}

export function graphNodeKey(node, index = 0) {
  const raw = tryGetFirst(node, ["dbNodeId", "nodeId", "id", "key", "globalId", "semanticElementId"]);
  return raw === null ? `node-${index}` : String(raw);
}

export function graphNodeDbNodeId(node) {
  const raw = tryGetFirst(node, ["dbNodeId", "db_node_id", "nodeId", "id"]);
  const parsed = Number.parseInt(String(raw ?? "").trim(), 10);
  return Number.isFinite(parsed) ? parsed : null;
}

export function graphNodeSourceResource(node) {
  const raw = tryGetFirst(node, ["sourceResource", "source_resource", "resource"]);
  const resource = raw === null ? "" : String(raw).trim();
  return isKnownResource(resource) ? resource : null;
}

export function graphEdgeKey(edge, index = 0) {
  const raw = tryGetFirst(edge, ["dbEdgeId", "edgeId", "id", "key"]);
  if (raw !== null) {
    return String(raw);
  }
  return `edge-${graphNodeKey(edge, index)}-${index}`;
}

export function graphNodeType(node) {
  const labels = tryGetFirst(node, ["labels"]);
  if (Array.isArray(labels) && labels.length > 0) {
    const firstLabel = labels.find((value) => typeof value === "string" && value.trim().length);
    if (firstLabel) {
      return firstLabel.trim();
    }
  }
  return tryGetFirst(node, ["entity", "declaredEntity", "type", "label"]) || "IfcEntity";
}

export function graphNodeLabel(node) {
  return graphNodeType(node);
}

export function graphNodeEntity(node) {
  return graphNodeType(node);
}

export function graphNodeName(node) {
  const value = tryGetFirst(node, ["name", "displayName", "title"]);
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const label = graphNodeLabel(node);
  return trimmed === label ? null : trimmed;
}

export function graphNodeText(node) {
  const label = graphNodeLabel(node);
  const name = graphNodeName(node);
  return name ? `${label} \u00b7 ${name}` : label;
}

export function graphNodeSemanticId(node) {
  return tryGetFirst(node, ["semanticElementId", "semanticId", "globalId"]);
}

export function graphIsRelationshipNode(node) {
  return graphNodeEntity(node).toLowerCase().includes("ifcrel");
}

export function graphNodeProperties(node) {
  const properties = tryGetFirst(node, ["properties"]);
  return properties && typeof properties === "object" ? properties : {};
}

export function graphIsRelationshipDot(node) {
  return graphIsRelationshipNode(node) && Boolean(graphNodeProperties(node).isRelationshipDot);
}

export function graphRelationshipDotTouchesSelected(node, selectedNodeId) {
  if (!selectedNodeId || !graphIsRelationshipDot(node)) {
    return false;
  }
  const selectedKey = String(selectedNodeId);
  const endpoints = graphNodeProperties(node).endpoints;
  return Array.isArray(endpoints) && endpoints.map(String).includes(selectedKey);
}

export function graphNodeShouldForceLabel(node, ratio, selectedNodeId = null) {
  if (graphIsRelationshipNode(node)) {
    return ratio <= GRAPH_RELATIONSHIP_FORCE_LABEL_MAX_RATIO;
  }
  if (tryGetFirst(node, ["properties"])?.isSeed) {
    return ratio <= GRAPH_NODE_FORCE_LABEL_MAX_RATIO * 1.15;
  }
  return ratio <= GRAPH_NODE_FORCE_LABEL_MAX_RATIO;
}

export function graphNodeSize(node, selected = false) {
  const degree = Number(tryGetFirst(node, ["degree"])) || 0;
  if (graphIsRelationshipNode(node)) {
    return selected ? 6.5 : 4.2;
  }
  if (selected) {
    return 19;
  }
  const base = tryGetFirst(node, ["properties"])?.isSeed ? 20 : 15;
  return Math.min(28, base + Math.sqrt(Math.max(degree, 1)) * 0.7);
}

export function graphNodeZIndex(node, selected = false) {
  if (graphIsRelationshipDot(node)) {
    return 0;
  }
  return selected ? 3 : 2;
}

export function graphNodeRenderLabel(
  node,
  expanded = false,
  ratio = Number.POSITIVE_INFINITY,
  selectedNodeId = null
) {
  if (graphIsRelationshipNode(node)) {
    return ratio <= GRAPH_RELATIONSHIP_LABEL_MAX_RATIO ? graphNodeLabel(node) : "";
  }
  return expanded ? graphNodeText(node) : graphNodeLabel(node);
}

export function graphEdgeCurvature(edge, index = 0) {
  const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
  const target = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
  if (tryGetFirst(edge, ["isRelationshipSegment"])) {
    return relationshipSegmentFallbackCurvature(edge, source, target);
  }
  if (tryGetFirst(edge, ["isRelationshipPath"])) {
    return relationshipPathCurvature({
      source,
      target,
      edgeId: tryGetFirst(edge, ["edgeId", "id", "key"]),
      relationSiblingIndex: tryGetFirst(edge, ["relationSiblingIndex"]),
      relationSiblingCount: tryGetFirst(edge, ["relationSiblingCount"]),
    });
  }
  if (source.startsWith("rel:") || target.startsWith("rel:")) {
    return 0;
  }
  const directionSign = source < target ? 1 : -1;
  const spread = (index % 4) * 0.04;
  return directionSign * (DEFAULT_EDGE_CURVATURE * 0.68 + spread);
}

export function graphEdgeCurvatureForPositions(edge, index = 0, positions = null) {
  const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
  const target = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
  if (tryGetFirst(edge, ["isRelationshipSegment"])) {
    const segmentCurvature = relationshipSegmentCurvature(edge, positions);
    if (Number.isFinite(segmentCurvature)) {
      return segmentCurvature;
    }
    return relationshipSegmentFallbackCurvature(edge, source, target);
  }
  return graphEdgeCurvature(edge, index);
}

export function graphEdgeRenderLabel(edge) {
  const label = tryGetFirst(edge, ["label", "relationshipType", "type"]);
  return label === null || label === undefined ? "" : String(label);
}

export function relationshipEndpointDirection(edge, endpoint) {
  const raw =
    endpoint === "source"
      ? tryGetFirst(edge, ["sourceRoleDirection"])
      : tryGetFirst(edge, ["targetRoleDirection"]);
  return String(raw || "").trim();
}

export function relationshipEndpointRole(edge, endpoint) {
  const raw =
    endpoint === "source"
      ? tryGetFirst(edge, ["sourceRole"])
      : tryGetFirst(edge, ["targetRole"]);
  return raw === null || raw === undefined ? "" : String(raw);
}

export function graphRelationshipDirectionIsKnown(direction) {
  return direction === "from_relation" || direction === "to_relation";
}

export function relationshipPathCurvature({
  source,
  target,
  edgeId,
  relationSiblingIndex,
  relationSiblingCount,
}) {
  const siblingCount = Math.max(1, Number(relationSiblingCount) || 1);
  const siblingIndex = Math.max(0, Number(relationSiblingIndex) || 0);
  const centeredIndex = siblingIndex - (siblingCount - 1) / 2;
  const side =
    Math.abs(centeredIndex) > 0.001
      ? Math.sign(centeredIndex)
      : stableGraphSide(edgeId || `${source}->${target}`);
  const spread = Math.abs(centeredIndex) * 0.08;
  return side * (DEFAULT_EDGE_CURVATURE * 0.62 + spread);
}

export function relationshipSegmentFallbackCurvature(edge, source, target) {
  const relationSource = String(tryGetFirst(edge, ["relationshipSource"]) || source);
  const relationTarget = String(tryGetFirst(edge, ["relationshipTarget"]) || target);
  const parentCurvature = relationshipPathCurvature({
    source: relationSource,
    target: relationTarget,
    edgeId: tryGetFirst(edge, ["edgeId", "id", "key"]),
    relationSiblingIndex: tryGetFirst(edge, ["relationSiblingIndex"]),
    relationSiblingCount: tryGetFirst(edge, ["relationSiblingCount"]),
  });
  return parentCurvature * 0.72;
}

export function relationshipCurveControlPoint(source, target, curvature) {
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const distance = Math.max(Math.sqrt(dx * dx + dy * dy), 0.001);
  return {
    x: (source.x + target.x) * 0.5 + (-dy / distance) * distance * curvature,
    y: (source.y + target.y) * 0.5 + (dx / distance) * distance * curvature,
  };
}

export function curvatureForControlPoint(source, target, control) {
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const distance = Math.max(Math.sqrt(dx * dx + dy * dy), 0.001);
  const midpoint = {
    x: (source.x + target.x) * 0.5,
    y: (source.y + target.y) * 0.5,
  };
  const normal = {
    x: -dy / distance,
    y: dx / distance,
  };
  return ((control.x - midpoint.x) * normal.x + (control.y - midpoint.y) * normal.y) / distance;
}

export function relationshipSegmentCurvature(edge, positions) {
  if (!positions) {
    return null;
  }
  const relationSourceKey = String(tryGetFirst(edge, ["relationshipSource"]) || "");
  const relationTargetKey = String(tryGetFirst(edge, ["relationshipTarget"]) || "");
  const relationNodeKey = String(tryGetFirst(edge, ["relationNodeId"]) || "");
  const sourceKey = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
  const targetKey = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
  const relationSource = positions.get(relationSourceKey);
  const relationTarget = positions.get(relationTargetKey);
  const relationNode = positions.get(relationNodeKey);
  const edgeSource = positions.get(sourceKey);
  const edgeTarget = positions.get(targetKey);
  if (!relationSource || !relationTarget || !relationNode || !edgeSource || !edgeTarget) {
    return null;
  }
  const parentCurvature = relationshipPathCurvature({
    source: relationSourceKey,
    target: relationTargetKey,
    edgeId: tryGetFirst(edge, ["relationshipEdgeId", "edgeId", "id", "key"]),
    relationSiblingIndex: tryGetFirst(edge, ["relationSiblingIndex"]),
    relationSiblingCount: tryGetFirst(edge, ["relationSiblingCount"]),
  });
  const parentControl = relationshipCurveControlPoint(relationSource, relationTarget, parentCurvature);
  const segment = String(tryGetFirst(edge, ["relationshipSegment"]) || "");
  const desiredControl =
    segment === "source"
      ? {
          x: (relationSource.x + parentControl.x) * 0.5,
          y: (relationSource.y + parentControl.y) * 0.5,
        }
      : {
          x: (parentControl.x + relationTarget.x) * 0.5,
          y: (parentControl.y + relationTarget.y) * 0.5,
        };
  return curvatureForControlPoint(edgeSource, edgeTarget, desiredControl);
}

export function stableGraphSide(value) {
  let hash = 0;
  const text = String(value || "");
  for (let index = 0; index < text.length; index += 1) {
    hash = (hash * 31 + text.charCodeAt(index)) | 0;
  }
  return hash % 2 === 0 ? 1 : -1;
}
