import { FitAddon } from "../../vendor/addon-fit.mjs";
import { Terminal } from "../../vendor/xterm.mjs";
import {
  TERMINAL_ACTIVITY_FRAMES,
  TERMINAL_MUTED_RGB,
  TERMINAL_WARNING_RGB,
  formatResult,
  formatTerminalErrorMessage,
  sanitizeTerminalText,
  terminalAnsiWrap,
  terminalPromptMarkup,
  terminalThemeForTheme,
  terminalWriteLine,
  terminalWriteRawLine,
} from "./ansi.js";

export const TERMINAL_NO_RESULT = Symbol("terminal-no-result");

export function installLineTerminal({
  screenId,
  hostId,
  introLines = [],
  execute,
  TerminalConstructor = Terminal,
  FitAddonConstructor = FitAddon,
  themeForTheme = terminalThemeForTheme,
} = {}) {
  const screen = document.getElementById(screenId);
  const host = document.getElementById(hostId);
  if (!screen || !host) {
    return null;
  }

  const term = new TerminalConstructor({
    allowTransparency: false,
    convertEol: true,
    cursorBlink: true,
    cursorStyle: "block",
    fontFamily:
      'ui-monospace, SFMono-Regular, SF Mono, Menlo, Monaco, Consolas, "Liberation Mono", monospace',
    fontSize: 12,
    lineHeight: 1.35,
    scrollback: 2000,
    smoothScrollDuration: 0,
    theme: themeForTheme(),
  });
  const fitAddon = new FitAddonConstructor();
  term.loadAddon(fitAddon);
  term.open(host);
  let shellApi = null;
  try {
    term.resize(120, 18);
  } catch (_error) {
    // Fall back to fit-based sizing below.
  }

  const fitTerminal = () => {
    if (host.hidden || !host.classList.contains("active")) {
      return;
    }
    try {
      fitAddon.fit();
    } catch (_error) {
      // Ignore pre-layout fit attempts; resize events will catch up.
    }
    if ((term.cols <= 0 || term.rows <= 0) && typeof term.resize === "function") {
      try {
        term.resize(120, 18);
      } catch (_error) {
        // Keep the terminal alive even if manual resize is unavailable.
      }
    }
  };

  let running = false;
  let currentLine = "";
  let cursorIndex = 0;
  const history = [];
  let historyIndex = 0;
  let historyDraft = "";
  let promptBooted = false;
  let inputActivated = false;
  let liveStatus = null;

  const blurInput = () => {
    inputActivated = false;
    try {
      term.blur?.();
    } catch (_error) {
      // Some xterm versions do not expose blur; the textarea fallback is enough.
    }
    term.textarea?.blur();
  };

  const focusInput = () => {
    inputActivated = true;
    term.focus();
  };

  const focusInputIfActivated = () => {
    if (inputActivated) {
      term.focus();
    }
  };

  const setRunning = (next) => {
    running = next;
    screen.classList.toggle("running", next);
  };

  const renderPromptLine = () => {
    term.write("\r\x1b[2K");
    term.write(terminalPromptMarkup());
    if (currentLine.length) {
      term.write(sanitizeTerminalText(currentLine));
    }
    const trailing = currentLine.length - cursorIndex;
    if (trailing > 0) {
      term.write(`\x1b[${trailing}D`);
    }
  };

  const moveCursorLeft = (columns) => {
    if (columns <= 0) {
      return;
    }
    term.write(columns === 1 ? "\x1b[D" : `\x1b[${columns}D`);
  };

  const moveCursorRight = (columns) => {
    if (columns <= 0) {
      return;
    }
    term.write(columns === 1 ? "\x1b[C" : `\x1b[${columns}C`);
  };

  const rewriteTailAtCursor = (tail, eraseColumns = 0) => {
    if (tail.length) {
      term.write(sanitizeTerminalText(tail));
    }
    if (eraseColumns > 0) {
      term.write(" ".repeat(eraseColumns));
    }
    moveCursorLeft(tail.length + eraseColumns);
  };

  const normalizePastedTerminalText = (text) =>
    String(text || "")
      .replace(/\r\n?/g, "\n")
      .replace(/\n/g, " ")
      .replace(/[^\S\t ]+/g, " ")
      .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");

  const unwrapBracketedPaste = (data) => {
    const start = "\u001b[200~";
    const end = "\u001b[201~";
    if (!data.startsWith(start) || !data.endsWith(end)) {
      return null;
    }
    return data.slice(start.length, -end.length);
  };

  const insertTextAtCursor = (text) => {
    const pasted = normalizePastedTerminalText(text);
    if (!pasted || running) {
      return;
    }
    const before = currentLine.slice(0, cursorIndex);
    const tail = currentLine.slice(cursorIndex);
    currentLine = before + pasted + tail;
    cursorIndex += pasted.length;
    renderPromptLine();
  };

  const replaceCurrentLine = (next) => {
    currentLine = next;
    cursorIndex = currentLine.length;
    renderPromptLine();
  };

  const showPrompt = () => {
    renderPromptLine();
  };

  const stopLiveStatus = ({ persist, finalText = null } = {}) => {
    if (!liveStatus) {
      return;
    }
    window.clearInterval(liveStatus.timer);
    const shouldPersist =
      typeof persist === "boolean" ? persist : Boolean(liveStatus.persistOnStop);
    const settledText = finalText || liveStatus.finalTextOnStop || liveStatus.label;
    const settledFrame = liveStatus.frame || TERMINAL_ACTIVITY_FRAMES[0];
    liveStatus = null;
    term.write("\r\x1b[2K");
    if (shouldPersist && settledText) {
      terminalWriteRawLine(
        term,
        terminalAnsiWrap(`${settledFrame} ${settledText}`, TERMINAL_MUTED_RGB, {
          dim: true,
        })
      );
    } else if (!running) {
      showPrompt();
    }
  };

  const renderLiveStatus = () => {
    if (!liveStatus) {
      return;
    }
    liveStatus.frame =
      TERMINAL_ACTIVITY_FRAMES[liveStatus.frameIndex % TERMINAL_ACTIVITY_FRAMES.length];
    liveStatus.frameIndex += 1;
    term.write("\r\x1b[2K");
    term.write(
      terminalAnsiWrap(`${liveStatus.frame} ${liveStatus.label}`, TERMINAL_MUTED_RGB, {
        dim: true,
      })
    );
  };

  const startLiveStatus = (label, { persistOnStop = false, finalTextOnStop = null } = {}) => {
    if (!promptBooted) {
      bootPrompt();
    }
    stopLiveStatus({ persist: false });
    liveStatus = {
      label: String(label || "Working..."),
      persistOnStop,
      finalTextOnStop,
      frameIndex: 0,
      frame: TERMINAL_ACTIVITY_FRAMES[0],
      timer: 0,
    };
    renderLiveStatus();
    liveStatus.timer = window.setInterval(renderLiveStatus, 120);
  };

  const writeBlankLine = () => {
    stopLiveStatus();
    term.write("\r\x1b[2K");
    terminalWriteRawLine(term, "");
    if (!running) {
      showPrompt();
    }
  };

  const writeRawLine = (text = "") => {
    if (!promptBooted) {
      bootPrompt();
    }
    stopLiveStatus();
    term.write("\r\x1b[2K");
    terminalWriteRawLine(term, text);
    if (!running) {
      showPrompt();
    }
  };

  const writeLine = (text = "") => {
    writeRawLine(sanitizeTerminalText(text));
  };

  const writeLines = (lines = []) => {
    for (const line of lines) {
      writeLine(line);
    }
  };

  const bootPrompt = () => {
    fitTerminal();
    if (promptBooted) {
      focusInputIfActivated();
      return;
    }
    for (const line of introLines) {
      terminalWriteLine(term, line);
    }
    term.write("\r\n");
    promptBooted = true;
    showPrompt();
    focusInputIfActivated();
  };

  if (typeof ResizeObserver !== "undefined") {
    const resizeObserver = new ResizeObserver(() => {
      fitTerminal();
      if (!promptBooted) {
        requestAnimationFrame(() => bootPrompt());
      }
    });
    resizeObserver.observe(host);
  } else {
    window.addEventListener("resize", () => {
      fitTerminal();
      if (!promptBooted) {
        requestAnimationFrame(() => bootPrompt());
      }
    });
  }

  const moveHistory = (direction) => {
    if (!history.length) {
      return;
    }

    if (historyIndex === history.length) {
      historyDraft = currentLine;
    }
    historyIndex = Math.max(0, Math.min(history.length, historyIndex + direction));
    replaceCurrentLine(historyIndex === history.length ? historyDraft : history[historyIndex]);
  };

  const resetLineState = () => {
    currentLine = "";
    cursorIndex = 0;
    historyIndex = history.length;
    historyDraft = "";
  };

  const runCurrentCommand = async (code) => {
    if (running) {
      return;
    }
    if (!code.trim()) {
      term.write("\r\n");
      showPrompt();
      return;
    }

    if (history[history.length - 1] !== code) {
      history.push(code);
    }
    term.write("\r\n");
    resetLineState();
    setRunning(true);
    try {
      const result = await execute(
        code,
        shellApi || {
          term,
          writeLine,
          writeLines,
          writeRawLine,
          writeBlankLine,
          startLiveStatus,
          stopLiveStatus,
          focus: () => term.focus(),
        }
      );
      if (result !== TERMINAL_NO_RESULT) {
        terminalWriteLine(term, formatResult(result ?? "ok"));
      }
    } catch (error) {
      stopLiveStatus();
      terminalWriteRawLine(
        term,
        terminalAnsiWrap(formatTerminalErrorMessage(error), TERMINAL_WARNING_RGB, {
          bold: true,
        })
      );
      console.error(error);
    } finally {
      stopLiveStatus();
      setRunning(false);
      showPrompt();
      focusInputIfActivated();
    }
  };

  term.onData((data) => {
    if (!promptBooted) {
      bootPrompt();
    }
    if (data === "\u001b[A" || data === "\u001bOA") {
      moveHistory(-1);
      return;
    }
    if (data === "\u001b[B" || data === "\u001bOB") {
      moveHistory(1);
      return;
    }

    if (running) {
      return;
    }

    const bracketedPaste = unwrapBracketedPaste(data);
    if (bracketedPaste !== null) {
      insertTextAtCursor(bracketedPaste);
      return;
    }

    if (data === "\r") {
      void runCurrentCommand(currentLine);
      return;
    }

    if (data === "\u0003") {
      term.write("\r\x1b[2K");
      term.write(`${terminalPromptMarkup()}${sanitizeTerminalText(currentLine)}^C\r\n`);
      resetLineState();
      showPrompt();
      return;
    }

    if (data === "\u0001" || data === "\u001b[H" || data === "\u001bOH") {
      if (cursorIndex > 0) {
        moveCursorLeft(cursorIndex);
        cursorIndex = 0;
      }
      return;
    }

    if (data === "\u0005" || data === "\u001b[F" || data === "\u001bOF") {
      const moveRight = currentLine.length - cursorIndex;
      if (moveRight > 0) {
        moveCursorRight(moveRight);
        cursorIndex = currentLine.length;
      }
      return;
    }

    if (data === "\u001b[D" || data === "\u001bOD") {
      if (cursorIndex > 0) {
        cursorIndex -= 1;
        moveCursorLeft(1);
      }
      return;
    }

    if (data === "\u001b[C" || data === "\u001bOC") {
      if (cursorIndex < currentLine.length) {
        cursorIndex += 1;
        moveCursorRight(1);
      }
      return;
    }

    if (data === "\u007F") {
      if (cursorIndex > 0) {
        const deletingAtLineEnd = cursorIndex === currentLine.length;
        currentLine = currentLine.slice(0, cursorIndex - 1) + currentLine.slice(cursorIndex);
        cursorIndex -= 1;
        if (deletingAtLineEnd) {
          term.write("\b \b");
        } else {
          moveCursorLeft(1);
          rewriteTailAtCursor(currentLine.slice(cursorIndex), 1);
        }
      }
      return;
    }

    if (data === "\u001b[3~") {
      if (cursorIndex < currentLine.length) {
        currentLine = currentLine.slice(0, cursorIndex) + currentLine.slice(cursorIndex + 1);
        rewriteTailAtCursor(currentLine.slice(cursorIndex), 1);
      }
      return;
    }

    for (const ch of data) {
      if (ch === "\n") {
        continue;
      }
      if (ch === "\r") {
        void runCurrentCommand(currentLine);
        return;
      }
      if (ch < " " && ch !== "\t") {
        continue;
      }
      const appendingAtLineEnd = cursorIndex === currentLine.length && ch !== "\t";
      const tail = currentLine.slice(cursorIndex);
      currentLine = currentLine.slice(0, cursorIndex) + ch + currentLine.slice(cursorIndex);
      cursorIndex += ch.length;
      if (appendingAtLineEnd) {
        term.write(sanitizeTerminalText(ch));
      } else if (ch === "\t") {
        renderPromptLine();
      } else {
        term.write(sanitizeTerminalText(ch));
        if (tail.length) {
          term.write(sanitizeTerminalText(tail));
          moveCursorLeft(tail.length);
        }
      }
    }
  });

  term.textarea?.addEventListener(
    "paste",
    (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (running) {
        return;
      }
      const text = event.clipboardData?.getData("text/plain") || "";
      insertTextAtCursor(text);
    },
    { capture: true }
  );

  term.onKey(() => {
    if (document.activeElement !== term.textarea) {
      term.focus();
    }
  });

  host.addEventListener("pointerdown", () => focusInput());
  window.addEventListener("w-viewer-keyboard-activate", () => blurInput());
  window.addEventListener("w-terminal-visibility-change", (event) => {
    if (!event.detail?.visible) {
      blurInput();
    }
  });
  if (host.classList.contains("active")) {
    requestAnimationFrame(() => bootPrompt());
    setTimeout(() => bootPrompt(), 50);
    setTimeout(() => bootPrompt(), 180);
  }

  shellApi = {
    resize: fitTerminal,
    focus: () => focusInput(),
    setTheme: (theme) => {
      term.options.theme = themeForTheme(theme);
    },
    writeLine,
    writeLines,
    writeRawLine,
    writeBlankLine,
    startLiveStatus,
    stopLiveStatus,
    activate: () => {
      host.hidden = false;
      host.classList.add("active");
      requestAnimationFrame(() => {
        fitTerminal();
        bootPrompt();
      });
    },
    deactivate: () => {
      blurInput();
      host.classList.remove("active");
      host.hidden = true;
    },
  };
  window.addEventListener("w-theme-change", (event) => {
    shellApi?.setTheme?.(event.detail?.theme);
  });
  return shellApi;
}

