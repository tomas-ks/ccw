export const DRAWINGS_FACET_ID = "drawings";

export const DEFAULT_SEMANTIC_OUTLINER_FACETS = Object.freeze([
  Object.freeze({ id: "workspace", label: "Workspace" }),
  Object.freeze({ id: "layers", label: "Layers" }),
  Object.freeze({ id: DRAWINGS_FACET_ID, label: "Drawings" }),
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
  ["drawing", DRAWINGS_FACET_ID],
  ["drawings", DRAWINGS_FACET_ID],
  ["path-drawing", DRAWINGS_FACET_ID],
  ["path-drawings", DRAWINGS_FACET_ID],
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

function structuredValue(value) {
  if (typeof value === "string") {
    const text = value.trim();
    if (!text) {
      return undefined;
    }
    if (
      (text.startsWith("{") && text.endsWith("}")) ||
      (text.startsWith("[") && text.endsWith("]"))
    ) {
      try {
        return JSON.parse(text);
      } catch {
        return undefined;
      }
    }
    return text;
  }
  return value == null ? undefined : value;
}

function numericValue(value) {
  const number = typeof value === "number" ? value : Number(value);
  return Number.isFinite(number) ? number : undefined;
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

const DRAWING_PART_LABELS = Object.freeze({
  line: "Line",
  stations: "Stations",
});

function normalizeDrawingPath(value) {
  if (value == null || value === "") {
    return undefined;
  }
  if (typeof value === "object" && !Array.isArray(value)) {
    const path = { ...value };
    const kind = stringValue(firstPresent(path.kind, path.type));
    const id = stringValue(firstPresent(path.id, path.pathId, path.path_id));
    if (kind) {
      path.kind = kind;
    }
    if (id) {
      path.id = id;
    }
    return path;
  }
  return stringValue(value) || undefined;
}

function normalizeDrawingPartToken(value) {
  const raw =
    value && typeof value === "object" && !Array.isArray(value)
      ? firstPresent(
          value.drawingPart,
          value.drawing_part,
          value.part,
          value.kind,
          value.type,
          value.id,
          value.label
        )
      : value;
  const token = slugify(raw).replace(/-/g, "_");
  if (["line", "path_line", "alignment_line"].includes(token)) {
    return "line";
  }
  if (
    [
      "station",
      "stations",
      "marker",
      "markers",
      "tick",
      "ticks",
      "chainage",
      "chainages",
    ].includes(token)
  ) {
    return "stations";
  }
  return "";
}

function drawingPartEntriesFromValue(value) {
  const values = Array.isArray(value)
    ? value
    : typeof value === "string"
      ? value.split(/[,|]/)
      : value == null
        ? []
        : [value];
  const entries = [];
  const seen = new Set();
  for (const entry of values) {
    const part = normalizeDrawingPartToken(entry);
    if (!part || seen.has(part)) {
      continue;
    }
    seen.add(part);
    entries.push({ part, input: entry });
  }
  return entries;
}

function drawingPartEntries(input, metadata) {
  const entries = [];
  const seen = new Set();
  const push = (entry) => {
    if (!entry?.part || seen.has(entry.part)) {
      return;
    }
    seen.add(entry.part);
    entries.push(entry);
  };

  for (const value of [
    input?.drawingParts,
    input?.drawing_parts,
    input?.parts,
    metadata?.drawingParts,
    metadata?.drawing_parts,
    metadata?.parts,
  ]) {
    for (const entry of drawingPartEntriesFromValue(value)) {
      push(entry);
    }
  }

  if (Object.prototype.hasOwnProperty.call(input || {}, "line") && input.line !== false) {
    push({ part: "line", input: input.line });
  }
  if (
    (Object.prototype.hasOwnProperty.call(input || {}, "stations") &&
      input.stations !== false) ||
    (Object.prototype.hasOwnProperty.call(input || {}, "markers") &&
      input.markers !== false)
  ) {
    push({ part: "stations", input: firstPresent(input.stations, input.markers) });
  }
  return entries;
}

function drawingGroupsFromInput(input) {
  if (Array.isArray(input)) {
    return input;
  }
  if (!input || typeof input !== "object") {
    return [];
  }
  for (const key of ["groups", "alignments", "paths", "drawings"]) {
    if (Array.isArray(input[key])) {
      return input[key];
    }
  }
  return [];
}

function syntheticDrawingPartGroup(parent, entry, inherited) {
  const partInput =
    entry.input && typeof entry.input === "object" && !Array.isArray(entry.input)
      ? entry.input
      : {};
  const metadataInput =
    partInput.metadata && typeof partInput.metadata === "object" ? partInput.metadata : {};
  const metadata = {
    ...parent.metadata,
    ...metadataInput,
    drawingPart: entry.part,
  };
  const path = normalizeDrawingPath(
    firstPresent(partInput.path, partInput.drawingPath, metadata.path, inherited.path)
  );
  if (path !== undefined) {
    metadata.path = path;
  }
  const resource = stringValue(
    firstPresent(partInput.resource, metadata.resource, inherited.resource)
  );
  if (resource) {
    metadata.resource = resource;
  }
  const explicitLayerId = stringValue(
    firstPresent(
      partInput.layerId,
      partInput.layer_id,
      partInput.annotationLayerId,
      partInput.annotation_layer_id,
      metadata.layerId,
      metadata.layer_id,
      metadata.annotationLayerId,
      metadata.annotation_layer_id
    )
  );
  if (explicitLayerId) {
    metadata.layerId = explicitLayerId;
  }
  const line = structuredValue(
    firstPresent(partInput.line, metadataInput.line, metadata.line, inherited.line)
  );
  if (line !== undefined && entry.part === "line") {
    metadata.line = line;
  }
  const markers = structuredValue(
    firstPresent(
      partInput.markers,
      metadataInput.markers,
      metadata.markers,
      metadata.stationMarkers,
      metadata.station_markers,
      inherited.markers
    )
  );
  if (markers !== undefined && entry.part === "stations") {
    metadata.markers = markers;
  }
  const maxSamples = numericValue(
    firstPresent(
      partInput.maxSamples,
      partInput.max_samples,
      metadataInput.maxSamples,
      metadataInput.max_samples,
      metadata.maxSamples,
      metadata.max_samples,
      inherited.maxSamples
    )
  );
  if (maxSamples !== undefined) {
    metadata.maxSamples = maxSamples;
  }

  const label =
    stringValue(firstPresent(partInput.label, DRAWING_PART_LABELS[entry.part])) ||
    DRAWING_PART_LABELS[entry.part];

  return {
    id: `${parent.id}:${entry.part}`,
    label,
    sourceKind: parent.sourceKind,
    sourceDetail: parent.sourceDetail,
    sourceResource: parent.sourceResource,
    semanticIds: [],
    children: [],
    counts: { total: 1 },
    metadata,
    diagnostics: [],
  };
}

function normalizeDrawingGroup(input, inherited = {}) {
  const group = normalizeSemanticGroup(input, inherited);
  if (!group) {
    return null;
  }

  const metadata = input?.metadata && typeof input.metadata === "object" ? input.metadata : {};
  let path = normalizeDrawingPath(
    firstPresent(
      input.path,
      input.drawingPath,
      input.drawing_path,
      metadata.path,
      metadata.drawingPath,
      metadata.drawing_path,
      inherited.path
    )
  );
  if (path === undefined) {
    const kind = stringValue(firstPresent(metadata.pathKind, metadata.path_kind));
    const id = stringValue(firstPresent(metadata.pathId, metadata.path_id));
    if (kind && id) {
      const measure = stringValue(firstPresent(metadata.pathMeasure, metadata.path_measure));
      path = measure ? { kind, id, measure } : { kind, id };
    }
  }
  if (path !== undefined) {
    group.metadata.path = path;
  }
  const resource = stringValue(
    firstPresent(input.resource, metadata.resource, inherited.resource)
  );
  if (resource) {
    group.metadata.resource = resource;
  }
  const drawingPart = normalizeDrawingPartToken(
    firstPresent(
      input.drawingPart,
      input.drawing_part,
      input.part,
      metadata.drawingPart,
      metadata.drawing_part,
      metadata.part
    )
  );
  if (drawingPart) {
    group.metadata.drawingPart = drawingPart;
  }

  const childScope = {
    sourceKind: group.sourceKind,
    sourceDetail: group.sourceDetail,
    sourceResource: group.sourceResource,
    path: group.metadata.path,
    resource: group.metadata.resource,
  };
  const children = (Array.isArray(input.children) ? input.children : [])
    .map((child) => normalizeDrawingGroup(child, childScope))
    .filter(Boolean);
  const childParts = new Set(children.map((child) => drawingGroupPart(child)).filter(Boolean));
  for (const entry of drawingPartEntries(input, group.metadata)) {
    if (!childParts.has(entry.part)) {
      children.push(syntheticDrawingPartGroup(group, entry, childScope));
      childParts.add(entry.part);
    }
  }
  group.children = children;
  return group;
}

export function normalizeDrawingsFacet(input, { resource = "" } = {}) {
  const facetInput =
    input && typeof input === "object" && !Array.isArray(input)
      ? input
      : { id: DRAWINGS_FACET_ID, groups: drawingGroupsFromInput(input) };
  const groups = drawingGroupsFromInput(facetInput)
    .map((group) => normalizeDrawingGroup(group, { resource }))
    .filter(Boolean);
  const diagnostics = Array.isArray(facetInput?.diagnostics)
    ? facetInput.diagnostics.map(semanticDiagnosticMessage).filter(Boolean)
    : [];
  return {
    id: DRAWINGS_FACET_ID,
    label: stringValue(facetInput?.label) || "Drawings",
    sourceKind: stringValue(facetInput?.provenance),
    groups,
    diagnostics,
  };
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
  let hasDrawingsFacet = false;

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
    const normalized =
      id === DRAWINGS_FACET_ID
        ? {
            ...normalizeDrawingsFacet({ ...inputFacet, label }, {
              resource: stringValue(payload?.resource),
            }),
            label,
          }
        : {
            id,
            label,
            sourceKind: stringValue(inputFacet.provenance),
            groups: Array.isArray(inputFacet.groups)
              ? inputFacet.groups
                  .map((group) => normalizeSemanticGroup(group))
                  .filter(Boolean)
              : [],
            diagnostics: Array.isArray(inputFacet.diagnostics)
              ? inputFacet.diagnostics.map(semanticDiagnosticMessage).filter(Boolean)
              : [],
          };
    if (id === DRAWINGS_FACET_ID) {
      hasDrawingsFacet = true;
    }
    if (existing) {
      knownFacets.set(id, normalized);
    } else {
      extraFacets.push(normalized);
    }
  }

  if (!hasDrawingsFacet && payload?.drawings != null) {
    knownFacets.set(
      DRAWINGS_FACET_ID,
      normalizeDrawingsFacet(payload.drawings, { resource: stringValue(payload?.resource) })
    );
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

function annotationIdFragment(value) {
  let fragment = "";
  let pushedSeparator = false;
  for (const character of stringValue(value)) {
    if (/^[a-z0-9]$/i.test(character)) {
      fragment += character.toLowerCase();
      pushedSeparator = false;
    } else if (fragment && !pushedSeparator) {
      fragment += "-";
      pushedSeparator = true;
    }
  }
  return fragment.replace(/-+$/g, "") || "path";
}

export function drawingGroupPart(group) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  return normalizeDrawingPartToken(
    firstPresent(metadata.drawingPart, metadata.drawing_part, metadata.part)
  );
}

export function drawingGroupPath(group) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  const path = normalizeDrawingPath(
    firstPresent(metadata.path, metadata.drawingPath, metadata.drawing_path)
  );
  if (path !== undefined) {
    return path;
  }
  const kind = stringValue(firstPresent(metadata.pathKind, metadata.path_kind));
  const id = stringValue(firstPresent(metadata.pathId, metadata.path_id));
  if (!kind || !id) {
    return undefined;
  }
  const measure = stringValue(firstPresent(metadata.pathMeasure, metadata.path_measure));
  return measure ? { kind, id, measure } : { kind, id };
}

export function drawingGroupResource(group, fallbackResource = "") {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  return stringValue(firstPresent(metadata.resource, fallbackResource));
}

function drawingPathKindAndId(group) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  const path = drawingGroupPath(group);
  const pathObject = path && typeof path === "object" && !Array.isArray(path) ? path : {};
  return {
    kind: stringValue(
      firstPresent(
        metadata.pathKind,
        metadata.path_kind,
        pathObject.kind,
        pathObject.type
      )
    ),
    id: stringValue(
      firstPresent(
        metadata.pathId,
        metadata.path_id,
        metadata.drawingPathId,
        metadata.drawing_path_id,
        pathObject.id,
        pathObject.pathId,
        pathObject.path_id
      )
    ),
  };
}

export function drawingGroupLayerId(group) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  const explicit = stringValue(
    firstPresent(
      metadata.layerId,
      metadata.layer_id,
      metadata.annotationLayerId,
      metadata.annotation_layer_id
    )
  );
  if (explicit) {
    return explicit;
  }
  const part = drawingGroupPart(group);
  const { kind, id } = drawingPathKindAndId(group);
  if (!part || !kind || !id) {
    return "";
  }
  const resource = drawingGroupResource(group);
  return `path-annotations-${annotationIdFragment(resource)}-${annotationIdFragment(kind)}-${annotationIdFragment(id)}-${annotationIdFragment(part)}`;
}

