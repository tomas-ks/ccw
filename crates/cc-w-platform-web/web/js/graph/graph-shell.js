import {
  DEFAULT_EDGE_CURVATURE,
  createEdgeCurveProgram,
  createDrawCurvedEdgeLabel,
} from "@sigma/edge-curve";
import { tryGetFirst } from "../util/object.js";
import {
  isIfcResource,
  safeViewerCurrentResource,
  scopedSemanticIdForViewer,
} from "../viewer/resource.js";
import { cssVariableOr } from "../ui/settings-menu.js";
import {
  GRAPH_CAMERA_BASE_PADDING,
  GRAPH_EDGE_LABEL_MAX_RATIO,
  GRAPH_NODE_FORCE_LABEL_MAX_RATIO,
  GRAPH_NODE_NAME_MAX_RATIO,
  graphEdgeCurvatureForPositions,
  graphEdgeKey,
  graphEdgeRenderLabel,
  graphIsRelationshipDot,
  graphIsRelationshipNode,
  graphNodeDbNodeId,
  graphNodeEntity,
  graphNodeKey,
  graphNodeRenderLabel,
  graphNodeSemanticId,
  graphNodeShouldForceLabel,
  graphNodeSize,
  graphNodeSourceResource,
  graphNodeText,
  graphNodeZIndex,
  graphRelationshipDotTouchesSelected,
} from "./graph-helpers.js";
import {
  computeGraphLayout,
  computeStableGraphLayout,
  graphNodePositionsFromModel,
  graphNodePosition,
  graphViewportScale,
  placeRelationshipDots,
} from "./graph-layout.js";
import { mapGraphSubgraphResponse } from "./graph-mapping.js";

export const GRAPH_RENDERER_IMPORTS = Object.freeze({
  sigma: ["../../vendor/sigma.mjs", "../../vendor/sigma.js"],
  graphology: ["../../vendor/graphology.mjs", "../../vendor/graphology.js"],
});

export const DrawCurvedGraphEdgeLabel = createDrawCurvedEdgeLabel({
  curvatureAttribute: "curvature",
  defaultCurvature: DEFAULT_EDGE_CURVATURE * 0.68,
  keepLabelUpright: true,
});

export const EmphasizedCurvedArrowProgram = createEdgeCurveProgram({
  arrowHead: {
    extremity: "target",
    lengthToThicknessRatio: 6.2,
    widenessToThicknessRatio: 4.6,
  },
  curvatureAttribute: "curvature",
  drawLabel: DrawCurvedGraphEdgeLabel,
});

function noop() {}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function browserWindow() {
  return typeof window !== "undefined" ? window : globalThis.window;
}

function browserDocument() {
  return typeof document !== "undefined" ? document : globalThis.document;
}

function normalizeElementList(value) {
  if (!value) {
    return [];
  }
  return Array.isArray(value) ? value : Array.from(value);
}

export function resolveGraphShellDom(doc = browserDocument()) {
  return {
    panelTabs: normalizeElementList(doc?.querySelectorAll?.("[data-panel-tab]")),
    panelViews: normalizeElementList(doc?.querySelectorAll?.("[data-panel-view]")),
    graphView: doc?.getElementById?.("graph-view") || null,
    graphFallbackList: doc?.getElementById?.("graph-fallback-list") || null,
    graphEmptyState: doc?.getElementById?.("graph-empty-state") || null,
    graphStatusLine: doc?.getElementById?.("graph-status-line") || null,
    graphClearButton: doc?.getElementById?.("graph-clear-button") || null,
    graphFocusButton: doc?.getElementById?.("graph-focus-button") || null,
    graphRelayoutButton: doc?.getElementById?.("graph-relayout-button") || null,
    graphModeButtons: normalizeElementList(doc?.querySelectorAll?.("[data-graph-mode]")),
  };
}

export function graphPalette(options = {}) {
  return {
    label: cssVariableOr("--graph-label", "#d8e4ff", options),
    edge: cssVariableOr("--graph-edge", "rgba(141, 182, 255, 0.32)", options),
    edgeSize: Number.parseFloat(cssVariableOr("--graph-edge-size", "1.45", options)) || 1.45,
    edgeLabel: cssVariableOr("--graph-edge-label", "rgba(224, 233, 255, 0.9)", options),
    hoverFill: cssVariableOr("--graph-hover-fill", "rgba(243, 246, 255, 0.12)", options),
    hoverPanel: cssVariableOr("--graph-hover-panel", "rgba(15, 19, 32, 0.92)", options),
    hoverBorder: cssVariableOr("--graph-hover-border", "rgba(141, 182, 255, 0.28)", options),
    selected: cssVariableOr("--graph-node-selected", "#f2c879", options),
    relation: cssVariableOr("--graph-node-relation", "#9fb5d9", options),
    spatial: cssVariableOr("--graph-node-spatial", "#8ec2ff", options),
    wall: cssVariableOr("--graph-node-wall", "#79d8b7", options),
    slab: cssVariableOr("--graph-node-slab", "#f0c187", options),
    space: cssVariableOr("--graph-node-space", "#dba8ff", options),
    fallback: cssVariableOr("--graph-node-default", "#7fb1e6", options),
  };
}

export function graphNodeColor(node, selected = false, options = {}) {
  const palette = graphPalette(options);
  if (selected) {
    return palette.selected;
  }
  const entity = graphNodeEntity(node).toLowerCase();
  if (entity.includes("ifcrel")) {
    return palette.relation;
  }
  if (entity.includes("project") || entity.includes("site") || entity.includes("building")) {
    return palette.spatial;
  }
  if (entity.includes("wall")) {
    return palette.wall;
  }
  if (entity.includes("slab") || entity.includes("roof")) {
    return palette.slab;
  }
  if (entity.includes("space")) {
    return palette.space;
  }
  return palette.fallback;
}

export function graphHoverRenderer(context, data, settings) {
  const label =
    typeof data.label === "string" && data.label.trim().length ? data.label.trim() : "";
  const nodeRadius = Math.max(data.size, 4);
  const palette = graphPalette();

  context.save();
  context.shadowOffsetX = 0;
  context.shadowOffsetY = 0;
  context.shadowBlur = 16;
  context.shadowColor = "rgba(0, 0, 0, 0.45)";

  context.beginPath();
  context.fillStyle = palette.hoverFill;
  context.arc(data.x, data.y, nodeRadius + 5, 0, Math.PI * 2);
  context.fill();

  context.shadowBlur = 0;
  context.beginPath();
  context.fillStyle = data.color || palette.label;
  context.arc(data.x, data.y, nodeRadius + 1.5, 0, Math.PI * 2);
  context.fill();

  if (label) {
    const fontSize = settings.labelSize;
    const font = settings.labelFont;
    const weight = settings.labelWeight;
    const paddingX = 8;
    const paddingY = 5;
    context.font = `${weight} ${fontSize}px ${font}`;
    const textWidth = context.measureText(label).width;
    const boxHeight = fontSize + paddingY * 2;
    const boxWidth = textWidth + paddingX * 2;
    const boxX = data.x + nodeRadius + 10;
    const boxY = data.y - boxHeight / 2;
    const radius = 7;

    context.beginPath();
    context.moveTo(boxX + radius, boxY);
    context.lineTo(boxX + boxWidth - radius, boxY);
    context.quadraticCurveTo(boxX + boxWidth, boxY, boxX + boxWidth, boxY + radius);
    context.lineTo(boxX + boxWidth, boxY + boxHeight - radius);
    context.quadraticCurveTo(
      boxX + boxWidth,
      boxY + boxHeight,
      boxX + boxWidth - radius,
      boxY + boxHeight
    );
    context.lineTo(boxX + radius, boxY + boxHeight);
    context.quadraticCurveTo(boxX, boxY + boxHeight, boxX, boxY + boxHeight - radius);
    context.lineTo(boxX, boxY + radius);
    context.quadraticCurveTo(boxX, boxY, boxX + radius, boxY);
    context.closePath();
    context.fillStyle = palette.hoverPanel;
    context.strokeStyle = palette.hoverBorder;
    context.lineWidth = 1;
    context.fill();
    context.stroke();

    context.fillStyle = palette.label;
    context.fillText(label, boxX + paddingX, data.y + fontSize / 3);
  }

  context.restore();
}

