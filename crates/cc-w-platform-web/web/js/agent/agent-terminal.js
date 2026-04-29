import {
  createAgentClient,
  extractAgentActions,
  extractAgentSchemaId,
  extractAgentSchemaSlug,
  extractAgentSessionId,
  extractAgentTranscriptItems,
  extractAgentTurnError,
  extractAgentTurnId,
  extractAgentTurnResult,
} from "./client.js";
import { createAgentActionApplier } from "./actions.js";
import { getJson, sleep } from "../net/http.js";
import { tryGetFirst } from "../util/object.js";
import {
  createResourceCatalogState,
  isIfcResource,
  isKnownResource,
  safeViewerCurrentResource,
  selectedAgentResource as selectedAgentResourceForViewer,
} from "../viewer/resource.js";
import { mapGraphSubgraphResponse } from "../graph/graph-mapping.js";
import {
  TERMINAL_SUCCESS_RGB,
  TERMINAL_WARNING_RGB,
  formatTerminalErrorMessage,
  terminalAnsiWrap,
} from "../terminal/ansi.js";
import {
  compactAgentTranscriptItems,
  renderAgentTranscriptItems,
} from "../terminal/transcript-renderer.js";
import {
  TERMINAL_NO_RESULT,
  installLineTerminal,
} from "../terminal/line-terminal.js";

function defaultRevealGraphPanel(win = globalThis.window) {
  win?.wHeader?.showGraph?.();
  const requestFrame = win?.requestAnimationFrame?.bind(win) || globalThis.requestAnimationFrame;
  if (typeof requestFrame !== "function") {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    requestFrame(() => {
      requestFrame(resolve);
    });
  });
}

function optionCatalogState(options, fallbackCatalogState, win = globalThis.window) {
  if (typeof options.getCatalogState === "function") {
    const catalogState = options.getCatalogState();
    if (catalogState && typeof catalogState === "object") {
      return catalogState;
    }
  }
  if (options.catalogState && typeof options.catalogState === "object") {
    return options.catalogState;
  }
  const pickerCatalogState = win?.wResourcePicker?.catalogState?.();
  if (pickerCatalogState && typeof pickerCatalogState === "object") {
    return pickerCatalogState;
  }
  return fallbackCatalogState;
}

