import { tryGetFirst } from "../util/object.js";
import { isIfcResource, safeViewerCurrentResource } from "../viewer/resource.js";
import {
  buildGraphNodeBalloonView,
  buildGraphNodeErrorBalloonView,
  buildGraphNodeLoadingBalloonView,
  buildNoGraphNodeBalloonView,
  buildNoPickBalloonView,
  buildPickLoadingBalloonView,
  buildPickedElementBalloonView,
  buildPickedElementErrorView,
  buildPickedElementMissingView,
  loadPickedElementDetails,
  normalizeDbNodeId,
  queryGraphNodeProperties,
} from "../semantic/properties.js";
import {
  applyHiddenPropertiesBalloon,
  hidePickAnchorMarker,
  pickRegionClientCenter,
  positionPropertiesBalloonFromAnchor,
  renderPropertiesBalloonView,
  resolvePropertiesBalloonDom,
} from "./balloon.js";

export function propertiesBalloonClientAnchor(
  clientX,
  clientY,
  {
    kind = "client",
    anchored = false,
    marker = false,
  } = {}
) {
  return {
    kind,
    clientX,
    clientY,
    anchored,
    marker,
  };
}

export function propertiesBalloonOpenAction({
  source = "none",
  anchor = null,
} = {}) {
  return {
    type: "balloon/open",
    source,
    anchor,
  };
}

export function propertiesBalloonCloseAction({ dismissed = undefined } = {}) {
  return {
    type: "balloon/close",
    dismissed,
  };
}

export function propertiesBalloonDismissAction() {
  return { type: "balloon/dismiss" };
}

export function openPropertiesBalloonAtClientPoint(
  appStateStore,
  clientX,
  clientY,
  {
    anchored = false,
    marker = false,
    source = "none",
    kind = "client",
  } = {}
) {
  return appStateStore.dispatch(
    propertiesBalloonOpenAction({
      source,
      anchor: propertiesBalloonClientAnchor(clientX, clientY, {
        kind,
        anchored,
        marker,
      }),
    })
  );
}

export function openPropertiesBalloonAtViewportCenter(
  appStateStore,
  viewport,
  { source = "none" } = {}
) {
  if (!viewport) {
    return null;
  }
  const rect = viewport.getBoundingClientRect();
  return openPropertiesBalloonAtClientPoint(
    appStateStore,
    rect.left + rect.width / 2,
    rect.top + rect.height / 2,
    {
      source,
      kind: "viewport-center",
      anchored: false,
      marker: false,
    }
  );
}

export function closePropertiesBalloon(appStateStore, options = {}) {
  return appStateStore.dispatch(propertiesBalloonCloseAction(options));
}

export function dismissPropertiesBalloon(appStateStore, state = null) {
  if (state) {
    state.propertiesBalloonDismissed = true;
  }
  return appStateStore.dispatch(propertiesBalloonDismissAction());
}

function focusKey(focus = {}) {
  return [
    focus?.source || "none",
    focus?.resource || "",
    focus?.dbNodeId ?? "",
    focus?.graphNodeId ?? "",
    focus?.semanticId || "",
  ].join("\u001f");
}

function nodeResource(node) {
  const raw = tryGetFirst(node, ["sourceResource", "source_resource", "resource"]);
  const resource = raw === null ? "" : String(raw).trim();
  return resource || null;
}

function nodeDbNodeId(node) {
  return normalizeDbNodeId(
    tryGetFirst(node, ["dbNodeId", "db_node_id", "nodeId", "id", "key"])
  );
}

function currentResourceFor(viewer, currentResource) {
  if (typeof currentResource === "function") {
    return currentResource();
  }
  return safeViewerCurrentResource(viewer);
}

function pickHitFromDetail(detail) {
  const hits = Array.isArray(detail?.hits) ? detail.hits : [];
  return hits[0] || null;
}

function isPickFocus(focus = {}) {
  return focus?.source === "pick" || (focus?.semanticId && !focus?.graphNodeId);
}

function isGraphFocus(focus = {}) {
  return focus?.source === "graph" || Boolean(focus?.graphNodeId || focus?.dbNodeId);
}

