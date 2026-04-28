import { postJson } from "../net/http.js";
import { tryGetFirst } from "../util/object.js";

export function createAgentClient({ post = postJson } = {}) {
  return {
    session(payload) {
      return post("/api/agent/session", payload);
    },
    turnStart(payload) {
      return post("/api/agent/turn-start", payload);
    },
    turnPoll(payload) {
      return post("/api/agent/turn-poll", payload);
    },
  };
}

export function extractAgentSessionId(payload) {
  const value = tryGetFirst(payload, ["sessionId", "id"]);
  return value === null || value === undefined ? null : String(value);
}

export function extractAgentSchemaId(payload) {
  const value = tryGetFirst(payload, ["schemaId", "schema_id"]);
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function extractAgentSchemaSlug(payload) {
  const value = tryGetFirst(payload, ["schemaSlug", "schema_slug"]);
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function extractAgentTranscriptItems(payload) {
  const items = tryGetFirst(payload, ["transcript", "events", "items"]);
  if (Array.isArray(items)) {
    return items.map((item) =>
      item && typeof item === "object" && item.item !== undefined ? item.item : item
    );
  }
  const message = tryGetFirst(payload, ["message", "notice"]);
  return typeof message === "string" && message.trim() ? [{ kind: "notice", text: message }] : [];
}

export function extractAgentTurnId(payload) {
  const value = tryGetFirst(payload, ["turnId", "id"]);
  return value === null || value === undefined ? null : String(value);
}

export function extractAgentTurnResult(payload) {
  const result = tryGetFirst(payload, ["result"]);
  return result && typeof result === "object" ? result : null;
}

export function extractAgentTurnError(payload) {
  const error = tryGetFirst(payload, ["error"]);
  return typeof error === "string" && error.trim() ? error.trim() : null;
}

export function extractAgentActions(payload) {
  const actions = tryGetFirst(payload, ["actions", "uiActions", "commands"]);
  return Array.isArray(actions) ? actions : [];
}