function drawingGroupStructuredMetadata(group, ...keys) {
  const metadata = group?.metadata && typeof group.metadata === "object" ? group.metadata : {};
  return structuredValue(firstPresent(...keys.map((key) => metadata[key])));
}

function drawingGroupCommand(group, resource) {
  const part = drawingGroupPart(group);
  const path = drawingGroupPath(group);
  if (!part || !path) {
    return null;
  }
  const command = {
    resource: drawingGroupResource(group, resource),
    path,
    drawingPart: part,
    layerId: drawingGroupLayerId(group),
  };
  const line = drawingGroupStructuredMetadata(group, "line", "lineRange", "line_range");
  if (line !== undefined && part === "line") {
    command.line = line;
  }
  const markers = drawingGroupStructuredMetadata(
    group,
    "markers",
    "markerGroups",
    "marker_groups"
  );
  if (markers !== undefined && part === "stations") {
    command.markers = markers;
  }
  const maxSamples = numericValue(
    drawingGroupStructuredMetadata(group, "maxSamples", "max_samples")
  );
  if (maxSamples !== undefined) {
    command.maxSamples = maxSamples;
  }
  return command;
}

function appendDrawingGroupCommands(group, resource, commands) {
  const command = drawingGroupCommand(group, resource);
  if (command) {
    commands.push(command);
  }
  if (!command) {
    for (const child of group?.children || []) {
      appendDrawingGroupCommands(child, resource, commands);
    }
  }
}

