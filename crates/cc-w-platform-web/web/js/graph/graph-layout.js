import { tryGetFirst } from "../util/object.js";
import {
  GRAPH_REFERENCE_VIEWPORT,
  graphIsRelationshipDot,
  graphIsRelationshipNode,
  graphNodeKey,
  graphNodeProperties,
  relationshipPathCurvature,
} from "./graph-helpers.js";

export function relationshipDotPosition(node, positions) {
  const properties = graphNodeProperties(node);
  if (!properties.isRelationshipDot) {
    return null;
  }
  const sourceKey = String(properties.source || "");
  const targetKey = String(properties.target || "");
  const source = positions.get(sourceKey);
  const target = positions.get(targetKey);
  if (!source || !target || sourceKey === targetKey) {
    const endpointKeys = Array.isArray(properties.endpoints)
      ? properties.endpoints.map((value) => String(value)).filter(Boolean)
      : [];
    const endpointPositions = endpointKeys
      .map((key) => positions.get(key))
      .filter((position) => position && Number.isFinite(position.x) && Number.isFinite(position.y));
    if (endpointPositions.length > 0) {
      const sum = endpointPositions.reduce(
        (accumulator, position) => ({
          x: accumulator.x + position.x,
          y: accumulator.y + position.y,
        }),
        { x: 0, y: 0 }
      );
      return {
        x: sum.x / endpointPositions.length,
        y: sum.y / endpointPositions.length,
      };
    }
    return null;
  }

  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const distance = Math.max(Math.sqrt(dx * dx + dy * dy), 0.001);
  const siblingCount = Math.max(1, Number(properties.relationSiblingCount) || 1);
  const curvature = relationshipPathCurvature({
    source: sourceKey,
    target: targetKey,
    edgeId: properties.edgeId || graphNodeKey(node),
    relationSiblingIndex: properties.relationSiblingIndex,
    relationSiblingCount: siblingCount,
  });
  const offset = distance * curvature * 0.5;
  const normalX = -dy / distance;
  const normalY = dx / distance;

  return {
    x: (source.x + target.x) * 0.5 + normalX * offset,
    y: (source.y + target.y) * 0.5 + normalY * offset,
  };
}

export function placeRelationshipDots(nodes, positions) {
  for (const [index, node] of nodes.entries()) {
    if (!graphIsRelationshipDot(node)) {
      continue;
    }
    const position = relationshipDotPosition(node, positions);
    if (position) {
      positions.set(graphNodeKey(node, index), position);
    }
  }
}

export function graphNodePositionsFromModel(graphModel, nodes) {
  const positions = new Map();
  if (!graphModel || typeof graphModel.getNodeAttributes !== "function") {
    return positions;
  }
  for (const [index, node] of nodes.entries()) {
    const key = graphNodeKey(node, index);
    try {
      const attributes = graphModel.getNodeAttributes(key);
      if (
        attributes &&
        Number.isFinite(attributes.x) &&
        Number.isFinite(attributes.y)
      ) {
        positions.set(key, { x: attributes.x, y: attributes.y });
      }
    } catch (_error) {
      // Ignore nodes that are not in the active graph model.
    }
  }
  return positions;
}

export function graphViewportScale(viewport) {
  if (!viewport) {
    return 1;
  }
  const widthScale = viewport.width / GRAPH_REFERENCE_VIEWPORT.width;
  const heightScale = viewport.height / GRAPH_REFERENCE_VIEWPORT.height;
  return Math.max(0.55, widthScale, heightScale);
}

export function graphNodePosition(node, index, total) {
  if (typeof node.x === "number" && typeof node.y === "number") {
    return { x: node.x, y: node.y };
  }
  const safeTotal = Math.max(total, 1);
  const angle = (index / safeTotal) * Math.PI * 2;
  const radius = Math.max(1.4, Math.sqrt(safeTotal) * 1.8);
  return {
    x: Math.cos(angle) * radius,
    y: Math.sin(angle) * radius,
  };
}

