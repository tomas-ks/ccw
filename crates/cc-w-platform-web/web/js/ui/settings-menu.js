import {
  VIEWER_THEME_STORAGE_KEY,
  normalizeViewerTheme,
} from "../state/app-state.js";

const TERMINAL_SURFACE_BACKGROUND = "#0c1018";

export function currentViewerTheme({ document: doc = globalThis.document } = {}) {
  return normalizeViewerTheme(doc?.documentElement?.dataset?.theme || "light");
}

export function cssVariable(
  name,
  { document: doc = globalThis.document, window: win = globalThis.window } = {}
) {
  if (!doc?.documentElement || typeof win?.getComputedStyle !== "function") {
    return "";
  }
  return win.getComputedStyle(doc.documentElement).getPropertyValue(name).trim();
}

export function cssVariableOr(name, fallback, options = {}) {
  return cssVariable(name, options) || fallback;
}

export function parseCssColorToRgb(color) {
  const value = String(color || "").trim();
  const hex = value.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i);
  if (hex) {
    const raw =
      hex[1].length === 3
        ? hex[1].split("").map((ch) => `${ch}${ch}`).join("")
        : hex[1];
    return [
      parseInt(raw.slice(0, 2), 16) / 255,
      parseInt(raw.slice(2, 4), 16) / 255,
      parseInt(raw.slice(4, 6), 16) / 255,
    ];
  }
  const rgb = value.match(/^rgba?\(([^)]+)\)$/i);
  if (rgb) {
    const parts = rgb[1].split(",").map((part) => Number.parseFloat(part.trim()));
    if (parts.length >= 3 && parts.slice(0, 3).every(Number.isFinite)) {
      return parts.slice(0, 3).map((part) => Math.max(0, Math.min(255, part)) / 255);
    }
  }
  return null;
}

export function viewerClearColorFromTheme(options = {}) {
  return parseCssColorToRgb(cssVariable("--viewer-clear", options)) || [0.04, 0.05, 0.08];
}

export function terminalThemeForTheme(theme = currentViewerTheme(), options = {}) {
  const normalized = normalizeViewerTheme(theme);
  const background =
    cssVariable("--surface-terminal-strong", options) || TERMINAL_SURFACE_BACKGROUND;
  const foreground = cssVariable("--terminal-foreground", options) || "#d7deee";
  const selectionBackground =
    cssVariable("--terminal-selection", options) || "rgba(141, 182, 255, 0.24)";

  if (normalized === "light") {
    return {
      background,
      foreground,
      cursor: "#3f6fbf",
      cursorAccent: background,
      selectionBackground,
      black: "#242a34",
      red: "#a9473b",
      green: "#2f7d4e",
      yellow: "#90620f",
      blue: "#3f6fbf",
      magenta: "#7954a8",
      cyan: "#247985",
      white: "#f7f8f8",
      brightBlack: "#77808d",
      brightRed: "#c95a4e",
      brightGreen: "#39965e",
      brightYellow: "#a77714",
      brightBlue: "#477ed8",
      brightMagenta: "#9165c8",
      brightCyan: "#2b929f",
      brightWhite: "#ffffff",
    };
  }

  return {
    background,
    foreground,
    cursor: "#8db6ff",
    cursorAccent: background,
    selectionBackground,
    black: background,
    red: "#ff9d8a",
    green: "#8fe0a2",
    yellow: "#f2c879",
    blue: "#8db6ff",
    magenta: "#d4a7ff",
    cyan: "#7edfe6",
    white: "#d7deee",
    brightBlack: "#4e576c",
    brightRed: "#ffb8aa",
    brightGreen: "#a9efb9",
    brightYellow: "#f7dca0",
    brightBlue: "#abc8ff",
    brightMagenta: "#dfbeff",
    brightCyan: "#9be9ef",
    brightWhite: "#f3f6ff",
  };
}

