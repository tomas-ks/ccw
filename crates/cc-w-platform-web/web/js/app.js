import {
  DEFAULT_EDGE_CURVATURE,
  createEdgeCurveProgram,
  createDrawCurvedEdgeLabel,
} from "@sigma/edge-curve";
import init, {
  viewer_clear_selection,
  viewer_available_profiles_json,
  viewer_current_resource,
  viewer_current_profile,
  viewer_default_element_ids,
  viewer_frame_visible,
  viewer_hide_elements,
  viewer_add_inspection_elements,
  viewer_clear_inspection,
  viewer_inspect_elements,
  viewer_inspected_element_ids,
  viewer_list_element_ids,
  viewer_pick_at_json,
  viewer_pick_rect_json,
  viewer_reference_grid_visible,
  viewer_resize_viewport,
  viewer_resource_catalog_json,
  viewer_reset_all_visibility,
  viewer_reset_element_visibility,
  viewer_remove_inspection_elements,
  viewer_select_elements,
  viewer_selected_element_ids,
  viewer_set_clear_color,
  viewer_set_profile,
  viewer_set_reference_grid_visible,
  viewer_set_view_mode,
  viewer_show_elements,
  viewer_stream_visible as viewer_stream_newly_visible_geometry,
  viewer_suppress_elements,
  viewer_unsuppress_elements,
  viewer_view_state_json,
  viewer_visible_element_ids,
} from "../pkg/cc_w_platform_web.js";
import {
  createAppStateStore,
  parseViewerToolValue,
  viewerToolsToPickerValue,
} from "./state/app-state.js";
import {
  createAgentClient,
  extractAgentActions,
  extractAgentSchemaId,
  extractAgentSchemaSlug,
  extractAgentSessionId,
  extractAgentTranscriptItems,
  extractAgentTurnError,
  extractAgentTurnId,
  extractAgentTurnResult,
} from "./agent/client.js";
import { createAgentActionApplier } from "./agent/actions.js";
import { getJson, sleep } from "./net/http.js";
import { tryGetFirst } from "./util/object.js";
import {
  isIfcResource,
  isKnownResource,
  isProjectResource,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
  scopedSemanticIdForViewer,
  semanticIdsForViewerResource,
  sourceScopedSemanticId,
} from "./viewer/resource.js";
import {
  GRAPH_CAMERA_BASE_PADDING,
  GRAPH_EDGE_LABEL_MAX_RATIO,
  GRAPH_NODE_FORCE_LABEL_MAX_RATIO,
  GRAPH_NODE_NAME_MAX_RATIO,
  GRAPH_RELATIONSHIP_FORCE_LABEL_MAX_RATIO,
  GRAPH_RELATIONSHIP_LABEL_MAX_RATIO,
  graphEdgeCurvature,
  graphEdgeCurvatureForPositions,
  graphEdgeKey,
  graphEdgeRenderLabel,
  graphIsRelationshipDot,
  graphIsRelationshipNode,
  graphNodeDbNodeId,
  graphNodeEntity,
  graphNodeKey,
  graphNodeName,
  graphNodeProperties,
  graphNodeRenderLabel,
  graphNodeSemanticId,
  graphNodeShouldForceLabel,
  graphNodeSize,
  graphNodeSourceResource,
  graphNodeText,
  graphNodeZIndex,
  graphRelationshipDotTouchesSelected,
} from "./graph/graph-helpers.js";
import {
  computeGraphLayout,
  computeStableGraphLayout,
  graphNodePositionsFromModel,
  graphNodePosition,
  graphViewportScale,
  placeRelationshipDots,
} from "./graph/graph-layout.js";
import { mapGraphSubgraphResponse } from "./graph/graph-mapping.js";
import {
  TERMINAL_SUCCESS_RGB,
  TERMINAL_WARNING_RGB,
  formatTerminalErrorMessage,
  terminalAnsiWrap,
} from "./terminal/ansi.js";
import {
  compactAgentTranscriptItems,
  renderAgentTranscriptItems,
} from "./terminal/transcript-renderer.js";
import {
  TERMINAL_NO_RESULT,
  installLineTerminal,
  installTerminalToolSelector,
} from "./terminal/line-terminal.js";
import {
  cssVariableOr,
  currentViewerTheme,
  createSettingsMenuController,
} from "./ui/settings-menu.js";
import { createPanelVisibilityController } from "./ui/panels.js";
import { installProjectOutliner as installProjectOutlinerController } from "./ui/outliner.js";

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const REPL_GLOBAL_ASSIGN = /(^|[;{]\s*)(?:var|let|const)\s+([A-Za-z_$][\w$]*)\s*=/gm;
const REPL_PROMPT_TEXT = "W> ";
const GRAPH_REPL_CALLS = [
  "reset",
  "expandSelected",
  "expand",
  "relayout",
  "clear",
  "focusSelected",
  "mode",
  "snapshot",
];
const GRAPH_RENDERER_IMPORTS = {
  sigma: ["../vendor/sigma.mjs", "../vendor/sigma.js"],
  graphology: ["../vendor/graphology.mjs", "../vendor/graphology.js"],
};
const resourceCatalogState = {
  resources: [],
  projects: [],
};
const DrawCurvedGraphEdgeLabel = createDrawCurvedEdgeLabel({
  curvatureAttribute: "curvature",
  defaultCurvature: DEFAULT_EDGE_CURVATURE * 0.68,
  keepLabelUpright: true,
});
const EmphasizedCurvedArrowProgram = createEdgeCurveProgram({
  arrowHead: {
    extremity: "target",
    lengthToThicknessRatio: 6.2,
    widenessToThicknessRatio: 4.6,
  },
  curvatureAttribute: "curvature",
  drawLabel: DrawCurvedGraphEdgeLabel,
});

function graphPalette() {
  return {
    label: cssVariableOr("--graph-label", "#d8e4ff"),
    edge: cssVariableOr("--graph-edge", "rgba(141, 182, 255, 0.32)"),
    edgeSize: Number.parseFloat(cssVariableOr("--graph-edge-size", "1.45")) || 1.45,
    edgeLabel: cssVariableOr("--graph-edge-label", "rgba(224, 233, 255, 0.9)"),
    hoverFill: cssVariableOr("--graph-hover-fill", "rgba(243, 246, 255, 0.12)"),
    hoverPanel: cssVariableOr("--graph-hover-panel", "rgba(15, 19, 32, 0.92)"),
    hoverBorder: cssVariableOr("--graph-hover-border", "rgba(141, 182, 255, 0.28)"),
    selected: cssVariableOr("--graph-node-selected", "#f2c879"),
    relation: cssVariableOr("--graph-node-relation", "#9fb5d9"),
    spatial: cssVariableOr("--graph-node-spatial", "#8ec2ff"),
    wall: cssVariableOr("--graph-node-wall", "#79d8b7"),
    slab: cssVariableOr("--graph-node-slab", "#f0c187"),
    space: cssVariableOr("--graph-node-space", "#dba8ff"),
    fallback: cssVariableOr("--graph-node-default", "#7fb1e6"),
  };
}

const appState = createAppStateStore();

function createViewerApi() {
  const parseViewState = (json) => JSON.parse(json);
  const parsePickResult = (json) => JSON.parse(json);
  const parseResourceCatalog = (json) => JSON.parse(json);
  const parseRenderProfiles = (json) => JSON.parse(json);
  const currentViewerElementIds = (ids, options = {}) =>
    semanticIdsForViewerResource(ids, viewer_current_resource(), options);
  const normalizeInspectionMode = (mode) => {
    const value = String(mode || "replace").trim().toLowerCase();
    if (["add", "append", "include", "plus"].includes(value)) {
      return "add";
    }
    if (["remove", "subtract", "exclude", "drop"].includes(value)) {
      return "remove";
    }
    return "replace";
  };
  const setViewModeAsync = async (mode) => {
    return parseViewState(await viewer_set_view_mode(mode));
  };
  const pickAtAsync = async (x, y) => parsePickResult(await viewer_pick_at_json(x, y));
  const pickRectAsync = async (x0, y0, x1, y1) =>
    parsePickResult(await viewer_pick_rect_json(x0, y0, x1, y1));
  const streamNewlyVisibleGeometrySoon = () => {
    viewer_stream_newly_visible_geometry().catch((error) => {
      console.error("viewer automatic geometry streaming failed", error);
    });
  };
  const setViewModeSoon = (mode) => {
    setViewModeAsync(mode).catch((error) => {
      console.error(`viewer setViewMode(${JSON.stringify(mode)}) failed`, error);
    });
    return api.viewState();
  };
  const pickAtSoon = (x, y) => {
    pickAtAsync(x, y).catch((error) => {
      console.error(`viewer pickAt(${x}, ${y}) failed`, error);
    });
    return api.viewState();
  };
  const pickRectSoon = (x0, y0, x1, y1) => {
    pickRectAsync(x0, y0, x1, y1).catch((error) => {
      console.error(`viewer pickRect(${x0}, ${y0}, ${x1}, ${y1}) failed`, error);
    });
    return api.viewState();
  };
  const api = {
    currentResource: () => viewer_current_resource(),
    currentProfile: () => viewer_current_profile(),
    profile: () => viewer_current_profile(),
    profiles: () => parseRenderProfiles(viewer_available_profiles_json()),
    referenceGridVisible: () => viewer_reference_grid_visible(),
    setReferenceGridVisible: (visible) =>
      parseViewState(viewer_set_reference_grid_visible(Boolean(visible))),
    toggleReferenceGrid: () =>
      parseViewState(viewer_set_reference_grid_visible(!viewer_reference_grid_visible())),
    theme: () => currentViewerTheme(),
    setTheme: (theme) => {
      appState.dispatch({ type: "theme/set", theme });
      return currentViewerTheme();
    },
    setClearColor: (red, green, blue) =>
      parseViewState(viewer_set_clear_color(red, green, blue)),
    resourceCatalog: () => parseResourceCatalog(viewer_resource_catalog_json()),
    viewState: () => parseViewState(viewer_view_state_json()),
    state: () => api.viewState(),
    setProfile: (profile) => parseViewState(viewer_set_profile(profile)),
    setViewMode: (mode) => setViewModeSoon(mode),
    defaultView: () => setViewModeSoon("default"),
    allView: () => setViewModeSoon("all"),
    async: {
      setViewMode: setViewModeAsync,
      defaultView: () => setViewModeAsync("default"),
      allView: () => setViewModeAsync("all"),
      pickAt: pickAtAsync,
      pickRect: pickRectAsync,
    },
    listElementIds: () => Array.from(viewer_list_element_ids()),
    defaultElementIds: () => Array.from(viewer_default_element_ids()),
    visibleElementIds: () => Array.from(viewer_visible_element_ids()),
    selectedElementIds: () => Array.from(viewer_selected_element_ids()),
    inspectedElementIds: () => Array.from(viewer_inspected_element_ids()),
    selectedInstanceIds: () => api.viewState().selectedInstanceIds || [],
    hide: (ids, options = {}) => {
      return viewer_hide_elements(currentViewerElementIds(ids, options));
    },
    show: (ids, options = {}) => {
      const changed = viewer_show_elements(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    suppress: (ids, options = {}) => {
      return viewer_suppress_elements(currentViewerElementIds(ids, options));
    },
    unsuppress: (ids, options = {}) => {
      const changed = viewer_unsuppress_elements(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    resetVisibility: (ids, options = {}) => {
      const changed = viewer_reset_element_visibility(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    resetAllVisibility: () => {
      const changed = viewer_reset_all_visibility();
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    select: (ids, options = {}) => viewer_select_elements(currentViewerElementIds(ids, options)),
    clearSelection: () => viewer_clear_selection(),
    inspect: (ids, options = {}) => {
      const elementIds = currentViewerElementIds(ids, options);
      const mode = normalizeInspectionMode(options.mode);
      if (mode === "add") {
        return viewer_add_inspection_elements(elementIds);
      }
      if (mode === "remove") {
        return viewer_remove_inspection_elements(elementIds);
      }
      return viewer_inspect_elements(elementIds);
    },
    addInspection: (ids, options = {}) =>
      api.inspect(ids, { ...options, mode: "add" }),
    removeInspection: (ids, options = {}) =>
      api.inspect(ids, { ...options, mode: "remove" }),
    clearInspection: () => viewer_clear_inspection(),
    pickAt: pickAtSoon,
    pickRect: pickRectSoon,
    frameVisible: () => viewer_frame_visible(),
    resizeViewport: () => viewer_resize_viewport(),
    queryCypher: async (cypher, resource = viewer_current_resource()) => {
      const response = await fetch("/api/cypher", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ resource, cypher }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Cypher query failed (${response.status})`);
      }
      return payload;
    },
    queryIds: async (cypher, resource = viewer_current_resource()) => {
      const payload = await api.queryCypher(cypher, resource);
      return Array.isArray(payload.semanticElementIds) ? payload.semanticElementIds : [];
    },
    queryGraphSubgraph: async (
      seedNodeIds,
      options = {},
      resource = viewer_current_resource()
    ) => {
      const response = await fetch("/api/graph/subgraph", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          resource,
          seedNodeIds,
          hops: options.hops,
          maxNodes: options.maxNodes,
          maxEdges: options.maxEdges,
          mode: options.mode,
        }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Graph query failed (${response.status})`);
      }
      return payload;
    },
    queryGraphNodeProperties: async (
      dbNodeId,
      options = {},
      resource = viewer_current_resource()
    ) => {
      const response = await fetch("/api/graph/node-properties", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          resource,
          dbNodeId,
          maxRelations: options.maxRelations,
        }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Node property query failed (${response.status})`);
      }
      return payload;
    },
  };
  return api;
}

function projectResourceForIfc(resource) {
  if (!isIfcResource(resource)) {
    return null;
  }
  return (
    resourceCatalogState.projects.find((project) => project.members.includes(resource))
      ?.resource || null
  );
}

function resourcePickerHasOption(resource) {
  if (!resource) {
    return false;
  }
  const picker = document.getElementById("resource-picker");
  return Array.from(picker?.options || []).some((option) => option.value === resource);
}

function selectedIfcResource(viewer, explicitResource) {
  if (isIfcResource(explicitResource)) {
    return explicitResource;
  }
  const pickerValue = document.getElementById("resource-picker")?.value;
  if (isIfcResource(pickerValue)) {
    return pickerValue;
  }
  const viewerResource = safeViewerCurrentResource(viewer);
  return isIfcResource(viewerResource) ? viewerResource : null;
}

function selectedAgentResource(viewer, explicitResource) {
  if (isProjectResource(explicitResource)) {
    return explicitResource;
  }
  const pickerValue = document.getElementById("resource-picker")?.value;
  if (isProjectResource(pickerValue)) {
    return pickerValue;
  }
  const viewerResource = safeViewerCurrentResource(viewer);
  if (isProjectResource(viewerResource)) {
    return viewerResource;
  }
  const ifcResource = selectedIfcResource(viewer, explicitResource);
  const projectResource = projectResourceForIfc(ifcResource);
  return projectResource && resourcePickerHasOption(projectResource)
    ? projectResource
    : ifcResource;
}

function updateResourceCatalogState(payload) {
  resourceCatalogState.resources = Array.isArray(payload?.resources)
    ? payload.resources.map((resource) => String(resource || "").trim()).filter(Boolean)
    : [];
  resourceCatalogState.projects = Array.isArray(payload?.projects)
    ? payload.projects
        .map((project) => ({
          resource: String(project?.resource || "").trim(),
          label: String(project?.label || "").trim(),
          members: Array.isArray(project?.members)
            ? project.members.map((member) => String(member || "").trim()).filter(isIfcResource)
            : [],
        }))
        .filter((project) => isProjectResource(project.resource) && project.members.length)
    : [];
  window.dispatchEvent(
    new CustomEvent("w-resource-catalog-change", {
      detail: {
        resources: resourceCatalogState.resources,
        projects: resourceCatalogState.projects,
      },
    })
  );
}

async function waitForViewerReady(viewer, attempts = 160) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const state = viewer.viewState();
      const catalog = viewer.resourceCatalog();
      if (state?.resource && Array.isArray(catalog?.resources) && catalog.resources.length) {
        return { state, catalog };
      }
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  throw new Error(
    `w web viewer did not initialize before the JS shell booted${
      lastError?.message ? `: ${lastError.message}` : ""
    }`
  );
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function installLayoutResizers(callbacks = {}) {
  const root = document.documentElement;
  const body = document.body;
  const header = document.querySelector(".viewer-header");
  const footer = document.querySelector(".viewer-footer");
  const workspace = document.querySelector(".workspace");
  const sidePanel = document.querySelector(".side-panel");
  const terminal = document.querySelector(".terminal");
  const sidePanelResizer = document.getElementById("side-panel-resizer");
  const terminalResizer = document.getElementById("terminal-resizer");

  if (!workspace || !sidePanel || !terminal || !sidePanelResizer || !terminalResizer) {
    return;
  }

  const onSidePanelResize =
    typeof callbacks.onSidePanelResize === "function" ? callbacks.onSidePanelResize : () => {};
  const onTerminalResize =
    typeof callbacks.onTerminalResize === "function" ? callbacks.onTerminalResize : () => {};
  let refreshFrame = 0;

  const runPaneRefresh = () => {
    refreshFrame = 0;
    onSidePanelResize();
    onTerminalResize();
  };

  const schedulePaneRefresh = ({ immediate = false } = {}) => {
    if (refreshFrame) {
      if (!immediate) {
        return;
      }
      cancelAnimationFrame(refreshFrame);
      refreshFrame = 0;
    }
    if (immediate) {
      refreshFrame = requestAnimationFrame(runPaneRefresh);
      return;
    }
    refreshFrame = requestAnimationFrame(runPaneRefresh);
  };

  const applySidePanelWidth = (width) => {
    root.style.setProperty("--side-panel-width", `${Math.round(width)}px`);
  };

  const applyTerminalHeight = (height) => {
    root.style.setProperty("--terminal-height", `${Math.round(height)}px`);
  };

  const constrainSidePanelWidth = (width) => {
    const workspaceRect = workspace.getBoundingClientRect();
    const minWidth = 420;
    const minViewportWidth = 320;
    const maxWidth = Math.max(minWidth, workspaceRect.width - minViewportWidth);
    return clamp(width, minWidth, maxWidth);
  };

  const constrainTerminalHeight = (height) => {
    const headerHeight = header ? header.getBoundingClientRect().height : 34;
    const footerHeight = footer ? footer.getBoundingClientRect().height : 34;
    const minHeight = 132;
    const minWorkspaceHeight = 260;
    const maxHeight = Math.max(
      minHeight,
      window.innerHeight - headerHeight - footerHeight - minWorkspaceHeight
    );
    return clamp(height, minHeight, maxHeight);
  };

  const syncResizerAria = () => {
    sidePanelResizer.setAttribute(
      "aria-valuenow",
      String(Math.round(sidePanel.getBoundingClientRect().width))
    );
    terminalResizer.setAttribute(
      "aria-valuenow",
      String(Math.round(terminal.getBoundingClientRect().height))
    );
  };

  const beginDrag = (resizer, axis, onMove) => {
    const activeClass = axis === "vertical" ? "vertical" : "horizontal";

    const stop = () => {
      body.classList.remove("is-resizing", activeClass);
      resizer.classList.remove("active");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
      syncResizerAria();
      schedulePaneRefresh({ immediate: true });
    };

    const move = (event) => {
      onMove(event);
    };

    return (event) => {
      event.preventDefault();
      body.classList.add("is-resizing", activeClass);
      resizer.classList.add("active");
      syncResizerAria();
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", stop);
      window.addEventListener("pointercancel", stop);
    };
  };

  sidePanelResizer.addEventListener(
    "pointerdown",
    beginDrag(sidePanelResizer, "vertical", (event) => {
      const workspaceRect = workspace.getBoundingClientRect();
      const nextWidth = constrainSidePanelWidth(workspaceRect.right - event.clientX);
      applySidePanelWidth(nextWidth);
      syncResizerAria();
      schedulePaneRefresh();
    })
  );

  terminalResizer.addEventListener(
    "pointerdown",
    beginDrag(terminalResizer, "horizontal", (event) => {
      const nextHeight = constrainTerminalHeight(window.innerHeight - event.clientY);
      applyTerminalHeight(nextHeight);
      syncResizerAria();
      schedulePaneRefresh();
    })
  );

  window.addEventListener("resize", () => {
    applySidePanelWidth(constrainSidePanelWidth(sidePanel.getBoundingClientRect().width));
    if (!body.classList.contains("terminal-hidden")) {
      applyTerminalHeight(constrainTerminalHeight(terminal.getBoundingClientRect().height));
    }
    syncResizerAria();
    schedulePaneRefresh();
  });

  applySidePanelWidth(constrainSidePanelWidth(sidePanel.getBoundingClientRect().width));
  applyTerminalHeight(constrainTerminalHeight(terminal.getBoundingClientRect().height));
  syncResizerAria();
  schedulePaneRefresh({ immediate: true });
}

function installElectronShellControls() {
  const shell = window.ccwElectron;
  if (!shell?.isElectron) {
    return null;
  }

  const maximizeButton = document.getElementById("electron-maximize-button");
  document
    .getElementById("electron-minimize-button")
    ?.addEventListener("click", () => shell.minimize?.());
  maximizeButton?.addEventListener("click", () => shell.toggleMaximize?.());
  document
    .getElementById("electron-close-button")
    ?.addEventListener("click", () => shell.close?.());

  shell.onWindowState?.((state) => {
    const maximized = Boolean(state?.maximized);
    maximizeButton?.classList.toggle("active", maximized);
    maximizeButton?.setAttribute(
      "aria-label",
      maximized ? "Restore window" : "Maximize window"
    );
    maximizeButton?.setAttribute("title", maximized ? "Restore" : "Maximize");
  });
  shell.setTheme?.(currentViewerTheme());
  window.addEventListener("w-theme-change", (event) => {
    shell.setTheme?.(event.detail?.theme);
  });
  return shell;
}

function installHeaderControls(viewer, graphShell, appStateStore) {
  const toolPicker = document.getElementById("tool-picker");
  const toolButtons = Array.from(document.querySelectorAll("[data-tool-button]"));
  const referenceGridToggleButton = document.getElementById("reference-grid-toggle-button");
  const panelController = createPanelVisibilityController({
    appStateStore,
    viewer,
    graphShell,
  });
  const settingsController = createSettingsMenuController({
    appStateStore,
    viewer,
  });
  let lastToolValue = null;
  let renderingTools = false;

  const render = (state) => {
    const nextTools = state.tools || { orbit: false, pick: false };
    const nextTool = viewerToolsToPickerValue(nextTools);
    if (toolPicker && toolPicker.value !== nextTool) {
      renderingTools = true;
      toolPicker.value = nextTool;
      toolPicker.dispatchEvent(new Event("change", { bubbles: true }));
      renderingTools = false;
    }
    for (const button of toolButtons) {
      const active = Boolean(nextTools[button.dataset.toolButton]);
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
    const referenceGridVisible = Boolean(state.committedViewerState?.referenceGridVisible);
    referenceGridToggleButton?.classList.toggle("active", referenceGridVisible);
    referenceGridToggleButton?.setAttribute(
      "aria-pressed",
      String(referenceGridVisible)
    );

    if (lastToolValue !== nextTool) {
      lastToolValue = nextTool;
      window.dispatchEvent(
        new CustomEvent("w-viewer-tool-change", {
          detail: { tool: nextTool, tools: nextTools },
        })
      );
    }
  };

  for (const button of toolButtons) {
    button.addEventListener("click", () => {
      appStateStore.dispatch({ type: "tools/toggle", tool: button.dataset.toolButton });
    });
  }
  toolPicker?.addEventListener("change", () => {
    if (renderingTools) {
      return;
    }
    appStateStore.dispatch({
      type: "tools/set",
      tools: parseViewerToolValue(toolPicker.value),
    });
  });
  appStateStore.subscribe((state) => render(state));

  referenceGridToggleButton?.addEventListener("click", () => {
    try {
      const visible = Boolean(appStateStore.getState().committedViewerState?.referenceGridVisible);
      viewer.setReferenceGridVisible(!visible);
    } catch (error) {
      console.error("viewer reference grid toggle failed", error);
    }
  });

  return {
    activeTool: () => viewerToolsToPickerValue(appStateStore.getState().tools),
    activeTools: () => appStateStore.getState().tools,
    referenceGridVisible: () =>
      Boolean(appStateStore.getState().committedViewerState?.referenceGridVisible),
    setReferenceGridVisible: (visible) => viewer.setReferenceGridVisible(Boolean(visible)),
    toggleReferenceGrid: () => viewer.toggleReferenceGrid(),
    setGraphVisible: (visible) => panelController.setGraphVisible(visible),
    showGraph: () => panelController.showGraph(),
    hideGraph: () => panelController.hideGraph(),
    toggleGraph: () => panelController.toggleGraph(),
    setTerminalVisible: (visible) => panelController.setTerminalVisible(visible),
    showTerminal: () => panelController.showTerminal(),
    hideTerminal: () => panelController.hideTerminal(),
    toggleTerminal: () => panelController.toggleTerminal(),
    theme: () => settingsController.theme(),
    setTheme: (theme) => settingsController.setTheme(theme),
  };
}

function installViewerKeyboardFocus() {
  const canvas = document.getElementById("viewer-canvas");
  if (!canvas) {
    return null;
  }

  const activate = () => {
    window.dispatchEvent(new CustomEvent("w-viewer-keyboard-activate"));
    try {
      canvas.focus({ preventScroll: true });
    } catch (_error) {
      canvas.focus();
    }
  };

  canvas.addEventListener("pointerenter", activate);
  canvas.addEventListener("pointerdown", activate);

  return { activate };
}

function installProjectResourcePickerSupport({ appStateStore }) {
  const picker = document.getElementById("resource-picker");
  if (!picker) {
    return;
  }
  picker.addEventListener("change", (event) => {
    const nextResource = String(event?.target?.value || "").trim();
    if (isKnownResource(nextResource)) {
      appStateStore.dispatch({ type: "resource/requested", resource: nextResource });
    }
  });
}

function rewriteReplSource(source) {
  let rewritten = source
    .replace(REPL_GLOBAL_ASSIGN, "$1globalThis.$2 =")
    .replace(/(^|[=(,:]\s*|return\s+)ids\s*\(/gm, "$1await ids(")
    .replace(/(^|[=(,:]\s*|return\s+)queryIds\s*\(/gm, "$1await queryIds(")
    .replace(/(^|[=(,:]\s*|return\s+)query\s*\(/gm, "$1await query(")
    .replace(/(^|[=(,:]\s*|return\s+)hideQuery\s*\(/gm, "$1await hideQuery(")
    .replace(/(^|[=(,:]\s*|return\s+)showQuery\s*\(/gm, "$1await showQuery(")
    .replace(/(^|[=(,:]\s*|return\s+)selectQuery\s*\(/gm, "$1await selectQuery(");
  for (const method of GRAPH_REPL_CALLS) {
    const pattern = new RegExp(
      `(^|[=(,:]\\s*|return\\s+)graph\\\\.${method}\\s*\\(`,
      "gm"
    );
    rewritten = rewritten.replace(pattern, `$1await graph.${method}(`);
  }
  return rewritten;
}

function graphNodeColor(node, selected = false) {
  const palette = graphPalette();
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

function graphHoverRenderer(context, data, settings) {
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

async function importFirst(paths) {
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

async function loadGraphRendererModules() {
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
      importFirst(GRAPH_RENDERER_IMPORTS.sigma),
      importFirst(GRAPH_RENDERER_IMPORTS.graphology),
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

function createPropertyRow(label, value) {
  const fragment = document.createDocumentFragment();
  const dt = document.createElement("div");
  dt.className = "property-label";
  dt.textContent = label;
  const dd = document.createElement("div");
  dd.className = "property-value";
  dd.textContent = value;
  fragment.append(dt, dd);
  return fragment;
}

function createGraphShell(viewer, appStateStore) {
  const dom = {
    panelTabs: Array.from(document.querySelectorAll("[data-panel-tab]")),
    panelViews: Array.from(document.querySelectorAll("[data-panel-view]")),
    graphView: document.getElementById("graph-view"),
    graphFallbackList: document.getElementById("graph-fallback-list"),
    graphEmptyState: document.getElementById("graph-empty-state"),
    graphStatusLine: document.getElementById("graph-status-line"),
    graphClearButton: document.getElementById("graph-clear-button"),
    graphFocusButton: document.getElementById("graph-focus-button"),
    graphRelayoutButton: document.getElementById("graph-relayout-button"),
    graphModeButtons: Array.from(document.querySelectorAll("[data-graph-mode]")),
    viewport: document.querySelector(".viewport"),
    viewerCanvas: document.getElementById("viewer-canvas"),
    pickAnchorMarker: document.getElementById("pick-anchor-marker"),
    propertiesBalloon: document.getElementById("properties-balloon"),
    propertiesCloseButton: document.getElementById("properties-close-button"),
    propertiesGraphButton: document.getElementById("properties-graph-button"),
    propertiesTitle: document.getElementById("properties-title"),
    propertiesSubtitle: document.getElementById("properties-subtitle"),
    propertiesEmptyState: document.getElementById("properties-empty-state"),
    propertiesCoreSection: document.getElementById("properties-core-section"),
    propertiesCoreGrid: document.getElementById("properties-core-grid"),
    propertiesExtraSection: document.getElementById("properties-extra-section"),
    propertiesExtraGrid: document.getElementById("properties-extra-grid"),
    propertiesRelationsSection: document.getElementById("properties-relations-section"),
    propertiesRelationsList: document.getElementById("properties-relations-list"),
    resourcePicker: document.getElementById("resource-picker"),
    toolPicker: document.getElementById("tool-picker"),
  };

  const state = {
    activeTab: "graph",
    interactionTools: parseViewerToolValue(dom.toolPicker?.value || "none"),
    selectionOrigin: "none",
    mode: "semantic",
    nodes: [],
    edges: [],
    nodesById: new Map(),
    selectedNodeId: null,
    pickedSemanticId: null,
    pickedDbNodeId: null,
    pickedResource: null,
    pickDetailsRequestId: 0,
    activePickHasHit: false,
    propertiesBalloonDismissed: false,
    expandedNodeIds: new Set(),
    expansionPinnedNodeIds: new Set(),
    layoutPositions: new Map(),
    cameraState: null,
    controller: null,
    lastResetQuery: "",
    lastResource: safeViewerCurrentResource(viewer),
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

  const waitForAnimationFrames = (count = 2) =>
    new Promise((resolve) => {
      const step = (remaining) => {
        if (remaining <= 0) {
          resolve();
          return;
        }
        requestAnimationFrame(() => step(remaining - 1));
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
    sigmaResizeFrame = requestAnimationFrame(() => {
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
          if (Number.isFinite(resizeScale) && resizeScale > 0 && Math.abs(resizeScale - 1) > 0.001) {
            camera.setState({
              ...previousCameraState,
              ratio: Math.max(0.05, previousCameraState.ratio * resizeScale),
            });
          }
        }
        state.graphViewportSize = nextViewportSize;
        if (typeof state.sigma.refresh === "function") {
          state.sigma.refresh();
        } else if (typeof state.sigma.scheduleRefresh === "function") {
          state.sigma.scheduleRefresh();
        } else if (typeof state.sigma.scheduleRender === "function") {
          state.sigma.scheduleRender();
        }
      } catch (_error) {
        // Ignore transient resize errors while the panel is settling.
      }
    });
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
  };

  const applyHiddenPropertiesBalloon = () => {
    if (dom.propertiesBalloon) {
      dom.propertiesBalloon.hidden = true;
    }
    if (dom.pickAnchorMarker) {
      dom.pickAnchorMarker.hidden = true;
    }
  };

  const hidePropertiesBalloon = () => {
    appStateStore.dispatch({ type: "balloon/close" });
  };

  const dismissPropertiesBalloon = () => {
    state.propertiesBalloonDismissed = true;
    appStateStore.dispatch({ type: "balloon/dismiss" });
  };

  const hidePickAnchorMarker = () => {
    if (dom.pickAnchorMarker) {
      dom.pickAnchorMarker.hidden = true;
    }
  };

  const positionPropertiesBalloonAtClientPoint = (
    clientX,
    clientY,
    { anchored = false, marker = false } = {}
  ) => {
    if (!dom.propertiesBalloon || !dom.viewport) {
      return;
    }
    state.propertiesBalloonDismissed = false;
    const viewportRect = dom.viewport.getBoundingClientRect();
    dom.propertiesBalloon.hidden = false;
    const balloonRect = dom.propertiesBalloon.getBoundingClientRect();
    const padding = 12;
    const sideGap = 22;
    const leftBias = 18;
    const localX = clientX - viewportRect.left;
    const localY = clientY - viewportRect.top;

    if (marker && dom.pickAnchorMarker) {
      dom.pickAnchorMarker.hidden = false;
      dom.pickAnchorMarker.style.left = `${Math.round(localX)}px`;
      dom.pickAnchorMarker.style.top = `${Math.round(localY)}px`;
    } else {
      hidePickAnchorMarker();
    }

    const maxLeft = Math.max(padding, viewportRect.width - balloonRect.width - padding);
    const maxTop = Math.max(padding, viewportRect.height - balloonRect.height - padding);
    let side = "center";
    let left = localX - balloonRect.width / 2 - leftBias;
    if (anchored) {
      const rightLeft = localX + sideGap - leftBias;
      const leftLeft = localX - sideGap - balloonRect.width - leftBias;
      const rightFits = rightLeft + balloonRect.width <= viewportRect.width - padding;
      const leftFits = leftLeft >= padding;
      const rightSpace = viewportRect.width - localX - padding;
      const leftSpace = localX - padding;
      if (rightFits || (!leftFits && rightSpace >= leftSpace)) {
        side = "right";
        left = rightLeft;
      } else {
        side = "left";
        left = leftLeft;
      }
    }
    left = clamp(left, padding, maxLeft);
    dom.propertiesBalloon.style.left = `${Math.round(left)}px`;
    const top = clamp(localY - balloonRect.height / 2, padding, maxTop);
    dom.propertiesBalloon.style.top = `${Math.round(top)}px`;
    if (anchored) {
      const rightEdge = left + balloonRect.width;
      if (localX < left) {
        side = "right";
      } else if (localX > rightEdge) {
        side = "left";
      } else {
        side = localX - left <= rightEdge - localX ? "right" : "left";
      }
    }
    dom.propertiesBalloon.dataset.side = side;
    const arrowY = clamp(localY - top, 18, balloonRect.height - 18);
    dom.propertiesBalloon.style.setProperty("--arrow-y", `${Math.round(arrowY)}px`);
  };

  const showPropertiesBalloonAtClientPoint = (
    clientX,
    clientY,
    { anchored = false, marker = false, source = state.selectionOrigin || "none" } = {}
  ) => {
    appStateStore.dispatch({
      type: "balloon/open",
      source,
      anchor: {
        kind: "client",
        clientX,
        clientY,
        anchored,
        marker,
      },
    });
  };

  const showPropertiesBalloonAtViewportCenter = () => {
    if (!dom.viewport) {
      return;
    }
    const rect = dom.viewport.getBoundingClientRect();
    appStateStore.dispatch({
      type: "balloon/open",
      source: state.selectionOrigin || "none",
      anchor: {
        kind: "viewport-center",
        clientX: rect.left + rect.width / 2,
        clientY: rect.top + rect.height / 2,
        anchored: false,
        marker: false,
      },
    });
  };

  appStateStore.subscribe((app) => {
    const balloon = app.balloon || {};
    state.propertiesBalloonDismissed = Boolean(balloon.dismissed);
    if (!balloon.open || !balloon.anchor) {
      applyHiddenPropertiesBalloon();
      return;
    }
    const anchor = balloon.anchor || {};
    if (
      anchor.kind === "client" ||
      anchor.kind === "viewport-center" ||
      (Number.isFinite(anchor.clientX) && Number.isFinite(anchor.clientY))
    ) {
      positionPropertiesBalloonAtClientPoint(anchor.clientX, anchor.clientY, {
        anchored: Boolean(anchor.anchored),
        marker: Boolean(anchor.marker),
      });
    } else {
      applyHiddenPropertiesBalloon();
    }
  });

  const pickRegionClientCenter = (detail) => {
    const region = detail?.region;
    if (!region || !dom.viewerCanvas) {
      return null;
    }
    const rect = dom.viewerCanvas.getBoundingClientRect();
    const surfaceWidth = Math.max(1, dom.viewerCanvas.width || rect.width);
    const surfaceHeight = Math.max(1, dom.viewerCanvas.height || rect.height);
    return {
      x: rect.left + ((Number(region.x) + Number(region.width) / 2) / surfaceWidth) * rect.width,
      y: rect.top + ((Number(region.y) + Number(region.height) / 2) / surfaceHeight) * rect.height,
    };
  };

  const setStatus = (text, tone = "info") => {
    if (!dom.graphStatusLine) {
      return;
    }
    dom.graphStatusLine.textContent = text;
    dom.graphStatusLine.dataset.tone = tone;
  };

  const currentSelectedNode = () =>
    state.selectedNodeId ? state.nodesById.get(state.selectedNodeId) || null : null;

  const pickInteractionEnabled = () => Boolean(state.interactionTools?.pick);

  const graphSelectionShouldShowBalloon = () => pickInteractionEnabled();

  const syncInteractionTool = (tool, tools = null) => {
    state.interactionTools = tools || parseViewerToolValue(tool);
    if (!state.interactionTools.pick) {
      state.activePickHasHit = false;
      hidePickAnchorMarker();
      hidePropertiesBalloon();
    }
    return state.interactionTools;
  };

  const selectedRenderableId = () => {
    const node = currentSelectedNode();
    return node ? graphNodeSemanticId(node) : state.pickedSemanticId;
  };

  const selectedRenderableSourceResource = () => {
    const node = currentSelectedNode();
    return node ? graphNodeSourceResource(node) : state.pickedResource;
  };

  const setAppFocusFromGraphNode = (nodeId) => {
    const node = nodeId ? graphNodeForKey(nodeId) : null;
    if (!node) {
      appStateStore.dispatch({ type: "focus/clear" });
      return;
    }
    appStateStore.dispatch({
      type: "focus/set",
      source: "graph",
      graphNodeId: String(nodeId),
      dbNodeId: graphDbNodeIdForKey(nodeId),
      semanticId: graphNodeSemanticId(node),
      resource: graphNodeSourceResource(node) || state.lastResource || safeViewerCurrentResource(viewer),
    });
  };

  const setAppFocusFromPick = ({
    semanticId = state.pickedSemanticId,
    dbNodeId = state.pickedDbNodeId,
    resource = state.pickedResource || safeViewerCurrentResource(viewer),
  } = {}) => {
    if (!semanticId && dbNodeId === null) {
      appStateStore.dispatch({ type: "focus/clear" });
      return;
    }
    appStateStore.dispatch({
      type: "focus/set",
      source: "pick",
      semanticId,
      dbNodeId,
      resource,
    });
  };

  const cypherStringLiteral = (value) => {
    return `'${String(value).replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
  };

  const formatPropertyLabel = (label) => {
    return String(label || "")
      .replace(/_/g, " ")
      .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
      .replace(/\s+/g, " ")
      .trim();
  };

  const propertyValueText = (value) => {
    if (value === null || value === undefined || value === "") {
      return null;
    }
    return typeof value === "object" ? JSON.stringify(value) : String(value);
  };

  const renderPropertyRows = (rows) => {
    const fragments = rows
      .map(([label, value]) => [formatPropertyLabel(label), propertyValueText(value)])
      .filter(([, value]) => value !== null)
      .map(([label, value]) => createPropertyRow(label, value));
    dom.propertiesCoreGrid.replaceChildren(...fragments);
    dom.propertiesCoreSection.hidden = fragments.length === 0;
    dom.propertiesExtraGrid.replaceChildren();
    dom.propertiesExtraSection.hidden = true;
    dom.propertiesRelationsList.replaceChildren();
    dom.propertiesRelationsSection.hidden = true;
    return fragments.length;
  };

  const resetPropertySections = () => {
    dom.propertiesCoreGrid.replaceChildren();
    dom.propertiesCoreSection.hidden = true;
    dom.propertiesExtraGrid.replaceChildren();
    dom.propertiesExtraSection.hidden = true;
    dom.propertiesRelationsList.replaceChildren();
    dom.propertiesRelationsSection.hidden = true;
  };

  const setPickedGraphButtonVisible = (visible) => {
    if (dom.propertiesGraphButton) {
      dom.propertiesGraphButton.hidden = !visible;
    }
  };

  const pickedElementLookupTarget = (semanticId) => {
    const scoped = parseSourceScopedSemanticId(semanticId);
    if (scoped) {
      return {
        resource: scoped.sourceResource,
        semanticId: scoped.semanticId,
      };
    }
    const resource = safeViewerCurrentResource(viewer);
    if (!semanticId || !isIfcResource(resource)) {
      return null;
    }
    return {
      resource,
      semanticId: String(semanticId),
    };
  };

  const findPickedElementNode = async (semanticId) => {
    const target = pickedElementLookupTarget(semanticId);
    if (!target) {
      return null;
    }
    const cypher = [
      `MATCH (n) WHERE n.GlobalId = ${cypherStringLiteral(target.semanticId)}`,
      "RETURN id(n) AS node_id LIMIT 1",
    ].join(" ");
    const payload = await viewer.queryCypher(cypher, target.resource);
    const dbNodeIds = extractDbNodeIdsFromCypherPayload(payload);
    if (dbNodeIds[0] === undefined || dbNodeIds[0] === null) {
      return null;
    }
    return {
      ...target,
      dbNodeId: dbNodeIds[0],
    };
  };

  const renderPickedElementDetails = (hit, details, lookup) => {
    const resource = lookup?.resource || safeViewerCurrentResource(viewer);
    const node = details?.node || {};
    const entity = node.declaredEntity || "IFC element";
    const ifcId = node.globalId || lookup?.semanticId || hit?.elementId || "";
    const properties = details?.properties && typeof details.properties === "object"
      ? details.properties
      : {};
    const hiddenKeys = new Set([
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

    dom.propertiesTitle.textContent = entity;
    dom.propertiesSubtitle.textContent = ifcId
      ? `id: ${ifcId}${resource ? ` in ${resource}` : ""}`
      : "id unavailable";
    dom.propertiesEmptyState.hidden = true;
    state.pickedDbNodeId = lookup?.dbNodeId || node.dbNodeId || null;
    state.pickedResource = resource;
    setAppFocusFromPick({
      semanticId: ifcId || hit?.elementId || null,
      dbNodeId: state.pickedDbNodeId,
      resource,
    });
    setPickedGraphButtonVisible(Boolean(state.pickedDbNodeId));

    const rows = [];
    if (node.name) {
      rows.push(["Name", node.name]);
    }
    for (const [key, value] of Object.entries(properties)) {
      if (hiddenKeys.has(key)) {
        continue;
      }
      rows.push([key, value]);
    }

    const rendered = renderPropertyRows(rows);
    if (rendered === 0) {
      resetPropertySections();
      dom.propertiesEmptyState.hidden = false;
      dom.propertiesEmptyState.textContent = `No scalar properties found for ${entity}.`;
    }
  };

  const loadPickedElementDetails = async (hit, requestId) => {
    try {
      const lookup = await findPickedElementNode(hit.elementId);
      if (requestId !== state.pickDetailsRequestId) {
        return;
      }
      if (!lookup?.dbNodeId) {
        dom.propertiesTitle.textContent = "IFC element";
        dom.propertiesSubtitle.textContent = hit.elementId ? `id: ${hit.elementId}` : "id unavailable";
        resetPropertySections();
        dom.propertiesEmptyState.hidden = false;
        dom.propertiesEmptyState.textContent = "No semantic IFC node was found for this pick.";
        state.pickedDbNodeId = null;
        state.pickedResource = null;
        setAppFocusFromPick({ semanticId: hit.elementId || null, dbNodeId: null, resource: lookup?.resource || null });
        setPickedGraphButtonVisible(false);
        return;
      }
      const details = await viewer.queryGraphNodeProperties(
        lookup.dbNodeId,
        { maxRelations: 1 },
        lookup.resource
      );
      if (requestId !== state.pickDetailsRequestId) {
        return;
      }
      renderPickedElementDetails(hit, details, lookup);
    } catch (error) {
      if (requestId !== state.pickDetailsRequestId) {
        return;
      }
      dom.propertiesTitle.textContent = "IFC element";
      dom.propertiesSubtitle.textContent = hit.elementId ? `id: ${hit.elementId}` : "id unavailable";
      resetPropertySections();
      dom.propertiesEmptyState.hidden = false;
      dom.propertiesEmptyState.textContent = `Could not load IFC properties: ${error.message || error}`;
      state.pickedDbNodeId = null;
      state.pickedResource = null;
      setAppFocusFromPick({ semanticId: hit.elementId || null, dbNodeId: null });
      setPickedGraphButtonVisible(false);
    }
  };

  const mergeGraphRelations = (left = [], right = []) => {
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
  };

  const mergeGraphNodes = (left = [], right = []) => {
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
  };

  const mergeGraphEdges = (left = [], right = []) => {
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

  const graphResourceForKey = (nodeId, fallback = safeViewerCurrentResource(viewer)) => {
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

  const renderProperties = () => {
    state.activePickHasHit = false;
    state.pickedDbNodeId = null;
    state.pickedResource = null;
    setPickedGraphButtonVisible(false);
    const node = currentSelectedNode();
    if (!node) {
      dom.propertiesTitle.textContent = "No graph node selected";
      dom.propertiesSubtitle.textContent =
        "Pick an object or select a graph node to inspect IFC properties.";
      dom.propertiesEmptyState.hidden = false;
      dom.propertiesEmptyState.textContent = "Pick an object in the model to inspect its IFC properties.";
      resetPropertySections();
      syncActionButtons();
      return;
    }

    state.pickedSemanticId = null;
    state.pickedDbNodeId = null;
    state.pickedResource = null;
    setPickedGraphButtonVisible(false);
    dom.propertiesTitle.textContent = graphNodeText(node);
    const currentResource =
      graphNodeSourceResource(node) || safeViewerCurrentResource(viewer);
    dom.propertiesSubtitle.textContent = currentResource
      ? `${graphNodeEntity(node)} in ${currentResource}`
      : `${graphNodeEntity(node)} while the viewer is still starting`;
    dom.propertiesEmptyState.hidden = true;

    const coreRows = [
      ["Entity", graphNodeEntity(node)],
      ["DB node id", graphNodeKey(node)],
    ];
    const globalId = tryGetFirst(node, ["globalId"]);
    if (globalId) {
      coreRows.push(["GlobalId", String(globalId)]);
    }
    const degree = tryGetFirst(node, ["degree"]);
    if (degree !== null) {
      coreRows.push(["Degree", String(degree)]);
    }
    dom.propertiesCoreGrid.replaceChildren(...coreRows.map(([label, value]) => createPropertyRow(label, value)));
    dom.propertiesCoreSection.hidden = coreRows.length === 0;

    const extraProperties = tryGetFirst(node, ["properties", "extraProperties", "attrs"]);
    const extraRows = [];
    if (extraProperties && typeof extraProperties === "object") {
      for (const [key, value] of Object.entries(extraProperties)) {
        if (value === null || value === undefined || value === "") {
          continue;
        }
        extraRows.push(createPropertyRow(key, typeof value === "object" ? JSON.stringify(value) : String(value)));
      }
    }
    dom.propertiesExtraGrid.replaceChildren(...extraRows);
    dom.propertiesExtraSection.hidden = extraRows.length === 0;

    const relations = Array.isArray(node.relations) ? node.relations : [];
    dom.propertiesRelationsList.replaceChildren();
    for (const relation of relations) {
      const row = document.createElement("div");
      row.className = "relation-row";
      const title = document.createElement("strong");
      title.textContent = tryGetFirst(relation, ["type", "label", "name"]) || "Relation";
      const detail = document.createElement("span");
      detail.textContent =
        tryGetFirst(relation, ["targetLabel", "target", "description", "to"]) || "";
      row.append(title, detail);
      dom.propertiesRelationsList.append(row);
    }
    dom.propertiesRelationsSection.hidden = relations.length === 0;
    syncActionButtons();
  };

  const renderPickProperties = (detail) => {
    const hits = Array.isArray(detail?.hits) ? detail.hits : [];
    const hit = hits[0] || null;
    const requestId = state.pickDetailsRequestId + 1;
    state.pickDetailsRequestId = requestId;
    state.selectionOrigin = "pick";
    state.activePickHasHit = Boolean(hit);
    state.selectedNodeId = null;
    state.pickedSemanticId = hit?.elementId ? String(hit.elementId) : null;
    state.pickedDbNodeId = null;
    state.pickedResource = null;
    setPickedGraphButtonVisible(false);
    applySelectionToSigma(null);
    renderFallbackList();

    if (!hit) {
      dom.propertiesTitle.textContent = "No element picked";
      dom.propertiesSubtitle.textContent = "No visible instance was found at that pixel.";
      dom.propertiesEmptyState.hidden = false;
      dom.propertiesEmptyState.textContent = "No visible IFC element was found at that pixel.";
      resetPropertySections();
      syncActionButtons();
      hidePropertiesBalloon();
      hidePickAnchorMarker();
      appStateStore.dispatch({ type: "focus/clear" });
      return false;
    }

    setAppFocusFromPick({ semanticId: state.pickedSemanticId, dbNodeId: null });
    dom.propertiesTitle.textContent = "Loading IFC properties";
    dom.propertiesSubtitle.textContent = hit.elementId ? `id: ${hit.elementId}` : "id unavailable";
    dom.propertiesEmptyState.hidden = true;
    resetPropertySections();
    void loadPickedElementDetails(hit, requestId);
    syncActionButtons();
    return true;
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
      const row = document.createElement("button");
      row.type = "button";
      row.className = "graph-fallback-row";
      if (graphNodeKey(node) === state.selectedNodeId) {
        row.classList.add("active");
      }
      row.dataset.nodeId = graphNodeKey(node);
      const title = document.createElement("strong");
      title.textContent = graphNodeText(node);
      const detail = document.createElement("span");
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
    const positions = new Map();
    if (
      state.graphModel &&
      typeof state.graphModel.getNodeAttributes === "function"
    ) {
      for (const node of state.nodes) {
        const key = graphNodeKey(node);
        try {
          const attributes = state.graphModel.getNodeAttributes(key);
          if (
            attributes &&
            Number.isFinite(attributes.x) &&
            Number.isFinite(attributes.y)
          ) {
            positions.set(key, { x: attributes.x, y: attributes.y });
          }
        } catch (_error) {
          // Ignore missing graph nodes while capturing layout.
        }
      }
    }

    let cameraState = null;
    if (
      state.sigma &&
      typeof state.sigma.getCamera === "function"
    ) {
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

  const computeStableGraphLayout = (nodes, edges, previousPositions = new Map()) => {
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
          const angle = (index % 12) / 12 * Math.PI * 2 + pass * 0.37;
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
    if (refresh && state.sigma && typeof state.sigma.refresh === "function") {
      state.sigma.refresh();
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
    if (
      !state.graphModel ||
      typeof state.graphModel.getNodeAttributes !== "function"
    ) {
      return;
    }
    const nextPositions = new Map();
    for (const [index, node] of state.nodes.entries()) {
      const key = graphNodeKey(node, index);
      try {
        const attributes = state.graphModel.getNodeAttributes(key);
        if (
          attributes &&
          Number.isFinite(attributes.x) &&
          Number.isFinite(attributes.y)
        ) {
          nextPositions.set(key, { x: attributes.x, y: attributes.y });
        }
      } catch (_error) {
        // Ignore graph nodes not present yet.
      }
    }
    state.layoutPositions = nextPositions;
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
    const palette = graphPalette();
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
          graphNodeColor(node, key === state.selectedNodeId)
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

  window.addEventListener("w-theme-change", () => applyGraphTheme());

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
        if (
          state.graphModel.hasEdge &&
          !state.graphModel.hasEdge(key)
        ) {
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
      const emphasized = selected ||
        graphRelationshipDotTouchesSelected(node, state.selectedNodeId);
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
          color: graphNodeColor(node, selected),
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
          graphNodeColor(node, selected)
        );
        state.graphModel.setNodeAttribute(
          key,
          "zIndex",
          graphNodeZIndex(node, selected)
        );
      }
    }

    const edgePositions = graphNodePositionsFromModel(state.graphModel, state.nodes);
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
      const palette = graphPalette();
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
      const palette = graphPalette();
      const graph = new state.renderer.GraphConstructor({ multi: true });
      const layout = computeStableGraphLayout(state.nodes, state.edges, previousPositions);
      for (const [index, node] of state.nodes.entries()) {
        const key = graphNodeKey(node, index);
        const position = layout.get(key) || graphNodePosition(node, index, state.nodes.length);
        const selected = key === state.selectedNodeId;
        const emphasized = selected ||
          graphRelationshipDotTouchesSelected(node, state.selectedNodeId);
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
          color: graphNodeColor(node, selected),
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
      renderProperties();
    }

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
      safeViewerCurrentResource(viewer)
    );
    if (!semanticId) {
      if (action === "select") {
        viewer.clearSelection();
      }
      return null;
    }
    if (action === "select") {
      viewer.clearSelection();
      viewer.select([semanticId]);
    } else if (action === "hide") {
      viewer.hide([semanticId]);
    } else if (action === "show") {
      viewer.show([semanticId]);
    }
    return semanticId;
  };

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
        appStateStore.dispatch({ type: "focus/clear" });
      }
      const preserveProperties = options.preserveProperties === true;
      if (!preserveProperties) {
        state.activePickHasHit = false;
        state.pickedSemanticId = null;
        state.pickedDbNodeId = null;
        state.pickedResource = null;
        setPickedGraphButtonVisible(false);
        hidePickAnchorMarker();
      }
      renderFallbackList();
      applySelectionToSigma(state.selectedNodeId);
      if (!preserveProperties) {
        renderProperties();
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
            showPropertiesBalloonAtClientPoint(options.clientX, options.clientY);
          } else {
            showPropertiesBalloonAtViewportCenter();
          }
        } else {
          hidePropertiesBalloon();
        }
      } else if (state.selectionOrigin === "graph" || !state.selectedNodeId) {
        if (!preserveProperties) {
          hidePropertiesBalloon();
        }
      }
      if (options.syncViewer) {
        performSelectedViewerAction("select");
      }
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
        renderProperties();
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
        renderProperties();
      }
      return node;
    },
    setStatus,
    clear(options = {}) {
      state.nodes = [];
      state.edges = [];
      state.nodesById = new Map();
      state.selectedNodeId = null;
      state.pickedSemanticId = null;
      state.pickedDbNodeId = null;
      state.pickedResource = null;
      setPickedGraphButtonVisible(false);
      appStateStore.dispatch({ type: "focus/clear" });
      state.expandedNodeIds = new Set();
      state.layoutPositions = new Map();
      state.cameraState = null;
      disposeSigma();
      syncGraphSurface();
      renderProperties();
      hidePropertiesBalloon();
      if (!options.silent) {
        setStatus(
          `Graph cleared for ${safeViewerCurrentResource(viewer) || "the current resource"}. Use graph.reset(...) to seed a new view.`
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
        `Resetting graph from ${safeViewerCurrentResource(viewer) || "the current resource"}…`
      );
      if (!state.controller || typeof state.controller.reset !== "function") {
        setStatus(
          "Graph shell is ready. Install a graph controller to connect graph.reset(...) to backend data.",
          "warn"
        );
        return {
          pendingIntegration: true,
          resource: safeViewerCurrentResource(viewer),
          mode: state.mode,
          cypher: state.lastResetQuery,
        };
      }
      let result;
      try {
        result = await state.controller.reset({
          cypher: state.lastResetQuery,
          resource: safeViewerCurrentResource(viewer),
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
      window.wHeader?.showGraph?.();
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
      requestAnimationFrame(() => {
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
      return {
        resource: safeViewerCurrentResource(viewer),
        mode: state.mode,
        activeTab: state.activeTab,
        selectedNodeId: state.selectedNodeId,
        nodes: state.nodes.length,
        edges: state.edges.length,
        lastResetQuery: state.lastResetQuery,
      };
    },
    getSelectedNode() {
      return currentSelectedNode();
    },
  };

  for (const tab of dom.panelTabs) {
    tab.addEventListener("click", () => setActiveTab(tab.dataset.panelTab));
  }

  for (const button of dom.graphModeButtons) {
    button.addEventListener("click", () => {
      void api.mode(button.dataset.graphMode);
    });
  }

  dom.graphClearButton?.addEventListener("click", () => {
    void api.clear();
  });

  dom.graphFocusButton?.addEventListener("click", () => {
    void api.focusSelected();
  });

  dom.graphRelayoutButton?.addEventListener("click", () => {
    api.relayout();
    dom.graphRelayoutButton.blur();
  });

  dom.propertiesGraphButton?.addEventListener("click", () => {
    const nodeId = state.pickedDbNodeId || state.selectedNodeId;
    const resource =
      state.pickedResource ||
      graphResourceForKey(state.selectedNodeId) ||
      safeViewerCurrentResource(viewer);
    if (!nodeId) {
      setStatus("Pick an IFC element before opening its graph neighborhood.", "warn");
      return;
    }
    void api.seedFromNode(nodeId, {
      resource,
      preserveProperties: Boolean(state.activePickHasHit),
      revealProperties: false,
    }).catch((error) => {
      setStatus(`Graph lookup failed: ${error.message || error}`, "error");
    });
  });

  dom.propertiesCloseButton?.addEventListener("click", dismissPropertiesBalloon);

  window.addEventListener("w-viewer-drag-start", () => {
    document.body.classList.add("viewer-dragging");
  });
  window.addEventListener("w-viewer-drag-end", () => {
    document.body.classList.remove("viewer-dragging");
  });

  window.addEventListener("w-viewer-tool-change", (event) => {
    syncInteractionTool(event.detail?.tool, event.detail?.tools);
  });
  dom.toolPicker?.addEventListener("change", () => {
    syncInteractionTool(dom.toolPicker.value);
  });
  syncInteractionTool(dom.toolPicker?.value);

  window.addEventListener("w-viewer-pick", (event) => {
    if (!pickInteractionEnabled()) {
      return;
    }
    const hasHit = renderPickProperties(event.detail || {});
    if (!hasHit) {
      return;
    }
    const anchor = pickRegionClientCenter(event.detail || {});
    if (anchor) {
      showPropertiesBalloonAtClientPoint(anchor.x, anchor.y);
    } else {
      showPropertiesBalloonAtViewportCenter();
    }
  });

  window.addEventListener("w-viewer-anchor", (event) => {
    const detail = event.detail || {};
    if (!pickInteractionEnabled()) {
      hidePickAnchorMarker();
      return;
    }
    if (!detail.visible) {
      hidePickAnchorMarker();
      if (state.activePickHasHit) {
        hidePropertiesBalloon();
      }
      return;
    }
    if (state.propertiesBalloonDismissed) {
      hidePickAnchorMarker();
      return;
    }
    showPropertiesBalloonAtClientPoint(detail.clientX, detail.clientY, {
      anchored: true,
      marker: true,
    });
  });

  window.addEventListener("w-viewer-state-change", (event) => {
    const nextResource = event.detail?.state?.resource || safeViewerCurrentResource(viewer);
    if (nextResource !== state.lastResource) {
      state.lastResource = nextResource;
      api.clear({ silent: true });
      setStatus(
        `Graph cleared for ${nextResource}. Use graph.reset(...) to seed the new resource.`
      );
    }
  });

  void loadGraphRendererModules().then((renderer) => {
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

  renderProperties();
    syncActionButtons();

  if (dom.graphView) {
    if (typeof ResizeObserver !== "undefined") {
      const resizeObserver = new ResizeObserver(() => {
        requestGraphResize();
      });
      resizeObserver.observe(dom.graphView);
    } else {
      window.addEventListener("resize", requestGraphResize);
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
      currentResource: () => safeViewerCurrentResource(viewer),
    },
  };
}

function normalizedColumnName(value) {
  return String(value || "")
    .replace(/[^a-zA-Z0-9]/g, "")
    .toLowerCase();
}

function findCypherColumnIndex(columns, candidates) {
  const normalizedCandidates = candidates.map(normalizedColumnName);
  return columns.findIndex((column) =>
    normalizedCandidates.includes(normalizedColumnName(column))
  );
}

function extractDbNodeIdsFromCypherPayload(payload) {
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

function createGraphController(viewer) {
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

function createReplApi(viewer, graph) {
  const currentQueryResource = (resource = viewer.currentResource()) => {
    if (!isKnownResource(resource)) {
      throw new Error(
        `Current resource \`${resource}\` is not an IFC model or project.`
      );
    }
    return resource;
  };

  const api = {
    viewer,
    graph,
    resource: () => viewer.currentResource(),
    profile: () => viewer.profile(),
    profiles: () => viewer.profiles(),
    referenceGridVisible: () => viewer.referenceGridVisible(),
    setReferenceGridVisible: (visible) => viewer.setReferenceGridVisible(visible),
    toggleReferenceGrid: () => viewer.toggleReferenceGrid(),
    theme: () => viewer.theme(),
    viewState: () => viewer.viewState(),
    state: () => viewer.state(),
    setProfile: (profile) => viewer.setProfile(profile),
    setTheme: (theme) => viewer.setTheme(theme),
    setViewMode: (mode) => viewer.setViewMode(mode),
    defaultView: () => viewer.defaultView(),
    allView: () => viewer.allView(),
    setViewModeAsync: (mode) => viewer.async.setViewMode(mode),
    defaultViewAsync: () => viewer.async.defaultView(),
    allViewAsync: () => viewer.async.allView(),
    pickAt: (x, y) => viewer.pickAt(x, y),
    pickRect: (x0, y0, x1, y1) => viewer.pickRect(x0, y0, x1, y1),
    pickAtAsync: (x, y) => viewer.async.pickAt(x, y),
    pickRectAsync: (x0, y0, x1, y1) => viewer.async.pickRect(x0, y0, x1, y1),
    listIds: () => viewer.listElementIds(),
    visibleIds: () => viewer.visibleElementIds(),
    selectedIds: () => viewer.selectedElementIds(),
    inspectedIds: () => viewer.inspectedElementIds(),
    selectedInstanceIds: () => viewer.selectedInstanceIds(),
    query: async (cypher, resource = currentQueryResource()) =>
      viewer.queryCypher(cypher, resource),
    queryIds: async (cypher, resource = currentQueryResource()) =>
      viewer.queryIds(cypher, resource),
    ids: async (cypher, resource = currentQueryResource()) =>
      viewer.queryIds(cypher, resource),
    hide: (ids, options = {}) => viewer.hide(ids, options),
    show: (ids, options = {}) => viewer.show(ids, options),
    select: (ids, options = {}) => viewer.select(ids, options),
    inspect: (ids, options = {}) => viewer.inspect(ids, options),
    addInspection: (ids, options = {}) => viewer.addInspection(ids, options),
    removeInspection: (ids, options = {}) => viewer.removeInspection(ids, options),
    resetVisibility: (ids, options = {}) =>
      Array.isArray(ids) ? viewer.resetVisibility(ids, options) : viewer.resetAllVisibility(),
    clearSelection: () => viewer.clearSelection(),
    clearInspection: () => viewer.clearInspection(),
    frame: () => viewer.frameVisible(),
    frameVisible: () => viewer.frameVisible(),
    resizeViewport: () => viewer.resizeViewport(),
    hideQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.hide(ids, { sourceResource: resource });
      return ids;
    },
    showQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.show(ids, { sourceResource: resource });
      return ids;
    },
    selectQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.select(ids, { sourceResource: resource });
      return ids;
    },
    inspectQuery: async (cypher, resource = currentQueryResource(), options = {}) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.inspect(ids, { ...options, sourceResource: resource });
      return ids;
    },
  };

  return api;
}

function installAgentTerminal(viewer, graph) {
  const state = {
    sessionId: null,
    resource: null,
    schemaId: null,
    schemaSlug: null,
    bindingPromise: null,
    capabilities: null,
    capabilitiesPromise: null,
    backendId: null,
    modelId: null,
    levelId: null,
    activated: false,
  };
  const modelSelect = document.getElementById("agent-model-select");
  const levelSelect = document.getElementById("agent-level-select");
  const levelControl = levelSelect?.closest("label");
  const agentClient = createAgentClient();
  const agentActions = createAgentActionApplier({
    viewer,
    graph,
    isKnownResource,
    isIfcResource,
    safeViewerCurrentResource,
    mapGraphSubgraphResponse,
    revealGraphPanel: async () => {
      window.wHeader?.showGraph?.();
      await new Promise((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(resolve);
        });
      });
    },
  });

  const currentAgentResource = (resource = null) =>
    selectedAgentResource(viewer, resource);

  const selectedBackendCapability = () => {
    const backends = Array.isArray(state.capabilities?.backends)
      ? state.capabilities.backends
      : [];
    return (
      backends.find((backend) => backend.id === state.backendId) ||
      backends.find((backend) => backend.id === state.capabilities?.defaultBackendId) ||
      backends[0] ||
      null
    );
  };

  const renderCapabilitySelect = (select, options, selectedId, fallbackLabel) => {
    if (!select) {
      return;
    }
    select.innerHTML = "";
    const safeOptions = Array.isArray(options) ? options : [];
    if (!safeOptions.length) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = fallbackLabel;
      select.appendChild(option);
      select.disabled = true;
      return;
    }
    for (const item of safeOptions) {
      const option = document.createElement("option");
      option.value = String(item.id || "");
      option.textContent = String(item.label || item.id || "unknown");
      select.appendChild(option);
    }
    select.value = safeOptions.some((item) => item.id === selectedId)
      ? selectedId
      : String(safeOptions[0].id || "");
    select.disabled = safeOptions.length <= 1;
  };

  const renderAgentCapabilityControls = () => {
    const backend = selectedBackendCapability();
    if (!backend) {
      renderCapabilitySelect(modelSelect, [], null, "no models");
      renderCapabilitySelect(levelSelect, [], null, "no levels");
      if (levelControl) {
        levelControl.hidden = true;
      }
      return;
    }
    state.backendId = backend.id;
    const models = Array.isArray(backend.models) ? backend.models : [];
    state.modelId =
      models.find((item) => item.id === state.modelId)?.id ||
      state.capabilities?.defaultModelId ||
      models[0]?.id ||
      null;
    const levelsByModel =
      backend.levelsByModel && typeof backend.levelsByModel === "object"
        ? backend.levelsByModel
        : {};
    const modelLevels = Array.isArray(levelsByModel[state.modelId])
      ? levelsByModel[state.modelId]
      : [];
    const backendLevels = Array.isArray(backend.levels) ? backend.levels : [];
    const levels = modelLevels.length ? modelLevels : backendLevels;
    const defaultLevelId = levels.some((item) => item.id === state.capabilities?.defaultLevelId)
      ? state.capabilities.defaultLevelId
      : levels[Math.floor(Math.max(levels.length - 1, 0) / 2)]?.id || null;
    state.levelId =
      levels.find((item) => item.id === state.levelId)?.id ||
      defaultLevelId ||
      null;
    renderCapabilitySelect(modelSelect, models, state.modelId, "no models");
    renderCapabilitySelect(levelSelect, levels, state.levelId, "no levels");
    if (levelControl) {
      levelControl.hidden = !levels.length;
    }
  };

  const loadAgentCapabilities = async () => {
    if (state.capabilities) {
      renderAgentCapabilityControls();
      return state.capabilities;
    }
    if (!state.capabilitiesPromise) {
      state.capabilitiesPromise = getJson("/api/agent/capabilities")
        .then((payload) => {
          state.capabilities = payload;
          state.backendId = payload.defaultBackendId || state.backendId;
          state.modelId = payload.defaultModelId || state.modelId;
          state.levelId = payload.defaultLevelId || state.levelId;
          renderAgentCapabilityControls();
          return payload;
        })
        .catch((error) => {
          renderCapabilitySelect(modelSelect, [], null, "unavailable");
          renderCapabilitySelect(levelSelect, [], null, "unavailable");
          throw error;
        })
        .finally(() => {
          state.capabilitiesPromise = null;
        });
    }
    return state.capabilitiesPromise;
  };

  modelSelect?.addEventListener("change", () => {
    state.modelId = modelSelect.value || null;
    state.levelId = null;
    renderAgentCapabilityControls();
  });
  levelSelect?.addEventListener("change", () => {
    state.levelId = levelSelect.value || null;
  });

  const bindSession = async ({ resource = currentAgentResource(), force = false } = {}) => {
    if (!resource) {
      state.sessionId = null;
      state.resource = null;
      return {
        sessionId: null,
        resource: null,
        schemaId: null,
        schemaSlug: null,
        transcript: [],
      };
    }

    if (!force && state.sessionId && state.resource === resource) {
      return {
        sessionId: state.sessionId,
        resource: state.resource,
        schemaId: state.schemaId,
        schemaSlug: state.schemaSlug,
        transcript: [],
        reused: true,
      };
    }

    if (!force && state.bindingPromise) {
      return state.bindingPromise;
    }

    const previousSessionId = state.sessionId;
    const previousResource = state.resource;
    const request = agentClient.session({
      sessionId: previousSessionId,
      resource,
    }).then((payload) => {
      const nextSessionId = extractAgentSessionId(payload);
      if (!nextSessionId) {
        throw new Error("Agent session response did not include a session id.");
      }
      state.sessionId = nextSessionId;
      state.resource = String(tryGetFirst(payload, ["resource"]) || resource);
      state.schemaId = extractAgentSchemaId(payload);
      state.schemaSlug = extractAgentSchemaSlug(payload);
      return {
        sessionId: state.sessionId,
        resource: state.resource,
        schemaId: state.schemaId,
        schemaSlug: state.schemaSlug,
        transcript: extractAgentTranscriptItems(payload),
        created: Boolean(tryGetFirst(payload, ["created"])),
        rebound: Boolean(tryGetFirst(payload, ["rebound"])) || previousResource !== state.resource,
        previousSessionId,
      };
    });
    state.bindingPromise = request.finally(() => {
      if (state.bindingPromise === request) {
        state.bindingPromise = null;
      }
    });
    return state.bindingPromise;
  };

  let shell = null;

  const writeResourceNotice = async (resource, { forceRebind = false } = {}) => {
    if (!shell) {
      return;
    }
    if (!resource) {
      state.sessionId = null;
      state.resource = null;
      state.schemaId = null;
      state.schemaSlug = null;
      shell.writeLine("AI context cleared. Select an IFC model or project to use the agent terminal.");
      return;
    }
    try {
      const session = await bindSession({ resource, force: forceRebind });
      renderAgentTranscriptItems(shell, compactAgentTranscriptItems(session.transcript));
      shell.writeLine(
        session.schemaId
          ? `AI context switched to ${session.resource} (${session.schemaId}).`
          : `AI context switched to ${session.resource}.`
      );
    } catch (error) {
      shell.writeLine(`AI context bind failed: ${error}`);
    }
  };

  const baseShell = installLineTerminal({
    screenId: "repl-screen",
    hostId: "agent-terminal",
    introLines: [
      "AI agent terminal.",
      "The agent is bound to the active IFC project/model and can only request read-only Cypher plus validated viewer actions.",
      "Enter runs. Up/Down walks history. Ctrl+C clears the line.",
    ],
    execute: async (code, terminal) => {
      const resource = currentAgentResource();
      if (!resource) {
        terminal.writeLine("AI agent is available only while an IFC model or project is selected.");
        return TERMINAL_NO_RESULT;
      }

      await loadAgentCapabilities();
      const session = await bindSession({ resource });
      if (!session.reused) {
        renderAgentTranscriptItems(
          terminal,
          compactAgentTranscriptItems(session.transcript)
        );
      }

      const startPayload = await agentClient.turnStart({
        sessionId: session.sessionId,
        resource: session.resource,
        input: code,
        prompt: code,
        backendId: state.backendId,
        modelId: state.modelId,
        levelId: state.levelId,
      });

      const turnId = extractAgentTurnId(startPayload);
      if (!turnId) {
        throw new Error("Agent turn did not return a turn id.");
      }

      const startedSessionId = extractAgentSessionId(startPayload);
      if (startedSessionId) {
        state.sessionId = startedSessionId;
      }
      state.resource = String(
        tryGetFirst(startPayload, ["resource"]) || session.resource || resource
      );
      state.schemaId = extractAgentSchemaId(startPayload) || session.schemaId || state.schemaId;
      state.schemaSlug = extractAgentSchemaSlug(startPayload) || session.schemaSlug || state.schemaSlug;
      state.backendId = String(tryGetFirst(startPayload, ["backendId"]) || state.backendId || "");
      state.modelId = String(tryGetFirst(startPayload, ["modelId"]) || state.modelId || "");
      state.levelId = String(tryGetFirst(startPayload, ["levelId"]) || state.levelId || "");
      renderAgentCapabilityControls();

      let afterSeq = 0;
      let resultPayload = null;
      while (true) {
        const pollPayload = await agentClient.turnPoll({
          turnId,
          afterSeq,
        });
        const events = extractAgentTranscriptItems(pollPayload);
        if (Array.isArray(pollPayload?.events) && pollPayload.events.length) {
          const lastEvent = pollPayload.events[pollPayload.events.length - 1];
          const seq = Number(lastEvent?.seq);
          if (Number.isFinite(seq)) {
            afterSeq = Math.max(afterSeq, seq);
          }
        }
        renderAgentTranscriptItems(terminal, events);
        if (Boolean(tryGetFirst(pollPayload, ["done"]))) {
          const pollError = extractAgentTurnError(pollPayload);
          if (pollError) {
            throw new Error(pollError);
          }
          resultPayload = extractAgentTurnResult(pollPayload);
          break;
        }
        await sleep(150);
      }

      if (!resultPayload) {
        return TERMINAL_NO_RESULT;
      }

      const nextSessionId = extractAgentSessionId(resultPayload);
      if (nextSessionId) {
        state.sessionId = nextSessionId;
      }
      state.resource = String(
        tryGetFirst(resultPayload, ["resource"]) || session.resource || resource
      );
      state.schemaId = extractAgentSchemaId(resultPayload) || session.schemaId || state.schemaId;
      state.schemaSlug = extractAgentSchemaSlug(resultPayload) || session.schemaSlug || state.schemaSlug;

      const actions = extractAgentActions(resultPayload);
      const appliedKinds = [];
      for (const action of actions) {
        try {
          const appliedKind = await agentActions.apply(action);
          if (appliedKind) {
            appliedKinds.push(appliedKind);
          }
        } catch (error) {
          terminal.writeRawLine(
            terminalAnsiWrap(`action failed: ${error}`, TERMINAL_WARNING_RGB, {
              bold: true,
            })
          );
        }
      }
      if (appliedKinds.length) {
        terminal.writeRawLine(
          terminalAnsiWrap(`applied: ${appliedKinds.join(", ")}`, TERMINAL_SUCCESS_RGB, {
            bold: true,
          })
        );
      }
      return TERMINAL_NO_RESULT;
    },
  });

  if (!baseShell) {
    return null;
  }

  void loadAgentCapabilities().catch((error) => {
    console.warn("AI capabilities unavailable", error);
  });

  shell = {
    ...baseShell,
    activate() {
      baseShell.activate();
      void loadAgentCapabilities().catch((error) => {
        baseShell.writeLine(`AI capabilities unavailable: ${formatTerminalErrorMessage(error)}`);
      });
      if (!state.activated) {
        state.activated = true;
        const resource = currentAgentResource();
        if (resource) {
          void bindSession({ resource })
            .then((session) => {
              renderAgentTranscriptItems(
                baseShell,
                compactAgentTranscriptItems(session.transcript)
              );
            })
            .catch((error) => {
              baseShell.writeLine(`AI context bind failed: ${error}`);
            });
        }
      }
    },
    async handleResourceSwitch(resource = null) {
      const nextResource = currentAgentResource(resource);
      await writeResourceNotice(nextResource, { forceRebind: true });
    },
  };

  return shell;
}

function installRepl(replApi) {
  return installLineTerminal({
    screenId: "repl-screen",
    hostId: "repl-terminal",
    introLines: [
      'resource(), profile(), profiles(), setProfile(name), referenceGridVisible(), setReferenceGridVisible(true), theme(), setTheme("light"), allView(), defaultView(), viewState(), state(), pickAt(x,y), pickRect(x0,y0,x1,y1), query(...), ids(...), hide([...]), show([...]), select([...]), inspect([...]), addInspection([...]), removeInspection([...]), clearInspection(), graph.reset(...), frame()',
      'Example: graph.reset("MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1");',
      'Example: var walls = ids("MATCH (w:IfcWall) RETURN w.GlobalId AS global_id LIMIT 8"); hide(walls);',
      "Enter runs. Up/Down walks history. Ctrl+C clears the line.",
    ],
    execute: async (code) => {
      const rewritten = rewriteReplSource(code);
      const fn = new AsyncFunction(
        "api",
        "window",
        "document",
        `"use strict"; const { viewer, graph, resource, profile, profiles, setProfile, referenceGridVisible, setReferenceGridVisible, toggleReferenceGrid, theme, setTheme, viewState, state, setViewMode, defaultView, allView, setViewModeAsync, defaultViewAsync, allViewAsync, pickAt, pickRect, pickAtAsync, pickRectAsync, listIds, visibleIds, selectedIds, inspectedIds, selectedInstanceIds, query, queryIds, ids, hide, show, select, inspect, addInspection, removeInspection, resetVisibility, clearSelection, clearInspection, frame, frameVisible, hideQuery, showQuery, selectQuery, inspectQuery } = api; return (async () => { ${rewritten} })();`
      );
      return fn(replApi, window, document);
    },
  });
}

init()
  .then(async () => {
    const viewer = createViewerApi();
    const initialViewer = await waitForViewerReady(viewer);
    updateResourceCatalogState(initialViewer.catalog);
    const graphShell = createGraphShell(viewer, appState);
    graphShell.api.installController(createGraphController(viewer));
    const electronShell = installElectronShellControls();
    window.wViewerKeyboardFocus = installViewerKeyboardFocus();
    const headerControls = installHeaderControls(viewer, graphShell, appState);
    const outliner = installProjectOutlinerController(viewer, appState, {
      catalogState: resourceCatalogState,
      getCatalogState: () => resourceCatalogState,
    });
    const repl = createReplApi(viewer, graphShell.api);
    window.wAppState = appState;
    window.wViewer = viewer;
    window.viewer = viewer;
    window.wHeader = headerControls;
    window.wElectronShell = electronShell;
    window.wOutliner = outliner;
    window.wGraph = graphShell.api;
    window.graph = graphShell.api;
    window.wGraphShell = graphShell.bridge;
    window.__wGraphShell = graphShell.bridge;
    window.query = repl.query;
    window.queryIds = repl.queryIds;
    window.ids = repl.ids;
    window.viewState = repl.viewState;
    window.state = repl.state;
    window.setViewMode = repl.setViewMode;
    window.defaultView = repl.defaultView;
    window.allView = repl.allView;
    window.setViewModeAsync = repl.setViewModeAsync;
    window.defaultViewAsync = repl.defaultViewAsync;
    window.allViewAsync = repl.allViewAsync;
    window.pickAt = repl.pickAt;
    window.pickRect = repl.pickRect;
    window.pickAtAsync = repl.pickAtAsync;
    window.pickRectAsync = repl.pickRectAsync;
    window.hideQuery = repl.hideQuery;
    window.showQuery = repl.showQuery;
    window.selectQuery = repl.selectQuery;
    window.resource = repl.resource;
    window.profile = repl.profile;
    window.profiles = repl.profiles;
    window.setProfile = repl.setProfile;
    window.referenceGridVisible = repl.referenceGridVisible;
    window.setReferenceGridVisible = repl.setReferenceGridVisible;
    window.toggleReferenceGrid = repl.toggleReferenceGrid;
    window.theme = repl.theme;
    window.setTheme = repl.setTheme;
    window.listIds = repl.listIds;
    window.visibleIds = repl.visibleIds;
    window.selectedIds = repl.selectedIds;
    window.selectedInstanceIds = repl.selectedInstanceIds;
    window.hide = repl.hide;
    window.show = repl.show;
    window.select = repl.select;
    window.resetVisibility = repl.resetVisibility;
    window.clearSelection = repl.clearSelection;
    window.frame = repl.frame;
    const replShell = installRepl(repl);
    const agentShell = installAgentTerminal(viewer, graphShell.api);
    window.addEventListener("w-viewer-state-change", (event) => {
      appState.dispatch({
        type: "viewer/committed",
        state: event.detail?.state || viewer.viewState(),
        reason: event.detail?.reason || "unknown",
      });
    });
    appState.dispatch({ type: "viewer/committed", state: initialViewer.state, reason: "init" });
    let committedResource = appState.getState().committedViewerState?.resource || null;
    appState.subscribe((state, previous) => {
      const nextResource = state.committedViewerState?.resource || null;
      const previousResource = previous.committedViewerState?.resource || committedResource;
      if (!nextResource || nextResource === previousResource) {
        return;
      }
      committedResource = nextResource;
      void agentShell?.handleResourceSwitch?.(nextResource);
    });
    installProjectResourcePickerSupport({
      appStateStore: appState,
    });
    const terminalShell = installTerminalToolSelector([
      { id: "ai", shell: agentShell, defaultActive: true },
      { id: "js", shell: replShell },
    ], appState);
    window.addEventListener("w-terminal-visibility-change", () => {
      terminalShell?.resize();
      viewer.resizeViewport();
      graphShell.resize();
    });
    installLayoutResizers({
      onSidePanelResize: () => {
        graphShell.resize();
        viewer.resizeViewport();
      },
      onTerminalResize: () => {
        terminalShell?.resize();
        viewer.resizeViewport();
      },
    });
  })
  .catch((error) => {
    const status = document.getElementById("status-line");
    if (status) {
      status.textContent = `w web viewer failed: ${error}`;
    }
    console.error(error);
  });
