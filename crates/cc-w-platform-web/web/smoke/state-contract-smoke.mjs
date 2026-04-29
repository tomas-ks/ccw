#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(SCRIPT_DIR, "..");
const REPO_ROOT = path.resolve(SCRIPT_DIR, "../../../..");

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertDeepEqual(actual, expected, message) {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`${message}: expected ${expectedJson}, got ${actualJson}`);
  }
}

async function readWebFile(relativePath) {
  return readFile(path.join(WEB_ROOT, relativePath), "utf8");
}

async function importWebModule(relativePath) {
  const source = await readWebFile(relativePath);
  const encoded = Buffer.from(source, "utf8").toString("base64");
  return import(`data:text/javascript;base64,${encoded}#${relativePath}`);
}

function installBrowserStubs() {
  const dispatchedEvents = [];

  globalThis.CustomEvent = class CustomEvent {
    constructor(type, init = {}) {
      this.type = type;
      this.detail = init.detail ?? null;
    }
  };

  globalThis.window = {
    CustomEvent: globalThis.CustomEvent,
    dispatchEvent: (event) => {
      dispatchedEvents.push(event);
      return true;
    },
    localStorage: {
      getItem: () => null,
    },
  };

  globalThis.document = {
    documentElement: {
      dataset: {},
    },
    body: {
      classList: {
        contains: () => false,
      },
    },
    getElementById: (id) => {
      if (id === "tool-picker") {
        return { value: "orbit-pick" };
      }
      return null;
    },
    querySelector: () => null,
  };

  return dispatchedEvents;
}

async function smokeAppState() {
  const events = installBrowserStubs();
  const {
    createAppStateStore,
    parseViewerToolValue,
    viewerToolsToPickerValue,
  } = await importWebModule("js/state/app-state.js");

  assertDeepEqual(
    parseViewerToolValue("orbit-pick-pan"),
    { orbit: true, pick: true, pan: true },
    "tool picker value should normalize to app-state tool flags"
  );
  assertEqual(
    viewerToolsToPickerValue({ orbit: true, pick: false }),
    "orbit-pan",
    "tool flags should serialize with pan coupled to orbit"
  );

  const store = createAppStateStore();
  assertEqual(store.getState().terminal.activeTool, "ai", "AI terminal should be the default tool");
  assertEqual(store.getState().panels.outliner, false, "outliner should default closed");

  const seenActions = [];
  const unsubscribe = store.subscribe((_state, _previous, action) => {
    seenActions.push(action.type);
  });

  store.dispatch({ type: "resource/requested", resource: "project/building" });
  assertEqual(
    store.getState().requestedResource,
    "project/building",
    "resource/requested should record pending intent"
  );

  store.dispatch({
    type: "viewer/committed",
    state: { resource: "ifc/building-architecture", visibleElementIds: [] },
  });
  assertEqual(
    store.getState().requestedResource,
    "project/building",
    "unrelated viewer commit should not clear pending resource intent"
  );

  store.dispatch({
    type: "viewer/committed",
    state: {
      resource: "project/building",
      visibleElementIds: ["ifc/a::1"],
      defaultElementIds: ["ifc/a::1"],
    },
  });
  assertEqual(
    store.getState().requestedResource,
    null,
    "matching viewer commit should clear pending resource intent"
  );

  store.dispatch({ type: "tools/set", tools: { orbit: false, pick: true, pan: true } });
  assertDeepEqual(
    store.getState().tools,
    { orbit: false, pick: true, pan: false },
    "pan should remain derived from orbit"
  );

  store.dispatch({
    type: "focus/set",
    source: "graph",
    resource: "project/building",
    dbNodeId: 42,
    graphNodeId: "node-42",
    semanticId: "ifc/a::wall-1",
  });
  store.dispatch({
    type: "balloon/open",
    source: "graph",
    anchor: { kind: "client", clientX: 10, clientY: 20, anchored: true },
  });
  assertEqual(store.getState().balloon.open, true, "balloon/open should mark balloon open");
  store.dispatch({ type: "focus/clear" });
  assertEqual(store.getState().focus.source, "none", "focus/clear should reset focus source");
  assertEqual(store.getState().balloon.open, false, "focus/clear should close the balloon");
  assertEqual(store.getState().balloon.anchor, null, "focus/clear should clear the balloon anchor");

  unsubscribe();
  assert(seenActions.includes("init"), "subscribers should receive an init action");
  assert(
    events.some((event) => event.type === "w-app-state-change"),
    "store should emit w-app-state-change events"
  );
}

