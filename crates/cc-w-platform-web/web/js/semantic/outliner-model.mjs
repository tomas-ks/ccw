export const DEFAULT_SEMANTIC_OUTLINER_FACETS = Object.freeze([
  Object.freeze({ id: "workspace", label: "Workspace" }),
  Object.freeze({ id: "layers", label: "Layers" }),
  Object.freeze({ id: "classes", label: "Classes" }),
  Object.freeze({ id: "spatial", label: "Spatial" }),
  Object.freeze({ id: "materials", label: "Materials" }),
  Object.freeze({ id: "construction", label: "State" }),
]);

const FACET_ALIASES = new Map([
  ["class", "classes"],
  ["classes", "classes"],
  ["ifc-class", "classes"],
  ["ifc-classes", "classes"],
  ["layer", "layers"],
  ["layers", "layers"],
  ["presentation-layer", "layers"],
  ["presentation-layers", "layers"],
  ["material", "materials"],
  ["materials", "materials"],
  ["spatial", "spatial"],
  ["space", "spatial"],
  ["spaces", "spatial"],
  ["project", "project"],
  ["projects", "project"],
  ["workspace", "workspace"],
  ["workspaces", "workspace"],
  ["resource", "workspace"],
  ["resources", "workspace"],
  ["construction", "construction"],
  ["construct", "construction"],
  ["construction-type", "construction"],
  ["construction-types", "construction"],
]);

function stringValue(value) {
  return String(value ?? "").trim();
}

function firstPresent(...values) {
  for (const value of values) {
    if (value == null) {
      continue;
    }
    if (typeof value === "string" && !value.trim()) {
      continue;
    }
    return value;
  }
  return undefined;
}