export function applyViewerTheme(
  theme,
  {
    viewer = null,
    terminals = [],
    document: doc = globalThis.document,
    window: win = globalThis.window,
    storage = win?.localStorage || null,
    storageKey = VIEWER_THEME_STORAGE_KEY,
    setViewerClearColor = null,
  } = {}
) {
  const normalized = normalizeViewerTheme(theme);
  if (doc?.documentElement?.dataset) {
    doc.documentElement.dataset.theme = normalized;
  }
  storage?.setItem?.(storageKey, normalized);

  const [red, green, blue] = viewerClearColorFromTheme({ document: doc, window: win });
  if (viewer?.setClearColor) {
    viewer.setClearColor(red, green, blue);
  } else if (typeof setViewerClearColor === "function") {
    try {
      setViewerClearColor(red, green, blue);
    } catch (_error) {
      // The viewer can be unavailable during early boot; the next state sync reapplies it.
    }
  }

  for (const terminal of terminals) {
    terminal?.setTheme?.(normalized);
  }
  win?.dispatchEvent?.(
    new CustomEvent("w-theme-change", {
      detail: { theme: normalized },
    })
  );
  return normalized;
}

export function setSettingsMenuOpen(toggleButton, dropdown, open) {
  const nextOpen = Boolean(open);
  if (dropdown) {
    dropdown.hidden = !nextOpen;
  }
  toggleButton?.setAttribute("aria-expanded", String(nextOpen));
  return nextOpen;
}

export function createSettingsMenuController({
  appStateStore,
  viewer = null,
  terminals = [],
  document: doc = globalThis.document,
  window: win = globalThis.window,
  settingsToggleButton = doc?.getElementById?.("settings-toggle-button") || null,
  settingsDropdown = doc?.getElementById?.("settings-dropdown") || null,
  themeSelect = doc?.getElementById?.("theme-select") || null,
  applyTheme = applyViewerTheme,
  subscribe = true,
} = {}) {
  if (!appStateStore) {
    throw new Error("createSettingsMenuController requires appStateStore.");
  }

  let lastTheme = null;
  let renderingTheme = false;

  const close = () => setSettingsMenuOpen(settingsToggleButton, settingsDropdown, false);
  const toggle = () =>
    setSettingsMenuOpen(settingsToggleButton, settingsDropdown, Boolean(settingsDropdown?.hidden));

  const render = (state = appStateStore.getState()) => {
    const theme = normalizeViewerTheme(state.theme);
    if (themeSelect && themeSelect.value !== theme) {
      renderingTheme = true;
      themeSelect.value = theme;
      renderingTheme = false;
    }
    if (lastTheme !== theme) {
      applyTheme(theme, { viewer, terminals, document: doc, window: win });
      lastTheme = theme;
    }
    return theme;
  };

  const onToggleClick = (event) => {
    event.stopPropagation();
    toggle();
  };
  const onDropdownClick = (event) => {
    event.stopPropagation();
  };
  const onDocumentClick = () => {
    if (!settingsDropdown?.hidden) {
      close();
    }
  };
  const onThemeChange = () => {
    if (renderingTheme) {
      return;
    }
    appStateStore.dispatch({ type: "theme/set", theme: themeSelect.value });
  };

  settingsToggleButton?.addEventListener("click", onToggleClick);
  settingsDropdown?.addEventListener("click", onDropdownClick);
  doc?.addEventListener?.("click", onDocumentClick);
  themeSelect?.addEventListener("change", onThemeChange);

  const unsubscribe = subscribe ? appStateStore.subscribe((state) => render(state)) : null;

  return {
    render,
    open: () => setSettingsMenuOpen(settingsToggleButton, settingsDropdown, true),
    close,
    toggle,
    theme: () => appStateStore.getState().theme,
    setTheme: (theme) => appStateStore.dispatch({ type: "theme/set", theme }),
    dispose: () => {
      settingsToggleButton?.removeEventListener("click", onToggleClick);
      settingsDropdown?.removeEventListener("click", onDropdownClick);
      doc?.removeEventListener?.("click", onDocumentClick);
      themeSelect?.removeEventListener("change", onThemeChange);
      unsubscribe?.();
    },
  };
}
