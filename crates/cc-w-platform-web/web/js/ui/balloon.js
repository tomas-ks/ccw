export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function pickRegionClientCenter(detail, viewerCanvas) {
  const region = detail?.region;
  if (!region || !viewerCanvas) {
    return null;
  }
  const rect = viewerCanvas.getBoundingClientRect();
  const surfaceWidth = Math.max(1, viewerCanvas.width || rect.width);
  const surfaceHeight = Math.max(1, viewerCanvas.height || rect.height);
  return {
    x: rect.left + ((Number(region.x) + Number(region.width) / 2) / surfaceWidth) * rect.width,
    y: rect.top + ((Number(region.y) + Number(region.height) / 2) / surfaceHeight) * rect.height,
  };
}

export function resolvePropertiesBalloonDom(doc = globalThis.document) {
  return {
    viewport: doc?.querySelector?.(".viewport") || null,
    viewerCanvas: doc?.getElementById?.("viewer-canvas") || null,
    pickAnchorMarker: doc?.getElementById?.("pick-anchor-marker") || null,
    propertiesBalloon: doc?.getElementById?.("properties-balloon") || null,
    propertiesCloseButton: doc?.getElementById?.("properties-close-button") || null,
  };
}

export function hidePickAnchorMarker(pickAnchorMarker) {
  if (pickAnchorMarker) {
    pickAnchorMarker.hidden = true;
  }
}

export function applyHiddenPropertiesBalloon({
  propertiesBalloon = null,
  pickAnchorMarker = null,
} = {}) {
  if (propertiesBalloon) {
    propertiesBalloon.hidden = true;
  }
  hidePickAnchorMarker(pickAnchorMarker);
}

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

export function positionPropertiesBalloonAtClientPoint(
  {
    viewport = null,
    propertiesBalloon = null,
    pickAnchorMarker = null,
  } = {},
  clientX,
  clientY,
  {
    anchored = false,
    marker = false,
    state = null,
  } = {}
) {
  if (!propertiesBalloon || !viewport) {
    return false;
  }
  if (state) {
    state.propertiesBalloonDismissed = false;
  }

  const viewportRect = viewport.getBoundingClientRect();
  propertiesBalloon.hidden = false;
  const balloonRect = propertiesBalloon.getBoundingClientRect();
  const padding = 12;
  const sideGap = 22;
  const leftBias = 18;
  const localX = clientX - viewportRect.left;
  const localY = clientY - viewportRect.top;

  if (marker && pickAnchorMarker) {
    pickAnchorMarker.hidden = false;
    pickAnchorMarker.style.left = `${Math.round(localX)}px`;
    pickAnchorMarker.style.top = `${Math.round(localY)}px`;
  } else {
    hidePickAnchorMarker(pickAnchorMarker);
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
  propertiesBalloon.style.left = `${Math.round(left)}px`;
  const top = clamp(localY - balloonRect.height / 2, padding, maxTop);
  propertiesBalloon.style.top = `${Math.round(top)}px`;
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
  propertiesBalloon.dataset.side = side;
  const arrowY = clamp(localY - top, 18, balloonRect.height - 18);
  propertiesBalloon.style.setProperty("--arrow-y", `${Math.round(arrowY)}px`);
  return true;
}

export function syncPropertiesBalloonFromAppState(
  app,
  {
    dom = {},
    state = null,
    applyHidden = () => applyHiddenPropertiesBalloon(dom),
    positionAtClientPoint = (clientX, clientY, options = {}) =>
      positionPropertiesBalloonAtClientPoint(dom, clientX, clientY, {
        ...options,
        state,
      }),
  } = {}
) {
  const balloon = app?.balloon || {};
  if (state) {
    state.propertiesBalloonDismissed = Boolean(balloon.dismissed);
  }
  if (!balloon.open || !balloon.anchor) {
    applyHidden();
    return false;
  }
  const anchor = balloon.anchor || {};
  if (
    anchor.kind === "client" ||
    anchor.kind === "viewport-center" ||
    (Number.isFinite(anchor.clientX) && Number.isFinite(anchor.clientY))
  ) {
    return positionAtClientPoint(anchor.clientX, anchor.clientY, {
      anchored: Boolean(anchor.anchored),
      marker: Boolean(anchor.marker),
    });
  }
  applyHidden();
  return false;
}

export function createPropertiesBalloonController({
  appStateStore,
  state = {},
  document: doc = globalThis.document,
  dom = resolvePropertiesBalloonDom(doc),
  subscribe = true,
} = {}) {
  if (!appStateStore) {
    throw new Error("createPropertiesBalloonController requires appStateStore.");
  }

  const applyHidden = () => applyHiddenPropertiesBalloon(dom);

  const close = (options = {}) => closePropertiesBalloon(appStateStore, options);

  const dismiss = () => dismissPropertiesBalloon(appStateStore, state);

  const hideMarker = () => hidePickAnchorMarker(dom.pickAnchorMarker);

  const positionAtClientPoint = (clientX, clientY, options = {}) =>
    positionPropertiesBalloonAtClientPoint(dom, clientX, clientY, {
      ...options,
      state,
    });

  const showAtClientPoint = (
    clientX,
    clientY,
    {
      anchored = false,
      marker = false,
      source = state.selectionOrigin || "none",
    } = {}
  ) =>
    openPropertiesBalloonAtClientPoint(appStateStore, clientX, clientY, {
      source,
      anchored,
      marker,
    });

  const showAtViewportCenter = ({ source = state.selectionOrigin || "none" } = {}) =>
    openPropertiesBalloonAtViewportCenter(appStateStore, dom.viewport, {
      source,
    });

  const syncFromAppState = (app) =>
    syncPropertiesBalloonFromAppState(app, {
      dom,
      state,
      applyHidden,
      positionAtClientPoint,
    });

  const onCloseClick = () => {
    dismiss();
  };

  dom.propertiesCloseButton?.addEventListener("click", onCloseClick);
  const unsubscribe = subscribe
    ? appStateStore.subscribe((app) => {
        syncFromAppState(app);
      })
    : null;

  return {
    dom,
    state,
    applyHidden,
    close,
    hide: close,
    dismiss,
    hideMarker,
    positionAtClientPoint,
    openAtClientPoint: showAtClientPoint,
    showAtClientPoint,
    openAtViewportCenter: showAtViewportCenter,
    showAtViewportCenter,
    syncFromAppState,
    pickRegionClientCenter: (detail) => pickRegionClientCenter(detail, dom.viewerCanvas),
    dispose: () => {
      dom.propertiesCloseButton?.removeEventListener("click", onCloseClick);
      unsubscribe?.();
    },
  };
}
