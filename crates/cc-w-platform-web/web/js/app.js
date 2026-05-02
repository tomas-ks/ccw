import init from "../pkg/cc_w_platform_web.js";
import { createAppStateStore } from "./state/app-state.js";
import { installAgentTerminal } from "./agent/agent-terminal.js";
import {
  createResourceCatalogState,
  safeViewerCurrentResource,
  updateResourceCatalogState as updateResourceCatalogStateInPlace,
} from "./viewer/resource.js";
import { createViewerApi, waitForViewerReady } from "./viewer/viewer-api.js";
import {
  createGraphController as createModuleGraphController,
  createGraphShell as createModuleGraphShell,
} from "./graph/graph-shell.js";
import { createReplApi, installRepl } from "./terminal/repl-controller.js";
import { installTerminalToolSelector } from "./terminal/line-terminal.js";
import {
  installElectronShellControls,
  installHeaderControls,
  installLayoutResizers,
  installViewerKeyboardFocus,
} from "./ui/app-shell.js";
import { installProjectOutliner as installProjectOutlinerController } from "./ui/outliner.js";
import { installPropertiesBalloonController } from "./ui/balloon-controller.js";
import { installProfilePicker } from "./viewer/profile-picker.js";
import { installResourcePicker } from "./viewer/resource-picker.js";

const resourceCatalogState = createResourceCatalogState();

const appState = createAppStateStore();


init()
  .then(async () => {
    const viewer = createViewerApi();
    const initialViewer = await waitForViewerReady(viewer);
    updateResourceCatalogStateInPlace(resourceCatalogState, initialViewer.catalog, {
      window,
    });
    let headerControls = null;
    let propertiesBalloonController = null;
    const graphShell = createModuleGraphShell(viewer, appState, {
      currentResource: () => safeViewerCurrentResource(viewer),
      hideProperties: () => propertiesBalloonController?.close(),
      hidePickAnchor: () => propertiesBalloonController?.hideMarker(),
      showPropertiesAtClientPoint: (clientX, clientY) =>
        propertiesBalloonController?.showAtClientPoint(clientX, clientY, {
          source: "graph",
        }),
      showPropertiesAtViewportCenter: () =>
        propertiesBalloonController?.showAtViewportCenter({
          source: "graph",
        }),
      shouldRevealProperties: () => Boolean(appState.getState()?.tools?.pick),
      showGraph: () => {
        if (headerControls?.showGraph) {
          headerControls.showGraph();
          return;
        }
        window.wHeader?.showGraph?.();
      },
    });
    graphShell.api.installController(createModuleGraphController(viewer));
    propertiesBalloonController = installPropertiesBalloonController({
      appStateStore: appState,
      viewer,
      graph: graphShell.api,
      listenToViewer: true,
      currentResource: () => safeViewerCurrentResource(viewer),
      getGraphNode: (nodeId) => graphShell.api.getNode?.(nodeId),
      pickInteractionEnabled: () => Boolean(appState.getState()?.tools?.pick),
      setStatus: graphShell.api.setStatus,
    });
    const electronShell = installElectronShellControls();
    window.wViewerKeyboardFocus = installViewerKeyboardFocus();
    headerControls = installHeaderControls(viewer, graphShell, appState);
    const outliner = installProjectOutlinerController(viewer, appState, {
      catalogState: resourceCatalogState,
      getCatalogState: () => resourceCatalogState,
    });
    const resourcePicker = installResourcePicker(viewer, appState, {
      catalogState: resourceCatalogState,
      catalog: initialViewer.catalog,
    });
    const profilePicker = installProfilePicker(viewer, appState);
    const repl = createReplApi(viewer, graphShell.api);
    window.wAppState = appState;
    window.wViewer = viewer;
    window.viewer = viewer;
    window.wHeader = headerControls;
    window.wElectronShell = electronShell;
    window.wOutliner = outliner;
    window.wResourcePicker = resourcePicker;
    window.wProfilePicker = profilePicker;
    window.wPropertiesBalloon = propertiesBalloonController;
    window.wGraph = graphShell.api;
    window.graph = graphShell.api;
    window.wGraphShell = graphShell.bridge;
    window.__wGraphShell = graphShell.bridge;
    window.query = repl.query;
    window.queryIds = repl.queryIds;
    window.ids = repl.ids;
    window.viewState = repl.viewState;
    window.state = repl.state;
    window.sceneBounds = repl.sceneBounds;
    window.section = repl.section;
    window.setSection = repl.setSection;
    window.clearSection = repl.clearSection;
    window.sectionState = repl.sectionState;
    window.annotations = repl.annotations;
    window.setAnnotationLayer = repl.setAnnotationLayer;
    window.clearAnnotations = repl.clearAnnotations;
    window.annotationsState = repl.annotationsState;
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
    const agentShell = installAgentTerminal(viewer, graphShell.api, {
      catalogState: resourceCatalogState,
    });
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