export function createPropertiesBalloonController({
  appStateStore,
  viewer = null,
  graph = null,
  state = {},
  document: doc = globalThis.document,
  window: win = globalThis.window,
  dom = resolvePropertiesBalloonDom(doc),
  subscribe = true,
  listenToViewer = false,
  fetchImpl = globalThis.fetch,
  currentResource = null,
  getGraphNode = null,
  pickInteractionEnabled = () => true,
  onPickSelectionChange = null,
  onViewRendered = null,
  openGraphForNode = null,
  setStatus = null,
  updatePickFocus = true,
  maxPickRelations = 1,
  maxGraphRelations = 16,
} = {}) {
  if (!appStateStore) {
    throw new Error("createPropertiesBalloonController requires appStateStore.");
  }

  const runtime = {
    activeTarget: null,
    requestId: 0,
    lastFocusKey: null,
    skipFocusKey: null,
    disposed: false,
  };

  const resolveCurrentResource = () => currentResourceFor(viewer, currentResource);

  const reportStatus = (text, tone = "info") => {
    if (typeof setStatus === "function") {
      setStatus(text, tone);
    } else if (graph && typeof graph.setStatus === "function") {
      graph.setStatus(text, tone);
    }
  };

  const renderView = (view) => {
    renderPropertiesBalloonView(dom, view);
    runtime.activeTarget = view?.target || null;
    syncLegacySelectionState(view?.target || null);
    if (typeof onViewRendered === "function") {
      onViewRendered(view, runtime.activeTarget);
    }
    return view;
  };

  const syncLegacySelectionState = (target) => {
    if (!state) {
      return;
    }
    if (!target) {
      state.pickedDbNodeId = null;
      state.pickedResource = null;
      state.pickedSemanticId = null;
      return;
    }
    if (target.source === "pick") {
      state.selectionOrigin = "pick";
      state.activePickHasHit = Boolean(target.semanticId);
      state.pickedSemanticId = target.semanticId || null;
      state.pickedDbNodeId = target.dbNodeId ?? null;
      state.pickedResource = target.resource || null;
      return;
    }
    if (target.source === "graph") {
      state.activePickHasHit = false;
      state.pickedSemanticId = null;
      state.pickedDbNodeId = null;
      state.pickedResource = null;
    }
  };

  const dispatchPickFocus = (target) => {
    if (!updatePickFocus || !target) {
      return;
    }
    const action = {
      type: "focus/set",
      source: "pick",
      semanticId: target.semanticId || null,
      dbNodeId: target.dbNodeId ?? null,
      resource: target.resource || resolveCurrentResource(),
    };
    runtime.skipFocusKey = focusKey(action);
    appStateStore.dispatch(action);
  };

  const clearFocus = () => {
    runtime.skipFocusKey = focusKey({
      source: "none",
      resource: null,
      dbNodeId: null,
      graphNodeId: null,
      semanticId: null,
    });
    appStateStore.dispatch({ type: "focus/clear" });
  };

  const syncLayoutFromAppState = (app) => {
    const balloon = app?.balloon || {};
    if (state) {
      state.propertiesBalloonDismissed = Boolean(balloon.dismissed);
    }
    if (!balloon.open || !balloon.anchor) {
      applyHiddenPropertiesBalloon(dom);
      return false;
    }
    return positionPropertiesBalloonFromAnchor(dom, balloon.anchor);
  };

  const selectedGraphNode = (focus = {}) => {
    if (typeof getGraphNode === "function") {
      const node = getGraphNode(focus.graphNodeId, focus);
      if (node) {
        return node;
      }
    }
    if (graph && typeof graph.getSelectedNode === "function") {
      return graph.getSelectedNode();
    }
    return null;
  };

  const renderPickFocus = async (focus = {}) => {
    const requestId = ++runtime.requestId;
    const semanticId = focus.semanticId || null;
    const dbNodeId = normalizeDbNodeId(focus.dbNodeId);
    const resource = focus.resource || resolveCurrentResource();
    if (!semanticId && dbNodeId === null) {
      renderView(buildNoPickBalloonView());
      return null;
    }

    const hit = { elementId: semanticId };
    renderView(buildPickLoadingBalloonView(hit));
    try {
      let lookup = null;
      let details = null;
      if (dbNodeId !== null) {
        lookup = {
          semanticId,
          dbNodeId,
          resource,
        };
        details = await queryGraphNodeProperties(dbNodeId, {
          resource,
          maxRelations: maxPickRelations,
          viewer,
          fetchImpl,
        });
      } else {
        const loaded = await loadPickedElementDetails(hit, {
          resource,
          viewer,
          fetchImpl,
          maxRelations: maxPickRelations,
        });
        lookup = loaded.lookup;
        details = loaded.details;
      }
      if (runtime.disposed || requestId !== runtime.requestId) {
        return null;
      }
      if (!lookup?.dbNodeId) {
        const view = renderView(buildPickedElementMissingView(hit, lookup));
        dispatchPickFocus(view.target);
        return view;
      }
      const view = renderView(
        buildPickedElementBalloonView({
          hit,
          details,
          lookup,
          resource,
        })
      );
      dispatchPickFocus(view.target);
      return view;
    } catch (error) {
      if (runtime.disposed || requestId !== runtime.requestId) {
        return null;
      }
      const view = renderView(buildPickedElementErrorView(hit, error));
      dispatchPickFocus(view.target);
      return view;
    }
  };

  const renderGraphFocus = async (focus = {}) => {
    const requestId = ++runtime.requestId;
    const node = selectedGraphNode(focus);
    const dbNodeId = normalizeDbNodeId(focus.dbNodeId) ?? nodeDbNodeId(node);
    const resource =
      focus.resource ||
      nodeResource(node) ||
      resolveCurrentResource();

    if (!node && dbNodeId === null) {
      renderView(buildNoGraphNodeBalloonView());
      return null;
    }

    if (node) {
      renderView(
        buildGraphNodeBalloonView({
          node,
          focus,
          resource,
        })
      );
    } else {
      renderView(buildGraphNodeLoadingBalloonView({ dbNodeId, resource }));
    }

    if (dbNodeId === null || !isIfcResource(resource)) {
      return runtime.activeTarget;
    }

    try {
      const details = await queryGraphNodeProperties(dbNodeId, {
        resource,
        maxRelations: maxGraphRelations,
        viewer,
        fetchImpl,
      });
      if (runtime.disposed || requestId !== runtime.requestId) {
        return null;
      }
      return renderView(
        buildGraphNodeBalloonView({
          node,
          details,
          focus,
          resource,
        })
      );
    } catch (error) {
      if (runtime.disposed || requestId !== runtime.requestId) {
        return null;
      }
      if (node) {
        reportStatus(`Could not refresh graph node properties: ${error.message || error}`, "warn");
        return runtime.activeTarget;
      }
      return renderView(
        buildGraphNodeErrorBalloonView({
          node,
          dbNodeId,
          resource,
          error,
        })
      );
    }
  };

  const renderFocus = (focus = {}) => {
    if (isPickFocus(focus)) {
      return renderPickFocus(focus);
    }
    if (isGraphFocus(focus)) {
      return renderGraphFocus(focus);
    }
    runtime.requestId += 1;
    return Promise.resolve(renderView(buildNoGraphNodeBalloonView()));
  };

  const syncFromAppState = (app, previous = null, action = null) => {
    syncLayoutFromAppState(app);
    const nextFocusKey = focusKey(app?.focus || {});
    if (nextFocusKey === runtime.lastFocusKey) {
      return null;
    }
    runtime.lastFocusKey = nextFocusKey;
    if (runtime.skipFocusKey && nextFocusKey === runtime.skipFocusKey) {
      runtime.skipFocusKey = null;
      return null;
    }
    return renderFocus(app?.focus || {});
  };

  const close = (options = {}) => closePropertiesBalloon(appStateStore, options);

  const dismiss = () => dismissPropertiesBalloon(appStateStore, state);

  const showAtClientPoint = (
    clientX,
    clientY,
    {
      anchored = false,
      marker = false,
      source = state?.selectionOrigin || "none",
    } = {}
  ) =>
    openPropertiesBalloonAtClientPoint(appStateStore, clientX, clientY, {
      source,
      anchored,
      marker,
    });

  const showAtViewportCenter = ({ source = state?.selectionOrigin || "none" } = {}) =>
    openPropertiesBalloonAtViewportCenter(appStateStore, dom.viewport, {
      source,
    });

  const openGraphForActiveTarget = async () => {
    const target = runtime.activeTarget || {};
    const dbNodeId = normalizeDbNodeId(target.dbNodeId);
    const resource = target.resource || resolveCurrentResource();
    if (dbNodeId === null) {
      reportStatus("Pick an IFC element before opening its graph neighborhood.", "warn");
      return null;
    }
    if (typeof openGraphForNode === "function") {
      return openGraphForNode({
        dbNodeId,
        resource,
        target,
      });
    }
    if (graph && typeof graph.seedFromNode === "function") {
      return graph.seedFromNode(dbNodeId, {
        resource,
        preserveProperties: target.source === "pick",
        revealProperties: false,
      });
    }
    reportStatus("Graph lookup is not wired to the properties balloon yet.", "warn");
    return null;
  };

  const renderPickedElement = async (detail = {}) => {
    const hit = pickHitFromDetail(detail);
    const requestId = ++runtime.requestId;
    state.selectionOrigin = "pick";
    state.activePickHasHit = Boolean(hit);
    state.pickedSemanticId = hit?.elementId ? String(hit.elementId) : null;
    state.pickedDbNodeId = null;
    state.pickedResource = null;
    if (typeof onPickSelectionChange === "function") {
      onPickSelectionChange({ hit, detail });
    }

    if (!hit) {
      renderView(buildNoPickBalloonView());
      close();
      hidePickAnchorMarker(dom.pickAnchorMarker);
      clearFocus();
      return false;
    }

    const loadingView = renderView(buildPickLoadingBalloonView(hit));
    dispatchPickFocus(loadingView.target);
    try {
      const { lookup, details } = await loadPickedElementDetails(hit, {
        resource: resolveCurrentResource(),
        viewer,
        fetchImpl,
        maxRelations: maxPickRelations,
      });
      if (runtime.disposed || requestId !== runtime.requestId) {
        return false;
      }
      if (!lookup?.dbNodeId) {
        const view = renderView(buildPickedElementMissingView(hit, lookup));
        dispatchPickFocus(view.target);
        return true;
      }
      const view = renderView(buildPickedElementBalloonView({ hit, lookup, details }));
      dispatchPickFocus(view.target);
      return true;
    } catch (error) {
      if (runtime.disposed || requestId !== runtime.requestId) {
        return false;
      }
      const view = renderView(buildPickedElementErrorView(hit, error));
      dispatchPickFocus(view.target);
      return true;
    }
  };

  const handleViewerPick = (event) => {
    if (!pickInteractionEnabled()) {
      return;
    }
    const detail = event?.detail || event || {};
    const hit = pickHitFromDetail(detail);
    void renderPickedElement(detail);
    if (!hit) {
      return;
    }
    const anchor = pickRegionClientCenter(detail, dom.viewerCanvas);
    if (anchor) {
      showAtClientPoint(anchor.x, anchor.y, { source: "pick" });
    } else {
      showAtViewportCenter({ source: "pick" });
    }
  };

  const handleViewerAnchor = (event) => {
    const detail = event?.detail || event || {};
    if (!pickInteractionEnabled()) {
      hidePickAnchorMarker(dom.pickAnchorMarker);
      return;
    }
    if (!detail.visible) {
      hidePickAnchorMarker(dom.pickAnchorMarker);
      if (state.activePickHasHit) {
        close();
      }
      return;
    }
    if (state.propertiesBalloonDismissed) {
      hidePickAnchorMarker(dom.pickAnchorMarker);
      return;
    }
    showAtClientPoint(detail.clientX, detail.clientY, {
      source: "pick",
      anchored: true,
      marker: true,
    });
  };

  const onCloseClick = () => {
    dismiss();
  };

  const onGraphButtonClick = () => {
    void openGraphForActiveTarget().catch((error) => {
      reportStatus(`Graph lookup failed: ${error.message || error}`, "error");
    });
  };

  dom.propertiesCloseButton?.addEventListener("click", onCloseClick);
  dom.propertiesGraphButton?.addEventListener("click", onGraphButtonClick);

  const unsubscribe = subscribe
    ? appStateStore.subscribe((app, previous, action) => {
        void syncFromAppState(app, previous, action);
      })
    : null;

  if (listenToViewer) {
    win?.addEventListener?.("w-viewer-pick", handleViewerPick);
    win?.addEventListener?.("w-viewer-anchor", handleViewerAnchor);
  }

  return {
    dom,
    state,
    renderFocus,
    renderPickedElement,
    syncFromAppState,
    syncLayoutFromAppState,
    close,
    hide: close,
    dismiss,
    hideMarker: () => hidePickAnchorMarker(dom.pickAnchorMarker),
    openAtClientPoint: showAtClientPoint,
    showAtClientPoint,
    openAtViewportCenter: showAtViewportCenter,
    showAtViewportCenter,
    openGraphForActiveTarget,
    pickRegionClientCenter: (detail) => pickRegionClientCenter(detail, dom.viewerCanvas),
    activeTarget: () => runtime.activeTarget,
    dispose: () => {
      runtime.disposed = true;
      runtime.requestId += 1;
      dom.propertiesCloseButton?.removeEventListener("click", onCloseClick);
      dom.propertiesGraphButton?.removeEventListener("click", onGraphButtonClick);
      if (listenToViewer) {
        win?.removeEventListener?.("w-viewer-pick", handleViewerPick);
        win?.removeEventListener?.("w-viewer-anchor", handleViewerAnchor);
      }
      unsubscribe?.();
    },
  };
}

export function installPropertiesBalloonController(options = {}) {
  return createPropertiesBalloonController({
    subscribe: true,
    ...options,
  });
}
