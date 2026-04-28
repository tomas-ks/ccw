function animationFrame(win, callback) {
  const request =
    typeof win?.requestAnimationFrame === "function"
      ? win.requestAnimationFrame.bind(win)
      : (fn) => setTimeout(fn, 0);
  return request(callback);
}

export function panelVisible(state, panel) {
  return Boolean(state?.panels?.[panel]);
}

export function setPanelVisible(appStateStore, panel, visible) {
  return appStateStore.dispatch({
    type: "panel/set",
    panel,
    visible: Boolean(visible),
  });
}

export function togglePanel(appStateStore, panel) {
  return appStateStore.dispatch({ type: "panel/toggle", panel });
}

export function syncPressedButton(button, active) {
  if (!button) {
    return Boolean(active);
  }
  const pressed = Boolean(active);
  button.classList.toggle("active", pressed);
  button.setAttribute("aria-pressed", String(pressed));
  return pressed;
}

export function syncBodyHiddenClass(body, className, visible) {
  if (body && className) {
    body.classList.toggle(className, !visible);
  }
  return Boolean(visible);
}

export function schedulePanelResize({
  graphShell = null,
  viewer = null,
  window: win = globalThis.window,
} = {}) {
  return animationFrame(win, () => {
    graphShell?.resize?.();
    viewer?.resizeViewport?.();
  });
}

export function dispatchTerminalVisibilityChange(
  visible,
  { window: win = globalThis.window } = {}
) {
  return animationFrame(win, () => {
    win?.dispatchEvent?.(
      new CustomEvent("w-terminal-visibility-change", {
        detail: { visible: Boolean(visible) },
      })
    );
  });
}

export function createPanelVisibilityController({
  appStateStore,
  viewer = null,
  graphShell = null,
  document: doc = globalThis.document,
  window: win = globalThis.window,
  body = doc?.body || null,
  graphToggleButton = doc?.getElementById?.("graph-toggle-button") || null,
  terminalToggleButton = doc?.getElementById?.("terminal-toggle-button") || null,
  terminalCloseButton = doc?.getElementById?.("terminal-close-button") || null,
  subscribe = true,
} = {}) {
  if (!appStateStore) {
    throw new Error("createPanelVisibilityController requires appStateStore.");
  }

  let lastGraphVisible = null;
  let lastTerminalVisible = null;

  graphToggleButton?.removeAttribute("disabled");
  terminalToggleButton?.removeAttribute("disabled");

  const render = (state = appStateStore.getState()) => {
    const graphVisible = panelVisible(state, "graph");
    syncBodyHiddenClass(body, "graph-hidden", graphVisible);
    syncPressedButton(graphToggleButton, graphVisible);

    const terminalVisible = panelVisible(state, "terminal");
    syncBodyHiddenClass(body, "terminal-hidden", terminalVisible);
    syncPressedButton(terminalToggleButton, terminalVisible);

    if (lastGraphVisible !== graphVisible || lastTerminalVisible !== terminalVisible) {
      schedulePanelResize({ graphShell, viewer, window: win });
    }
    if (lastTerminalVisible !== null && lastTerminalVisible !== terminalVisible) {
      dispatchTerminalVisibilityChange(terminalVisible, { window: win });
    }

    lastGraphVisible = graphVisible;
    lastTerminalVisible = terminalVisible;
    return { graphVisible, terminalVisible };
  };

  const onGraphToggleClick = () => {
    togglePanel(appStateStore, "graph");
  };
  const onTerminalToggleClick = () => {
    togglePanel(appStateStore, "terminal");
  };
  const onTerminalCloseClick = () => {
    setPanelVisible(appStateStore, "terminal", false);
  };

  graphToggleButton?.addEventListener("click", onGraphToggleClick);
  terminalToggleButton?.addEventListener("click", onTerminalToggleClick);
  terminalCloseButton?.addEventListener("click", onTerminalCloseClick);

  const unsubscribe = subscribe ? appStateStore.subscribe((state) => render(state)) : null;

  return {
    render,
    graphVisible: () => panelVisible(appStateStore.getState(), "graph"),
    terminalVisible: () => panelVisible(appStateStore.getState(), "terminal"),
    setGraphVisible: (visible) => setPanelVisible(appStateStore, "graph", visible),
    showGraph: () => setPanelVisible(appStateStore, "graph", true),
    hideGraph: () => setPanelVisible(appStateStore, "graph", false),
    toggleGraph: () => togglePanel(appStateStore, "graph"),
    setTerminalVisible: (visible) => setPanelVisible(appStateStore, "terminal", visible),
    showTerminal: () => setPanelVisible(appStateStore, "terminal", true),
    hideTerminal: () => setPanelVisible(appStateStore, "terminal", false),
    toggleTerminal: () => togglePanel(appStateStore, "terminal"),
    dispose: () => {
      graphToggleButton?.removeEventListener("click", onGraphToggleClick);
      terminalToggleButton?.removeEventListener("click", onTerminalToggleClick);
      terminalCloseButton?.removeEventListener("click", onTerminalCloseClick);
      unsubscribe?.();
    },
  };
}
