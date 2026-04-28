import {
  isIfcResource,
  isKnownResource,
  isProjectResource,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
} from "../viewer/resource.js";

export {
  isIfcResource,
  isKnownResource,
  isProjectResource,
  parseSourceScopedSemanticId,
  safeViewerCurrentResource,
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
  const containing = projects.find((project) =>
    Array.isArray(project.members) && project.members.includes(active)
  );
  if (containing) {
    return containing;
  }
  if (isIfcResource(active)) {
    return {
      resource: active,
      label: "IFC",
      members: [active],
    };
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
  closeButton = doc?.getElementById?.("outliner-close-button") || null,
  picker = doc?.getElementById?.("resource-picker") || null,
  subscribe = true,
} = {}) {
  if (!viewer || !appStateStore || !panel || !body || !toggleButton) {
    return null;
  }

  toggleButton.removeAttribute("disabled");

  const readCatalogState = () => {
    const state =
      typeof getCatalogState === "function" ? getCatalogState() : catalogState;
    return state && typeof state === "object" ? state : defaultProjectCatalogState;
  };

  const setVisible = (visible) => {
    appStateStore.dispatch({ type: "panel/set", panel: "outliner", visible });
    return visible;
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
        subtitle.textContent = "No project selected";
      }
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      empty.textContent = "Select a project resource to see its IFC assets.";
      fragment.appendChild(empty);
      body.replaceChildren(fragment);
      return;
    }

    if (subtitle) {
      subtitle.textContent =
        project.resource === resource
          ? project.label || project.resource
          : `${project.label || project.resource} assets`;
    }
    if (!viewState) {
      const empty = doc.createElement("div");
      empty.className = "outliner-empty";
      empty.textContent = "Loading project assets.";
      fragment.appendChild(empty);
      body.replaceChildren(fragment);
      return;
    }

    for (const member of project.members) {
      const memberState = memberOutlinerState(member, viewState, resource);
      const row = doc.createElement("label");
      row.className = "outliner-row";

      const checkbox = doc.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = memberState.checked;
      checkbox.indeterminate = memberState.indeterminate;
      checkbox.disabled = memberState.disabled;
      checkbox.setAttribute("aria-label", `Toggle ${member}`);
      checkbox.addEventListener("change", () => {
        toggleMember(member, memberState.ids, checkbox.checked);
      });

      const name = doc.createElement("span");
      name.className = "outliner-name";
      name.textContent = labelForMember(member);
      name.title = member;

      const count = doc.createElement("span");
      count.className = "outliner-count";
      count.textContent = memberState.totalCount
        ? `${memberState.enabledCount}/${memberState.totalCount} default`
        : "not loaded";

      row.append(checkbox, name, count);
      fragment.appendChild(row);
    }
    body.replaceChildren(fragment);
  };

  const renderSafely = () => {
    try {
      render();
    } catch (error) {
      console.error("project outliner render failed", error);
    }
  };

  const renderVisibility = (visible) => {
    panel.hidden = !visible;
    toggleButton.classList.toggle("active", visible);
    toggleButton.setAttribute("aria-pressed", String(visible));
    if (visible) {
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
  const onCatalogChange = () => {
    if (!panel.hidden) {
      renderSafely();
    }
  };

  toggleButton.addEventListener("click", onToggleClick);
  closeButton?.addEventListener("click", onCloseClick);
  win?.addEventListener?.("w-resource-catalog-change", onCatalogChange);

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
    toggleMember,
    catalogState: readCatalogState,
    dispose: () => {
      toggleButton.removeEventListener("click", onToggleClick);
      closeButton?.removeEventListener("click", onCloseClick);
      win?.removeEventListener?.("w-resource-catalog-change", onCatalogChange);
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