export async function importFirst(paths) {
  let lastError = null;
  for (const path of paths) {
    try {
      return await import(path);
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError || new Error("Module import failed.");
}

export async function loadGraphRendererModules(imports = GRAPH_RENDERER_IMPORTS) {
  const graphologyGlobal = globalThis.graphology;
  const SigmaGlobal = globalThis.Sigma;
  if (
    SigmaGlobal &&
    typeof SigmaGlobal === "function" &&
    graphologyGlobal &&
    typeof graphologyGlobal.Graph === "function"
  ) {
    return {
      SigmaConstructor: SigmaGlobal,
      GraphConstructor: graphologyGlobal.Graph,
    };
  }
  try {
    const [sigmaModule, graphologyModule] = await Promise.all([
      importFirst(imports.sigma),
      importFirst(imports.graphology),
    ]);
    const SigmaConstructor =
      sigmaModule.default || sigmaModule.Sigma || sigmaModule.sigma || sigmaModule;
    const GraphConstructor =
      graphologyModule.Graph || graphologyModule.default || graphologyModule;
    if (typeof SigmaConstructor !== "function" || typeof GraphConstructor !== "function") {
      throw new Error("Sigma or Graphology exports did not resolve to constructors.");
    }
    return {
      SigmaConstructor,
      GraphConstructor,
    };
  } catch (error) {
    return {
      error,
    };
  }
}

export function mergeGraphRelations(left = [], right = []) {
  const merged = new Map();
  for (const relation of [...left, ...right]) {
    const type = String(tryGetFirst(relation, ["type", "label", "name"]) || "RELATION");
    const target = String(tryGetFirst(relation, ["target", "to"]) || "");
    const targetLabel = String(
      tryGetFirst(relation, ["targetLabel", "description"]) || ""
    );
    const key = `${type}::${target}::${targetLabel}`;
    merged.set(key, {
      ...(merged.get(key) || {}),
      ...(relation || {}),
    });
  }
  return Array.from(merged.values());
}

export function mergeGraphNodes(left = [], right = []) {
  const merged = new Map();
  for (const node of left) {
    merged.set(graphNodeKey(node), {
      ...node,
      properties: { ...(node.properties || {}) },
      relations: Array.isArray(node.relations) ? [...node.relations] : [],
    });
  }
  for (const node of right) {
    const key = graphNodeKey(node);
    const previous = merged.get(key);
    if (!previous) {
      merged.set(key, {
        ...node,
        properties: { ...(node.properties || {}) },
        relations: Array.isArray(node.relations) ? [...node.relations] : [],
      });
      continue;
    }
    merged.set(key, {
      ...previous,
      ...node,
      properties: {
        ...(previous.properties || {}),
        ...(node.properties || {}),
      },
      relations: mergeGraphRelations(previous.relations, node.relations),
      degree: Math.max(Number(previous.degree) || 0, Number(node.degree) || 0),
    });
  }
  return Array.from(merged.values());
}

export function mergeGraphEdges(left = [], right = []) {
  const merged = new Map();
  for (const edge of left) {
    merged.set(graphEdgeKey(edge), { ...edge });
  }
  for (const edge of right) {
    const key = graphEdgeKey(edge);
    merged.set(key, {
      ...(merged.get(key) || {}),
      ...(edge || {}),
    });
  }
  return Array.from(merged.values());
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
    const raw = row?.[nodeIdColumn];
    const parsed = Number.parseInt(String(raw ?? "").trim(), 10);
    if (Number.isFinite(parsed)) {
      ids.push(parsed);
    }
  }

  return Array.from(new Set(ids));
}

export function createGraphController(viewer) {
  const currentIfcResource = (resource = viewer.currentResource()) => {
    if (!resource || !resource.startsWith("ifc/")) {
      throw new Error(`Current resource \`${resource}\` is not an IFC model.`);
    }
    return resource;
  };

  return {
    async reset({ cypher, resource = currentIfcResource(), mode, graph }) {
      resource = currentIfcResource(resource);
      const queryPayload = await viewer.queryCypher(cypher, resource);
      const seedNodeIds = extractDbNodeIdsFromCypherPayload(queryPayload);
      if (!seedNodeIds.length) {
        await graph.clear({ silent: true });
        graph.setStatus(
          "Seed query returned no graph nodes. Return `id(n) AS node_id` to seed the explorer.",
          "warn"
        );
        return {
          nodes: [],
          edges: [],
          selectedNodeId: null,
          status:
            "Seed query returned no graph nodes. Return `id(n) AS node_id` to seed the explorer.",
        };
      }

      const payload = await viewer.queryGraphSubgraph(
        seedNodeIds,
        {
          hops: 1,
          maxNodes: 120,
          maxEdges: 240,
          mode,
        },
        resource
      );
      return mapGraphSubgraphResponse(payload, {
        status: `Graph reset from ${seedNodeIds.length} seed node${seedNodeIds.length === 1 ? "" : "s"} in ${resource}${payload.truncated ? " (truncated)" : ""}.`,
      });
    },
    async expand({ nodeIds, resource = currentIfcResource(), mode, options = {} }) {
      resource = currentIfcResource(resource);
      const seedNodeIds = (Array.isArray(nodeIds) ? nodeIds : [nodeIds])
        .map((value) => Number.parseInt(String(value), 10))
        .filter((value) => Number.isFinite(value));

      if (!seedNodeIds.length) {
        throw new Error("Graph expansion needs at least one selected DB node id.");
      }

      const payload = await viewer.queryGraphSubgraph(
        seedNodeIds,
        {
          hops: options.hops ?? 1,
          maxNodes: options.maxNodes ?? 120,
          maxEdges: options.maxEdges ?? 240,
          mode,
        },
        resource
      );
      return mapGraphSubgraphResponse(payload, {
        selectedNodeId: String(seedNodeIds[0]),
        status: `Expanded graph around ${seedNodeIds.length} selected node${seedNodeIds.length === 1 ? "" : "s"} in ${resource}${payload.truncated ? " (truncated)" : ""}.`,
      });
    },
  };
}

export function createGraphShell(viewer, appStateStore = null, options = {}) {
  const doc = options.document || browserDocument();
  const win = options.window || browserWindow();
  const dom = {
    ...resolveGraphShellDom(doc),
    ...(options.dom || {}),
  };
  const callbacks = {
    setActiveTab: typeof options.setActiveTab === "function" ? options.setActiveTab : null,
    onStatus: typeof options.onStatus === "function" ? options.onStatus : noop,
    onSelectionChange:
      typeof options.onSelectionChange === "function" ? options.onSelectionChange : noop,
    onDataChange: typeof options.onDataChange === "function" ? options.onDataChange : noop,
    onClear: typeof options.onClear === "function" ? options.onClear : noop,
    renderProperties:
      typeof options.renderProperties === "function" ? options.renderProperties : noop,
    hideProperties:
      typeof options.hideProperties === "function" ? options.hideProperties : noop,
    hidePickAnchor:
      typeof options.hidePickAnchor === "function" ? options.hidePickAnchor : noop,
    showPropertiesAtClientPoint:
      typeof options.showPropertiesAtClientPoint === "function"
        ? options.showPropertiesAtClientPoint
        : noop,
    showPropertiesAtViewportCenter:
      typeof options.showPropertiesAtViewportCenter === "function"
        ? options.showPropertiesAtViewportCenter
        : noop,
    shouldRevealProperties:
      typeof options.shouldRevealProperties === "function"
        ? options.shouldRevealProperties
        : () => false,
    showGraph:
      typeof options.showGraph === "function"
        ? options.showGraph
        : () => win?.wHeader?.showGraph?.(),
    currentResource:
      typeof options.currentResource === "function"
        ? options.currentResource
        : () => safeViewerCurrentResource(viewer),
  };

  const state = {
    activeTab: "graph",
    selectionOrigin: "none",
    mode: "semantic",
    nodes: [],
    edges: [],
    nodesById: new Map(),
    selectedNodeId: null,
    expandedNodeIds: new Set(),
    expansionPinnedNodeIds: new Set(),
    layoutPositions: new Map(),
    cameraState: null,
    controller: null,
    lastResetQuery: "",
    lastResource: callbacks.currentResource(),
    renderer: null,
    graphModel: null,
    sigma: null,
    cameraUpdatedHandler: null,
    edgeLabelsVisible: true,
    nodeLabelsExpanded: false,
    rendererReady: false,
    rendererFailed: false,
    graphViewportSize: null,
  };

  let sigmaResizeFrame = 0;
  let resizeObserver = null;
  const cleanup = [];

  const addWindowListener = (eventName, handler) => {
    win?.addEventListener?.(eventName, handler);
    cleanup.push(() => win?.removeEventListener?.(eventName, handler));
  };

  const waitForAnimationFrames = (count = 2) =>
    new Promise((resolve) => {
      const step = (remaining) => {
        if (remaining <= 0) {
          resolve();
          return;
        }
        win?.requestAnimationFrame
          ? win.requestAnimationFrame(() => step(remaining - 1))
          : setTimeout(() => step(remaining - 1), 16);
      };
      step(count);
    });

  const graphViewportSize = () => {
    if (!dom.graphView) {
      return null;
    }
    const rect = dom.graphView.getBoundingClientRect();
    if (!Number.isFinite(rect.width) || !Number.isFinite(rect.height)) {
      return null;
    }
    return {
      width: Math.max(1, rect.width),
      height: Math.max(1, rect.height),
    };
  };

  const graphVisualRatio = (cameraRatio, viewport = state.graphViewportSize || graphViewportSize()) => {
    return cameraRatio / graphViewportScale(viewport);
  };

  const requestGraphResize = () => {
    if (sigmaResizeFrame) {
      return;
    }
    sigmaResizeFrame = win?.requestAnimationFrame
      ? win.requestAnimationFrame(() => {
          sigmaResizeFrame = 0;
          if (!state.sigma) {
            state.graphViewportSize = graphViewportSize();
            return;
          }
          try {
            const previousViewportSize = state.graphViewportSize;
            const nextViewportSize = graphViewportSize();
            const camera =
              typeof state.sigma.getCamera === "function" ? state.sigma.getCamera() : null;
            const previousCameraState =
              camera && typeof camera.getState === "function" ? camera.getState() : null;
            if (typeof state.sigma.resize === "function") {
              state.sigma.resize(true);
            }
            if (
              camera &&
              typeof camera.setState === "function" &&
              previousViewportSize &&
              nextViewportSize &&
              previousCameraState &&
              Number.isFinite(previousCameraState.ratio)
            ) {
              const previousScale = graphViewportScale(previousViewportSize);
              const nextScale = graphViewportScale(nextViewportSize);
              const resizeScale = nextScale / previousScale;
              if (
                Number.isFinite(resizeScale) &&
                resizeScale > 0 &&
                Math.abs(resizeScale - 1) > 0.001
              ) {
                camera.setState({
                  ...previousCameraState,
                  ratio: Math.max(0.05, previousCameraState.ratio * resizeScale),
                });
              }
            }
            state.graphViewportSize = nextViewportSize;
            refreshGraphRenderer();
          } catch (_error) {
            // Ignore transient resize errors while the panel is settling.
          }
        })
      : setTimeout(() => {
          sigmaResizeFrame = 0;
          state.graphViewportSize = graphViewportSize();
          refreshGraphRenderer();
        }, 16);
  };

  const setActiveTab = (nextTab) => {
    state.activeTab = nextTab;
    for (const tab of dom.panelTabs) {
      const active = tab.dataset.panelTab === nextTab;
      tab.classList.toggle("active", active);
      tab.setAttribute("aria-selected", String(active));
    }
    for (const view of dom.panelViews) {
      view.classList.toggle("active", view.dataset.panelView === nextTab);
    }
    callbacks.setActiveTab?.(nextTab);
  };

  const setStatus = (text, tone = "info") => {
    if (dom.graphStatusLine) {
      dom.graphStatusLine.textContent = text;
      dom.graphStatusLine.dataset.tone = tone;
    }
    callbacks.onStatus(text, tone, api);
  };

  const currentSelectedNode = () =>
    state.selectedNodeId ? state.nodesById.get(state.selectedNodeId) || null : null;

  const graphSelectionShouldShowBalloon = () => Boolean(callbacks.shouldRevealProperties(api));

  const selectedRenderableId = () => {
    const node = currentSelectedNode();
    return node ? graphNodeSemanticId(node) : null;
  };

  const selectedRenderableSourceResource = () => {
    const node = currentSelectedNode();
    return node ? graphNodeSourceResource(node) : null;
  };

  const setAppFocusFromGraphNode = (nodeId) => {
    const node = nodeId ? graphNodeForKey(nodeId) : null;
    if (!node) {
      appStateStore?.dispatch?.({ type: "focus/clear" });
      return;
    }
    appStateStore?.dispatch?.({
      type: "focus/set",
      source: "graph",
      graphNodeId: String(nodeId),
      dbNodeId: graphDbNodeIdForKey(nodeId),
      semanticId: graphNodeSemanticId(node),
      resource: graphNodeSourceResource(node) || state.lastResource || callbacks.currentResource(),
    });
  };

  const syncEmptyState = () => {
    if (!dom.graphEmptyState) {
      return;
    }
    dom.graphEmptyState.hidden = state.nodes.length > 0;
  };

  const syncActionButtons = () => {
    const hasSelectedNode = Boolean(currentSelectedNode());
    if (dom.graphFocusButton) {
      dom.graphFocusButton.disabled = !hasSelectedNode;
    }
    if (dom.graphRelayoutButton) {
      dom.graphRelayoutButton.disabled = state.nodes.length < 2;
    }
  };

  const graphNodeForKey = (nodeId) => state.nodesById.get(String(nodeId)) || null;

  const graphResourceForKey = (nodeId, fallback = callbacks.currentResource()) => {
    const resource = graphNodeSourceResource(graphNodeForKey(nodeId));
    return resource || state.lastResource || fallback;
  };

  const graphDbNodeIdForKey = (nodeId) => {
    const node = graphNodeForKey(nodeId);
    const fromNode = graphNodeDbNodeId(node);
    if (fromNode !== null) {
      return fromNode;
    }
    const parsed = Number.parseInt(String(nodeId ?? "").trim(), 10);
    return Number.isFinite(parsed) ? parsed : null;
  };

  const renderFallbackList = () => {
    if (!dom.graphFallbackList) {
      return;
    }
    if (!state.rendererFailed) {
      dom.graphFallbackList.hidden = true;
      return;
    }
    dom.graphFallbackList.hidden = state.nodes.length === 0;
    dom.graphFallbackList.replaceChildren();
    for (const node of state.nodes) {
      const row = doc.createElement("button");
      row.type = "button";
      row.className = "graph-fallback-row";
      if (graphNodeKey(node) === state.selectedNodeId) {
        row.classList.add("active");
      }
      row.dataset.nodeId = graphNodeKey(node);
      const title = doc.createElement("strong");
      title.textContent = graphNodeText(node);
      const detail = doc.createElement("span");
      detail.textContent = `${graphNodeEntity(node)} · ${graphNodeKey(node)}`;
      row.append(title, detail);
      row.addEventListener("click", () => {
        const nodeId = graphNodeKey(node);
        api.setSelectedNode(nodeId, {
          syncViewer: true,
          origin: "graph",
          revealProperties: graphSelectionShouldShowBalloon(),
        });
        void api.expand(nodeId, { merge: true, silentIfExpanded: true });
      });
      dom.graphFallbackList.append(row);
    }
  };

  const snapshotGraphViewport = () => {
    const positions = graphNodePositionsFromModel(state.graphModel, state.nodes);

    let cameraState = null;
    if (state.sigma && typeof state.sigma.getCamera === "function") {
      const camera = state.sigma.getCamera();
      if (camera && typeof camera.getState === "function") {
        const nextState = camera.getState();
        if (
          nextState &&
          Number.isFinite(nextState.x) &&
          Number.isFinite(nextState.y) &&
          Number.isFinite(nextState.ratio)
        ) {
          cameraState = {
            x: nextState.x,
            y: nextState.y,
            ratio: nextState.ratio,
            angle: Number.isFinite(nextState.angle) ? nextState.angle : 0,
          };
        }
      }
    }

    return { positions, cameraState };
  };

  const disposeSigma = () => {
    if (
      state.sigma &&
      state.cameraUpdatedHandler &&
      typeof state.sigma.getCamera === "function"
    ) {
      const camera = state.sigma.getCamera();
      if (camera && typeof camera.off === "function") {
        camera.off("updated", state.cameraUpdatedHandler);
      }
    }
    state.cameraUpdatedHandler = null;
    if (state.sigma && typeof state.sigma.kill === "function") {
      state.sigma.kill();
    }
    state.sigma = null;
    state.graphModel = null;
  };

  const syncGraphPresentation = () => {
    if (!state.sigma || typeof state.sigma.getCamera !== "function") {
      return;
    }
    const camera = state.sigma.getCamera();
    if (!camera || typeof camera.getState !== "function") {
      return;
    }
    const ratio = graphVisualRatio(camera.getState().ratio);
    const shouldShowEdgeLabels = ratio <= GRAPH_EDGE_LABEL_MAX_RATIO;
    const edgeLabelModeChanged = state.edgeLabelsVisible !== shouldShowEdgeLabels;
    if (edgeLabelModeChanged) {
      state.edgeLabelsVisible = shouldShowEdgeLabels;
      if (typeof state.sigma.setSetting === "function") {
        state.sigma.setSetting("renderEdgeLabels", shouldShowEdgeLabels);
      }
    }
    const shouldExpandNodeLabels = ratio <= GRAPH_NODE_NAME_MAX_RATIO;
    const nodeLabelModeChanged = state.nodeLabelsExpanded !== shouldExpandNodeLabels;
    state.nodeLabelsExpanded = shouldExpandNodeLabels;
    let needsRefresh = edgeLabelModeChanged;

    if (
      state.graphModel &&
      typeof state.graphModel.setNodeAttribute === "function" &&
      typeof state.graphModel.getNodeAttribute === "function"
    ) {
      for (const node of state.nodes) {
        const key = graphNodeKey(node);
        const nextLabel = graphNodeRenderLabel(
          node,
          shouldExpandNodeLabels,
          ratio,
          state.selectedNodeId
        );
        const nextForceLabel = graphNodeShouldForceLabel(
          node,
          ratio,
          state.selectedNodeId
        );
        if (state.graphModel.getNodeAttribute(key, "label") !== nextLabel) {
          state.graphModel.setNodeAttribute(key, "label", nextLabel);
          needsRefresh = true;
        }
        if (state.graphModel.getNodeAttribute(key, "forceLabel") !== nextForceLabel) {
          state.graphModel.setNodeAttribute(key, "forceLabel", nextForceLabel);
          needsRefresh = true;
        }
      }
    }

    if (nodeLabelModeChanged) {
      needsRefresh = true;
    }

    if (typeof state.sigma.refresh === "function" && needsRefresh) {
      state.sigma.refresh();
    }
  };

  const applySelectionToSigma = (nodeId, { refresh = true } = {}) => {
    if (!state.graphModel) {
      return;
    }
    for (const node of state.nodes) {
      const key = graphNodeKey(node);
      const selected = key === nodeId;
      const emphasized = selected || graphRelationshipDotTouchesSelected(node, nodeId);
      if (typeof state.graphModel.setNodeAttribute === "function") {
        state.graphModel.setNodeAttribute(key, "color", graphNodeColor(node, selected));
        state.graphModel.setNodeAttribute(key, "size", graphNodeSize(node, emphasized));
        state.graphModel.setNodeAttribute(key, "zIndex", graphNodeZIndex(node, selected));
        state.graphModel.setNodeAttribute(
          key,
          "label",
          graphNodeRenderLabel(
            node,
            state.nodeLabelsExpanded,
            Number.POSITIVE_INFINITY,
            nodeId
          )
        );
        state.graphModel.setNodeAttribute(
          key,
          "forceLabel",
          graphNodeShouldForceLabel(
            node,
            Number.POSITIVE_INFINITY,
            nodeId
          )
        );
      }
    }
    if (refresh) {
      refreshGraphRenderer();
    }
  };

  const graphNodePositionFromState = (key) => {
    if (
      state.graphModel &&
      typeof state.graphModel.hasNode === "function" &&
      state.graphModel.hasNode(key) &&
      typeof state.graphModel.getNodeAttributes === "function"
    ) {
      try {
        const attributes = state.graphModel.getNodeAttributes(key);
        if (
          attributes &&
          Number.isFinite(attributes.x) &&
          Number.isFinite(attributes.y)
        ) {
          return { x: attributes.x, y: attributes.y };
        }
      } catch (_error) {
        // Ignore missing node state during incremental placement.
      }
    }
    const previous = state.layoutPositions.get(key);
    if (previous && Number.isFinite(previous.x) && Number.isFinite(previous.y)) {
      return { x: previous.x, y: previous.y };
    }
    return null;
  };

  const graphAdjacentNodeKeys = (key) => {
    const adjacent = new Set();
    for (const edge of state.edges) {
      const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
      const target = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
      if (source === key && target && target !== key) {
        adjacent.add(target);
      } else if (target === key && source && source !== key) {
        adjacent.add(source);
      }
    }
    return Array.from(adjacent);
  };

  const incrementalGraphNodePosition = (node, index) => {
    const key = graphNodeKey(node, index);
    const neighborKeys = graphAdjacentNodeKeys(key).filter((neighborKey) => neighborKey !== key);
    const neighborPositions = neighborKeys
      .map((neighborKey) => graphNodePositionFromState(neighborKey))
      .filter(Boolean);
    if (!neighborPositions.length) {
      return graphNodePosition(node, index, state.nodes.length);
    }

    const localSpacings = [];
    for (const neighborKey of neighborKeys) {
      const neighborPosition = graphNodePositionFromState(neighborKey);
      if (!neighborPosition) {
        continue;
      }
      for (const adjacentKey of graphAdjacentNodeKeys(neighborKey)) {
        if (adjacentKey === key) {
          continue;
        }
        const adjacentPosition = graphNodePositionFromState(adjacentKey);
        if (!adjacentPosition) {
          continue;
        }
        const dx = adjacentPosition.x - neighborPosition.x;
        const dy = adjacentPosition.y - neighborPosition.y;
        const distance = Math.sqrt(dx * dx + dy * dy);
        if (distance > 0.01) {
          localSpacings.push(distance);
        }
      }
    }

    const averageLocalSpacing =
      localSpacings.length > 0
        ? localSpacings.reduce((sum, value) => sum + value, 0) / localSpacings.length
        : 0;
    const hasRelationshipEndpoint =
      graphIsRelationshipNode(node) ||
      neighborKeys.some((neighborKey) =>
        graphIsRelationshipNode(state.nodesById.get(neighborKey))
      );
    const desiredRadius = hasRelationshipEndpoint ? 1.9 : 2.8;
    const radius = Math.max(
      desiredRadius,
      Math.min(4.4, averageLocalSpacing * (hasRelationshipEndpoint ? 0.95 : 1.1))
    );

    const centroid = neighborPositions.reduce(
      (acc, position) => ({
        x: acc.x + position.x,
        y: acc.y + position.y,
      }),
      { x: 0, y: 0 }
    );
    centroid.x /= neighborPositions.length;
    centroid.y /= neighborPositions.length;

    let angle = ((index % 16) / 16) * Math.PI * 2 + neighborKeys.length * 0.19;
    if (neighborPositions.length === 1) {
      const anchor = neighborPositions[0];
      const anchorKey = neighborKeys[0];
      let baseAngle = angle;
      const anchorAdjacentPositions = graphAdjacentNodeKeys(anchorKey)
        .filter((adjacentKey) => adjacentKey !== key)
        .map((adjacentKey) => graphNodePositionFromState(adjacentKey))
        .filter(Boolean);
      if (anchorAdjacentPositions.length) {
        const away = anchorAdjacentPositions.reduce(
          (acc, position) => ({
            x: acc.x + (anchor.x - position.x),
            y: acc.y + (anchor.y - position.y),
          }),
          { x: 0, y: 0 }
        );
        if (Math.abs(away.x) > 0.001 || Math.abs(away.y) > 0.001) {
          baseAngle = Math.atan2(away.y, away.x);
        }
      }
      const siblingKeys = Array.from(
        new Set([...graphAdjacentNodeKeys(anchorKey), key].filter((siblingKey) => siblingKey !== anchorKey))
      ).sort();
      const siblingIndex = Math.max(0, siblingKeys.indexOf(key));
      const siblingCount = Math.max(siblingKeys.length, 1);
      const spreadWidth = Math.min(Math.PI * 0.95, 0.42 * Math.max(siblingCount - 1, 1));
      const centeredOffset =
        siblingCount === 1
          ? 0
          : ((siblingIndex / (siblingCount - 1)) - 0.5) * spreadWidth;
      angle = baseAngle + centeredOffset;
    }
    return {
      x: centroid.x + Math.cos(angle) * radius,
      y: centroid.y + Math.sin(angle) * radius,
    };
  };

  const refreshLayoutPositionsFromGraphModel = () => {
    const nextPositions = graphNodePositionsFromModel(state.graphModel, state.nodes);
    if (nextPositions.size) {
      state.layoutPositions = nextPositions;
    }
  };

  const relaxGraphLayout = ({ pinnedNodeIds = new Set(), newNodeKeys = new Set() } = {}) => {
    if (
      !state.graphModel ||
      typeof state.graphModel.getNodeAttributes !== "function" ||
      typeof state.graphModel.setNodeAttribute !== "function"
    ) {
      return;
    }

    const nodeKeys = state.nodes.map((node, index) => graphNodeKey(node, index));
    if (nodeKeys.length < 2) {
      return;
    }

    const nodesByKey = new Map(
      state.nodes.map((node, index) => [graphNodeKey(node, index), node])
    );
    const adjacency = new Map(nodeKeys.map((key) => [key, new Set()]));
    for (const edge of state.edges) {
      const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
      const target = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
      if (!adjacency.has(source) || !adjacency.has(target) || source === target) {
        continue;
      }
      adjacency.get(source).add(target);
      adjacency.get(target).add(source);
    }

    const positions = new Map();
    for (const key of nodeKeys) {
      const attributes = state.graphModel.getNodeAttributes(key);
      positions.set(key, { x: attributes.x, y: attributes.y });
    }

    const iterations = 26;
    const repulsion = 0.055;
    const attraction = 0.04;
    const gravity = 0.008;
    const baseAnchor = 0.13;
    const minDistance = 0.22;
    const stepStart = 0.28;

    for (let iteration = 0; iteration < iterations; iteration += 1) {
      const displacement = new Map(nodeKeys.map((key) => [key, { x: 0, y: 0 }]));

      for (let leftIndex = 0; leftIndex < nodeKeys.length; leftIndex += 1) {
        const leftKey = nodeKeys[leftIndex];
        if (graphIsRelationshipDot(nodesByKey.get(leftKey))) {
          continue;
        }
        const leftPosition = positions.get(leftKey);
        for (let rightIndex = leftIndex + 1; rightIndex < nodeKeys.length; rightIndex += 1) {
          const rightKey = nodeKeys[rightIndex];
          if (graphIsRelationshipDot(nodesByKey.get(rightKey))) {
            continue;
          }
          const rightPosition = positions.get(rightKey);
          let dx = leftPosition.x - rightPosition.x;
          let dy = leftPosition.y - rightPosition.y;
          let distanceSquared = dx * dx + dy * dy;
          if (distanceSquared < minDistance * minDistance) {
            dx += 0.05 * (rightIndex - leftIndex + 1);
            dy -= 0.05 * (rightIndex - leftIndex + 1);
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

      for (const edge of state.edges) {
        if (tryGetFirst(edge, ["isRelationshipSegment"])) {
          continue;
        }
        const sourceKey = String(tryGetFirst(edge, ["source", "from", "sourceId"]) || "");
        const targetKey = String(tryGetFirst(edge, ["target", "to", "targetId"]) || "");
        if (!positions.has(sourceKey) || !positions.has(targetKey)) {
          continue;
        }
        const sourcePosition = positions.get(sourceKey);
        const targetPosition = positions.get(targetKey);
        const dx = targetPosition.x - sourcePosition.x;
        const dy = targetPosition.y - sourcePosition.y;
        const distance = Math.max(Math.sqrt(dx * dx + dy * dy), minDistance);
        const sourceNode = nodesByKey.get(sourceKey);
        const targetNode = nodesByKey.get(targetKey);
        const desiredLength =
          graphIsRelationshipNode(sourceNode) || graphIsRelationshipNode(targetNode)
            ? 2.45
            : 3.1;
        const force = (distance - desiredLength) * attraction;
        const fx = (dx / distance) * force;
        const fy = (dy / distance) * force;
        displacement.get(sourceKey).x += fx;
        displacement.get(sourceKey).y += fy;
        displacement.get(targetKey).x -= fx;
        displacement.get(targetKey).y -= fy;
      }

      const cooling = 1 - iteration / iterations;
      const stepLimit = stepStart * cooling + 0.03;
      for (const key of nodeKeys) {
        if (pinnedNodeIds.has(key)) {
          continue;
        }
        const node = nodesByKey.get(key);
        if (graphIsRelationshipDot(node)) {
          continue;
        }
        const position = positions.get(key);
        const delta = displacement.get(key);
        const anchor = state.layoutPositions.get(key);
        if (anchor && !newNodeKeys.has(key)) {
          delta.x += (anchor.x - position.x) * baseAnchor;
          delta.y += (anchor.y - position.y) * baseAnchor;
        }
        delta.x += -position.x * gravity;
        delta.y += -position.y * gravity;
        const magnitude = Math.sqrt(delta.x * delta.x + delta.y * delta.y);
        if (magnitude > 0) {
          const scale = Math.min(stepLimit, magnitude) / magnitude;
          position.x += delta.x * scale;
          position.y += delta.y * scale;
        }
      }
      placeRelationshipDots(state.nodes, positions);
    }
    placeRelationshipDots(state.nodes, positions);

    for (const key of nodeKeys) {
      if (pinnedNodeIds.has(key) && state.layoutPositions.has(key)) {
        const anchor = state.layoutPositions.get(key);
        state.graphModel.setNodeAttribute(key, "x", anchor.x);
        state.graphModel.setNodeAttribute(key, "y", anchor.y);
        continue;
      }
      const position = positions.get(key);
      state.graphModel.setNodeAttribute(key, "x", position.x);
      state.graphModel.setNodeAttribute(key, "y", position.y);
    }
  };

  const refreshGraphRenderer = () => {
    if (!state.sigma) {
      return;
    }
    if (typeof state.sigma.refresh === "function") {
      state.sigma.refresh();
    } else if (typeof state.sigma.scheduleRefresh === "function") {
      state.sigma.scheduleRefresh();
    } else if (typeof state.sigma.scheduleRender === "function") {
      state.sigma.scheduleRender();
    }
  };

  const applyGraphTheme = ({ refresh = true } = {}) => {
    const palette = graphPalette({ document: doc, window: win });
    if (state.sigma && typeof state.sigma.setSetting === "function") {
      state.sigma.setSetting("labelColor", { color: palette.label });
      state.sigma.setSetting("edgeLabelColor", { color: palette.edgeLabel });
    }
    if (
      state.graphModel &&
      typeof state.graphModel.setNodeAttribute === "function"
    ) {
      for (const node of state.nodes) {
        const key = graphNodeKey(node);
        if (state.graphModel.hasNode && !state.graphModel.hasNode(key)) {
          continue;
        }
        state.graphModel.setNodeAttribute(
          key,
          "color",
          graphNodeColor(node, key === state.selectedNodeId, { document: doc, window: win })
        );
      }
    }
    if (
      state.graphModel &&
      typeof state.graphModel.setEdgeAttribute === "function"
    ) {
      for (const [index, edge] of state.edges.entries()) {
        const key = graphEdgeKey(edge, index);
        if (state.graphModel.hasEdge && !state.graphModel.hasEdge(key)) {
          continue;
        }
        state.graphModel.setEdgeAttribute(key, "color", palette.edge);
        state.graphModel.setEdgeAttribute(key, "size", palette.edgeSize);
      }
    }
    if (refresh) {
      refreshGraphRenderer();
    }
  };

  const focusGraphLayoutCenter = (layout, options = {}) => {
    if (!state.sigma || typeof state.sigma.getCamera !== "function" || !layout.size) {
      return;
    }
    const camera = state.sigma.getCamera();
    if (!camera || typeof camera.setState !== "function") {
      return;
    }
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    for (const position of layout.values()) {
      minX = Math.min(minX, position.x);
      minY = Math.min(minY, position.y);
      maxX = Math.max(maxX, position.x);
      maxY = Math.max(maxY, position.y);
    }
    let center = {
      x: (minX + maxX) / 2,
      y: (minY + maxY) / 2,
    };
    const layoutSpan = Math.max(maxX - minX, maxY - minY);
    if (typeof state.sigma.normalizationFunction === "function") {
      center = state.sigma.normalizationFunction(center);
    }
    const currentState =
      typeof camera.getState === "function" ? camera.getState() : {};
    camera.setState({
      ...currentState,
      x: center.x,
      y: center.y,
      ratio: options.ratio ?? clamp(layoutSpan / 5.5, 1.7, 4),
    });
  };

  const relayoutGraph = (options = {}) => {
    if (!state.nodes.length) {
      setStatus("Seed the graph before recalculating its layout.", "warn");
      return api.snapshot();
    }
    if (
      !state.graphModel ||
      typeof state.graphModel.hasNode !== "function" ||
      typeof state.graphModel.setNodeAttribute !== "function"
    ) {
      setStatus("Graph renderer is not ready for relayout yet.", "warn");
      return api.snapshot();
    }

    const layout = computeGraphLayout(state.nodes, state.edges);
    placeRelationshipDots(state.nodes, layout);
    for (const [index, node] of state.nodes.entries()) {
      const key = graphNodeKey(node, index);
      const position = layout.get(key);
      if (!position || !state.graphModel.hasNode(key)) {
        continue;
      }
      state.graphModel.setNodeAttribute(key, "x", position.x);
      state.graphModel.setNodeAttribute(key, "y", position.y);
    }

    if (typeof state.graphModel.setEdgeAttribute === "function") {
      for (const [index, edge] of state.edges.entries()) {
        const key = graphEdgeKey(edge, index);
        if (state.graphModel.hasEdge && !state.graphModel.hasEdge(key)) {
          continue;
        }
        state.graphModel.setEdgeAttribute(
          key,
          "curvature",
          graphEdgeCurvatureForPositions(edge, index, layout)
        );
      }
    }

    state.layoutPositions = new Map(layout);
    refreshGraphRenderer();

    const shouldFocusSelected = options.focusSelected !== false && state.selectedNodeId;
    if (shouldFocusSelected) {
      focusSelectedNode({ instant: true, ratio: options.ratio ?? 0.72 });
    } else if (options.recenter !== false) {
      focusGraphLayoutCenter(layout, { ratio: options.ratio });
    }
    setStatus(
      `Graph layout recalculated for ${state.nodes.length} node${state.nodes.length === 1 ? "" : "s"} and ${state.edges.length} edge${state.edges.length === 1 ? "" : "s"}.`
    );
    return api.snapshot();
  };

  const patchSigmaGraph = () => {
    if (
      !state.graphModel ||
      !state.sigma ||
      typeof state.graphModel.hasNode !== "function" ||
      typeof state.graphModel.addNode !== "function" ||
      typeof state.graphModel.setNodeAttribute !== "function" ||
      typeof state.graphModel.hasEdge !== "function" ||
      typeof state.graphModel.addEdgeWithKey !== "function"
    ) {
      return false;
    }

    const newNodeKeys = new Set();
    for (const [index, node] of state.nodes.entries()) {
      const key = graphNodeKey(node, index);
      const selected = key === state.selectedNodeId;
      const emphasized =
        selected || graphRelationshipDotTouchesSelected(node, state.selectedNodeId);
      if (!state.graphModel.hasNode(key)) {
        const position = incrementalGraphNodePosition(node, index);
        state.graphModel.addNode(key, {
          label: graphNodeRenderLabel(
            node,
            state.nodeLabelsExpanded,
            Number.POSITIVE_INFINITY,
            state.selectedNodeId
          ),
          forceLabel: graphNodeShouldForceLabel(
            node,
            GRAPH_NODE_FORCE_LABEL_MAX_RATIO,
            state.selectedNodeId
          ),
          size: graphNodeSize(node, emphasized),
          color: graphNodeColor(node, selected, { document: doc, window: win }),
          zIndex: graphNodeZIndex(node, selected),
          x: position.x,
          y: position.y,
        });
        newNodeKeys.add(key);
      } else {
        state.graphModel.setNodeAttribute(
          key,
          "label",
          graphNodeRenderLabel(
            node,
            state.nodeLabelsExpanded,
            Number.POSITIVE_INFINITY,
            state.selectedNodeId
          )
        );
        state.graphModel.setNodeAttribute(
          key,
          "forceLabel",
          graphNodeShouldForceLabel(
            node,
            GRAPH_NODE_FORCE_LABEL_MAX_RATIO,
            state.selectedNodeId
          )
        );
        state.graphModel.setNodeAttribute(
          key,
          "size",
          graphNodeSize(node, emphasized)
        );
        state.graphModel.setNodeAttribute(
          key,
          "color",
          graphNodeColor(node, selected, { document: doc, window: win })
        );
        state.graphModel.setNodeAttribute(
          key,
          "zIndex",
          graphNodeZIndex(node, selected)
        );
      }
    }

    const edgePositions = graphNodePositionsFromModel(state.graphModel, state.nodes);
    const palette = graphPalette({ document: doc, window: win });
    for (const [index, edge] of state.edges.entries()) {
      const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]));
      const target = String(tryGetFirst(edge, ["target", "to", "targetId"]));
      if (!state.graphModel.hasNode(source) || !state.graphModel.hasNode(target)) {
        continue;
      }
      const key = graphEdgeKey(edge, index);
      if (state.graphModel.hasEdge(key)) {
        continue;
      }
      state.graphModel.addEdgeWithKey(key, source, target, {
        color: palette.edge,
        size: palette.edgeSize,
        label: graphEdgeRenderLabel(edge),
        forceLabel: true,
        isRelationshipPath: Boolean(tryGetFirst(edge, ["isRelationshipPath"])),
        isRelationshipSegment: Boolean(tryGetFirst(edge, ["isRelationshipSegment"])),
        edgeId: tryGetFirst(edge, ["edgeId", "id", "key"]),
        relationshipEdgeId: tryGetFirst(edge, ["relationshipEdgeId"]),
        relationshipSegment: tryGetFirst(edge, ["relationshipSegment"]),
        relationNodeId: tryGetFirst(edge, ["relationNodeId"]),
        relationshipSource: tryGetFirst(edge, ["relationshipSource"]),
        relationshipTarget: tryGetFirst(edge, ["relationshipTarget"]),
        relationSiblingIndex: tryGetFirst(edge, ["relationSiblingIndex"]),
        relationSiblingCount: tryGetFirst(edge, ["relationSiblingCount"]),
        type: "curvedArrow",
        curvature: graphEdgeCurvatureForPositions(edge, index, edgePositions),
      });
    }

    relaxGraphLayout({
      pinnedNodeIds: state.expansionPinnedNodeIds,
      newNodeKeys,
    });
    refreshLayoutPositionsFromGraphModel();
    state.expansionPinnedNodeIds = new Set();
    syncGraphPresentation();
    applySelectionToSigma(state.selectedNodeId, { refresh: true });
    return true;
  };

  const renderSigmaGraph = () => {
    if (!state.rendererReady || !state.renderer || !dom.graphView) {
      return;
    }
    const previousPositions = state.layoutPositions;
    const previousCameraState = state.cameraState;
    disposeSigma();
    if (!state.nodes.length) {
      return;
    }

    try {
      const palette = graphPalette({ document: doc, window: win });
      const graph = new state.renderer.GraphConstructor({ multi: true });
      const layout = computeStableGraphLayout(state.nodes, state.edges, previousPositions);
      for (const [index, node] of state.nodes.entries()) {
        const key = graphNodeKey(node, index);
        const position = layout.get(key) || graphNodePosition(node, index, state.nodes.length);
        const selected = key === state.selectedNodeId;
        const emphasized =
          selected || graphRelationshipDotTouchesSelected(node, state.selectedNodeId);
        graph.addNode(key, {
          label: graphNodeRenderLabel(
            node,
            false,
            Number.POSITIVE_INFINITY,
            state.selectedNodeId
          ),
          forceLabel: graphNodeShouldForceLabel(
            node,
            GRAPH_NODE_FORCE_LABEL_MAX_RATIO,
            state.selectedNodeId
          ),
          size: graphNodeSize(node, emphasized),
          color: graphNodeColor(node, selected, { document: doc, window: win }),
          zIndex: graphNodeZIndex(node, selected),
          x: position.x,
          y: position.y,
        });
      }
      for (const [index, edge] of state.edges.entries()) {
        const source = String(tryGetFirst(edge, ["source", "from", "sourceId"]));
        const target = String(tryGetFirst(edge, ["target", "to", "targetId"]));
        if (!graph.hasNode(source) || !graph.hasNode(target)) {
          continue;
        }
        const key = graphEdgeKey(edge, index);
        if (graph.hasEdge && graph.hasEdge(key)) {
          continue;
        }
        graph.addEdgeWithKey(key, source, target, {
          color: palette.edge,
          size: palette.edgeSize,
          label: graphEdgeRenderLabel(edge),
          forceLabel: true,
          isRelationshipPath: Boolean(tryGetFirst(edge, ["isRelationshipPath"])),
          isRelationshipSegment: Boolean(tryGetFirst(edge, ["isRelationshipSegment"])),
          edgeId: tryGetFirst(edge, ["edgeId", "id", "key"]),
          relationshipEdgeId: tryGetFirst(edge, ["relationshipEdgeId"]),
          relationshipSegment: tryGetFirst(edge, ["relationshipSegment"]),
          relationNodeId: tryGetFirst(edge, ["relationNodeId"]),
          relationshipSource: tryGetFirst(edge, ["relationshipSource"]),
          relationshipTarget: tryGetFirst(edge, ["relationshipTarget"]),
          relationSiblingIndex: tryGetFirst(edge, ["relationSiblingIndex"]),
          relationSiblingCount: tryGetFirst(edge, ["relationSiblingCount"]),
          type: "curvedArrow",
          curvature: graphEdgeCurvatureForPositions(edge, index, layout),
        });
      }
      state.graphModel = graph;
      state.layoutPositions = new Map(layout);
      state.sigma = new state.renderer.SigmaConstructor(graph, dom.graphView, {
        allowInvalidContainer: true,
        defaultEdgeType: "curvedArrow",
        edgeProgramClasses: {
          curvedArrow: EmphasizedCurvedArrowProgram,
        },
        hoverRenderer: graphHoverRenderer,
        labelColor: { color: palette.label },
        labelDensity: 0.035,
        labelGridCellSize: 110,
        labelRenderedSizeThreshold: 10,
        edgeLabelColor: { color: palette.edgeLabel },
        edgeLabelSize: 10,
        edgeLabelWeight: "500",
        defaultDrawEdgeLabel: DrawCurvedGraphEdgeLabel,
        renderEdgeLabels: true,
        zIndex: true,
      });
      state.edgeLabelsVisible = true;
      state.nodeLabelsExpanded = false;
      state.graphViewportSize = graphViewportSize();
      if (typeof state.sigma.getCamera === "function") {
        const camera = state.sigma.getCamera();
        if (
          previousCameraState &&
          camera &&
          typeof camera.setState === "function"
        ) {
          camera.setState(previousCameraState);
        } else if (camera && typeof camera.setState === "function") {
          const currentState =
            typeof camera.getState === "function" ? camera.getState() : null;
          const viewport = state.graphViewportSize || graphViewportSize();
          if (currentState && viewport) {
            camera.setState({
              ...currentState,
              ratio: Math.max(
                0.05,
                currentState.ratio *
                  graphViewportScale(viewport) *
                  GRAPH_CAMERA_BASE_PADDING
              ),
            });
          }
        }
        if (camera && typeof camera.on === "function") {
          state.cameraUpdatedHandler = () => {
            syncGraphPresentation();
          };
          camera.on("updated", state.cameraUpdatedHandler);
        }
      }
      syncGraphPresentation();
      if (typeof state.sigma.on === "function") {
        state.sigma.on("clickNode", ({ node }) => {
          const nodeId = String(node);
          api.setSelectedNode(nodeId, {
            syncViewer: true,
            origin: "graph",
            revealProperties: graphSelectionShouldShowBalloon(),
          });
          void api.expand(nodeId, { merge: true, silentIfExpanded: true });
        });
        state.sigma.on("clickStage", () => {
          api.setSelectedNode(null, { syncViewer: true });
        });
      }
    } catch (error) {
      console.warn("Graph renderer unavailable", error);
      disposeSigma();
      state.rendererReady = false;
      state.rendererFailed = true;
      setStatus(
        `Graph renderer unavailable. Falling back to list view. (${error})`,
        "warn"
      );
      renderFallbackList();
    }
  };

  const syncGraphSurface = () => {
    syncEmptyState();
    renderFallbackList();
    if (state.rendererReady) {
      renderSigmaGraph();
    }
  };

  const updateGraphData = (payload = {}, options = {}) => {
    const merge = Boolean(options.merge);
    if (isIfcResource(payload.resource)) {
      state.lastResource = payload.resource;
    }
    if (merge) {
      const snapshot = snapshotGraphViewport();
      if (snapshot.positions.size) {
        state.layoutPositions = snapshot.positions;
      }
      if (snapshot.cameraState) {
        state.cameraState = snapshot.cameraState;
      }
      const focusNodeId = state.selectedNodeId;
      state.expansionPinnedNodeIds = focusNodeId
        ? new Set([focusNodeId, ...graphAdjacentNodeKeys(focusNodeId)])
        : new Set();
    } else {
      state.layoutPositions = new Map();
      state.cameraState = null;
      state.expansionPinnedNodeIds = new Set();
    }
    const incomingNodes = Array.isArray(payload.nodes) ? payload.nodes : [];
    const incomingEdges = Array.isArray(payload.edges) ? payload.edges : [];
    const nodes = merge ? mergeGraphNodes(state.nodes, incomingNodes) : incomingNodes;
    const edges = merge ? mergeGraphEdges(state.edges, incomingEdges) : incomingEdges;
    const dedupedNodes = [];
    const seenNodes = new Set();
    for (const [index, node] of nodes.entries()) {
      const key = graphNodeKey(node, index);
      if (seenNodes.has(key)) {
        continue;
      }
      seenNodes.add(key);
      dedupedNodes.push(node);
    }

    state.nodes = dedupedNodes;
    state.edges = edges;
    state.nodesById = new Map(
      dedupedNodes.map((node, index) => [graphNodeKey(node, index), node])
    );

    const requestedSelection =
      payload.selectedNodeId !== undefined && payload.selectedNodeId !== null
        ? String(payload.selectedNodeId)
        : state.selectedNodeId;
    state.selectedNodeId =
      requestedSelection && state.nodesById.has(requestedSelection)
        ? requestedSelection
        : dedupedNodes[0]
          ? graphNodeKey(dedupedNodes[0])
          : null;

    syncEmptyState();
    renderFallbackList();
    const patchedInPlace =
      merge && state.rendererReady && Boolean(state.sigma) && patchSigmaGraph();
    if (!patchedInPlace) {
      syncGraphSurface();
    }
    if (!options.preserveProperties) {
      callbacks.renderProperties(api);
    }
    syncActionButtons();
    callbacks.onDataChange(graphSnapshot(), api);

    const summary =
      payload.status ||
      `Graph loaded: ${state.nodes.length} node${state.nodes.length === 1 ? "" : "s"}, ${state.edges.length} edge${state.edges.length === 1 ? "" : "s"}${payload.truncated ? " (truncated)" : ""}.`;
    setStatus(summary, payload.truncated ? "warn" : "info");
  };

  const focusSelectedNode = (options = {}) => {
    const node = currentSelectedNode();
    if (!node || !state.graphModel || !state.sigma) {
      return null;
    }
    const key = graphNodeKey(node);
    if (
      typeof state.graphModel.getNodeAttributes !== "function" ||
      typeof state.sigma.getCamera !== "function"
    ) {
      return key;
    }
    const attributes = state.graphModel.getNodeAttributes(key);
    const camera = state.sigma.getCamera();
    let cameraCenter = {
      x: attributes.x,
      y: attributes.y,
    };
    if (typeof state.sigma.normalizationFunction === "function") {
      cameraCenter = state.sigma.normalizationFunction(cameraCenter);
    }
    const nextCameraState = {
      x: cameraCenter.x,
      y: cameraCenter.y,
      ratio: options.ratio ?? 0.55,
    };
    if (options.instant && camera && typeof camera.setState === "function") {
      camera.setState(nextCameraState);
    } else if (camera && typeof camera.animate === "function") {
      camera.animate(
        nextCameraState,
        { duration: 220 }
      );
    } else if (camera && typeof camera.setState === "function") {
      camera.setState(nextCameraState);
    }
    return key;
  };

  const performSelectedViewerAction = (action) => {
    const semanticId = scopedSemanticIdForViewer(
      selectedRenderableId(),
      selectedRenderableSourceResource(),
      callbacks.currentResource()
    );
    if (!semanticId) {
      if (action === "select") {
        viewer.clearSelection?.();
      }
      return null;
    }
    if (action === "select") {
      viewer.clearSelection?.();
      viewer.select?.([semanticId]);
    } else if (action === "hide") {
      viewer.hide?.([semanticId]);
    } else if (action === "show") {
      viewer.show?.([semanticId]);
    }
    return semanticId;
  };

  const graphSnapshot = () => ({
    resource: callbacks.currentResource(),
    mode: state.mode,
    activeTab: state.activeTab,
    selectedNodeId: state.selectedNodeId,
    nodes: state.nodes.length,
    edges: state.edges.length,
    lastResetQuery: state.lastResetQuery,
  });

  const api = {
    installController(controller) {
      state.controller = controller || null;
      if (!controller) {
        setStatus(
          "Graph controller removed. The shell is still ready for graph.reset(...).",
          "warn"
        );
      }
      return api.snapshot();
    },
    setData(payload, options = {}) {
      updateGraphData(payload || {}, { ...options, merge: false });
      return api.snapshot();
    },
    mergeData(payload, options = {}) {
      updateGraphData(payload || {}, { ...options, merge: true });
      return api.snapshot();
    },
    setSelectedNode(nodeId, options = {}) {
      const nextId =
        nodeId === null || nodeId === undefined ? null : String(nodeId);
      state.selectedNodeId = nextId && state.nodesById.has(nextId) ? nextId : null;
      state.selectionOrigin = state.selectedNodeId ? options.origin || "graph" : "none";
      if (state.selectedNodeId) {
        setAppFocusFromGraphNode(state.selectedNodeId);
      } else {
        appStateStore?.dispatch?.({ type: "focus/clear" });
      }
      const preserveProperties = options.preserveProperties === true;
      if (!preserveProperties) {
        callbacks.hidePickAnchor(api);
      }
      renderFallbackList();
      applySelectionToSigma(state.selectedNodeId);
      if (!preserveProperties) {
        callbacks.renderProperties(api);
      }
      syncActionButtons();
      const shouldRevealProperties =
        !preserveProperties &&
        (options.revealProperties === true ||
          (options.revealProperties !== false &&
            state.selectionOrigin === "graph" &&
            graphSelectionShouldShowBalloon()));
      if (shouldRevealProperties) {
        if (state.selectedNodeId) {
          if (Number.isFinite(options.clientX) && Number.isFinite(options.clientY)) {
            callbacks.showPropertiesAtClientPoint(options.clientX, options.clientY, api);
          } else {
            callbacks.showPropertiesAtViewportCenter(api);
          }
        } else {
          callbacks.hideProperties(api);
        }
      } else if (state.selectionOrigin === "graph" || !state.selectedNodeId) {
        if (!preserveProperties) {
          callbacks.hideProperties(api);
        }
      }
      if (options.syncViewer) {
        performSelectedViewerAction("select");
      }
      callbacks.onSelectionChange(currentSelectedNode(), graphSnapshot(), api);
      return currentSelectedNode();
    },
    setNodeProperties(nodeId, properties) {
      const key = String(nodeId);
      const node = state.nodesById.get(key);
      if (!node) {
        return null;
      }
      node.properties = {
        ...(node.properties || {}),
        ...(properties || {}),
      };
      if (state.selectedNodeId === key) {
        callbacks.renderProperties(api);
      }
      return node;
    },
    setNodeDetails(nodeId, details = {}) {
      const key = String(nodeId);
      const node = state.nodesById.get(key);
      if (!node) {
        return null;
      }
      node.properties = {
        ...(node.properties || {}),
        ...(details.properties || {}),
      };
      if (Array.isArray(details.relations)) {
        node.relations = mergeGraphRelations(node.relations, details.relations);
      }
      if (state.selectedNodeId === key) {
        callbacks.renderProperties(api);
      }
      return node;
    },
    setStatus,
    clear(options = {}) {
      state.nodes = [];
      state.edges = [];
      state.nodesById = new Map();
      state.selectedNodeId = null;
      appStateStore?.dispatch?.({ type: "focus/clear" });
      state.expandedNodeIds = new Set();
      state.layoutPositions = new Map();
      state.cameraState = null;
      disposeSigma();
      syncGraphSurface();
      callbacks.renderProperties(api);
      callbacks.hideProperties(api);
      callbacks.onClear(graphSnapshot(), api);
      if (!options.silent) {
        setStatus(
          `Graph cleared for ${callbacks.currentResource() || "the current resource"}. Use graph.reset(...) to seed a new view.`
        );
      }
      return api.snapshot();
    },
    isExpanded(nodeId) {
      return state.expandedNodeIds.has(String(nodeId));
    },
    markExpanded(nodeIds) {
      for (const nodeId of Array.isArray(nodeIds) ? nodeIds : [nodeIds]) {
        if (nodeId !== null && nodeId !== undefined) {
          state.expandedNodeIds.add(String(nodeId));
        }
      }
      return api.snapshot();
    },
    async mode(nextMode) {
      if (!nextMode) {
        return state.mode;
      }
      const normalized = String(nextMode).toLowerCase() === "raw" ? "raw" : "semantic";
      state.mode = normalized;
      for (const button of dom.graphModeButtons) {
        button.classList.toggle("active", button.dataset.graphMode === normalized);
      }
      setStatus(
        `Graph mode set to ${normalized}. Reset or expand the graph to load matching data.`
      );
      return state.mode;
    },
    async reset(cypher) {
      setActiveTab("graph");
      state.lastResetQuery = String(cypher || "");
      setStatus(
        `Resetting graph from ${callbacks.currentResource() || "the current resource"}…`
      );
      if (!state.controller || typeof state.controller.reset !== "function") {
        setStatus(
          "Graph shell is ready. Install a graph controller to connect graph.reset(...) to backend data.",
          "warn"
        );
        return {
          pendingIntegration: true,
          resource: callbacks.currentResource(),
          mode: state.mode,
          cypher: state.lastResetQuery,
        };
      }
      let result;
      try {
        result = await state.controller.reset({
          cypher: state.lastResetQuery,
          resource: callbacks.currentResource(),
          mode: state.mode,
          graph: api,
        });
      } catch (error) {
        setStatus(`Graph reset failed: ${error}`, "error");
        throw error;
      }
      if (result && (Array.isArray(result.nodes) || Array.isArray(result.edges))) {
        state.expandedNodeIds = new Set(
          (Array.isArray(result.seedNodeIds) ? result.seedNodeIds : [])
            .map((value) => String(value))
        );
        api.setData(result);
      }
      return result ?? api.snapshot();
    },
    async expandSelected(options = {}) {
      if (!state.selectedNodeId) {
        setStatus("Select a graph node before expanding.", "warn");
        return {
          expanded: false,
          reason: "No graph node selected.",
        };
      }
      if (options.silentIfExpanded && api.isExpanded(state.selectedNodeId)) {
        return api.snapshot();
      }
      if (!state.controller || typeof state.controller.expand !== "function") {
        setStatus(
          "Graph expansion is ready for integration. Install a controller to fetch neighbors.",
          "warn"
        );
        return {
          pendingIntegration: true,
          selectedNodeIds: [state.selectedNodeId],
          mode: state.mode,
        };
      }
      let result;
      const resource = graphResourceForKey(state.selectedNodeId);
      const selectedDbNodeId = graphDbNodeIdForKey(state.selectedNodeId);
      if (selectedDbNodeId === null || !isIfcResource(resource)) {
        setStatus("Selected graph node is not tied to an IFC resource.", "warn");
        return api.snapshot();
      }
      try {
        result = await state.controller.expand({
          nodeIds: [selectedDbNodeId],
          resource,
          mode: state.mode,
          options,
          graph: api,
        });
      } catch (error) {
        setStatus(`Graph expand failed: ${error}`, "error");
        throw error;
      }
      if (result && (Array.isArray(result.nodes) || Array.isArray(result.edges))) {
        api.markExpanded(state.selectedNodeId);
        if (options.replace) {
          api.setData(result, { preserveProperties: options.preserveProperties });
        } else {
          api.mergeData(result, { preserveProperties: options.preserveProperties });
        }
      }
      return result ?? api.snapshot();
    },
    async expand(nodeIds, options = {}) {
      const ids = Array.isArray(nodeIds) ? nodeIds.map(String) : [String(nodeIds)];
      if (options.silentIfExpanded && ids.every((nodeId) => api.isExpanded(nodeId))) {
        return api.snapshot();
      }
      if (!state.controller || typeof state.controller.expand !== "function") {
        setStatus(
          "Graph expansion is ready for integration. Install a controller to fetch neighbors.",
          "warn"
        );
        return {
          pendingIntegration: true,
          nodeIds: ids,
          mode: state.mode,
        };
      }
      let result;
      const resource = options.resource || graphResourceForKey(ids[0]);
      const dbNodeIds = ids
        .map((nodeId) => graphDbNodeIdForKey(nodeId))
        .filter((value) => value !== null);
      if (!dbNodeIds.length || !isIfcResource(resource)) {
        setStatus("Graph expansion needs DB node ids tied to an IFC resource.", "warn");
        return api.snapshot();
      }
      try {
        result = await state.controller.expand({
          nodeIds: dbNodeIds,
          resource,
          mode: state.mode,
          options,
          graph: api,
        });
      } catch (error) {
        setStatus(`Graph expand failed: ${error}`, "error");
        throw error;
      }
      if (result && (Array.isArray(result.nodes) || Array.isArray(result.edges))) {
        api.markExpanded(ids);
        if (options.replace) {
          api.setData(result, { preserveProperties: options.preserveProperties });
        } else {
          api.mergeData(result, { preserveProperties: options.preserveProperties });
        }
      }
      return result ?? api.snapshot();
    },
    async seedFromNode(nodeId, options = {}) {
      const numericNodeId = graphDbNodeIdForKey(nodeId);
      if (!Number.isFinite(numericNodeId)) {
        setStatus("Graph needs a valid DB node id for the picked element.", "warn");
        return {
          seeded: false,
          reason: "Invalid DB node id.",
        };
      }
      const resource = options.resource || graphResourceForKey(nodeId);
      if (!isIfcResource(resource)) {
        setStatus("Graph needs an IFC resource for the picked element.", "warn");
        return {
          seeded: false,
          reason: "Invalid IFC resource.",
        };
      }
      callbacks.showGraph(api);
      await waitForAnimationFrames(2);
      requestGraphResize();
      await api.mode(options.mode || "semantic");
      const result = await api.expand([numericNodeId], {
        resource,
        replace: true,
        hops: options.hops ?? 2,
        maxNodes: options.maxNodes ?? 80,
        maxEdges: options.maxEdges ?? 160,
        preserveProperties: options.preserveProperties === true,
      });
      api.setSelectedNode(numericNodeId, {
        origin: "graph",
        revealProperties: options.revealProperties ?? false,
        preserveProperties: options.preserveProperties === true,
      });
      await waitForAnimationFrames(2);
      requestGraphResize();
      focusSelectedNode({ instant: true, ratio: options.ratio ?? 0.55 });
      setTimeout(() => {
        focusSelectedNode({ instant: true, ratio: options.ratio ?? 0.55 });
        requestGraphResize();
      }, 90);
      win?.requestAnimationFrame?.(() => {
        focusSelectedNode({ instant: true, ratio: options.ratio ?? 0.55 });
      });
      return result;
    },
    async focusSelected() {
      setActiveTab("graph");
      return focusSelectedNode();
    },
    relayout(options = {}) {
      setActiveTab("graph");
      return relayoutGraph(options);
    },
    async snapshot() {
      return graphSnapshot();
    },
    getSelectedNode() {
      return currentSelectedNode();
    },
    getNode(nodeId) {
      return graphNodeForKey(nodeId);
    },
    nodeResource(nodeId) {
      return graphResourceForKey(nodeId);
    },
    nodeDbId(nodeId) {
      return graphDbNodeIdForKey(nodeId);
    },
    resize: requestGraphResize,
    refresh: refreshGraphRenderer,
    applyTheme: applyGraphTheme,
    dispose() {
      if (sigmaResizeFrame) {
        if (win?.cancelAnimationFrame) {
          win.cancelAnimationFrame(sigmaResizeFrame);
        } else {
          clearTimeout(sigmaResizeFrame);
        }
        sigmaResizeFrame = 0;
      }
      resizeObserver?.disconnect?.();
      resizeObserver = null;
      disposeSigma();
      while (cleanup.length) {
        cleanup.pop()();
      }
    },
  };

  for (const tab of dom.panelTabs) {
    const handler = () => setActiveTab(tab.dataset.panelTab);
    tab.addEventListener("click", handler);
    cleanup.push(() => tab.removeEventListener("click", handler));
  }

  for (const button of dom.graphModeButtons) {
    const handler = () => {
      void api.mode(button.dataset.graphMode);
    };
    button.addEventListener("click", handler);
    cleanup.push(() => button.removeEventListener("click", handler));
  }

  const clearHandler = () => {
    void api.clear();
  };
  dom.graphClearButton?.addEventListener("click", clearHandler);
  cleanup.push(() => dom.graphClearButton?.removeEventListener("click", clearHandler));

  const focusHandler = () => {
    void api.focusSelected();
  };
  dom.graphFocusButton?.addEventListener("click", focusHandler);
  cleanup.push(() => dom.graphFocusButton?.removeEventListener("click", focusHandler));

  const relayoutHandler = () => {
    api.relayout();
    dom.graphRelayoutButton.blur();
  };
  dom.graphRelayoutButton?.addEventListener("click", relayoutHandler);
  cleanup.push(() => dom.graphRelayoutButton?.removeEventListener("click", relayoutHandler));

  addWindowListener("w-theme-change", () => applyGraphTheme());

  addWindowListener("w-viewer-state-change", (event) => {
    const nextResource = event.detail?.state?.resource || callbacks.currentResource();
    if (nextResource !== state.lastResource) {
      state.lastResource = nextResource;
      api.clear({ silent: true });
      setStatus(
        `Graph cleared for ${nextResource}. Use graph.reset(...) to seed the new resource.`
      );
    }
  });

  void loadGraphRendererModules(options.imports || GRAPH_RENDERER_IMPORTS).then((renderer) => {
    if (renderer.error) {
      state.rendererReady = false;
      state.rendererFailed = true;
      setStatus(
        "Sigma graph renderer is not installed in the viewer artifact yet. Using the fallback list shell for now.",
        "warn"
      );
      renderFallbackList();
      return;
    }
    state.renderer = renderer;
    state.rendererReady = true;
    state.rendererFailed = false;
    if (state.nodes.length) {
      syncGraphSurface();
    } else {
      renderFallbackList();
      setStatus(
        "Graph explorer ready. Use graph.reset(...) to seed the panel from the terminal."
      );
    }
  });

  callbacks.renderProperties(api);
  syncActionButtons();

  if (dom.graphView) {
    if (typeof win?.ResizeObserver !== "undefined") {
      resizeObserver = new win.ResizeObserver(() => {
        requestGraphResize();
      });
      resizeObserver.observe(dom.graphView);
    } else {
      addWindowListener("resize", requestGraphResize);
    }
  }

  return {
    api,
    resize: requestGraphResize,
    bridge: {
      installController: api.installController,
      setData: api.setData,
      setSelectedNode: api.setSelectedNode,
      setNodeProperties: api.setNodeProperties,
      setNodeDetails: api.setNodeDetails,
      setStatus: api.setStatus,
      clear: api.clear,
      snapshot: api.snapshot,
      getSelectedNode: api.getSelectedNode,
      currentResource: callbacks.currentResource,
    },
    dispose: api.dispose,
  };
}

export const installGraphShell = createGraphShell;
