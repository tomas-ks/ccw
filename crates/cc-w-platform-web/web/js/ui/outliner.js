import {
  isIfcResource,
  isKnownResource,
  isProjectResource,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
} from "../viewer/resource.js";
import {
  DEFAULT_SEMANTIC_OUTLINER_FACETS,
  DRAWINGS_FACET_ID,
  drawingGroupOutlinerState,
  drawingGroupVisibilityOperation,
  normalizeSemanticOutliner,
  normalizeDrawingsFacet,
  semanticDiagnosticMessage,
  semanticGroupDeclaredCount,
  semanticGroupInspectionOperation,
  semanticGroupInspectionState,
  semanticGroupOutlinerState,
  semanticGroupVisibilityOperation,
  semanticGroupViewerBuckets,
} from "../semantic/outliner-model.mjs";

const WORKSPACE_FACET_ID = "workspace";
const SEMANTICS_GROUP_ID = "semantics";
const SEMANTIC_DEFAULT_FACET_ID = "classes";
const SEMANTIC_FACET_IDS = Object.freeze([
  "layers",
  "classes",
  "spatial",
  "materials",
  "construction",
]);
const BACKEND_ONLY_FACET_IDS = new Set([WORKSPACE_FACET_ID, "project"]);

const PRIMARY_FACET_GROUPS = Object.freeze([
  Object.freeze({ id: WORKSPACE_FACET_ID, label: "Workspace", facetId: WORKSPACE_FACET_ID }),
  Object.freeze({ id: DRAWINGS_FACET_ID, label: "Drawings", facetId: DRAWINGS_FACET_ID }),
  Object.freeze({
    id: SEMANTICS_GROUP_ID,
    label: "Semantics",
    facetIds: SEMANTIC_FACET_IDS,
    defaultFacetId: SEMANTIC_DEFAULT_FACET_ID,
  }),
]);

export {
  DEFAULT_SEMANTIC_OUTLINER_FACETS,
  DRAWINGS_FACET_ID,
  drawingGroupOutlinerState,
  drawingGroupVisibilityOperation,
  isIfcResource,
  isKnownResource,
  isProjectResource,
  normalizeSemanticOutliner,
  normalizeDrawingsFacet,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
  semanticDiagnosticMessage,
  semanticGroupDeclaredCount,
  semanticGroupInspectionOperation,
  semanticGroupInspectionState,
  semanticGroupOutlinerState,
  semanticGroupVisibilityOperation,
  semanticGroupViewerBuckets,
};

export function normalizeProjectCatalog(payload) {
  return {
    resources: Array.isArray(payload?.resources)
      ? payload.resources.map((resource) => String(resource || "").trim()).filter(Boolean)
      : [],
    projects: Array.isArray(payload?.projects)
      ? payload.projects
          .map((project) => ({
            resource: String(project?.resource || "").trim(),
            label: String(project?.label || "").trim(),
            members: Array.isArray(project?.members)
              ? project.members.map((member) => String(member || "").trim()).filter(isIfcResource)
              : [],
          }))
          .filter((project) => isProjectResource(project.resource) && project.members.length)
      : [],
  };
}

export function createProjectCatalogState(initial = {}) {
  return normalizeProjectCatalog(initial);
}

export const defaultProjectCatalogState = createProjectCatalogState();

export function updateProjectCatalogState(
  catalogState = defaultProjectCatalogState,
  payload,
  { window: win = globalThis.window, dispatchEvent = true } = {}
) {
  const normalized = normalizeProjectCatalog(payload);
  catalogState.resources = normalized.resources;
  catalogState.projects = normalized.projects;
  const CustomEventCtor = win?.CustomEvent || globalThis.CustomEvent;
  if (dispatchEvent && win?.dispatchEvent && typeof CustomEventCtor === "function") {
    win?.dispatchEvent?.(
      new CustomEventCtor("w-resource-catalog-change", {
        detail: {
          resources: catalogState.resources,
          projects: catalogState.projects,
        },
      })
    );
  }
  return catalogState;
}

export function projectEntryForResource(
  resource,
  catalogState = defaultProjectCatalogState
) {
  const active = String(resource || "").trim();
  if (!active) {
    return null;
  }
  const projects = Array.isArray(catalogState?.projects) ? catalogState.projects : [];
  const exact = projects.find((project) => project.resource === active);
  if (exact) {
    return exact;
  }
  if (isIfcResource(active)) {
    return {
      resource: active,
      label: "IFC",
      members: [active],
    };
  }
  const containing = projects.find((project) =>
    Array.isArray(project.members) && project.members.includes(active)
  );
  if (containing) {
    return containing;
  }
  return null;
}

export function memberScopedIds(member, ids, resource) {
  const values = Array.isArray(ids)
    ? ids.map((id) => String(id || "").trim()).filter(Boolean)
    : [];
  if (isProjectResource(resource)) {
    const prefix = `${member}::`;
    return values.filter((id) => id.startsWith(prefix));
  }
  if (resource === member) {
    return values.filter((id) => !parseSourceScopedSemanticId(id));
  }
  return [];
}