async function smokeResourceHelpers() {
  const {
    normalizeResourceCatalog,
    parseSourceScopedSemanticId,
    resourceCatalogEntries,
    semanticIdsForViewerResource,
  } = await importWebModule("js/viewer/resource.js");

  const catalog = normalizeResourceCatalog({
    resources: ["ifc/a", "ifc/a", "demo/ignored"],
    projects: [
      {
        resource: "project/building",
        label: "Building",
        members: ["ifc/a", "ifc/b", "demo/not-ifc"],
      },
    ],
  });
  assertDeepEqual(
    catalog,
    {
      resources: ["ifc/a", "project/building"],
      projects: [{ resource: "project/building", label: "Building", members: ["ifc/a", "ifc/b"] }],
    },
    "resource catalog should filter, dedupe, and include project resources"
  );

  const entries = resourceCatalogEntries(catalog);
  assertEqual(entries[0].kind, "ifc", "ifc resources should be labeled as ifc entries");
  assertEqual(entries[1].kind, "project", "project resources should be labeled as project entries");
  assertEqual(entries[1].label, "Building", "project resource entry should use catalog label");

  assertDeepEqual(
    parseSourceScopedSemanticId("ifc/a::wall-1"),
    { sourceResource: "ifc/a", semanticId: "wall-1" },
    "source-scoped semantic ids should parse"
  );

  assertDeepEqual(
    semanticIdsForViewerResource(["wall-1"], "project/building", { sourceResource: "ifc/a" }),
    ["ifc/a::wall-1"],
    "project viewer actions should scope unscoped ids to their IFC source"
  );
  assertDeepEqual(
    semanticIdsForViewerResource(["ifc/a::wall-1"], "ifc/a"),
    ["wall-1"],
    "single-IFC viewer actions should un-scope ids from the active source"
  );
}

async function smokeBalloonHelpers() {
  const {
    formatPropertyLabel,
    pickRegionClientCenter,
    positionPropertiesBalloonFromAnchor,
    propertyValueText,
  } = await importWebModule("js/ui/balloon.js");

  const canvas = {
    width: 200,
    height: 100,
    getBoundingClientRect: () => ({ left: 100, top: 50, width: 400, height: 200 }),
  };
  assertDeepEqual(
    pickRegionClientCenter({ region: { x: 10, y: 20, width: 30, height: 40 } }, canvas),
    { x: 150, y: 130 },
    "pick region center should project into client coordinates"
  );

  assertEqual(formatPropertyLabel("ObjectType"), "Object Type", "property labels should format");
  assertEqual(propertyValueText({ a: 1 }), "{\"a\":1}", "object property values should stringify");
  assertEqual(propertyValueText(""), null, "empty property values should be treated as absent");

  const marker = { hidden: true, style: {} };
  const balloonStyle = {
    setProperty(name, value) {
      this[name] = value;
    },
  };
  const balloon = {
    hidden: true,
    dataset: {},
    style: balloonStyle,
    getBoundingClientRect: () => ({ width: 120, height: 80 }),
  };
  const positioned = positionPropertiesBalloonFromAnchor(
    {
      viewport: {
        getBoundingClientRect: () => ({ left: 0, top: 0, width: 400, height: 300 }),
      },
      propertiesBalloon: balloon,
      pickAnchorMarker: marker,
    },
    { kind: "client", clientX: 100, clientY: 100, anchored: true, marker: true }
  );
  assertEqual(positioned, true, "client anchors should position the balloon");
  assertEqual(balloon.hidden, false, "positioned balloon should be visible");
  assertEqual(marker.hidden, false, "marker anchors should show the pick marker");
  assert(Boolean(balloon.style.left), "positioned balloon should set left style");
  assert(Boolean(balloon.style.top), "positioned balloon should set top style");
}

async function smokeStaticContracts() {
  const [indexHtml, plan] = await Promise.all([
    readWebFile("index.html"),
    readFile(path.join(REPO_ROOT, "docs/state-management-implementation-plan.md"), "utf8"),
  ]);

  for (const id of [
    "resource-picker",
    "render-profile-picker",
    "agent-model-select",
    "agent-level-select",
    "properties-balloon",
  ]) {
    assert(indexHtml.includes(`id="${id}"`), `index.html should keep #${id} as a stable DOM anchor`);
  }

  for (const heading of [
    "## Post-Refactor Module Ownership Target",
    "### Graph Shell",
    "### Balloon Controller",
    "### Resource And Profile Pickers",
    "### Chat Readiness",
  ]) {
    assert(plan.includes(heading), `implementation plan should document ${heading}`);
  }
}

async function run() {
  const checks = [
    ["app-state dispatch contract", smokeAppState],
    ["resource helper contract", smokeResourceHelpers],
    ["balloon helper contract", smokeBalloonHelpers],
    ["static state-contract anchors", smokeStaticContracts],
  ];

  for (const [name, check] of checks) {
    await check();
    console.log(`ok - ${name}`);
  }
}

run().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