export function computeGraphLayout(nodes, edges) {
  if (!nodes.length) {
    return new Map();
  }

  const nodeKeys = nodes.map((node, index) => graphNodeKey(node, index));
  const nodesByKey = new Map(nodeKeys.map((key, index) => [key, nodes[index]]));
  const adjacency = new Map(nodeKeys.map((key) => [key, new Set()]));
  for (const edge of edges) {
    const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]));
    const target = String(tryGetFirst(edge, ["target", "to", "targetId"]));
    if (!adjacency.has(source) || !adjacency.has(target) || source === target) {
      continue;
    }
    adjacency.get(source).add(target);
    adjacency.get(target).add(source);
  }

  const positions = new Map();
  const anchors = new Map();
  const nodesByHop = new Map();
  for (const key of nodeKeys) {
    const node = nodesByKey.get(key);
    const hopDistance = Math.max(
      0,
      Number(tryGetFirst(node, ["hopDistance"]) ?? tryGetFirst(node, ["properties"])?.hopDistance ?? 0) ||
        0
    );
    if (!nodesByHop.has(hopDistance)) {
      nodesByHop.set(hopDistance, []);
    }
    nodesByHop.get(hopDistance).push(key);
  }

  for (const hopKeys of nodesByHop.values()) {
    hopKeys.sort();
  }

  for (const [hopDistance, hopKeys] of Array.from(nodesByHop.entries()).sort((a, b) => a[0] - b[0])) {
    const count = hopKeys.length;
    const ringRadius = hopDistance === 0 ? 0.9 : 2.4 + hopDistance * 2.2;
    for (const [slot, key] of hopKeys.entries()) {
      const angleOffset = hopDistance % 2 === 0 ? 0 : Math.PI / Math.max(count, 3);
      const angle = count === 1 ? 0 : (slot / count) * Math.PI * 2 + angleOffset;
      const position = count === 1 && hopDistance === 0
        ? { x: 0, y: 0 }
        : {
            x: Math.cos(angle) * ringRadius,
            y: Math.sin(angle) * ringRadius,
          };
      positions.set(key, position);
      anchors.set(key, { ...position });
    }
  }
  placeRelationshipDots(nodes, positions);
  for (const [key, position] of positions.entries()) {
    anchors.set(key, { ...position });
  }

  if (nodeKeys.length === 1) {
    return positions;
  }

  const iterations = Math.min(180, 90 + nodeKeys.length * 2);
  const repulsion = 0.06;
  const attraction = 0.025;
  const gravity = 0.01;
  const relationDamping = 0.9;
  const startStep = 0.34;
  const minDistance = 0.18;

  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const displacement = new Map(nodeKeys.map((key) => [key, { x: 0, y: 0 }]));

    for (let leftIndex = 0; leftIndex < nodeKeys.length; leftIndex += 1) {
      const leftKey = nodeKeys[leftIndex];
      const leftPosition = positions.get(leftKey);
      for (let rightIndex = leftIndex + 1; rightIndex < nodeKeys.length; rightIndex += 1) {
        const rightKey = nodeKeys[rightIndex];
        const rightPosition = positions.get(rightKey);
        let dx = leftPosition.x - rightPosition.x;
        let dy = leftPosition.y - rightPosition.y;
        let distanceSquared = dx * dx + dy * dy;
        if (distanceSquared < minDistance * minDistance) {
          const nudge = 0.03 * (rightIndex - leftIndex + 1);
          dx += nudge;
          dy -= nudge;
          distanceSquared = dx * dx + dy * dy;
        }
        const distance = Math.sqrt(distanceSquared);
        const force = repulsion / distanceSquared;
        const fx = (dx / distance) * force;
        const fy = (dy / distance) * force;
        displacement.get(leftKey).x += fx;
        displacement.get(leftKey).y += fy;
        displacement.get(rightKey).x -= fx;
        displacement.get(rightKey).y -= fy;
      }
    }

    for (const edge of edges) {
      const sourceKey = String(tryGetFirst(edge, ["source", "from", "sourceId"]));
      const targetKey = String(tryGetFirst(edge, ["target", "to", "targetId"]));
      if (!positions.has(sourceKey) || !positions.has(targetKey)) {
        continue;
      }
      const sourcePosition = positions.get(sourceKey);
      const targetPosition = positions.get(targetKey);
      let dx = targetPosition.x - sourcePosition.x;
      let dy = targetPosition.y - sourcePosition.y;
      const distance = Math.max(Math.sqrt(dx * dx + dy * dy), minDistance);
      const sourceNode = nodesByKey.get(sourceKey);
      const targetNode = nodesByKey.get(targetKey);
      const sourceRelation = graphIsRelationshipNode(sourceNode);
      const targetRelation = graphIsRelationshipNode(targetNode);
      const desiredLength = sourceRelation || targetRelation ? 2.05 : 2.45;
      const force = (distance - desiredLength) * attraction;
      const fx = (dx / distance) * force;
      const fy = (dy / distance) * force;
      displacement.get(sourceKey).x += fx;
      displacement.get(sourceKey).y += fy;
      displacement.get(targetKey).x -= fx;
      displacement.get(targetKey).y -= fy;
    }

    const cooling = 1 - iteration / iterations;
    const stepLimit = startStep * cooling + 0.02;
    for (const key of nodeKeys) {
      const node = nodesByKey.get(key);
      const position = positions.get(key);
      const anchor = anchors.get(key);
      const delta = displacement.get(key);
      const isSeed = Boolean(tryGetFirst(node, ["properties"])?.isSeed);
      if (isSeed || graphIsRelationshipDot(node)) {
        position.x = anchor.x;
        position.y = anchor.y;
        continue;
      }
      const anchorStrength = 0.06;

      delta.x += (anchor.x - position.x) * anchorStrength;
      delta.y += (anchor.y - position.y) * anchorStrength;
      delta.x += -position.x * gravity;
      delta.y += -position.y * gravity;

      if (graphIsRelationshipNode(node)) {
        delta.x *= relationDamping;
        delta.y *= relationDamping;
      }

      const magnitude = Math.sqrt(delta.x * delta.x + delta.y * delta.y);
      if (magnitude > 0) {
        const scale = Math.min(stepLimit, magnitude) / magnitude;
        position.x += delta.x * scale;
        position.y += delta.y * scale;
      }
    }
    placeRelationshipDots(nodes, positions);
  }

  let maxRadius = 0;
  for (const { x, y } of positions.values()) {
    maxRadius = Math.max(maxRadius, Math.sqrt(x * x + y * y));
  }
  const scale = maxRadius > 0 ? 9 / maxRadius : 1;
  for (const position of positions.values()) {
    position.x *= scale;
    position.y *= scale;
  }
  placeRelationshipDots(nodes, positions);

  return positions;
}

