import { isKnownResource } from "../viewer/resource.js";
import { installLineTerminal } from "./line-terminal.js";

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const REPL_GLOBAL_ASSIGN = /(^|[;{]\s*)(?:var|let|const)\s+([A-Za-z_$][\w$]*)\s*=/gm;
const GRAPH_REPL_CALLS = [
  "reset",
  "expandSelected",
  "expand",
  "relayout",
  "clear",
  "focusSelected",
  "mode",
  "snapshot",
];
const REPL_INTRO_LINES = [
  'resource(), profile(), profiles(), setProfile(name), referenceGridVisible(), setReferenceGridVisible(true), theme(), setTheme("light"), allView(), defaultView(), sceneBounds(), viewState(), state(), section.state(), setSection({...}), clearSection(), annotations.state(), setAnnotationLayer({...}), showPath({...}), clearAnnotations(), pickAt(x,y), pickRect(x0,y0,x1,y1), query(...), ids(...), hide([...]), show([...]), select([...]), inspect([...]), addInspection([...]), removeInspection([...]), clearInspection(), graph.reset(...), frame()',
  'Example: graph.reset("MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1");',
  'Example: var walls = ids("MATCH (w:IfcWall) RETURN w.GlobalId AS global_id LIMIT 8"); hide(walls);',
  "Enter runs. Up/Down walks history. Ctrl+C clears the line.",
];
const REPL_SCOPE_PREAMBLE =
  "const { viewer, graph, resource, profile, profiles, setProfile, referenceGridVisible, setReferenceGridVisible, toggleReferenceGrid, theme, setTheme, viewState, state, sceneBounds, section, setSection, clearSection, sectionState, annotations, setAnnotationLayer, showPath, clearAnnotations, annotationsState, setViewMode, defaultView, allView, setViewModeAsync, defaultViewAsync, allViewAsync, pickAt, pickRect, pickAtAsync, pickRectAsync, listIds, visibleIds, selectedIds, inspectedIds, selectedInstanceIds, query, queryIds, ids, hide, show, select, inspect, addInspection, removeInspection, resetVisibility, clearSelection, clearInspection, frame, frameVisible, hideQuery, showQuery, selectQuery, inspectQuery } = api;";

export function rewriteReplSource(source) {
  let rewritten = source
    .replace(REPL_GLOBAL_ASSIGN, "$1globalThis.$2 =")
    .replace(/(^|[=(,:]\s*|return\s+)ids\s*\(/gm, "$1await ids(")
    .replace(/(^|[=(,:]\s*|return\s+)queryIds\s*\(/gm, "$1await queryIds(")
    .replace(/(^|[=(,:]\s*|return\s+)query\s*\(/gm, "$1await query(")
    .replace(/(^|[=(,:]\s*|return\s+)hideQuery\s*\(/gm, "$1await hideQuery(")
    .replace(/(^|[=(,:]\s*|return\s+)showQuery\s*\(/gm, "$1await showQuery(")
    .replace(/(^|[=(,:]\s*|return\s+)selectQuery\s*\(/gm, "$1await selectQuery(")
    .replace(/(^|[=(,:]\s*|return\s+)showPath\s*\(/gm, "$1await showPath(");
  for (const method of GRAPH_REPL_CALLS) {
    const pattern = new RegExp(
      `(^|[=(,:]\\s*|return\\s+)graph\\\\.${method}\\s*\\(`,
      "gm"
    );
    rewritten = rewritten.replace(pattern, `$1await graph.${method}(`);
  }
  return rewritten;
}

export function createReplApi(viewer, graph) {
  const currentQueryResource = (resource = viewer.currentResource()) => {
    if (!isKnownResource(resource)) {
      throw new Error(
        `Current resource \`${resource}\` is not an IFC model or project.`
      );
    }
    return resource;
  };

  const api = {
    viewer,
    graph,
    resource: () => viewer.currentResource(),
    profile: () => viewer.profile(),
    profiles: () => viewer.profiles(),
    referenceGridVisible: () => viewer.referenceGridVisible(),
    setReferenceGridVisible: (visible) => viewer.setReferenceGridVisible(visible),
    toggleReferenceGrid: () => viewer.toggleReferenceGrid(),
    theme: () => viewer.theme(),
    viewState: () => viewer.viewState(),
    state: () => viewer.state(),
    sceneBounds: () => viewer.viewState().sceneBounds,
    section: viewer.section,
    setSection: (spec) => viewer.section.set(spec),
    clearSection: () => viewer.section.clear(),
    sectionState: () => viewer.section.state(),
    annotations: viewer.annotations,
    setAnnotationLayer: (spec) => viewer.annotations.set(spec),
    showPath: (spec) => viewer.annotations.showPath(spec),
    clearAnnotations: (layerId = null) => viewer.annotations.clear(layerId),
    annotationsState: () => viewer.annotations.state(),
    setProfile: (profile) => viewer.setProfile(profile),
    setTheme: (theme) => viewer.setTheme(theme),
    setViewMode: (mode) => viewer.setViewMode(mode),
    defaultView: () => viewer.defaultView(),
    allView: () => viewer.allView(),
    setViewModeAsync: (mode) => viewer.async.setViewMode(mode),
    defaultViewAsync: () => viewer.async.defaultView(),
    allViewAsync: () => viewer.async.allView(),
    pickAt: (x, y) => viewer.pickAt(x, y),
    pickRect: (x0, y0, x1, y1) => viewer.pickRect(x0, y0, x1, y1),
    pickAtAsync: (x, y) => viewer.async.pickAt(x, y),
    pickRectAsync: (x0, y0, x1, y1) => viewer.async.pickRect(x0, y0, x1, y1),
    listIds: () => viewer.listElementIds(),
    visibleIds: () => viewer.visibleElementIds(),
    selectedIds: () => viewer.selectedElementIds(),
    inspectedIds: () => viewer.inspectedElementIds(),
    selectedInstanceIds: () => viewer.selectedInstanceIds(),
    query: async (cypher, resource = currentQueryResource()) =>
      viewer.queryCypher(cypher, resource),
    queryIds: async (cypher, resource = currentQueryResource()) =>
      viewer.queryIds(cypher, resource),
    ids: async (cypher, resource = currentQueryResource()) =>
      viewer.queryIds(cypher, resource),
    hide: (ids, options = {}) => viewer.hide(ids, options),
    show: (ids, options = {}) => viewer.show(ids, options),
    select: (ids, options = {}) => viewer.select(ids, options),
    inspect: (ids, options = {}) => viewer.inspect(ids, options),
    addInspection: (ids, options = {}) => viewer.addInspection(ids, options),
    removeInspection: (ids, options = {}) => viewer.removeInspection(ids, options),
    resetVisibility: (ids, options = {}) =>
      Array.isArray(ids) ? viewer.resetVisibility(ids, options) : viewer.resetAllVisibility(),
    clearSelection: () => viewer.clearSelection(),
    clearInspection: () => viewer.clearInspection(),
    frame: () => viewer.frameVisible(),
    frameVisible: () => viewer.frameVisible(),
    resizeViewport: () => viewer.resizeViewport(),
    hideQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.hide(ids, { sourceResource: resource });
      return ids;
    },
    showQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.show(ids, { sourceResource: resource });
      return ids;
    },
    selectQuery: async (cypher, resource = currentQueryResource()) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.select(ids, { sourceResource: resource });
      return ids;
    },
    inspectQuery: async (cypher, resource = currentQueryResource(), options = {}) => {
      const ids = await api.queryIds(cypher, resource);
      viewer.inspect(ids, { ...options, sourceResource: resource });
      return ids;
    },
  };

  return api;
}

export function installRepl(replApi) {
  return installLineTerminal({
    screenId: "repl-screen",
    hostId: "repl-terminal",
    introLines: [...REPL_INTRO_LINES],
    execute: async (code) => {
      const rewritten = rewriteReplSource(code);
      const expression = rewritten.trim().replace(/;+$/g, "");
      let fn = null;
      if (expression) {
        try {
          fn = new AsyncFunction(
            "api",
            "window",
            "document",
            `"use strict"; ${REPL_SCOPE_PREAMBLE} return (async () => (${expression}))();`
          );
        } catch (_error) {
          fn = null;
        }
      }
      fn =
        fn ||
        new AsyncFunction(
          "api",
          "window",
          "document",
          `"use strict"; ${REPL_SCOPE_PREAMBLE} return (async () => { ${rewritten} })();`
        );
      return fn(replApi, window, document);
    },
  });
}
