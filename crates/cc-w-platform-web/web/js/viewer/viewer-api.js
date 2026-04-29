import {
  viewer_clear_selection,
  viewer_available_profiles_json,
  viewer_current_resource,
  viewer_current_profile,
  viewer_default_element_ids,
  viewer_frame_visible,
  viewer_hide_elements,
  viewer_add_inspection_elements,
  viewer_clear_inspection,
  viewer_inspect_elements,
  viewer_inspected_element_ids,
  viewer_list_element_ids,
  viewer_pick_at_json,
  viewer_pick_rect_json,
  viewer_reference_grid_visible,
  viewer_resize_viewport,
  viewer_resource_catalog_json,
  viewer_reset_all_visibility,
  viewer_reset_element_visibility,
  viewer_remove_inspection_elements,
  viewer_select_elements,
  viewer_selected_element_ids,
  viewer_set_clear_color,
  viewer_set_profile,
  viewer_set_reference_grid_visible,
  viewer_set_view_mode,
  viewer_show_elements,
  viewer_stream_visible as viewer_stream_newly_visible_geometry,
  viewer_suppress_elements,
  viewer_unsuppress_elements,
  viewer_view_state_json,
  viewer_visible_element_ids,
} from "../../pkg/cc_w_platform_web.js";
import { sleep } from "../net/http.js";
import { currentViewerTheme } from "../ui/settings-menu.js";
import { semanticIdsForViewerResource } from "./resource.js";

