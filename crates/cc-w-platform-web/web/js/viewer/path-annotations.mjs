export const firstPresent = (...values) =>
  values.find((value) => value !== undefined && value !== null && value !== "");

const numericValue = (value) => {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : undefined;
  }
  if (typeof value === "string") {
    const match = value.trim().match(/^-?\d+(?:\.\d+)?/);
    if (match) {
      const number = Number(match[0]);
      return Number.isFinite(number) ? number : undefined;
    }
  }
  const number = Number(value);
  return Number.isFinite(number) ? number : undefined;
};

const positiveNumberOr = (value, fallback) => {
  const number = numericValue(value);
  return Number.isFinite(number) && number > 0 ? number : fallback;
};

const optionalFiniteNumber = (value) => {
  const number = numericValue(value);
  return Number.isFinite(number) ? number : undefined;
};

const booleanishTrue = (value) => {
  if (value === true) {
    return true;
  }
  if (typeof value !== "string") {
    return false;
  }
  return ["1", "true", "yes", "y", "on"].includes(value.trim().toLowerCase());
};

const isExplicitPathEndToken = (value) => {
  if (typeof value !== "string") {
    return false;
  }
  const token = value.trim().toLowerCase().replace(/[\s-]+/g, "_");
  return ["end", "path", "path_end", "explicit_end", "explicit_path_end"].includes(token);
};

export const normalizeMeasureRange = (range) => {
  if (!range || typeof range !== "object" || Array.isArray(range)) {
    return null;
  }
  const normalized = {};
  for (const [target, keys] of [
    ["from", ["from", "start", "from_measure", "fromMeasure"]],
    ["to", ["to", "end", "to_measure", "toMeasure"]],
    ["from_offset", ["from_offset", "fromOffset", "start_offset", "startOffset"]],
    ["to_offset", ["to_offset", "toOffset", "end_offset", "endOffset"]],
  ]) {
    const number = optionalFiniteNumber(firstPresent(...keys.map((key) => range[key])));
    if (number !== undefined) {
      normalized[target] = number;
    }
  }
  const pathEndRequested =
    booleanishTrue(firstPresent(range.to_end, range.toEnd, range.path_end, range.pathEnd)) ||
    isExplicitPathEndToken(firstPresent(range.to, range.end, range.to_measure, range.toMeasure));
  if (pathEndRequested) {
    delete normalized.to;
    delete normalized.to_offset;
    normalized.to_end = true;
  }
  if (normalized.from !== undefined) {
    delete normalized.from_offset;
  }
  if (normalized.to !== undefined) {
    delete normalized.to_offset;
  }
  const startsAfterPathStart =
    (Number.isFinite(normalized.from) && normalized.from > 0) ||
    (Number.isFinite(normalized.from_offset) && normalized.from_offset > 0);
  if (
    normalized.to === undefined &&
    normalized.to_offset !== undefined &&
    Math.abs(normalized.to_offset) <= 1.0e-9 &&
    startsAfterPathStart
  ) {
    delete normalized.to_offset;
  }
  return Object.keys(normalized).length ? normalized : {};
};

export const normalizeMeasureRanges = (value) => {
  if (!Array.isArray(value)) {
    return undefined;
  }
  return value.map(normalizeMeasureRange).filter(Boolean);
};

const normalizeMeasureRangeSet = (value) => {
  if (Array.isArray(value)) {
    return normalizeMeasureRanges(value);
  }
  const range = normalizeMeasureRange(value);
  return range ? [range] : undefined;
};

const normalizePathLine = (value) => {
  if (value === undefined || value === null || value === "") {
    return undefined;
  }
  if (Array.isArray(value)) {
    const ranges = normalizeMeasureRanges(value);
    return ranges ? { ranges } : {};
  }
  if (typeof value !== "object") {
    return undefined;
  }
  const rangeSource = Object.prototype.hasOwnProperty.call(value, "ranges")
    ? value.ranges
    : Object.prototype.hasOwnProperty.call(value, "range")
      ? value.range
      : value;
  const ranges = normalizeMeasureRangeSet(rangeSource);
  const line = {};
  if (ranges) {
    line.ranges = ranges;
  }
  return line;
};

const hasExplicitMeasureBoundary = (range) =>
  range &&
  typeof range === "object" &&
  !Array.isArray(range) &&
  Object.keys(range).length > 0;

