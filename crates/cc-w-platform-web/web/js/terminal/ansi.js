import { normalizeViewerTheme } from "../state/app-state.js";

export const TERMINAL_SURFACE_BACKGROUND = "#0c1018";
export const TERMINAL_ACCENT_RGB = [230, 93, 71];
export const TERMINAL_PROMPT_BLUE_RGB = [141, 182, 255];
export const TERMINAL_MUTED_RGB = [160, 168, 184];
export const TERMINAL_SOFT_RGB = [191, 198, 214];
export const TERMINAL_QUERY_RGB = [150, 190, 255];
export const TERMINAL_SUCCESS_RGB = [143, 224, 162];
export const TERMINAL_WARNING_RGB = [242, 200, 121];
export const TERMINAL_ACTIVITY_FRAMES = ["|", "/", "-", "\\"];

export function currentTerminalTheme() {
  if (typeof document === "undefined") {
    return "light";
  }
  return normalizeViewerTheme(document.documentElement.dataset.theme || "light");
}

export function terminalCssVariable(name) {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") {
    return "";
  }
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function terminalThemeForTheme(
  theme = currentTerminalTheme(),
  { cssVariable = terminalCssVariable } = {}
) {
  const normalized = normalizeViewerTheme(theme);
  const background = cssVariable("--surface-terminal-strong") || TERMINAL_SURFACE_BACKGROUND;
  const foreground = cssVariable("--terminal-foreground") || "#d7deee";
  const selectionBackground = cssVariable("--terminal-selection") || "rgba(141, 182, 255, 0.24)";
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

export function sanitizeTerminalText(text) {
  return String(text).replace(/\x1b/g, "\\x1b");
}

export function terminalAnsiRgb([r, g, b]) {
  return `\x1b[38;2;${r};${g};${b}m`;
}

export function terminalAnsiWrap(text, rgb, { bold = false, dim = false } = {}) {
  const open = `${bold ? "\x1b[1m" : ""}${dim ? "\x1b[2m" : ""}${terminalAnsiRgb(rgb)}`;
  return `${open}${sanitizeTerminalText(text)}\x1b[0m`;
}

export function terminalAssistantMarkup(text) {
  const source = String(text);
  const parts = [];
  let cursor = 0;
  const boldPattern = /\*\*([^*\r\n](?:.*?[^*\r\n])?)\*\*/g;
  for (const match of source.matchAll(boldPattern)) {
    const start = match.index ?? 0;
    if (start > cursor) {
      parts.push(sanitizeTerminalText(source.slice(cursor, start)));
    }
    parts.push(`\x1b[1m${sanitizeTerminalText(match[1])}\x1b[22m`);
    cursor = start + match[0].length;
  }
  if (cursor < source.length) {
    parts.push(sanitizeTerminalText(source.slice(cursor)));
  }
  return parts.join("");
}

export function terminalPromptMarkup() {
  return (
    `\x1b[1m${terminalAnsiRgb(TERMINAL_ACCENT_RGB)}W\x1b[0m` +
    `${terminalAnsiRgb(TERMINAL_PROMPT_BLUE_RGB)}>\x1b[0m `
  );
}

export function terminalWriteRawLine(term, text = "") {
  term.writeln(String(text));
}

export function terminalWriteLine(term, text = "") {
  terminalWriteRawLine(term, sanitizeTerminalText(text));
}

export function formatResult(value) {
  if (value === undefined) {
    return "undefined";
  }
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch (_error) {
    return String(value);
  }
}

export function formatTerminalErrorMessage(error) {
  if (error && typeof error === "object" && typeof error.message === "string") {
    const message = error.message.trim();
    if (message) {
      return message;
    }
  }
  return String(error);
}
