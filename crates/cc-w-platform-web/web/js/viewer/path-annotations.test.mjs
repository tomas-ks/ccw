import assert from "node:assert/strict";
import test from "node:test";

import {
  deterministicPathPartLayerId,
  diagnosticMessage,
  normalizePathAnnotationRequest,
  normalizePathPartVisibilityRequest,
} from "./path-annotations.mjs";

const path = {
  kind: "ifc_alignment",
  id: "curve:215711",
  measure: "station",
};

test("path annotation normalization preserves explicit line ranges", () => {
  const { mode, payload } = normalizePathAnnotationRequest(
    {
      resource: "ifc/bridge-for-minnd",
      path,
      line: { ranges: [{ from: 100, to: 200 }] },
    },
    "project/bridge-for-minnd"
  );

  assert.equal(mode, "replace");
  assert.deepEqual(payload.line.ranges, [{ from: 100, to: 200 }]);
  assert.equal(payload.resource, "ifc/bridge-for-minnd");
});

test("path annotation normalization accepts composable line range shorthand", () => {
  const { payload } = normalizePathAnnotationRequest(
    {
      path,
      line_ranges: [{ from: 100, to: 200 }],
    },
    "ifc/bridge-for-minnd"
  );

  assert.deepEqual(payload.line, { ranges: [{ from: 100, to: 200 }] });
});

test("path annotation normalization accepts a direct line range array", () => {
  const { payload } = normalizePathAnnotationRequest(
    {
      path,
      line: [{ start: "100m", end: "200m" }],
    },
    "ifc/bridge-for-minnd"
  );

  assert.deepEqual(payload.line, { ranges: [{ from: 100, to: 200 }] });
});

test("path annotation normalization accepts a singular line range object", () => {
  const { payload } = normalizePathAnnotationRequest(
    {
      path,
      line: { range: { from: 100, to: 200 } },
    },
    "ifc/bridge-for-minnd"
  );

  assert.deepEqual(payload.line, { ranges: [{ from: 100, to: 200 }] });
});

test("absolute station bounds win over duplicate offset bounds", () => {
  const { payload } = normalizePathAnnotationRequest(
    {
      path,
      line: {
        ranges: [{ from: 100, from_offset: 100, to: 200, to_offset: 200 }],
      },
      markers: [
        {
          range: { from: 100, fromOffset: 100, to: 200, toOffset: 200 },
          every: 20,
        },
      ],
    },
    "ifc/bridge-for-minnd"
  );

  assert.deepEqual(payload.line, { ranges: [{ from: 100, to: 200 }] });
  assert.deepEqual(payload.markers, [{ range: { from: 100, to: 200 }, every: 20 }]);
});

test("marker-only add with default line does not invent a full-path line", () => {
  const { mode, payload } = normalizePathAnnotationRequest(
    {
      mode: "add",
      path,
      line: {},
      markers: [{ range: { from: 400, toEnd: true }, every: 50, label: "measure" }],
    },
    "ifc/bridge-for-minnd"
  );

  assert.equal(mode, "add");
  assert.equal(payload.line, undefined);
  assert.deepEqual(payload.markers, [
    { range: { from: 400, to_end: true }, every: 50, label: "measure" },
  ]);
});

test("add keeps an explicit line range when the user asked for a new segment", () => {
  const { mode, payload } = normalizePathAnnotationRequest(
    {
      update: "plus",
      path,
      line: { ranges: [{ start: "300m", end: "400m" }] },
      markers: [{ range: { from: 300, to: 400 }, interval: "20m", label: "measure" }],
    },
    "ifc/bridge-for-minnd"
  );

  assert.equal(mode, "add");
  assert.deepEqual(payload.line.ranges, [{ from: 300, to: 400 }]);
  assert.deepEqual(payload.markers, [
    { range: { from: 300, to: 400 }, every: 20, label: "measure" },
  ]);
});

test("explicit end tokens normalize to to_end", () => {
  const { payload } = normalizePathAnnotationRequest(
    {
      path,
      line: { ranges: [{ from: 400, to: "end" }] },
      markers: [{ range: { from: 400, toEnd: true }, step: 50 }],
    },
    "ifc/bridge-for-minnd"
  );

  assert.deepEqual(payload.line.ranges, [{ from: 400, to_end: true }]);
  assert.deepEqual(payload.markers, [{ range: { from: 400, to_end: true }, every: 50 }]);
});

test("path drawing normalization defaults a visible line to the explicit path", () => {
  const { visible, layer_id, payload } = normalizePathPartVisibilityRequest(
    {
      resource: "ifc/bridge-for-minnd",
      path,
      part: "line",
      visible: "true",
    },
    "project/bridge-for-minnd"
  );

  assert.equal(visible, true);
  assert.equal(
    layer_id,
    "path-annotations-ifc-bridge-for-minnd-ifc-alignment-curve-215711-line"
  );
  assert.deepEqual(payload, {
    resource: "ifc/bridge-for-minnd",
    path,
    part: "line",
    layer_id,
    line: {},
  });
});

test("path drawing normalization uses one deterministic layer for show and hide", () => {
  const shown = normalizePathPartVisibilityRequest(
    {
      resource: "ifc/bridge-for-minnd",
      path,
      part: "stations",
      visible: true,
      markers: [{ interval: "20m", range: { from: 100, to: "end" }, label: "measure" }],
    },
    "project/bridge-for-minnd"
  );
  const hidden = normalizePathPartVisibilityRequest(
    {
      resource: "ifc/bridge-for-minnd",
      path,
      part: "station",
      visible: false,
    },
    "project/bridge-for-minnd"
  );

  assert.equal(shown.layer_id, hidden.layer_id);
  assert.equal(
    shown.layer_id,
    deterministicPathPartLayerId({
      resource: "ifc/bridge-for-minnd",
      path,
      part: "stations",
    })
  );
  assert.deepEqual(shown.payload.markers, [
    { range: { from: 100, to_end: true }, every: 20, label: "measure" },
  ]);
  assert.equal(shown.payload.line, undefined);
  assert.equal(hidden.visible, false);
});

test("diagnostics include explicit and requested measure ranges", () => {
  const message = diagnosticMessage({
    code: "measure_range_outside_explicit_path",
    message: "Requested measure range falls outside the explicit IFC alignment station range.",
    details: {
      explicit_measure_start: 0,
      explicit_measure_end: 500,
      from: 400,
      to: 600,
    },
  });

  assert.equal(
    message,
    "Requested measure range falls outside the explicit IFC alignment station range. Explicit path: 0..500; requested: 400..600."
  );
});
