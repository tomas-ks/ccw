import {
  createResourceCatalogState,
  friendlyResourceLabel,
  isKnownResource,
  resourceCatalogEntries,
  resourceSelectHasOption,
  safeViewerCurrentResource,
  updateResourceCatalogState,
} from "./resource.js";

function normalizedResource(value) {
  const resource = String(value || "").trim();
  return isKnownResource(resource) ? resource : null;
}

function committedResource(state) {
  return normalizedResource(state?.committedViewerState?.resource);
}

function requestedResource(state) {
  return normalizedResource(state?.requestedResource);
}

function appendMissingResources(entries, resources) {
  const seen = new Set(entries.map((entry) => entry.resource));
  for (const resource of resources) {
    if (!resource || seen.has(resource)) {
      continue;
    }
    seen.add(resource);
    entries.push({
      resource,
      label: friendlyResourceLabel(resource),
      project: null,
      kind: resource.startsWith("project/") ? "project" : "ifc",
    });
  }
  return entries;
}

function optionSignature(entries) {
  return entries
    .map((entry) => `${entry.resource}\u0000${entry.label}\u0000${entry.kind}`)
    .join("\u0001");
}

function createOption(doc, entry) {
  const option = doc.createElement("option");
  option.value = entry.resource;
  option.textContent = entry.label || friendlyResourceLabel(entry.resource);
  option.dataset.resourceKind = entry.kind;
  if (option.textContent !== entry.resource) {
    option.title = entry.resource;
  }
  return option;
}

export function resourcePickerSelection({
  state,
  viewer = null,
  picker = null,
  catalogState = null,
} = {}) {
  const preferred = [
    requestedResource(state),
    committedResource(state),
    normalizedResource(safeViewerCurrentResource(viewer)),
    normalizedResource(picker?.value),
  ];
  for (const resource of preferred) {
    if (resource && (!picker || resourceSelectHasOption(picker, resource))) {
      return resource;
    }
  }
  const firstEntry = resourceCatalogEntries(catalogState)[0];
  return firstEntry?.resource || "";
}

export function createResourcePickerController({
  viewer = null,
  appStateStore,
  document: doc = globalThis.document,
  window: win = globalThis.window,
  picker = doc?.getElementById?.("resource-picker") || null,
  catalog = null,
  catalogState = createResourceCatalogState(catalog || {}),
  getCatalogState = null,
  subscribe = true,
  listenForCatalogEvents = true,
  onResourceRequested = null,
} = {}) {
  if (!appStateStore || !picker) {
    return null;
  }

  let lastOptionSignature = "";
  let handlingCatalogUpdate = false;

  const readCatalogState = () => {
    const state =
      typeof getCatalogState === "function" ? getCatalogState() : catalogState;
    return state && typeof state === "object" ? state : catalogState;
  };

  const render = (state = appStateStore.getState()) => {
    const current =
      committedResource(state) || normalizedResource(safeViewerCurrentResource(viewer));
    const pending = requestedResource(state);
    const entries = appendMissingResources(resourceCatalogEntries(readCatalogState()), [
      current,
      pending,
    ]);
    const signature = optionSignature(entries);
    if (signature !== lastOptionSignature) {
      picker.replaceChildren(...entries.map((entry) => createOption(doc, entry)));
      lastOptionSignature = signature;
    }

    const selected = resourcePickerSelection({
      state,
      viewer,
      picker,
      catalogState: readCatalogState(),
    });
    if (selected && picker.value !== selected) {
      picker.value = selected;
    }
    picker.disabled = entries.length === 0;
    return selected;
  };

  const renderSafely = (state = appStateStore.getState()) => {
    try {
      return render(state);
    } catch (error) {
      console.error("resource picker render failed", error);
      return "";
    }
  };

  const setCatalog = (payload, { dispatchEvent = true } = {}) => {
    handlingCatalogUpdate = true;
    try {
      updateResourceCatalogState(readCatalogState(), payload, {
        window: win,
        dispatchEvent,
      });
    } finally {
      handlingCatalogUpdate = false;
    }
    lastOptionSignature = "";
    return renderSafely();
  };

  const refreshCatalog = () => {
    if (!viewer || typeof viewer.resourceCatalog !== "function") {
      return renderSafely();
    }
    try {
      return setCatalog(viewer.resourceCatalog());
    } catch (error) {
      console.error("resource picker catalog refresh failed", error);
      return renderSafely();
    }
  };

  const onChange = (event) => {
    const nextResource = normalizedResource(event?.target?.value);
    if (!nextResource) {
      renderSafely();
      return;
    }
    appStateStore.dispatch({ type: "resource/requested", resource: nextResource });
    if (typeof onResourceRequested === "function") {
      onResourceRequested(nextResource);
    }
  };

  const onCatalogChange = (event) => {
    if (handlingCatalogUpdate) {
      return;
    }
    updateResourceCatalogState(readCatalogState(), event?.detail || {}, {
      window: win,
      dispatchEvent: false,
    });
    lastOptionSignature = "";
    renderSafely();
  };

  picker.addEventListener("change", onChange);
  if (listenForCatalogEvents) {
    win?.addEventListener?.("w-resource-catalog-change", onCatalogChange);
  }

  const unsubscribe = subscribe
    ? appStateStore.subscribe((state) => {
        renderSafely(state);
      })
    : null;

  if (catalog) {
    setCatalog(catalog, { dispatchEvent: false });
  } else {
    refreshCatalog();
  }

  return {
    picker,
    catalogState: readCatalogState,
    render,
    renderSafely,
    setCatalog,
    refreshCatalog,
    selectedResource: () =>
      resourcePickerSelection({
        state: appStateStore.getState(),
        viewer,
        picker,
        catalogState: readCatalogState(),
      }),
    hasResource: (resource) => resourceSelectHasOption(picker, resource),
    dispose: () => {
      picker.removeEventListener("change", onChange);
      if (listenForCatalogEvents) {
        win?.removeEventListener?.("w-resource-catalog-change", onCatalogChange);
      }
      unsubscribe?.();
    },
  };
}

export function installResourcePicker(viewer, appStateStore, options = {}) {
  return createResourcePickerController({
    ...options,
    viewer,
    appStateStore,
  });
}