const isDefaultPathLineRequest = (line, ranges) => {
  if (!line || typeof line !== "object" || Array.isArray(line)) {
    return false;
  }
  const keys = Object.keys(line).filter((key) => line[key] !== undefined && line[key] !== null);
  if (!keys.length) {
    return true;
  }
  return keys.length === 1 && keys[0] === "ranges"
    ? !(ranges?.some(hasExplicitMeasureBoundary) ?? false)
    : false;
};

export const normalizePathMarkers = (value) => {
  if (!Array.isArray(value)) {
    return undefined;
  }
  return value
    .map((marker) => {
      if (!marker || typeof marker !== "object" || Array.isArray(marker)) {
        return null;
      }
      const every = optionalFiniteNumber(firstPresent(marker.every, marker.interval, marker.step));
      const normalized = {};
      if (every !== undefined) {
        normalized.every = every;
      }
      const range = normalizeMeasureRange(marker.range);
      if (range) {
        normalized.range = range;
      }
      if (marker.label !== undefined) {
        normalized.label = marker.label;
      }
      return Object.keys(normalized).length ? normalized : null;
    })
    .filter(Boolean);
};

export const normalizeAnnotationUpdateMode = (value) => {
  const mode = String(value || "replace").trim().toLowerCase();
  if (["add", "append", "include", "plus"].includes(mode)) {
    return "add";
  }
  return "replace";
};

const formatMeasure = (value) => {
  const number = optionalFiniteNumber(value);
  return number === undefined ? null : Number(number.toFixed(6)).toString();
};

export const diagnosticMessage = (diagnostic) => {
  const base = diagnostic?.message || diagnostic?.code;
  if (!base) {
    return null;
  }
  const details = diagnostic?.details;
  if (
    diagnostic?.code === "measure_range_outside_explicit_path" &&
    details &&
    typeof details === "object"
  ) {
    const explicitStart = formatMeasure(details.explicit_measure_start);
    const explicitEnd = formatMeasure(details.explicit_measure_end);
    const from = formatMeasure(details.from);
    const to = formatMeasure(details.to);
    if (explicitStart && explicitEnd && from && to) {
      return `${base} Explicit path: ${explicitStart}..${explicitEnd}; requested: ${from}..${to}.`;
    }
  }
  return base;
};

export function normalizePathAnnotationRequest(spec = {}, fallbackResource = "") {
  const mode = normalizeAnnotationUpdateMode(
    firstPresent(spec.mode, spec.operation, spec.update, spec.behavior)
  );
  const inputPath = spec.path;
  if (!inputPath || typeof inputPath !== "object" || Array.isArray(inputPath)) {
    throw new Error("viewer.annotations.showPath requires path.");
  }
  const path = {
    kind: String(firstPresent(inputPath.kind, inputPath.type) || "").trim(),
    id: String(inputPath.id || "").trim(),
  };
  if (!path.kind || !path.id) {
    throw new Error("viewer.annotations.showPath requires path.kind and path.id.");
  }
  const measure = firstPresent(inputPath.measure, inputPath.measureKind);
  if (measure !== undefined && measure !== null && measure !== "") {
    path.measure = String(measure).trim();
  }

  const payload = {
    resource: String(firstPresent(spec.resource, fallbackResource) || "").trim(),
    path,
  };
  const markers = normalizePathMarkers(
    firstPresent(spec.markers, spec.marker_groups, spec.markerGroups)
  );
  const hasMarkers = Boolean(markers?.length);

  const line = normalizePathLine(
    firstPresent(
      spec.line,
      spec.line_range,
      spec.lineRange,
      spec.line_ranges,
      spec.lineRanges,
      spec.ranges
    )
  );
  if (line) {
    const ranges = line.ranges;
    const skipDefaultLineOnMarkerAdd =
      mode === "add" && hasMarkers && isDefaultPathLineRequest(line, ranges);
    if (!skipDefaultLineOnMarkerAdd) {
      payload.line = line;
    }
  }

  if (markers?.length) {
    payload.markers = markers;
  }

  const maxSamples = firstPresent(spec.max_samples, spec.maxSamples);
  if (maxSamples !== undefined) {
    payload.max_samples = Math.max(1, Math.floor(positiveNumberOr(maxSamples, 500)));
  }

  return { mode, payload };
}
