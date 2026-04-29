import {
  parseViewerToolValue,
  viewerToolsToPickerValue,
} from "../state/app-state.js";
import { createPanelVisibilityController } from "./panels.js";
import {
  createSettingsMenuController,
  currentViewerTheme,
} from "./settings-menu.js";

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function installLayoutResizers(callbacks = {}) {
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

export function installElectronShellControls() {
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

export function installHeaderControls(viewer, graphShell, appStateStore) {
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

export function installViewerKeyboardFocus() {
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
