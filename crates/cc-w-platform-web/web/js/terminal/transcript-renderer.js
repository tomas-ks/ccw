import { tryGetFirst } from "../util/object.js";
import {
  TERMINAL_MUTED_RGB,
  TERMINAL_QUERY_RGB,
  TERMINAL_SOFT_RGB,
  TERMINAL_SUCCESS_RGB,
  TERMINAL_WARNING_RGB,
  formatResult,
  terminalAnsiWrap,
  terminalAssistantMarkup,
} from "./ansi.js";

export function isAgentTranscriptReplayMetaText(text) {
  const lowered = String(text || "").trim().toLowerCase();
  return (
    lowered.startsWith("thinking about the request.") ||
    lowered.startsWith("opencode progress:") ||
    lowered.startsWith("ai session bound to ") ||
    lowered.startsWith("ai context switched to ") ||
    lowered.startsWith("ai context cleared.")
  );
}

export function tryParseJsonText(text) {
  if (typeof text !== "string") {
    return null;
  }
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
    return null;
  }
  try {
    return JSON.parse(trimmed);
  } catch (_error) {
    return null;
  }
}

export function isJsonLikeText(text) {
  const trimmed = String(text || "").trim();
  return trimmed.startsWith("{") || trimmed.startsWith("[");
}

export function countRowsLikeResult(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  const rows = tryGetFirst(value, ["rows", "items", "results"]);
  return Array.isArray(rows) ? rows.length : null;
}

export function canonicalAgentToolName(toolName) {
  const trimmed = String(toolName || "").trim();
  const unprefixed = trimmed.startsWith("ifc_") ? trimmed.slice(4) : trimmed;
  switch (unprefixed) {
    case "runReadonlyCypher":
      return "run_readonly_cypher";
    case "runProjectReadonlyCypher":
    case "projectReadonlyCypher":
      return "run_project_readonly_cypher";
    case "get_schema_context":
    case "getSchema":
    case "getSchemaContext":
      return "schema_context";
    case "get_model_details":
    case "getModelDetails":
      return "model_details";
    default:
      return unprefixed;
  }
}

export function isCypherToolName(toolName) {
  switch (canonicalAgentToolName(toolName)) {
    case "readonly_cypher":
    case "run_readonly_cypher":
    case "project_readonly_cypher":
    case "run_project_readonly_cypher":
      return true;
    default:
      return false;
  }
}

export function summarizeAgentToolResult(toolName, item) {
  const displayToolName = String(toolName || "tool").trim() || "tool";
  const canonicalToolName = canonicalAgentToolName(displayToolName);
  const text = agentTextFromItem(item);
  const result = tryGetFirst(item, ["result", "output", "data", "response"]);
  const resultObject = result && typeof result === "object" ? result : null;

  if (isCypherToolName(displayToolName)) {
    const rowCount =
      countRowsLikeResult(resultObject) ??
      countRowsLikeResult(tryParseJsonText(text || ""));
    if (typeof rowCount === "number") {
      return `Read-only Cypher returned ${rowCount} row${rowCount === 1 ? "" : "s"}.`;
    }
    return "Read-only Cypher completed.";
  }

  if (canonicalToolName === "schema_context" && resultObject) {
    const schemaId =
      tryGetFirst(resultObject, ["schemaId", "schema_id"]) || "schema context";
    const cautions = Array.isArray(resultObject.cautions) ? resultObject.cautions.length : 0;
    const queryHabits = Array.isArray(resultObject.queryHabits)
      ? resultObject.queryHabits.length
      : 0;
    const playbooks =
      resultObject.queryPlaybooks && typeof resultObject.queryPlaybooks === "object"
        ? Object.keys(resultObject.queryPlaybooks).length
        : 0;
    return `${schemaId} loaded with ${cautions} caution${cautions === 1 ? "" : "s"}, ${queryHabits} query habit${queryHabits === 1 ? "" : "s"}, ${playbooks} playbook${playbooks === 1 ? "" : "s"}`;
  }

  if (canonicalToolName === "model_details" && resultObject) {
    const summary = tryGetFirst(resultObject, ["summary", "message", "text"]);
    if (typeof summary === "string" && summary.trim()) {
      return summary.trim();
    }
  }

  const rowCount = countRowsLikeResult(resultObject);
  if (typeof rowCount === "number") {
    return `Result from \`${displayToolName}\`: returned ${rowCount} item${
      rowCount === 1 ? "" : "s"
    }.`;
  }

  if (typeof text === "string" && text.trim() && !isJsonLikeText(text)) {
    return text.trim();
  }

  return `Result from \`${displayToolName}\`: completed.`;
}