export function memberDefaultElementIds(member, defaultElementIds, resource) {
  return memberScopedIds(member, defaultElementIds, resource);
}

function memberWorkspaceElementIds(member, viewState, resource) {
  const listElementIds = viewStateElementIds(viewState, "listElementIds");
  const defaultElementIds = viewStateElementIds(viewState, "defaultElementIds");
  const elementIds = listElementIds.length ? listElementIds : defaultElementIds;
  return memberScopedIds(member, elementIds, resource);
}

export function labelForMember(member) {
  return String(member || "").replace(/^ifc\//, "");
}

export function viewStateElementIds(viewState, key) {
  return Array.isArray(viewState?.[key])
    ? viewState[key].map((id) => String(id || "").trim()).filter(Boolean)
    : [];
}

export function memberOutlinerState(member, viewState, resource) {
  const defaultElementIds = viewStateElementIds(viewState, "defaultElementIds");
  const suppressed = new Set(viewStateElementIds(viewState, "suppressedElementIds"));
  const ids = memberDefaultElementIds(member, defaultElementIds, resource);
  const enabledCount = ids.filter((id) => !suppressed.has(id)).length;
  return {
    ids,
    enabledCount,
    totalCount: ids.length,
    checked: ids.length > 0 && enabledCount > 0,
    disabled: ids.length === 0,
    indeterminate: ids.length > 0 && enabledCount > 0 && enabledCount < ids.length,
  };
}

function outlinerEndpointForResource(resource) {
  const encoded = encodeURIComponent(String(resource || "").trim());
  return `/api/semantic/outliner?resource=${encoded}`;
}

function groupCountText(group, state) {
  if (state.allTotalCount > 0) {
    return `${state.allEnabledCount}/${state.allTotalCount}`;
  }
  if (state.totalCount > 0) {
    return `${state.enabledCount}/${state.totalCount}`;
  }
  const declared = semanticGroupDeclaredCount(group);
  return declared > 0 ? `${declared} listed` : "empty";
}

function bucketCountText(state) {
  return state.totalCount > 0 ? String(state.totalCount) : "0";
}

function groupDetailText(group) {
  const detail = String(group?.sourceDetail || "").trim();
  const kind = String(group?.sourceKind || "").trim();
  if (kind === "ifc_graph" || kind === "viewer_inference") {
    return detail && detail !== kind ? detail : "";
  }
  if (detail && kind && detail !== kind) {
    return `${kind}: ${detail}`;
  }
  return detail || kind;
}

function drawingPathText(path) {
  if (!path) {
    return "";
  }
  if (typeof path === "object" && !Array.isArray(path)) {
    const kind = String(path.kind || path.type || "").trim();
    const id = String(path.id || path.pathId || path.path_id || "").trim();
    return [kind, id].filter(Boolean).join(": ");
  }
  return String(path || "").trim();
}

function drawingGroupDetailText(group) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  const path = drawingPathText(metadata.path);
  const resource = String(metadata.resource || "").trim();
  if (path && resource) {
    return `${resource} / ${path}`;
  }
  return path || resource || groupDetailText(group);
}

function drawingCountText(state) {
  if (state.layerIds.length) {
    return `${state.enabledCount}/${state.totalCount}`;
  }
  if (state.totalCount > 0) {
    return `${state.totalCount}`;
  }
  return "empty";
}

function preferredSemanticFacetId(outliner, current = "") {
  const facets = Array.isArray(outliner?.facets) ? outliner.facets : [];
  if (current && facets.some((facet) => facet.id === current)) {
    return current;
  }
  const workspace = facets.find((facet) => facet.id === WORKSPACE_FACET_ID);
  if (workspace) {
    return workspace.id;
  }
  const firstPopulated = facets.find((facet) => Array.isArray(facet.groups) && facet.groups.length);
  return firstPopulated?.id || facets[0]?.id || WORKSPACE_FACET_ID;
}

function hasFacet(outliner, facetId) {
  return Array.isArray(outliner?.facets)
    ? outliner.facets.some((facet) => facet.id === facetId)
    : false;
}

function isSemanticFacetId(facetId) {
  return SEMANTIC_FACET_IDS.includes(String(facetId || ""));
}

function availableSemanticFacets(outliner) {
  const facets = Array.isArray(outliner?.facets) ? outliner.facets : [];
  return SEMANTIC_FACET_IDS
    .map((id) => facets.find((facet) => facet.id === id))
    .filter(Boolean);
}

function preferredSemanticSubFacetId(outliner, current = "") {
  if (isSemanticFacetId(current) && hasFacet(outliner, current)) {
    return current;
  }
  const configuredDefault = PRIMARY_FACET_GROUPS.find(
    (group) => group.id === SEMANTICS_GROUP_ID
  )?.defaultFacetId;
  if (configuredDefault && hasFacet(outliner, configuredDefault)) {
    return configuredDefault;
  }
  return availableSemanticFacets(outliner)[0]?.id || WORKSPACE_FACET_ID;
}

