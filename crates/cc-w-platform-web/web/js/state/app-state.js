export const VIEWER_THEME_STORAGE_KEY = "w-viewer-theme";

const VIEWER_THEMES = new Set(["dark", "light"]);

export function parseViewerToolValue(value) {
  const tokens = new Set(String(value || "").split("-").filter(Boolean));
  const orbit = tokens.has("orbit");
  return {
    orbit,
    pick: tokens.has("pick"),
    pan: orbit,
  };
}

export function viewerToolsToPickerValue(tools) {
  const tokens = [];
  if (tools?.orbit) {
    tokens.push("orbit");
  }
  if (tools?.pick) {
    tokens.push("pick");
  }
  if (tools?.orbit) {
    tokens.push("pan");
  }
  return tokens.length ? tokens.join("-") : "none";
}

export function normalizeViewerTheme(theme) {
  const normalized = String(theme || "").trim().toLowerCase();
  return VIEWER_THEMES.has(normalized) ? normalized : "light";
}

function readStoredViewerTheme() {
  return normalizeViewerTheme(
    window.localStorage?.getItem(VIEWER_THEME_STORAGE_KEY) ||
      document.documentElement.dataset.theme ||
      "light"
  );
}

export function createAppStateStore(initial = {}) {
  const defaultState = {
    theme: readStoredViewerTheme(),
    requestedResource: null,
    committedViewerState: null,
    panels: {
      graph: !document.body.classList.contains("graph-hidden"),
      terminal: !document.body.classList.contains("terminal-hidden"),
      outliner: false,
    },
    tools: parseViewerToolValue(
      document.getElementById("tool-picker")?.value || "orbit-pick"
    ),
    terminal: {
      activeTool: "ai",
    },
    focus: {
      source: "none",
      resource: null,
      dbNodeId: null,
      graphNodeId: null,
      semanticId: null,
    },
    balloon: {
      open: false,
      anchor: null,
      source: "none",
      dismissed: false,
    },
  };
  let state = {
    ...defaultState,
    ...initial,
    panels: { ...defaultState.panels, ...(initial.panels || {}) },
    tools: { ...defaultState.tools, ...(initial.tools || {}) },
    terminal: { ...defaultState.terminal, ...(initial.terminal || {}) },
    focus: { ...defaultState.focus, ...(initial.focus || {}) },
    balloon: { ...defaultState.balloon, ...(initial.balloon || {}) },
  };
  const listeners = new Set();
  const emit = (previous, action) => {
    const snapshot = state;
    for (const listener of Array.from(listeners)) {
      listener(snapshot, previous, action);
    }
    window.dispatchEvent(
      new CustomEvent("w-app-state-change", {
        detail: { state: snapshot, previous, action },
      })
    );
  };
  const patch = (action, updater) => {
    const previous = state;
    const next = updater(previous);
    if (next === previous) {
      return previous;
    }
    state = next;
    emit(previous, action);
    return state;
  };
  return {
    getState: () => state,
    subscribe: (listener) => {
      listeners.add(listener);
      listener(state, state, { type: "init" });
      return () => listeners.delete(listener);
    },
    renderAll: () => {
      emit(state, { type: "render" });
      return state;
    },
    dispatch: (action) => {
      switch (action?.type) {
        case "resource/requested": {
          const resource = String(action.resource || "").trim() || null;
          return patch(action, (previous) => ({
            ...previous,
            requestedResource: resource,
          }));
        }
        case "viewer/committed": {
          const committedResource = String(action.state?.resource || "").trim() || null;
          return patch(action, (previous) => ({
            ...previous,
            committedViewerState: action.state || null,
            requestedResource:
              committedResource && committedResource === previous.requestedResource
                ? null
                : previous.requestedResource,
          }));
        }
        case "theme/set": {
          const theme = normalizeViewerTheme(action.theme);
          return patch(action, (previous) => ({
            ...previous,
            theme,
          }));
        }
        case "panel/set": {
          const panel = String(action.panel || "");
          if (!Object.prototype.hasOwnProperty.call(state.panels, panel)) {
            return state;
          }
          const visible = Boolean(action.visible);
          return patch(action, (previous) => ({
            ...previous,
            panels: {
              ...previous.panels,
              [panel]: visible,
            },
          }));
        }
        case "panel/toggle": {
          const panel = String(action.panel || "");
          if (!Object.prototype.hasOwnProperty.call(state.panels, panel)) {
            return state;
          }
          return patch(action, (previous) => ({
            ...previous,
            panels: {
              ...previous.panels,
              [panel]: !previous.panels[panel],
            },
          }));
        }
        case "tools/set": {
          const orbit = Boolean(action.tools?.orbit);
          return patch(action, (previous) => ({
            ...previous,
            tools: {
              orbit,
              pick: Boolean(action.tools?.pick),
              pan: orbit,
            },
          }));
        }
        case "tools/toggle": {
          const tool = String(action.tool || "");
          if (tool !== "orbit" && tool !== "pick") {
            return state;
          }
          if (tool === "orbit") {
            return patch(action, (previous) => {
              const orbit = !previous.tools.orbit;
              return {
                ...previous,
                tools: {
                  ...previous.tools,
                  orbit,
                  pan: orbit,
                },
              };
            });
          }
          return patch(action, (previous) => ({
            ...previous,
            tools: {
              ...previous.tools,
              [tool]: !previous.tools[tool],
            },
          }));
        }
        case "terminal/set-active": {
          const activeTool = String(action.activeTool || "").trim();
          if (!activeTool) {
            return state;
          }
          return patch(action, (previous) => ({
            ...previous,
            terminal: {
              ...previous.terminal,
              activeTool,
            },
          }));
        }
        case "focus/set": {
          return patch(action, (previous) => ({
            ...previous,
            focus: {
              source: String(action.source || "none"),
              resource: action.resource || null,
              dbNodeId: action.dbNodeId ?? null,
              graphNodeId: action.graphNodeId ?? null,
              semanticId: action.semanticId || null,
            },
          }));
        }
        case "focus/clear": {
          return patch(action, (previous) => ({
            ...previous,
            focus: {
              source: "none",
              resource: null,
              dbNodeId: null,
              graphNodeId: null,
              semanticId: null,
            },
            balloon: {
              ...previous.balloon,
              open: false,
              anchor: null,
              source: "none",
            },
          }));
        }
        case "balloon/open": {
          return patch(action, (previous) => ({
            ...previous,
            balloon: {
              open: true,
              source: String(action.source || previous.focus?.source || "none"),
              anchor: action.anchor || null,
              dismissed: false,
            },
          }));
        }
        case "balloon/close": {
          return patch(action, (previous) => ({
            ...previous,
            balloon: {
              ...previous.balloon,
              open: false,
              anchor: null,
              source: "none",
              dismissed: Boolean(action.dismissed ?? previous.balloon?.dismissed),
            },
          }));
        }
        case "balloon/dismiss": {
          return patch(action, (previous) => ({
            ...previous,
            balloon: {
              ...previous.balloon,
              open: false,
              anchor: null,
              source: "none",
              dismissed: true,
            },
          }));
        }
        case "balloon/anchor": {
          return patch(action, (previous) => ({
            ...previous,
            balloon: {
              ...previous.balloon,
              anchor: action.anchor || null,
            },
          }));
        }
        default:
          return state;
      }
    },
  };
}