export function installTerminalToolSelector(terminals, appStateStore) {
  const buttons = Array.from(document.querySelectorAll("[data-terminal-tool]"));
  const terminalMap = new Map(
    terminals.map((terminal) => [terminal.id, terminal.shell]).filter((entry) => entry[1])
  );

  const setActiveTool = (nextTool) => {
    if (!terminalMap.has(nextTool)) {
      return;
    }
    for (const [toolId, shell] of terminalMap.entries()) {
      if (toolId === nextTool) {
        shell.activate();
      } else {
        shell.deactivate();
      }
    }
    for (const button of buttons) {
      const active = button.dataset.terminalTool === nextTool;
      button.classList.toggle("active", active);
      button.setAttribute("aria-selected", String(active));
    }
  };

  for (const button of buttons) {
    button.addEventListener("click", () => {
      const activeTool = button.dataset.terminalTool;
      appStateStore.dispatch({
        type: "terminal/set-active",
        activeTool,
      });
      if (!document.body.classList.contains("terminal-hidden")) {
        requestAnimationFrame(() => terminalMap.get(activeTool)?.focus());
      }
    });
  }

  appStateStore.subscribe((state) => {
    const activeTool =
      state.terminal?.activeTool ||
      terminals.find((terminal) => terminal.defaultActive)?.id ||
      terminals[0]?.id ||
      null;
    if (activeTool) {
      setActiveTool(activeTool);
    }
  });

  return {
    activeTool: () => appStateStore.getState().terminal?.activeTool || null,
    resize: () => terminalMap.get(appStateStore.getState().terminal?.activeTool)?.resize(),
    focus: () => terminalMap.get(appStateStore.getState().terminal?.activeTool)?.focus(),
    setActiveTool: (activeTool) =>
      appStateStore.dispatch({ type: "terminal/set-active", activeTool }),
  };
}