export function createViewerApi(options = {}) {
  const appStateStore =
    typeof options?.dispatch === "function" ? options : options?.appStateStore || null;
  const setTheme = typeof options?.setTheme === "function" ? options.setTheme : null;
  const parseViewState = (json) => JSON.parse(json);
  const parsePickResult = (json) => JSON.parse(json);
  const parseResourceCatalog = (json) => JSON.parse(json);
  const parseRenderProfiles = (json) => JSON.parse(json);
  const currentViewerElementIds = (ids, options = {}) =>
    semanticIdsForViewerResource(ids, viewer_current_resource(), options);
  const normalizeInspectionMode = (mode) => {
    const value = String(mode || "replace").trim().toLowerCase();
    if (["add", "append", "include", "plus"].includes(value)) {
      return "add";
    }
    if (["remove", "subtract", "exclude", "drop"].includes(value)) {
      return "remove";
    }
    return "replace";
  };
  const dispatchTheme = (theme) => {
    if (typeof setTheme === "function") {
      setTheme(theme);
      return;
    }
    if (appStateStore?.dispatch) {
      appStateStore.dispatch({ type: "theme/set", theme });
      return;
    }
    const root = globalThis.window || globalThis;
    if (root?.wAppState?.dispatch) {
      root.wAppState.dispatch({ type: "theme/set", theme });
      return;
    }
    root?.wHeader?.setTheme?.(theme);
  };
  const setViewModeAsync = async (mode) => {
    return parseViewState(await viewer_set_view_mode(mode));
  };
  const pickAtAsync = async (x, y) => parsePickResult(await viewer_pick_at_json(x, y));
  const pickRectAsync = async (x0, y0, x1, y1) =>
    parsePickResult(await viewer_pick_rect_json(x0, y0, x1, y1));
  const streamNewlyVisibleGeometrySoon = () => {
    viewer_stream_newly_visible_geometry().catch((error) => {
      console.error("viewer automatic geometry streaming failed", error);
    });
  };
  const setViewModeSoon = (mode) => {
    setViewModeAsync(mode).catch((error) => {
      console.error(`viewer setViewMode(${JSON.stringify(mode)}) failed`, error);
    });
    return api.viewState();
  };
  const pickAtSoon = (x, y) => {
    pickAtAsync(x, y).catch((error) => {
      console.error(`viewer pickAt(${x}, ${y}) failed`, error);
    });
    return api.viewState();
  };
  const pickRectSoon = (x0, y0, x1, y1) => {
    pickRectAsync(x0, y0, x1, y1).catch((error) => {
      console.error(`viewer pickRect(${x0}, ${y0}, ${x1}, ${y1}) failed`, error);
    });
    return api.viewState();
  };
  const api = {
    currentResource: () => viewer_current_resource(),
    currentProfile: () => viewer_current_profile(),
    profile: () => viewer_current_profile(),
    profiles: () => parseRenderProfiles(viewer_available_profiles_json()),
    referenceGridVisible: () => viewer_reference_grid_visible(),
    setReferenceGridVisible: (visible) =>
      parseViewState(viewer_set_reference_grid_visible(Boolean(visible))),
    toggleReferenceGrid: () =>
      parseViewState(viewer_set_reference_grid_visible(!viewer_reference_grid_visible())),
    theme: () => currentViewerTheme(),
    setTheme: (theme) => {
      dispatchTheme(theme);
      return currentViewerTheme();
    },
    setClearColor: (red, green, blue) =>
      parseViewState(viewer_set_clear_color(red, green, blue)),
    resourceCatalog: () => parseResourceCatalog(viewer_resource_catalog_json()),
    viewState: () => parseViewState(viewer_view_state_json()),
    state: () => api.viewState(),
    setProfile: (profile) => parseViewState(viewer_set_profile(profile)),
    setViewMode: (mode) => setViewModeSoon(mode),
    defaultView: () => setViewModeSoon("default"),
    allView: () => setViewModeSoon("all"),
    async: {
      setViewMode: setViewModeAsync,
      defaultView: () => setViewModeAsync("default"),
      allView: () => setViewModeAsync("all"),
      pickAt: pickAtAsync,
      pickRect: pickRectAsync,
    },
    listElementIds: () => Array.from(viewer_list_element_ids()),
    defaultElementIds: () => Array.from(viewer_default_element_ids()),
    visibleElementIds: () => Array.from(viewer_visible_element_ids()),
    selectedElementIds: () => Array.from(viewer_selected_element_ids()),
    inspectedElementIds: () => Array.from(viewer_inspected_element_ids()),
    selectedInstanceIds: () => api.viewState().selectedInstanceIds || [],
    hide: (ids, options = {}) => {
      return viewer_hide_elements(currentViewerElementIds(ids, options));
    },
    show: (ids, options = {}) => {
      const changed = viewer_show_elements(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    suppress: (ids, options = {}) => {
      return viewer_suppress_elements(currentViewerElementIds(ids, options));
    },
    unsuppress: (ids, options = {}) => {
      const changed = viewer_unsuppress_elements(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    resetVisibility: (ids, options = {}) => {
      const changed = viewer_reset_element_visibility(currentViewerElementIds(ids, options));
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    resetAllVisibility: () => {
      const changed = viewer_reset_all_visibility();
      streamNewlyVisibleGeometrySoon();
      return changed;
    },
    select: (ids, options = {}) => viewer_select_elements(currentViewerElementIds(ids, options)),
    clearSelection: () => viewer_clear_selection(),
    inspect: (ids, options = {}) => {
      const elementIds = currentViewerElementIds(ids, options);
      const mode = normalizeInspectionMode(options.mode);
      if (mode === "add") {
        return viewer_add_inspection_elements(elementIds);
      }
      if (mode === "remove") {
        return viewer_remove_inspection_elements(elementIds);
      }
      return viewer_inspect_elements(elementIds);
    },
    addInspection: (ids, options = {}) =>
      api.inspect(ids, { ...options, mode: "add" }),
    removeInspection: (ids, options = {}) =>
      api.inspect(ids, { ...options, mode: "remove" }),
    clearInspection: () => viewer_clear_inspection(),
    pickAt: pickAtSoon,
    pickRect: pickRectSoon,
    frameVisible: () => viewer_frame_visible(),
    resizeViewport: () => viewer_resize_viewport(),
    queryCypher: async (cypher, resource = viewer_current_resource()) => {
      const response = await fetch("/api/cypher", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ resource, cypher }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Cypher query failed (${response.status})`);
      }
      return payload;
    },
    queryIds: async (cypher, resource = viewer_current_resource()) => {
      const payload = await api.queryCypher(cypher, resource);
      return Array.isArray(payload.semanticElementIds) ? payload.semanticElementIds : [];
    },
    queryGraphSubgraph: async (
      seedNodeIds,
      options = {},
      resource = viewer_current_resource()
    ) => {
      const response = await fetch("/api/graph/subgraph", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          resource,
          seedNodeIds,
          hops: options.hops,
          maxNodes: options.maxNodes,
          maxEdges: options.maxEdges,
          mode: options.mode,
        }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Graph query failed (${response.status})`);
      }
      return payload;
    },
    queryGraphNodeProperties: async (
      dbNodeId,
      options = {},
      resource = viewer_current_resource()
    ) => {
      const response = await fetch("/api/graph/node-properties", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          resource,
          dbNodeId,
          maxRelations: options.maxRelations,
        }),
      });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) {
        throw new Error(payload.error || `Node property query failed (${response.status})`);
      }
      return payload;
    },
  };
  return api;
}

export async function waitForViewerReady(viewer, attempts = 160) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const state = viewer.viewState();
      const catalog = viewer.resourceCatalog();
      if (state?.resource && Array.isArray(catalog?.resources) && catalog.resources.length) {
        return { state, catalog };
      }
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  throw new Error(
    `w web viewer did not initialize before the JS shell booted${
      lastError?.message ? `: ${lastError.message}` : ""
    }`
  );
}