export function computeStableGraphLayout(nodes, edges, previousPositions = new Map()) {
  if (!previousPositions || previousPositions.size === 0) {
    return computeGraphLayout(nodes, edges);
  }

  const nodeKeys = nodes.map((node, index) => graphNodeKey(node, index));
  const adjacency = new Map(nodeKeys.map((key) => [key, new Set()]));
  for (const edge of edges) {
    const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]));
    const target = String(tryGetFirst(edge, ["target", "to", "targetId"]));
    if (!adjacency.has(source) || !adjacency.has(target) || source === target) {
      continue;
    }
    adjacency.get(source).add(target);
    adjacency.get(target).add(source);
  }

  const positions = new Map();
  for (const [index, node] of nodes.entries()) {
    const key = graphNodeKey(node, index);
    const previous = previousPositions.get(key);
    if (previous && Number.isFinite(previous.x) && Number.isFinite(previous.y)) {
      positions.set(key, { x: previous.x, y: previous.y });
    }
  }

  let unresolved = nodes
    .map((node, index) => ({ node, index, key: graphNodeKey(node, index) }))
    .filter(({ key }) => !positions.has(key));

  let pass = 0;
  while (unresolved.length) {
    let placedThisPass = false;
    for (const { node, index, key } of unresolved) {
      const neighborPositions = Array.from(adjacency.get(key) || [])
        .map((neighborKey) => positions.get(neighborKey))
        .filter(Boolean);
      if (!neighborPositions.length && pass === 0) {
        continue;
      }
      if (neighborPositions.length) {
        const centroid = neighborPositions.reduce(
          (acc, position) => ({
            x: acc.x + position.x,
            y: acc.y + position.y,
          }),
          { x: 0, y: 0 }
        );
        centroid.x /= neighborPositions.length;
        centroid.y /= neighborPositions.length;
        const angle = ((index % 12) / 12) * Math.PI * 2 + pass * 0.37;
        const radius = graphIsRelationshipNode(node) ? 0.8 + pass * 0.18 : 1.4 + pass * 0.22;
        positions.set(key, {
          x: centroid.x + Math.cos(angle) * radius,
          y: centroid.y + Math.sin(angle) * radius,
        });
      } else {
        positions.set(key, graphNodePosition(node, index, nodes.length));
      }
      placedThisPass = true;
    }
    unresolved = unresolved.filter(({ key }) => !positions.has(key));
    if (!placedThisPass) {
      for (const { node, index, key } of unresolved) {
        positions.set(key, graphNodePosition(node, index, nodes.length));
      }
      break;
    }
    pass += 1;
  }
  placeRelationshipDots(nodes, positions);

  return positions;
}