function primaryGroupIdForFacet(facetId) {
  if (facetId === WORKSPACE_FACET_ID) {
    return WORKSPACE_FACET_ID;
  }
  if (facetId === DRAWINGS_FACET_ID) {
    return DRAWINGS_FACET_ID;
  }
  if (isSemanticFacetId(facetId)) {
    return SEMANTICS_GROUP_ID;
  }
  return WORKSPACE_FACET_ID;
}

function backendSemanticFacetsForRender(outliner, resource, { loading = false, error = "" } = {}) {
  if (outliner && outliner.resource === resource) {
    return (outliner.facets || []).filter((facet) => !BACKEND_ONLY_FACET_IDS.has(facet.id));
  }

  const diagnostic = error || (loading ? "Loading semantic groups." : "");
  return DEFAULT_SEMANTIC_OUTLINER_FACETS
    .filter((facet) => !BACKEND_ONLY_FACET_IDS.has(facet.id))
    .map((facet) => ({
      id: facet.id,
      label: facet.label,
      sourceKind: "",
      groups: [],
      diagnostics: diagnostic ? [diagnostic] : [],
    }));
}

function drawingsFacetFromViewState(viewState, resource) {
  for (const candidate of [
    viewState?.drawings,
    viewState?.drawing,
    viewState?.drawingOutliner,
    viewState?.drawing_outliner,
  ]) {
    if (candidate == null) {
      continue;
    }
    const facet = normalizeDrawingsFacet(candidate, { resource });
    if (facet.groups.length) {
      return facet;
    }
  }
  return null;
}

function withViewStateDrawingsFacet(outliner, viewState, resource) {
  const viewStateFacet = drawingsFacetFromViewState(viewState, resource);
  if (!viewStateFacet) {
    return outliner;
  }
  const facets = Array.isArray(outliner?.facets) ? [...outliner.facets] : [];
  const index = facets.findIndex((facet) => facet.id === DRAWINGS_FACET_ID);
  if (index >= 0) {
    if (!facets[index].groups?.length) {
      facets[index] = viewStateFacet;
    }
  } else {
    const layersIndex = facets.findIndex((facet) => facet.id === "layers");
    facets.splice(layersIndex >= 0 ? layersIndex + 1 : facets.length, 0, viewStateFacet);
  }
  return { ...outliner, facets };
}

function workspaceFacetForProject(project, viewState, resource) {
  const members = Array.isArray(project?.members) ? project.members : [];
  return {
    id: WORKSPACE_FACET_ID,
    label: "Workspace",
    sourceKind: "viewer_catalog",
    groups: members.map((member) => {
      const ids = memberWorkspaceElementIds(member, viewState, resource);
      return {
        id: `${WORKSPACE_FACET_ID}:${member}`,
        label: labelForMember(member),
        sourceKind: WORKSPACE_FACET_ID,
        sourceDetail: "",
        sourceResource: member,
        semanticIds: ids,
        children: [],
        counts: {
          elementCount: ids.length,
          total: ids.length,
        },
        metadata: {
          resource: member,
        },
        diagnostics: [],
      };
    }),
    diagnostics: [],
  };
}

function semanticOutlinerForRender(
  backendOutliner,
  project,
  viewState,
  resource,
  status = {}
) {
  return withViewStateDrawingsFacet(
    {
      resource,
      facets: [
        workspaceFacetForProject(project, viewState, resource),
        ...backendSemanticFacetsForRender(backendOutliner, resource, status),
      ],
      diagnostics: backendOutliner?.diagnostics || [],
    },
    viewState,
    resource
  );
}

function renderFacetTabButton(doc, { label, active, onClick, className = "" }) {
  const button = doc.createElement("button");
  button.type = "button";
  button.className = `outliner-facet-tab${className ? ` ${className}` : ""}`;
  button.textContent = label;
  button.setAttribute("role", "tab");
  button.setAttribute("aria-selected", String(active));
  button.classList.toggle("active", active);
  button.addEventListener("click", onClick);
  return button;
}

function renderPrimaryFacetTabs(
  doc,
  outliner,
  activeFacetId,
  lastSemanticFacetId,
  setActiveFacet
) {
  const tabs = doc.createElement("div");
  tabs.className = "outliner-facet-tabs outliner-primary-facet-tabs";
  tabs.setAttribute("role", "tablist");

  const activePrimaryId = primaryGroupIdForFacet(activeFacetId);
  for (const group of PRIMARY_FACET_GROUPS) {
    const targetFacetId =
      group.id === SEMANTICS_GROUP_ID
        ? preferredSemanticSubFacetId(outliner, lastSemanticFacetId)
        : group.facetId;
    if (!targetFacetId || !hasFacet(outliner, targetFacetId)) {
      continue;
    }
    const button = renderFacetTabButton(doc, {
      label: group.label,
      active: group.id === activePrimaryId,
      onClick: () => setActiveFacet(targetFacetId),
      className: "outliner-primary-facet-tab",
    });
    tabs.appendChild(button);
  }
  return tabs;
}