export function compactAgentTranscriptItems(items) {
  if (!Array.isArray(items) || !items.length) {
    return [];
  }

  const conclusion = [...items].reverse().find((item) => {
    const text = agentTextFromItem(item);
    const kind = String(tryGetFirst(item, ["kind", "type", "event"]) || "text").toLowerCase();
    return (
      text &&
      (kind === "assistant" || kind === "message" || kind === "notice" || kind === "info") &&
      !isAgentTranscriptReplayMetaText(text)
    );
  });

  const summary = [...items].reverse().find((item) => {
    const text = agentTextFromItem(item);
    const kind = String(tryGetFirst(item, ["kind", "type", "event"]) || "text").toLowerCase();
    if (!text) {
      return false;
    }
    if (kind !== "assistant" && kind !== "system") {
      return false;
    }
    const lowered = text.trim().toLowerCase();
    return (
      lowered.startsWith("prepared ") ||
      lowered.startsWith("completed ") ||
      lowered.startsWith("applied:")
    );
  });

  const compacted = [];
  if (conclusion) {
    compacted.push(conclusion);
  }
  if (summary && summary !== conclusion) {
    compacted.push(summary);
  }
  if (compacted.length > 0) {
    return compacted;
  }

  const lastItem = items[items.length - 1];
  const lastText = agentTextFromItem(lastItem);
  if (
    lastText &&
    /^(ai session bound to |ai context switched to |ai context cleared\.)/i.test(
      lastText.trim()
    )
  ) {
    return [lastItem];
  }

  return [];
}

export function agentTextFromItem(item) {
  if (typeof item === "string") {
    return item;
  }
  if (!item || typeof item !== "object") {
    return null;
  }
  const text = tryGetFirst(item, ["text", "message", "content", "summary"]);
  if (typeof text === "string" && text.trim()) {
    return text.trim();
  }
  return null;
}

export function isPlaceholderToolTranscript(text) {
  const lines = String(text || "")
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter((line) => line.trim().length > 0);
  if (lines.length !== 1) {
    return false;
  }
  const lowered = lines[0].trim().toLowerCase();
  return /:\s*tool call$/.test(lowered) || /:\s*input keys:\s*[^:]+$/.test(lowered);
}

export function looksLikeCypherLine(text) {
  return /^(MATCH|RETURN|WHERE|WITH|UNION|ORDER BY|LIMIT|OPTIONAL MATCH|CALL|UNWIND)\b/i.test(
    String(text || "").trim()
  );
}