export function drawingGroupVisibilityCommands(group, resource = "") {
  const commands = [];
  const seen = new Set();
  appendDrawingGroupCommands(group, resource, commands);
  return commands.filter((command) => {
    const key = [
      command.resource,
      JSON.stringify(command.path),
      command.drawingPart,
      command.layerId,
    ].join("\u0000");
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function annotationLayerVisibilityById(viewState) {
  const layers = viewState?.annotations?.layers;
  const entries = Array.isArray(layers)
    ? layers
    : layers && typeof layers === "object"
      ? Object.values(layers)
      : [];
  const map = new Map();
  for (const layer of entries) {
    const id = stringValue(layer?.id);
    if (id) {
      map.set(id, layer?.visible !== false);
    }
  }
  return map;
}

export function drawingGroupOutlinerState(group, viewState, resource = "") {
  const commands = drawingGroupVisibilityCommands(group, resource);
  const layerMap = annotationLayerVisibilityById(viewState);
  const layerIds = uniqueStrings(commands.map((command) => command.layerId));
  const knownLayerIds = layerIds.filter((id) => layerMap.has(id));
  const visibleLayerIds = layerIds.filter((id) => layerMap.get(id) === true);
  const enabledCount = visibleLayerIds.length;
  const totalCount = layerIds.length || commands.length;
  return {
    commands,
    layerIds,
    knownLayerIds,
    visibleLayerIds,
    enabledCount,
    totalCount,
    checked: layerIds.length > 0 && enabledCount > 0,
    disabled: commands.length === 0,
    indeterminate:
      layerIds.length > 0 && enabledCount > 0 && enabledCount < layerIds.length,
  };
}

export function drawingGroupVisibilityOperation(group, viewState, resource, visible) {
  const state = drawingGroupOutlinerState(group, viewState, resource);
  return {
    action: visible ? "show" : "hide",
    commands: state.commands,
    state,
  };
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