function renderSemanticSubFacetTabs(doc, outliner, activeFacetId, setActiveFacet) {
  if (!isSemanticFacetId(activeFacetId)) {
    return null;
  }
  const facets = availableSemanticFacets(outliner);
  if (facets.length <= 1) {
    return null;
  }

  const tabs = doc.createElement("div");
  tabs.className = "outliner-facet-tabs outliner-secondary-facet-tabs";
  tabs.setAttribute("role", "tablist");
  for (const facet of facets) {
    const button = renderFacetTabButton(doc, {
      label: facet.label || facet.id,
      active: facet.id === activeFacetId,
      onClick: () => setActiveFacet(facet.id),
      className: "outliner-secondary-facet-tab",
    });
    tabs.appendChild(button);
  }
  return tabs;
}

function renderSemanticFacetSwitchers(
  doc,
  outliner,
  activeFacetId,
  lastSemanticFacetId,
  setActiveFacet
) {
  const switcher = doc.createElement("div");
  switcher.className = "outliner-facet-switcher";
  switcher.appendChild(
    renderPrimaryFacetTabs(
      doc,
      outliner,
      activeFacetId,
      lastSemanticFacetId,
      setActiveFacet
    )
  );
  const secondary = renderSemanticSubFacetTabs(
    doc,
    outliner,
    activeFacetId,
    setActiveFacet
  );
  if (secondary) {
    switcher.classList.add("has-secondary-facets");
    switcher.appendChild(secondary);
  }
  return switcher;
}

function semanticGroupSourceLabel(group) {
  const source = String(group?.sourceKind || group?.sourceDetail || "").trim();
  if (!source) {
    return "";
  }
  if (source === WORKSPACE_FACET_ID || source === "viewer_catalog") {
    return "DB";
  }
  if (source === "ifc_graph") {
    return "IFC";
  }
  if (source === "viewer_inference") {
    return "inferred";
  }
  return source.replace(/_/g, " ");
}

function appendSourceBadgeSlot(doc, meta, sourceLabel) {
  const slot = doc.createElement("span");
  slot.className = "outliner-source-slot";
  if (sourceLabel) {
    const badge = doc.createElement("span");
    badge.className = "outliner-source-badge";
    badge.textContent = sourceLabel;
    slot.appendChild(badge);
  }
  meta.appendChild(slot);
}

function semanticGroupActionLabel(group) {
  if (group?.sourceKind === WORKSPACE_FACET_ID && group?.sourceResource) {
    return group.sourceResource;
  }
  return group?.label || "group";
}