function slugify(value) {
  return stringValue(value)
    .toLowerCase()
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function humanizeSlug(value) {
  const text = slugify(value);
  if (!text) {
    return "Facet";
  }
  return text
    .split("-")
    .filter(Boolean)
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function isIfcResource(value) {
  return stringValue(value).startsWith("ifc/");
}

function isProjectResource(value) {
  return stringValue(value).startsWith("project/");
}

function parseSourceScopedSemanticId(value) {
  const text = stringValue(value);
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

function uniqueStrings(values) {
  const source = Array.isArray(values) ? values : values == null ? [] : [values];
  const normalized = [];
  const seen = new Set();
  for (const value of source) {
    const text = stringValue(value);
    if (!text || seen.has(text)) {
      continue;
    }
    seen.add(text);
    normalized.push(text);
  }
  return normalized;
}

function normalizeCounts(value) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return { total: value };
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  const counts = {};
  for (const [key, raw] of Object.entries(value)) {
    const number = typeof raw === "number" ? raw : Number(raw);
    if (Number.isFinite(number)) {
      counts[key] = number;
    }
  }
  return counts;
}

function normalizeSourceResource(value) {
  const text = stringValue(value);
  return isIfcResource(text) ? text : "";
}

function firstSourceResource(input) {
  const direct = normalizeSourceResource(
    firstPresent(input?.sourceResource, input?.source_resource)
  );
  if (direct) {
    return direct;
  }
  const values = firstPresent(input?.sourceResources, input?.source_resources);
  if (!Array.isArray(values)) {
    return "";
  }
  for (const value of values) {
    const resource = normalizeSourceResource(value);
    if (resource) {
      return resource;
    }
  }
  return "";
}

export function normalizeSemanticFacetId(id, label = "") {
  const slug = slugify(firstPresent(id, label));
  if (!slug) {
    return "";
  }
  return FACET_ALIASES.get(slug) || slug;
}

export function semanticDiagnosticMessage(diagnostic) {
  if (typeof diagnostic === "string") {
    return diagnostic.trim();
  }
  if (!diagnostic || typeof diagnostic !== "object") {
    return "";
  }
  return stringValue(
    firstPresent(
      diagnostic.message,
      diagnostic.detail,
      diagnostic.code,
      JSON.stringify(diagnostic)
    )
  );
}

export function normalizeSemanticGroup(input, inherited = {}) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    return null;
  }

  const rawSourceDetail = firstPresent(
    input.sourceDetail,
    input.source_detail,
    input.kind,
    input.provenance,
    inherited.sourceDetail
  );
  const sourceDetail = stringValue(rawSourceDetail);
  const sourceKind = stringValue(
    firstPresent(
      input.sourceKind,
      input.source_kind,
      input.provenance,
      inherited.sourceKind,
      input.kind
    )
  );
  const sourceResource =
    firstSourceResource(input) ||
    normalizeSourceResource(sourceDetail) ||
    normalizeSourceResource(inherited.sourceResource);
  const rawId = firstPresent(input.id, input.key, input.value, input.label, input.name);
  const label =
    stringValue(firstPresent(input.label, input.name, rawId)) || "Untitled group";
  const id = stringValue(rawId) || label;
  const semanticIds = uniqueStrings(
    firstPresent(
      input.semanticIds,
      input.semantic_ids,
      input.elementIds,
      input.element_ids,
      input.globalIds,
      input.global_ids,
      input.ids
    )
  );
  const childInput = Array.isArray(input.children) ? input.children : [];
  const childScope = {
    sourceKind,
    sourceDetail,
    sourceResource,
  };
  const children = childInput
    .map((child) => normalizeSemanticGroup(child, childScope))
    .filter(Boolean);

  const counts = normalizeCounts(firstPresent(input.counts, input.count));
  for (const [alias, target] of [
    ["elementCount", "elementCount"],
    ["element_count", "elementCount"],
    ["total", "total"],
  ]) {
    const raw = input[alias];
    const number = typeof raw === "number" ? raw : Number(raw);
    if (Number.isFinite(number) && counts[target] == null) {
      counts[target] = number;
    }
  }

  return {
    id,
    label,
    sourceKind,
    sourceDetail,
    sourceResource,
    semanticIds,
    children,
    counts,
    metadata: input.metadata && typeof input.metadata === "object" ? { ...input.metadata } : {},
    diagnostics: Array.isArray(input.diagnostics)
      ? input.diagnostics.map(semanticDiagnosticMessage).filter(Boolean)
      : [],
  };
}

export function normalizeSemanticOutliner(payload) {
  const knownFacets = new Map(
    DEFAULT_SEMANTIC_OUTLINER_FACETS.map((facet) => [
      facet.id,
      { id: facet.id, label: facet.label, groups: [] },
    ])
  );
  const extraFacets = [];
  const facets = Array.isArray(payload?.facets) ? payload.facets : [];

  for (const inputFacet of facets) {
    if (!inputFacet || typeof inputFacet !== "object" || Array.isArray(inputFacet)) {
      continue;
    }
    const id = normalizeSemanticFacetId(inputFacet.id, inputFacet.label);
    if (!id) {
      continue;
    }
    const existing = knownFacets.get(id) || null;
    const label =
      stringValue(inputFacet.label) ||
      existing?.label ||
      humanizeSlug(firstPresent(inputFacet.id, id));
    const groups = Array.isArray(inputFacet.groups)
      ? inputFacet.groups
          .map((group) => normalizeSemanticGroup(group))
          .filter(Boolean)
      : [];
    const diagnostics = Array.isArray(inputFacet.diagnostics)
      ? inputFacet.diagnostics.map(semanticDiagnosticMessage).filter(Boolean)
      : [];
    const normalized = {
      id,
      label,
      sourceKind: stringValue(inputFacet.provenance),
      groups,
      diagnostics,
    };
    if (existing) {
      knownFacets.set(id, normalized);
    } else {
      extraFacets.push(normalized);
    }
  }

  const diagnostics = Array.isArray(payload?.diagnostics)
    ? payload.diagnostics.map(semanticDiagnosticMessage).filter(Boolean)
    : [];

  return {
    resource: stringValue(payload?.resource),
    facets: [
      ...DEFAULT_SEMANTIC_OUTLINER_FACETS.map((facet) => knownFacets.get(facet.id)),
      ...extraFacets,
    ].filter(Boolean),
    diagnostics,
  };
}