export function renderAgentTranscriptItems(shell, items) {
  if (!shell || !Array.isArray(items) || !items.length) {
    return;
  }

  const stopLiveStatus =
    typeof shell.stopLiveStatus === "function"
      ? (options) => shell.stopLiveStatus(options)
      : () => {};
  const writeRawLine =
    typeof shell.writeRawLine === "function"
      ? (text) => shell.writeRawLine(text)
      : (text) => shell.writeLine(text);
  const writeBlankLine =
    typeof shell.writeBlankLine === "function"
      ? () => shell.writeBlankLine()
      : () => shell.writeLine("");
  const startLiveStatus =
    typeof shell.startLiveStatus === "function"
      ? (label, options) => shell.startLiveStatus(label, options)
      : () => {};

  const state = (shell.__agentRenderState ||= {
    lastBlock: null,
  });

  const ensureBlock = (nextBlock) => {
    if (state.lastBlock && state.lastBlock !== nextBlock) {
      writeBlankLine();
    }
    state.lastBlock = nextBlock;
  };

  const startMetaStatus = (label, options = {}) => {
    ensureBlock("meta");
    startLiveStatus(label, options);
  };

  const writeMuted = (text) => {
    ensureBlock("meta");
    writeRawLine(
      terminalAnsiWrap(text, TERMINAL_MUTED_RGB, {
        dim: true,
      })
    );
  };

  const writeWarning = (text) => {
    ensureBlock("warning");
    writeRawLine(
      terminalAnsiWrap(text, TERMINAL_WARNING_RGB, {
        bold: true,
      })
    );
  };

  const writeSuccess = (text) => {
    ensureBlock("success");
    writeRawLine(
      terminalAnsiWrap(text, TERMINAL_SUCCESS_RGB, {
        bold: true,
      })
    );
  };

  const writeAssistant = (text) => {
    ensureBlock("assistant");
    const lines = String(text).split(/\r?\n/);
    for (const line of lines) {
      if (!line.trim()) {
        writeBlankLine();
        continue;
      }
      writeRawLine(terminalAssistantMarkup(line));
    }
  };

  const writeToolBlock = (text) => {
    const lines = String(text || "")
      .split(/\r?\n/)
      .map((line) => line.trimEnd());
    if (!lines.length) {
      return;
    }

    ensureBlock("tool");

    const queryStart = lines.findIndex((line) => looksLikeCypherLine(line) || line === "Cypher:");
    if (queryStart > 0 && lines[queryStart] === "Cypher:") {
      writeRawLine(
        terminalAnsiWrap(`[tool] ${lines[0]}`, TERMINAL_MUTED_RGB, { dim: true })
      );
      for (const line of lines.slice(queryStart + 1)) {
        if (!line.trim()) {
          continue;
        }
        writeRawLine(
          `${terminalAnsiWrap("    ", TERMINAL_QUERY_RGB)}${terminalAnsiWrap(
            line,
            TERMINAL_QUERY_RGB
          )}`
        );
      }
      return;
    }

    if (queryStart > 0) {
      writeRawLine(
        terminalAnsiWrap(`[tool] ${lines[0]}`, TERMINAL_MUTED_RGB, { dim: true })
      );
      for (const line of lines.slice(queryStart)) {
        if (!line.trim()) {
          continue;
        }
        writeRawLine(
          `${terminalAnsiWrap("    ", TERMINAL_QUERY_RGB)}${terminalAnsiWrap(
            line,
            TERMINAL_QUERY_RGB
          )}`
        );
      }
      return;
    }

    writeRawLine(
      terminalAnsiWrap(`[tool] ${lines[0]}`, TERMINAL_MUTED_RGB, { dim: true })
    );
    for (const line of lines.slice(1)) {
      if (!line.trim()) {
        continue;
      }
      writeRawLine(terminalAnsiWrap(`       ${line}`, TERMINAL_SOFT_RGB, { dim: true }));
    }
  };

  for (const item of items) {
    if (typeof item === "string") {
      stopLiveStatus({ persist: false });
      writeAssistant(item);
      continue;
    }
    if (!item || typeof item !== "object") {
      stopLiveStatus({ persist: false });
      writeAssistant(String(item));
      continue;
    }

    const kind = String(tryGetFirst(item, ["kind", "type", "event"]) || "text").toLowerCase();
    const text = agentTextFromItem(item);
    if (kind === "cypher" || kind === "query") {
      const query = tryGetFirst(item, ["cypher", "query"]) || text;
      if (query) {
        stopLiveStatus({ persist: false });
        writeToolBlock(`Cypher:\n${query}`);
      }
      continue;
    }
    if (kind === "tool" || kind === "tool_call") {
      if (text && isPlaceholderToolTranscript(text)) {
        continue;
      }
      stopLiveStatus({ persist: false });
      const toolName = tryGetFirst(item, ["name", "tool", "toolName"]);
      const input = tryGetFirst(item, ["input", "args", "arguments"]);
      if (text) {
        writeToolBlock(text);
      } else {
        writeToolBlock(
          input
            ? `${toolName || "tool"}\n${formatResult(input)}`
            : String(toolName || "tool")
        );
      }
      continue;
    }
    if (kind === "tool_result") {
      stopLiveStatus({ persist: false });
      const toolName = tryGetFirst(item, ["name", "tool", "toolName"]) || "tool result";
      writeMuted(`${toolName}: ${summarizeAgentToolResult(toolName, item)}`);
      continue;
    }
    if (kind === "error") {
      stopLiveStatus({ persist: false });
      writeWarning(text || formatResult(item));
      continue;
    }
    if (kind === "notice" || kind === "info") {
      stopLiveStatus({ persist: false });
      writeMuted(text || formatResult(item));
      continue;
    }
    if (kind === "system") {
      if (text && /^thinking about the request\.?$/i.test(text.trim())) {
        startMetaStatus("Thinking about the request");
        continue;
      }
      if (text && /^opencode progress:/i.test(text.trim())) {
        const progressText = text.trim().replace(/^opencode progress:\s*/i, "");
        const label = progressText ? `Progress: ${progressText}` : "Progress";
        startMetaStatus(label.length > 120 ? `${label.slice(0, 117)}...` : label);
        continue;
      }
      stopLiveStatus({ persist: false });
      writeMuted(text || formatResult(item));
      continue;
    }
    if (kind === "assistant" || kind === "message") {
      if (text) {
        if (/^preparing \d+ viewer action/i.test(text)) {
          startMetaStatus(text, { persistOnStop: true });
        } else if (
          /^prepared \d+ validated ui action/i.test(text) ||
          /^completed \d+ read-only cypher quer/i.test(text)
        ) {
          stopLiveStatus({ persist: false });
          writeMuted(text);
        } else if (/^applied:/i.test(text)) {
          stopLiveStatus({ persist: false });
          writeSuccess(text);
        } else {
          stopLiveStatus({ persist: false });
          writeAssistant(text);
        }
      } else {
        stopLiveStatus({ persist: false });
      }
      continue;
    }
    if (kind === "action") {
      stopLiveStatus({ persist: false });
      const actionKind = tryGetFirst(item, ["actionKind", "name", "label"]) || "action";
      writeSuccess(text ? `${actionKind}: ${text}` : String(actionKind));
      continue;
    }
    stopLiveStatus({ persist: false });
    writeAssistant(text || formatResult(item));
  }
}