export function installAgentTerminal(viewer, graph, options = {}) {
  const doc = options.document || globalThis.document;
  const win = options.window || globalThis.window;
  const fallbackCatalogState = createResourceCatalogState();
  const readCatalogState = () => optionCatalogState(options, fallbackCatalogState, win);
  const state = {
    sessionId: null,
    resource: null,
    schemaId: null,
    schemaSlug: null,
    bindingPromise: null,
    capabilities: null,
    capabilitiesPromise: null,
    backendId: null,
    modelId: null,
    levelId: null,
    activated: false,
  };
  const modelSelect =
    options.modelSelect === undefined
      ? doc?.getElementById?.("agent-model-select")
      : options.modelSelect;
  const levelSelect =
    options.levelSelect === undefined
      ? doc?.getElementById?.("agent-level-select")
      : options.levelSelect;
  const levelControl = levelSelect?.closest("label");
  const agentClient = createAgentClient();
  const agentActions = createAgentActionApplier({
    viewer,
    graph,
    isKnownResource,
    isIfcResource,
    safeViewerCurrentResource,
    mapGraphSubgraphResponse,
    revealGraphPanel: async () => {
      if (typeof options.revealGraphPanel === "function") {
        await options.revealGraphPanel();
        return;
      }
      await defaultRevealGraphPanel(win);
    },
  });

  const currentAgentResource = (resource = null) =>
    selectedAgentResourceForViewer(viewer, resource, {
      picker:
        options.resourcePicker === undefined
          ? doc?.getElementById?.("resource-picker")
          : options.resourcePicker,
      catalogState: readCatalogState(),
    });

  const selectedBackendCapability = () => {
    const backends = Array.isArray(state.capabilities?.backends)
      ? state.capabilities.backends
      : [];
    return (
      backends.find((backend) => backend.id === state.backendId) ||
      backends.find((backend) => backend.id === state.capabilities?.defaultBackendId) ||
      backends[0] ||
      null
    );
  };

  const renderCapabilitySelect = (select, options, selectedId, fallbackLabel) => {
    if (!select) {
      return;
    }
    const selectDocument = select.ownerDocument || doc || globalThis.document;
    select.innerHTML = "";
    const safeOptions = Array.isArray(options) ? options : [];
    if (!safeOptions.length) {
      const option = selectDocument.createElement("option");
      option.value = "";
      option.textContent = fallbackLabel;
      select.appendChild(option);
      select.disabled = true;
      return;
    }
    for (const item of safeOptions) {
      const option = selectDocument.createElement("option");
      option.value = String(item.id || "");
      option.textContent = String(item.label || item.id || "unknown");
      select.appendChild(option);
    }
    select.value = safeOptions.some((item) => item.id === selectedId)
      ? selectedId
      : String(safeOptions[0].id || "");
    select.disabled = safeOptions.length <= 1;
  };

  const renderAgentCapabilityControls = () => {
    const backend = selectedBackendCapability();
    if (!backend) {
      renderCapabilitySelect(modelSelect, [], null, "no models");
      renderCapabilitySelect(levelSelect, [], null, "no levels");
      if (levelControl) {
        levelControl.hidden = true;
      }
      return;
    }
    state.backendId = backend.id;
    const models = Array.isArray(backend.models) ? backend.models : [];
    state.modelId =
      models.find((item) => item.id === state.modelId)?.id ||
      state.capabilities?.defaultModelId ||
      models[0]?.id ||
      null;
    const levelsByModel =
      backend.levelsByModel && typeof backend.levelsByModel === "object"
        ? backend.levelsByModel
        : {};
    const modelLevels = Array.isArray(levelsByModel[state.modelId])
      ? levelsByModel[state.modelId]
      : [];
    const backendLevels = Array.isArray(backend.levels) ? backend.levels : [];
    const levels = modelLevels.length ? modelLevels : backendLevels;
    const defaultLevelId = levels.some((item) => item.id === state.capabilities?.defaultLevelId)
      ? state.capabilities.defaultLevelId
      : levels[Math.floor(Math.max(levels.length - 1, 0) / 2)]?.id || null;
    state.levelId =
      levels.find((item) => item.id === state.levelId)?.id ||
      defaultLevelId ||
      null;
    renderCapabilitySelect(modelSelect, models, state.modelId, "no models");
    renderCapabilitySelect(levelSelect, levels, state.levelId, "no levels");
    if (levelControl) {
      levelControl.hidden = !levels.length;
    }
  };

  const loadAgentCapabilities = async () => {
    if (state.capabilities) {
      renderAgentCapabilityControls();
      return state.capabilities;
    }
    if (!state.capabilitiesPromise) {
      state.capabilitiesPromise = getJson("/api/agent/capabilities")
        .then((payload) => {
          state.capabilities = payload;
          state.backendId = payload.defaultBackendId || state.backendId;
          state.modelId = payload.defaultModelId || state.modelId;
          state.levelId = payload.defaultLevelId || state.levelId;
          renderAgentCapabilityControls();
          return payload;
        })
        .catch((error) => {
          renderCapabilitySelect(modelSelect, [], null, "unavailable");
          renderCapabilitySelect(levelSelect, [], null, "unavailable");
          throw error;
        })
        .finally(() => {
          state.capabilitiesPromise = null;
        });
    }
    return state.capabilitiesPromise;
  };

  modelSelect?.addEventListener("change", () => {
    state.modelId = modelSelect.value || null;
    state.levelId = null;
    renderAgentCapabilityControls();
  });
  levelSelect?.addEventListener("change", () => {
    state.levelId = levelSelect.value || null;
  });

  const bindSession = async ({ resource = currentAgentResource(), force = false } = {}) => {
    if (!resource) {
      state.sessionId = null;
      state.resource = null;
      return {
        sessionId: null,
        resource: null,
        schemaId: null,
        schemaSlug: null,
        transcript: [],
      };
    }

    if (!force && state.sessionId && state.resource === resource) {
      return {
        sessionId: state.sessionId,
        resource: state.resource,
        schemaId: state.schemaId,
        schemaSlug: state.schemaSlug,
        transcript: [],
        reused: true,
      };
    }

    if (!force && state.bindingPromise) {
      return state.bindingPromise;
    }

    const previousSessionId = state.sessionId;
    const previousResource = state.resource;
    const request = agentClient.session({
      sessionId: previousSessionId,
      resource,
    }).then((payload) => {
      const nextSessionId = extractAgentSessionId(payload);
      if (!nextSessionId) {
        throw new Error("Agent session response did not include a session id.");
      }
      state.sessionId = nextSessionId;
      state.resource = String(tryGetFirst(payload, ["resource"]) || resource);
      state.schemaId = extractAgentSchemaId(payload);
      state.schemaSlug = extractAgentSchemaSlug(payload);
      return {
        sessionId: state.sessionId,
        resource: state.resource,
        schemaId: state.schemaId,
        schemaSlug: state.schemaSlug,
        transcript: extractAgentTranscriptItems(payload),
        created: Boolean(tryGetFirst(payload, ["created"])),
        rebound: Boolean(tryGetFirst(payload, ["rebound"])) || previousResource !== state.resource,
        previousSessionId,
      };
    });
    state.bindingPromise = request.finally(() => {
      if (state.bindingPromise === request) {
        state.bindingPromise = null;
      }
    });
    return state.bindingPromise;
  };

  let shell = null;

  const writeResourceNotice = async (resource, { forceRebind = false } = {}) => {
    if (!shell) {
      return;
    }
    if (!resource) {
      state.sessionId = null;
      state.resource = null;
      state.schemaId = null;
      state.schemaSlug = null;
      shell.writeLine("AI context cleared. Select an IFC model or project to use the agent terminal.");
      return;
    }
    try {
      const session = await bindSession({ resource, force: forceRebind });
      renderAgentTranscriptItems(shell, compactAgentTranscriptItems(session.transcript));
      shell.writeLine(
        session.schemaId
          ? `AI context switched to ${session.resource} (${session.schemaId}).`
          : `AI context switched to ${session.resource}.`
      );
    } catch (error) {
      shell.writeLine(`AI context bind failed: ${error}`);
    }
  };

  const baseShell = installLineTerminal({
    screenId: options.screenId || "repl-screen",
    hostId: options.hostId || "agent-terminal",
    introLines: [
      "AI agent terminal.",
      "The agent is bound to the active IFC project/model and can only request read-only Cypher plus validated viewer actions.",
      "Enter runs. Up/Down walks history. Ctrl+C clears the line.",
    ],
    execute: async (code, terminal) => {
      const resource = currentAgentResource();
      if (!resource) {
        terminal.writeLine("AI agent is available only while an IFC model or project is selected.");
        return TERMINAL_NO_RESULT;
      }

      await loadAgentCapabilities();
      const session = await bindSession({ resource });
      if (!session.reused) {
        renderAgentTranscriptItems(
          terminal,
          compactAgentTranscriptItems(session.transcript)
        );
      }

      const startPayload = await agentClient.turnStart({
        sessionId: session.sessionId,
        resource: session.resource,
        input: code,
        prompt: code,
        backendId: state.backendId,
        modelId: state.modelId,
        levelId: state.levelId,
      });

      const turnId = extractAgentTurnId(startPayload);
      if (!turnId) {
        throw new Error("Agent turn did not return a turn id.");
      }

      const startedSessionId = extractAgentSessionId(startPayload);
      if (startedSessionId) {
        state.sessionId = startedSessionId;
      }
      state.resource = String(
        tryGetFirst(startPayload, ["resource"]) || session.resource || resource
      );
      state.schemaId = extractAgentSchemaId(startPayload) || session.schemaId || state.schemaId;
      state.schemaSlug = extractAgentSchemaSlug(startPayload) || session.schemaSlug || state.schemaSlug;
      state.backendId = String(tryGetFirst(startPayload, ["backendId"]) || state.backendId || "");
      state.modelId = String(tryGetFirst(startPayload, ["modelId"]) || state.modelId || "");
      state.levelId = String(tryGetFirst(startPayload, ["levelId"]) || state.levelId || "");
      renderAgentCapabilityControls();

      let afterSeq = 0;
      let resultPayload = null;
      while (true) {
        const pollPayload = await agentClient.turnPoll({
          turnId,
          afterSeq,
        });
        const events = extractAgentTranscriptItems(pollPayload);
        if (Array.isArray(pollPayload?.events) && pollPayload.events.length) {
          const lastEvent = pollPayload.events[pollPayload.events.length - 1];
          const seq = Number(lastEvent?.seq);
          if (Number.isFinite(seq)) {
            afterSeq = Math.max(afterSeq, seq);
          }
        }
        renderAgentTranscriptItems(terminal, events);
        if (Boolean(tryGetFirst(pollPayload, ["done"]))) {
          const pollError = extractAgentTurnError(pollPayload);
          if (pollError) {
            throw new Error(pollError);
          }
          resultPayload = extractAgentTurnResult(pollPayload);
          break;
        }
        await sleep(150);
      }

      if (!resultPayload) {
        return TERMINAL_NO_RESULT;
      }

      const nextSessionId = extractAgentSessionId(resultPayload);
      if (nextSessionId) {
        state.sessionId = nextSessionId;
      }
      state.resource = String(
        tryGetFirst(resultPayload, ["resource"]) || session.resource || resource
      );
      state.schemaId = extractAgentSchemaId(resultPayload) || session.schemaId || state.schemaId;
      state.schemaSlug = extractAgentSchemaSlug(resultPayload) || session.schemaSlug || state.schemaSlug;

      const actions = extractAgentActions(resultPayload);
      const appliedKinds = [];
      for (const action of actions) {
        try {
          const appliedKind = await agentActions.apply(action);
          if (appliedKind) {
            appliedKinds.push(appliedKind);
          }
        } catch (error) {
          terminal.writeRawLine(
            terminalAnsiWrap(`action failed: ${error}`, TERMINAL_WARNING_RGB, {
              bold: true,
            })
          );
        }
      }
      if (appliedKinds.length) {
        terminal.writeRawLine(
          terminalAnsiWrap(`applied: ${appliedKinds.join(", ")}`, TERMINAL_SUCCESS_RGB, {
            bold: true,
          })
        );
      }
      return TERMINAL_NO_RESULT;
    },
  });

  if (!baseShell) {
    return null;
  }

  void loadAgentCapabilities().catch((error) => {
    console.warn("AI capabilities unavailable", error);
  });

  shell = {
    ...baseShell,
    activate() {
      baseShell.activate();
      void loadAgentCapabilities().catch((error) => {
        baseShell.writeLine(`AI capabilities unavailable: ${formatTerminalErrorMessage(error)}`);
      });
      if (!state.activated) {
        state.activated = true;
        const resource = currentAgentResource();
        if (resource) {
          void bindSession({ resource })
            .then((session) => {
              renderAgentTranscriptItems(
                baseShell,
                compactAgentTranscriptItems(session.transcript)
              );
            })
            .catch((error) => {
              baseShell.writeLine(`AI context bind failed: ${error}`);
            });
        }
      }
    },
    async handleResourceSwitch(resource = null) {
      const nextResource = currentAgentResource(resource);
      await writeResourceNotice(nextResource, { forceRebind: true });
    },
  };

  return shell;
}
