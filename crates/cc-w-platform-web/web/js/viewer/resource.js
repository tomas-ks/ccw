export function safeViewerCurrentResource(viewer) {
  try {
    return viewer.currentResource();
  } catch (_error) {
    return null;
  }
}

export function isIfcResource(resource) {
  return typeof resource === "string" && resource.startsWith("ifc/");
}

export function isProjectResource(resource) {
  return typeof resource === "string" && resource.startsWith("project/");
}

export function isKnownResource(resource) {
  return isIfcResource(resource) || isProjectResource(resource);
}

export function friendlyResourceLabel(resource) {
  const text = String(resource || "").trim();
  if (text === "demo/mapped-pentagon-pair") {
    return "mapped-pentagon-pair (per-instance color)";
  }
  return text.replace(/^(demo|ifc|project)\//, "");
}

export function normalizeResourceCatalog(payload) {
  const resources = [];
  const seen = new Set();
  const addResource = (resource) => {
    const value = String(resource || "").trim();
    if (!isKnownResource(value) || seen.has(value)) {
      return;
    }
    seen.add(value);
    resources.push(value);
  };

  if (Array.isArray(payload?.resources)) {
    for (const resource of payload.resources) {
      addResource(resource);
    }
  }

  const projects = Array.isArray(payload?.projects)
    ? payload.projects
        .map((project) => ({
          resource: String(project?.resource || "").trim(),
          label: String(project?.label || "").trim(),
          members: Array.isArray(project?.members)
            ? project.members
                .map((member) => String(member || "").trim())
                .filter(isIfcResource)
            : [],
        }))
        .filter((project) => isProjectResource(project.resource) && project.members.length)
    : [];

  for (const project of projects) {
    addResource(project.resource);
  }

  return { resources, projects };
}

export function createResourceCatalogState(initial = {}) {
  return normalizeResourceCatalog(initial);
}

export function updateResourceCatalogState(
  catalogState,
  payload,
  { window: win = globalThis.window, dispatchEvent = true } = {}
) {
  const target =
    catalogState && typeof catalogState === "object"
      ? catalogState
      : createResourceCatalogState();
  const normalized = normalizeResourceCatalog(payload);
  target.resources = normalized.resources;
  target.projects = normalized.projects;

  const CustomEventCtor = win?.CustomEvent || globalThis.CustomEvent;
  if (dispatchEvent && win?.dispatchEvent && typeof CustomEventCtor === "function") {
    win.dispatchEvent(
      new CustomEventCtor("w-resource-catalog-change", {
        detail: {
          resources: target.resources,
          projects: target.projects,
        },
      })
    );
  }

  return target;
}

export function resourceCatalogEntries(catalogState) {
  const normalized = normalizeResourceCatalog(catalogState);
  const projectsByResource = new Map(
    normalized.projects.map((project) => [project.resource, project])
  );
  return normalized.resources.map((resource) => {
    const project = projectsByResource.get(resource) || null;
    return {
      resource,
      label: project?.label || friendlyResourceLabel(resource),
      project,
      kind: isProjectResource(resource) ? "project" : "ifc",
    };
  });
}

export function projectResourceForIfc(resource, catalogState) {
  const value = String(resource || "").trim();
  if (!isIfcResource(value)) {
    return null;
  }
  const projects = Array.isArray(catalogState?.projects) ? catalogState.projects : [];
  return (
    projects.find((project) =>
      Array.isArray(project.members) && project.members.includes(value)
    )?.resource || null
  );
}

export function resourceSelectHasOption(picker, resource) {
  const value = String(resource || "").trim();
  if (!value || !picker) {
    return false;
  }
  return Array.from(picker.options || []).some((option) => option.value === value);
}

export function selectedIfcResource(viewer, explicitResource, { picker = null } = {}) {
  if (isIfcResource(explicitResource)) {
    return explicitResource;
  }
  const pickerValue = picker?.value;
  if (isIfcResource(pickerValue)) {
    return pickerValue;
  }
  const viewerResource = safeViewerCurrentResource(viewer);
  return isIfcResource(viewerResource) ? viewerResource : null;
}

export function selectedAgentResource(
  viewer,
  explicitResource,
  { picker = null, catalogState = null } = {}
) {
  if (isProjectResource(explicitResource)) {
    return explicitResource;
  }
  const pickerValue = picker?.value;
  if (isProjectResource(pickerValue)) {
    return pickerValue;
  }
  const viewerResource = safeViewerCurrentResource(viewer);
  if (isProjectResource(viewerResource)) {
    return viewerResource;
  }
  const ifcResource = selectedIfcResource(viewer, explicitResource, { picker });
  const projectResource = projectResourceForIfc(ifcResource, catalogState);
  return projectResource && resourceSelectHasOption(picker, projectResource)
    ? projectResource
    : ifcResource;
}

export function parseSourceScopedSemanticId(value) {
  const text = String(value ?? "").trim();
  const separator = text.indexOf("::");
  if (separator <= 0) {
    return null;
  }
  const sourceResource = text.slice(0, separator).trim();
  const semanticId = text.slice(separator + 2).trim();
  if (!semanticId || !isIfcResource(sourceResource)) {
    return null;
  }
  return { sourceResource, semanticId };
}

export function sourceScopedSemanticId(sourceResource, semanticId) {
  return `${sourceResource}::${semanticId}`;
}

export function actionSourceResource(options = {}) {
  if (typeof options === "string") {
    return options.trim();
  }
  const raw =
    options?.sourceResource ??
    options?.source_resource ??
    options?.resource ??
    options?.ifcResource ??
    "";
  return String(raw || "").trim();
}

export function semanticIdsForViewerResource(ids, viewerResource, options = {}) {
  const values = Array.isArray(ids) ? ids : [ids];
  const actionResource = actionSourceResource(options);
  const currentResource = isKnownResource(viewerResource) ? viewerResource : "";
  const normalized = [];
  const seen = new Set();
  for (const value of values) {
    const text = String(value ?? "").trim();
    if (!text) {
      continue;
    }
    const scoped = parseSourceScopedSemanticId(text);
    let semanticId = text;
    if (scoped) {
      if (isProjectResource(currentResource)) {
        if (isIfcResource(actionResource) && actionResource !== scoped.sourceResource) {
          continue;
        }
        semanticId = sourceScopedSemanticId(scoped.sourceResource, scoped.semanticId);
      } else if (isIfcResource(currentResource)) {
        if (scoped.sourceResource !== currentResource) {
          continue;
        }
        semanticId = scoped.semanticId;
      } else {
        continue;
      }
    } else if (isProjectResource(currentResource)) {
      if (!isIfcResource(actionResource)) {
        continue;
      }
      semanticId = sourceScopedSemanticId(actionResource, text);
    } else if (isIfcResource(currentResource)) {
      if (isIfcResource(actionResource) && actionResource !== currentResource) {
        continue;
      }
      if (isProjectResource(actionResource)) {
        continue;
      }
    } else {
      continue;
    }
    if (!seen.has(semanticId)) {
      seen.add(semanticId);
      normalized.push(semanticId);
    }
  }
  return normalized;
}

export function scopedSemanticIdForViewer(semanticId, sourceResource, viewerResource) {
  return (
    semanticIdsForViewerResource([semanticId], viewerResource, { sourceResource })[0] ||
    null
  );
}