function appendGroupIdEntries(group, entries, inheritedSourceResource = "") {
  if (!group || typeof group !== "object") {
    return;
  }
  const sourceResource =
    normalizeSourceResource(group.sourceResource) ||
    normalizeSourceResource(inheritedSourceResource);
  for (const id of uniqueStrings(group.semanticIds)) {
    entries.push({ id, sourceResource });
  }
  const children = Array.isArray(group.children) ? group.children : [];
  for (const child of children) {
    appendGroupIdEntries(child, entries, sourceResource);
  }
}

export function semanticGroupIdEntries(group) {
  const entries = [];
  const seen = new Set();
  appendGroupIdEntries(group, entries);
  return entries.filter((entry) => {
    const key = `${entry.sourceResource}\u0000${entry.id}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function viewStateElementIds(viewState, key) {
  return Array.isArray(viewState?.[key])
    ? viewState[key].map((id) => stringValue(id)).filter(Boolean)
    : [];
}

function candidateViewerIds(entry, resource) {
  const raw = stringValue(entry?.id);
  if (!raw) {
    return [];
  }
  const sourceResource = normalizeSourceResource(entry?.sourceResource);
  const scoped = parseSourceScopedSemanticId(raw);
  const viewerResource = stringValue(resource);
  const candidates = [];

  if (scoped) {
    if (isProjectResource(viewerResource)) {
      candidates.push(`${scoped.sourceResource}::${scoped.semanticId}`);
    } else if (isIfcResource(viewerResource)) {
      if (scoped.sourceResource === viewerResource) {
        candidates.push(scoped.semanticId);
      }
    } else {
      candidates.push(raw);
    }
  } else if (isProjectResource(viewerResource)) {
    if (sourceResource) {
      candidates.push(`${sourceResource}::${raw}`);
    }
    candidates.push(raw);
  } else if (isIfcResource(viewerResource)) {
    if (!sourceResource || sourceResource === viewerResource) {
      candidates.push(raw);
    }
  } else {
    candidates.push(raw);
  }

  return uniqueStrings(candidates);
}

export function semanticGroupViewerIds(group, viewState, resource) {
  const listIds = viewStateElementIds(viewState, "listElementIds");
  const fallbackIds = viewStateElementIds(viewState, "defaultElementIds");
  const elementIds = listIds.length ? listIds : fallbackIds;
  if (!elementIds.length) {
    return [];
  }
  const elementSet = new Set(elementIds);
  const ids = [];
  const seen = new Set();
  for (const entry of semanticGroupIdEntries(group)) {
    for (const candidate of candidateViewerIds(entry, resource)) {
      if (!elementSet.has(candidate) || seen.has(candidate)) {
        continue;
      }
      seen.add(candidate);
      ids.push(candidate);
    }
  }
  return ids;
}

function visibilityStateForIds(ids, viewState) {
  const visible = new Set(viewStateElementIds(viewState, "visibleElementIds"));
  const enabledCount = ids.filter((id) => visible.has(id)).length;
  return {
    ids,
    enabledCount,
    totalCount: ids.length,
    checked: ids.length > 0 && enabledCount > 0,
    disabled: ids.length === 0,
    indeterminate: ids.length > 0 && enabledCount > 0 && enabledCount < ids.length,
  };
}

function inspectionStateForIds(ids, viewState) {
  const inspected = new Set(viewStateElementIds(viewState, "inspectedElementIds"));
  const enabledCount = ids.filter((id) => inspected.has(id)).length;
  return {
    ids,
    enabledCount,
    totalCount: ids.length,
    checked: ids.length > 0 && enabledCount > 0,
    disabled: ids.length === 0,
    indeterminate: ids.length > 0 && enabledCount > 0 && enabledCount < ids.length,
  };
}

export function semanticGroupViewerBuckets(group, viewState, resource) {
  const ids = semanticGroupViewerIds(group, viewState, resource);
  const defaultSet = new Set(viewStateElementIds(viewState, "defaultElementIds"));
  const defaultIds = [];
  const hiddenIds = [];
  for (const id of ids) {
    if (defaultSet.has(id)) {
      defaultIds.push(id);
    } else {
      hiddenIds.push(id);
    }
  }
  return { ids, defaultIds, hiddenIds };
}

export function semanticGroupOutlinerState(
  group,
  viewState,
  resource,
  { bucket = "primary" } = {}
) {
  const buckets = semanticGroupViewerBuckets(group, viewState, resource);
  const normalizedBucket = stringValue(bucket);
  let ids = buckets.defaultIds.length ? buckets.defaultIds : buckets.hiddenIds;
  let activeBucket = buckets.defaultIds.length ? "default" : "hidden";
  if (normalizedBucket === "all") {
    ids = buckets.ids;
    activeBucket = "all";
  } else if (normalizedBucket === "default") {
    ids = buckets.defaultIds;
    activeBucket = "default";
  } else if (normalizedBucket === "hidden") {
    ids = buckets.hiddenIds;
    activeBucket = "hidden";
  }
  const bucketVisibility = visibilityStateForIds(ids, viewState);
  const allVisibility = visibilityStateForIds(buckets.ids, viewState);

  return {
    ...bucketVisibility,
    bucket: activeBucket,
    allIds: buckets.ids,
    allEnabledCount: allVisibility.enabledCount,
    allTotalCount: allVisibility.totalCount,
    defaultIds: buckets.defaultIds,
    hiddenIds: buckets.hiddenIds,
    defaultCount: buckets.defaultIds.length,
    hiddenCount: buckets.hiddenIds.length,
  };
}

export function semanticGroupInspectionState(
  group,
  viewState,
  resource,
  options = {}
) {
  const state = semanticGroupOutlinerState(group, viewState, resource, options);
  return {
    ...inspectionStateForIds(state.ids, viewState),
    bucket: state.bucket,
    allIds: state.allIds,
    defaultIds: state.defaultIds,
    hiddenIds: state.hiddenIds,
    defaultCount: state.defaultCount,
    hiddenCount: state.hiddenCount,
  };
}

export function semanticGroupVisibilityOperation(
  group,
  viewState,
  resource,
  visible,
  options = {}
) {
  const state = semanticGroupOutlinerState(group, viewState, resource, options);
  let action = visible ? "reset" : "hide";
  if (state.bucket === "hidden") {
    action = visible ? "reveal" : "reset";
  }
  return { action, ids: state.ids, state };
}

export function semanticGroupInspectionOperation(
  group,
  viewState,
  resource,
  inspected,
  options = {}
) {
  const state = semanticGroupInspectionState(group, viewState, resource, options);
  return { action: inspected ? "add" : "remove", ids: state.ids, state };
}

export function semanticGroupDeclaredCount(group) {
  const counts = group?.counts && typeof group.counts === "object" ? group.counts : {};
  for (const key of [
    "elements",
    "elementCount",
    "element_count",
    "semanticIds",
    "semantic_ids",
    "total",
    "count",
  ]) {
    const count = counts[key];
    if (Number.isFinite(count)) {
      return count;
    }
  }
  const own = Array.isArray(group?.semanticIds) ? group.semanticIds.length : 0;
  const children = Array.isArray(group?.children) ? group.children : [];
  return children.reduce(
    (total, child) => total + semanticGroupDeclaredCount(child),
    own
  );
}
