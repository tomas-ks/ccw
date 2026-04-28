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