export function createProjectOutlinerController({
  viewer,
  appStateStore,
  document: doc = globalThis.document,
  window: win = globalThis.window,
  catalogState = defaultProjectCatalogState,
  getCatalogState = null,
  panel = doc?.getElementById?.("project-outliner") || null,
  body = doc?.getElementById?.("outliner-body") || null,
  subtitle = doc?.getElementById?.("outliner-subtitle") || null,
  toggleButton = doc?.getElementById?.("outliner-toggle-button") || null,
  resetButton = doc?.getElementById?.("outliner-reset-button") || null,
  closeButton = doc?.getElementById?.("outliner-close-button") || null,
  dragHandle = panel?.querySelector?.(".outliner-header") || null,
  picker = doc?.getElementById?.("resource-picker") || null,
  subscribe = true,
} = {}) {
  if (!viewer || !appStateStore || !panel || !body || !toggleButton) {
    return null;
  }

  toggleButton.removeAttribute("disabled");
  let semanticResource = "";
  let semanticOutliner = null;
  let semanticLoadingResource = "";
  let semanticErrorResource = "";
  let semanticError = "";
  let activeSemanticFacetId = WORKSPACE_FACET_ID;
  let lastSemanticFacetId = SEMANTIC_DEFAULT_FACET_ID;
  let renderedScrollKey = "";
  let dragState = null;

  const readCatalogState = () => {
    const state =
      typeof getCatalogState === "function" ? getCatalogState() : catalogState;
    return state && typeof state === "object" ? state : defaultProjectCatalogState;
  };

  const outlinerScrollElement = () => body.querySelector(".outliner-list") || body;

  const replaceBodyChildren = (fragment, resource) => {
    const normalizedResource = String(resource || "").trim();
    const scrollKey = `${normalizedResource}\u0000${activeSemanticFacetId}`;
    const scrollElement = outlinerScrollElement();
    const scrollTop =
      renderedScrollKey === scrollKey && scrollElement.scrollTop > 0
        ? scrollElement.scrollTop
        : 0;
    body.replaceChildren(fragment);
    renderedScrollKey = scrollKey;
    if (scrollTop > 0) {
      const nextScrollElement = outlinerScrollElement();
      const maxScrollTop = Math.max(
        0,
        nextScrollElement.scrollHeight - nextScrollElement.clientHeight
      );
      nextScrollElement.scrollTop = Math.min(scrollTop, maxScrollTop);
    }
    clampCurrentPanelPosition();
  };

  const setVisible = (visible) => {
    appStateStore.dispatch({ type: "panel/set", panel: "outliner", visible });
    return visible;
  };

  const panelBoundsParent = () => {
    const HTMLElementCtor = win?.HTMLElement || globalThis.HTMLElement;
    return HTMLElementCtor && panel.offsetParent instanceof HTMLElementCtor
      ? panel.offsetParent
      : panel.parentElement;
  };

  const clampPanelPosition = (left, top) => {
    const parent = panelBoundsParent();
    if (!parent) {
      return { left, top };
    }
    const parentRect = parent.getBoundingClientRect();
    const panelRect = panel.getBoundingClientRect();
    const inset = 8;
    const maxLeft = Math.max(inset, parentRect.width - panelRect.width - inset);
    const maxTop = Math.max(inset, parentRect.height - panelRect.height - inset);
    return {
      left: Math.round(Math.min(Math.max(left, inset), maxLeft)),
      top: Math.round(Math.min(Math.max(top, inset), maxTop)),
    };
  };

  const setPanelPosition = (left, top) => {
    const position = clampPanelPosition(left, top);
    panel.style.left = `${position.left}px`;
    panel.style.top = `${position.top}px`;
  };

  const currentPanelPosition = () => {
    const parent = panelBoundsParent();
    const panelRect = panel.getBoundingClientRect();
    const parentRect = parent?.getBoundingClientRect?.() || { left: 0, top: 0 };
    return {
      left: panelRect.left - parentRect.left,
      top: panelRect.top - parentRect.top,
    };
  };

  const panelPositionForPointer = (event) => {
    const parent = panelBoundsParent();
    const parentRect = parent?.getBoundingClientRect?.() || { left: 0, top: 0 };
    return {
      left: event.clientX - parentRect.left - dragState.offsetX,
      top: event.clientY - parentRect.top - dragState.offsetY,
    };
  };

  const onPanelDragMove = (event) => {
    if (!dragState) {
      return;
    }
    event.preventDefault();
    const position = panelPositionForPointer(event);
    setPanelPosition(position.left, position.top);
  };

  const endPanelDrag = () => {
    if (!dragState) {
      return;
    }
    dragState = null;
    panel.classList.remove("dragging");
    win?.removeEventListener?.("pointermove", onPanelDragMove);
    win?.removeEventListener?.("pointerup", endPanelDrag);
    win?.removeEventListener?.("pointercancel", endPanelDrag);
  };

  const onPanelDragStart = (event) => {
    if (event.button != null && event.button !== 0) {
      return;
    }
    if (event.target?.closest?.("button,input,select,textarea,a")) {
      return;
    }
    const position = currentPanelPosition();
    const parent = panelBoundsParent();
    const parentRect = parent?.getBoundingClientRect?.() || { left: 0, top: 0 };
    dragState = {
      offsetX: event.clientX - parentRect.left - position.left,
      offsetY: event.clientY - parentRect.top - position.top,
    };
    panel.classList.add("dragging");
    event.preventDefault();
    win?.addEventListener?.("pointermove", onPanelDragMove);
    win?.addEventListener?.("pointerup", endPanelDrag);
    win?.addEventListener?.("pointercancel", endPanelDrag);
  };

  const clampCurrentPanelPosition = () => {
    if (panel.hidden) {
      return;
    }
    const position = currentPanelPosition();
    setPanelPosition(position.left, position.top);
  };

  const resetToDefaultView = () => {
    try {
      if (typeof viewer.resetDefaultView === "function") {
        return viewer.resetDefaultView();
      }
      viewer.clearInspection();
      viewer.resetAllVisibility();
      return viewer.defaultView();
    } finally {
      renderSafely();
    }
  };

  const setActiveSemanticFacet = (facetId) => {
    const nextFacetId = String(facetId || "").trim();
    if (!nextFacetId) {
      return;
    }
    activeSemanticFacetId = nextFacetId;
    if (isSemanticFacetId(nextFacetId)) {
      lastSemanticFacetId = nextFacetId;
    }
    renderSafely();
  };

  const toggleMember = (member, ids, visible) => {
    const elementIds = Array.isArray(ids) ? ids : [];
    if (!elementIds.length) {
      renderSafely();
      return 0;
    }
    try {
      return visible
        ? viewer.unsuppress(elementIds, { sourceResource: member })
        : viewer.suppress(elementIds, { sourceResource: member });
    } finally {
      renderSafely();
    }
  };

  const loadSemanticOutliner = (resource) => {
    const normalizedResource = String(resource || "").trim();
    if (
      !normalizedResource ||
      semanticResource === normalizedResource ||
      semanticLoadingResource === normalizedResource ||
      semanticErrorResource === normalizedResource
    ) {
      return;
    }
    semanticLoadingResource = normalizedResource;
    semanticErrorResource = "";
    semanticError = "";
    fetch(outlinerEndpointForResource(normalizedResource))
      .then((response) => {
        if (!response.ok) {
          throw new Error(`semantic outliner request failed (${response.status})`);
        }
        return response.json();
      })
      .then((payload) => {
        semanticResource = normalizedResource;
        semanticOutliner = normalizeSemanticOutliner(payload);
      })
      .catch((error) => {
        semanticResource = "";
        semanticOutliner = null;
        semanticErrorResource = normalizedResource;
        semanticError = error?.message || String(error);
      })
      .finally(() => {
        if (semanticLoadingResource === normalizedResource) {
          semanticLoadingResource = "";
        }
        renderSafely();
      });
  };

  const toggleSemanticGroup = (
    group,
    viewState,
    resource,
    visible,
    { bucket = "primary" } = {}
  ) => {
    const operation = semanticGroupVisibilityOperation(group, viewState, resource, visible, {
      bucket,
    });
    if (!operation.ids.length) {
      renderSafely();
      return 0;
    }
    try {
      if (operation.action === "reveal") {
        return viewer.setVisible(operation.ids, true);
      }
      if (operation.action === "reset") {
        return viewer.resetVisibility(operation.ids);
      }
      return viewer.hide(operation.ids);
    } finally {
      renderSafely();
    }
  };

  const toggleSemanticInspection = (
    group,
    viewState,
    resource,
    inspected,
    { bucket = "primary" } = {}
  ) => {
    const operation = semanticGroupInspectionOperation(
      group,
      viewState,
      resource,
      inspected,
      { bucket }
    );
    if (!operation.ids.length) {
      renderSafely();
      return 0;
    }
    try {
      return operation.action === "add"
        ? viewer.addInspection(operation.ids)
        : viewer.removeInspection(operation.ids);
    } finally {
      renderSafely();
    }
  };

  const drawingApi = () => viewer?.drawings || null;
  const drawingSetPathPartVisible = () => {
    const api = drawingApi();
    return typeof api?.setPathPartVisible === "function"
      ? api.setPathPartVisible
      : null;
  };

  const callSetPathPartVisible = (command, visible) => {
    const api = drawingApi();
    const setPathPartVisible = drawingSetPathPartVisible();
    if (!api || !setPathPartVisible) {
      return null;
    }
    const resource = String(command.resource || "").trim();
    const spec = {
      resource,
      path: command.path,
      drawingPart: command.drawingPart,
      part: command.drawingPart,
      visible: Boolean(visible),
    };
    if (command.layerId) {
      spec.layerId = command.layerId;
    }
    if (command.line !== undefined) {
      spec.line = command.line;
    }
    if (command.markers !== undefined) {
      spec.markers = command.markers;
    }
    if (command.maxSamples !== undefined) {
      spec.maxSamples = command.maxSamples;
    }
    if (setPathPartVisible.length >= 3) {
      return setPathPartVisible.call(
        api,
        command.path,
        command.drawingPart,
        Boolean(visible),
        {
          resource,
          layerId: command.layerId || undefined,
          line: command.line,
          markers: command.markers,
          maxSamples: command.maxSamples,
        }
      );
    }
    return setPathPartVisible.call(api, spec, Boolean(visible));
  };

  const toggleDrawingGroup = (group, viewState, resource, visible) => {
    const operation = drawingGroupVisibilityOperation(group, viewState, resource, visible);
    if (!operation.commands.length || !drawingSetPathPartVisible()) {
      renderSafely();
      return 0;
    }
    const pending = [];
    try {
      for (const command of operation.commands) {
        const result = callSetPathPartVisible(command, visible);
        if (result && typeof result.then === "function") {
          pending.push(result);
        }
      }
    } catch (error) {
      console.error("workspace outliner drawing toggle failed", error);
    }
    if (pending.length) {
      Promise.allSettled(pending)
        .then((results) => {
          for (const result of results) {
            if (result.status === "rejected") {
              console.error("workspace outliner drawing toggle failed", result.reason);
            }
          }
        })
        .finally(() => renderSafely());
    } else {
      renderSafely();
    }
    return operation.commands.length;
  };

  const renderSemanticInspectButton = (
    group,
    viewState,
    resource,
    { bucket = "primary", label = group.label } = {}
  ) => {
    const inspectionState = semanticGroupInspectionState(group, viewState, resource, {
      bucket,
    });
    const button = doc.createElement("button");
    button.type = "button";
    button.className = "outliner-inspect-toggle";
    button.classList.toggle("active", inspectionState.checked);
    button.classList.toggle("mixed", inspectionState.indeterminate);
    button.disabled = inspectionState.disabled;
    button.textContent = "I";
    button.title = inspectionState.checked
      ? `Remove ${label} from inspection`
      : `Add ${label} to inspection`;
    button.setAttribute("aria-label", button.title);
    button.setAttribute("aria-pressed", String(inspectionState.checked));
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      toggleSemanticInspection(group, viewState, resource, !inspectionState.checked, {
        bucket,
      });
    });
    return button;
  };

  const renderSemanticHiddenBucket = (
    fragment,
    group,
    viewState,
    resource,
    depth
  ) => {
    const bucketState = semanticGroupOutlinerState(group, viewState, resource, {
      bucket: "hidden",
    });
    if (!bucketState.totalCount) {
      return;
    }

    const row = doc.createElement("div");
    row.className = "outliner-row outliner-group-row outliner-bucket-row";
    row.style.setProperty("--outliner-depth", String(depth));

    const checkbox = doc.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = bucketState.checked;
    checkbox.indeterminate = bucketState.indeterminate;
    checkbox.disabled = bucketState.disabled;
    checkbox.setAttribute("aria-label", `Toggle hidden ${group.label}`);
    checkbox.addEventListener("change", () => {
      toggleSemanticGroup(group, viewState, resource, checkbox.checked, {
        bucket: "hidden",
      });
    });

    const text = doc.createElement("span");
    text.className = "outliner-name-stack";

    const name = doc.createElement("span");
    name.className = "outliner-name";
    name.textContent = "Hidden by default";
    name.title = `${group.label}: hidden by default`;
    text.appendChild(name);

    const detail = doc.createElement("span");
    detail.className = "outliner-detail";
    detail.textContent = group.label;
    text.appendChild(detail);

    const meta = doc.createElement("span");
    meta.className = "outliner-row-meta";
    meta.appendChild(
      renderSemanticInspectButton(group, viewState, resource, {
        bucket: "hidden",
        label: `${group.label} hidden`,
      })
    );
    appendSourceBadgeSlot(doc, meta, "");
    const count = doc.createElement("span");
    count.className = "outliner-count";
    count.textContent = bucketCountText(bucketState);
    meta.appendChild(count);

    row.append(checkbox, text, meta);
    fragment.appendChild(row);
  };

  const renderDrawingGroup = (fragment, group, viewState, resource, depth = 0) => {
    const groupState = drawingGroupOutlinerState(group, viewState, resource);
    const apiAvailable = Boolean(drawingSetPathPartVisible());
    const row = doc.createElement("div");
    row.className = "outliner-row outliner-group-row outliner-drawing-row";
    row.style.setProperty("--outliner-depth", String(depth));

    const checkbox = doc.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = groupState.checked;
    checkbox.indeterminate = groupState.indeterminate;
    checkbox.disabled = groupState.disabled || !apiAvailable;
    checkbox.setAttribute("aria-label", `Toggle drawing ${group.label}`);
    checkbox.title = apiAvailable
      ? `Toggle drawing ${group.label}`
      : "Viewer drawings API unavailable";
    checkbox.addEventListener("change", () => {
      toggleDrawingGroup(group, viewState, resource, checkbox.checked);
    });

    const text = doc.createElement("span");
    text.className = "outliner-name-stack";

    const name = doc.createElement("span");
    name.className = "outliner-name";
    name.textContent = group.label;
    name.title = group.label;
    text.appendChild(name);

    const detail = drawingGroupDetailText(group);
    if (detail) {
      const detailNode = doc.createElement("span");
      detailNode.className = "outliner-detail";
      detailNode.textContent = detail;
      text.appendChild(detailNode);
    }

    const meta = doc.createElement("span");
    meta.className = "outliner-row-meta outliner-drawing-row-meta";
    appendSourceBadgeSlot(doc, meta, "path");
    const count = doc.createElement("span");
    count.className = "outliner-count";
    count.textContent = drawingCountText(groupState);
    meta.appendChild(count);

    row.append(checkbox, text, meta);
    fragment.appendChild(row);

    for (const child of group.children || []) {
      renderDrawingGroup(fragment, child, viewState, resource, depth + 1);
    }
  };

  const renderSemanticGroup = (fragment, group, viewState, resource, depth = 0) => {
    const groupState = semanticGroupOutlinerState(group, viewState, resource);
    const row = doc.createElement("div");
    row.className = "outliner-row outliner-group-row";
    row.style.setProperty("--outliner-depth", String(depth));

    const checkbox = doc.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = groupState.checked;
    checkbox.indeterminate = groupState.indeterminate;
    checkbox.disabled = groupState.disabled;
    checkbox.setAttribute("aria-label", `Toggle ${semanticGroupActionLabel(group)}`);
    checkbox.addEventListener("change", () => {
      toggleSemanticGroup(group, viewState, resource, checkbox.checked);
    });

    const text = doc.createElement("span");
    text.className = "outliner-name-stack";

    const name = doc.createElement("span");
    name.className = "outliner-name";
    name.textContent = group.label;
    name.title = semanticGroupActionLabel(group);
    text.appendChild(name);

    const detail = groupDetailText(group);
    if (detail) {
      const detailNode = doc.createElement("span");
      detailNode.className = "outliner-detail";
      detailNode.textContent = detail;
      text.appendChild(detailNode);
    }

    const meta = doc.createElement("span");
    meta.className = "outliner-row-meta";
    meta.appendChild(renderSemanticInspectButton(group, viewState, resource));
    appendSourceBadgeSlot(doc, meta, semanticGroupSourceLabel(group));
    const count = doc.createElement("span");
    count.className = "outliner-count";
    count.textContent = groupCountText(group, groupState);
    meta.appendChild(count);

    row.append(checkbox, text, meta);
    fragment.appendChild(row);

    if (groupState.defaultCount > 0 && groupState.hiddenCount > 0) {
      renderSemanticHiddenBucket(fragment, group, viewState, resource, depth + 1);
    }

    for (const child of group.children || []) {
      renderSemanticGroup(fragment, child, viewState, resource, depth + 1);
    }
  };

  const renderSemanticOutliner = (fragment, outliner, viewState, resource) => {
    activeSemanticFacetId = preferredSemanticFacetId(outliner, activeSemanticFacetId);
    if (isSemanticFacetId(activeSemanticFacetId)) {
      lastSemanticFacetId = activeSemanticFacetId;
    }
    fragment.appendChild(
      renderSemanticFacetSwitchers(
        doc,
        outliner,
        activeSemanticFacetId,
        lastSemanticFacetId,
        setActiveSemanticFacet
      )
    );
    const list = doc.createElement("div");
    list.className = "outliner-list";
    fragment.appendChild(list);

    const facet =
      (outliner.facets || []).find((candidate) => candidate.id === activeSemanticFacetId) ||
      outliner.facets?.[0] ||
      null;
    if (!facet) {
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      empty.textContent = "No semantic groups.";
      list.appendChild(empty);
      return;
    }

    if (!facet.groups?.length) {
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      const diagnostic = facet.diagnostics?.map(semanticDiagnosticMessage).find(Boolean);
      empty.textContent = diagnostic || "No groups in this facet.";
      list.appendChild(empty);
      return;
    }

    for (const group of facet.groups) {
      if (facet.id === DRAWINGS_FACET_ID) {
        renderDrawingGroup(list, group, viewState, resource);
      } else {
        renderSemanticGroup(list, group, viewState, resource);
      }
    }
  };

  const render = () => {
    const state = appStateStore.getState();
    const viewState = state.committedViewerState || null;
    const resource =
      String(viewState?.resource || "").trim() ||
      safeViewerCurrentResource(viewer) ||
      picker?.value ||
      "";
    const project = projectEntryForResource(resource, readCatalogState());
    const fragment = doc.createDocumentFragment();

    if (!project || !project.members.length) {
      if (subtitle) {
        subtitle.textContent = "No workspace selected";
      }
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      empty.textContent = "Select a workspace or IFC resource to see loaded data.";
      fragment.appendChild(empty);
      replaceBodyChildren(fragment, resource);
      return;
    }

    if (subtitle) {
      subtitle.textContent =
        project.resource === resource
          ? project.label || project.resource
          : `${project.label || project.resource} workspace`;
    }
    if (!viewState) {
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      empty.textContent = "Loading workspace.";
      fragment.appendChild(empty);
      replaceBodyChildren(fragment, resource);
      return;
    }

    loadSemanticOutliner(resource);
    const renderOutliner = semanticOutlinerForRender(
      semanticResource === resource ? semanticOutliner : null,
      project,
      viewState,
      resource,
      {
        loading: semanticLoadingResource === resource,
        error: semanticErrorResource === resource ? semanticError : "",
      }
    );
    renderSemanticOutliner(fragment, renderOutliner, viewState, resource);
    replaceBodyChildren(fragment, resource);
  };

  const renderSafely = () => {
    try {
      render();
    } catch (error) {
      console.error("workspace outliner render failed", error);
    }
  };

  const renderVisibility = (visible) => {
    panel.hidden = !visible;
    toggleButton.classList.toggle("active", visible);
    toggleButton.setAttribute("aria-pressed", String(visible));
    if (visible) {
      clampCurrentPanelPosition();
      renderSafely();
    }
    return visible;
  };

  const onToggleClick = () => {
    appStateStore.dispatch({ type: "panel/toggle", panel: "outliner" });
  };
  const onCloseClick = () => {
    setVisible(false);
  };
  const onResetClick = () => {
    resetToDefaultView();
  };
  const onCatalogChange = () => {
    if (!panel.hidden) {
      renderSafely();
    }
  };

  toggleButton.addEventListener("click", onToggleClick);
  resetButton?.addEventListener("click", onResetClick);
  closeButton?.addEventListener("click", onCloseClick);
  dragHandle?.addEventListener?.("pointerdown", onPanelDragStart);
  win?.addEventListener?.("w-resource-catalog-change", onCatalogChange);
  win?.addEventListener?.("resize", clampCurrentPanelPosition);

  const unsubscribe = subscribe
    ? appStateStore.subscribe((state) => {
        renderVisibility(Boolean(state.panels?.outliner));
      })
    : null;

  return {
    render,
    renderSafely,
    renderVisibility,
    setVisible,
    show: () => setVisible(true),
    hide: () => setVisible(false),
    toggle: () => appStateStore.dispatch({ type: "panel/toggle", panel: "outliner" }),
    resetToDefaultView,
    toggleMember,
    catalogState: readCatalogState,
    dispose: () => {
      toggleButton.removeEventListener("click", onToggleClick);
      resetButton?.removeEventListener("click", onResetClick);
      closeButton?.removeEventListener("click", onCloseClick);
      dragHandle?.removeEventListener?.("pointerdown", onPanelDragStart);
      win?.removeEventListener?.("w-resource-catalog-change", onCatalogChange);
      win?.removeEventListener?.("resize", clampCurrentPanelPosition);
      endPanelDrag();
      unsubscribe?.();
    },
  };
}

export function installProjectOutliner(viewer, appStateStore, options = {}) {
  return createProjectOutlinerController({
    ...options,
    viewer,
    appStateStore,
  });
}
