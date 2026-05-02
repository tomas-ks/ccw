# Scene Annotations

Scene annotations are renderer-owned infographic overlays such as alignment curves, station ticks,
chainage labels, diagnostic markers, or measurement callouts.

They are intentionally ontology-neutral. The renderer does not know whether an annotation came from
IFC, STEP, a user drawing tool, or a future analysis service. It only receives explicit world-space
primitives.

## Boundary

Annotation producers may be ontology-specific:

- IFC tools can resolve `IfcAlignment`, stationing, and curve facts.
- STEP tools may later resolve datum axes, edges, or assembly references.
- User tools may create manual measurements or markup.

Renderer/runtime annotation state is neutral:

- layer id
- lifecycle
- visibility
- polylines
- markers
- text labels
- opaque provenance

The renderer must not parse provenance to determine placement, units, or meaning.

## Data Flow

The intended flow avoids sending large polylines through the language model transcript:

```text
agent/tool intent
  -> small viewer action, e.g. show alignment annotations
  -> server compiles explicit source facts into a neutral annotation layer
  -> web bridge sends the layer to the Rust/WASM runtime
  -> renderer draws polylines, markers, and labels
```

This keeps the expensive/precise data path in ordinary application code:

```text
database/server -> web bridge -> renderer
```

and keeps the model-facing path small:

```text
alignment id + station interval + styling intent
```

## Integrity Rules

- Do not infer annotation geometry from model bounds, names, or visual heuristics.
- If the source graph does not expose the explicit facts required to build an annotation, fail
  loudly with diagnostics.
- Presentation defaults such as line color, label size, or marker style are allowed as visual
  policy. They are not source facts.
- IFC-specific facts may appear in endpoint/tool names and provenance, but not in renderer-facing
  primitive names.

## First Target

The first real annotation producer is IFC alignment visualization:

- draw requested measure ranges from an explicit path source
- sample markers along the path with one or more spacing rules
- label markers from the path measure when requested
- store provenance such as resource, path id, and resolver evidence

Example console-facing API:

```js
viewer.annotations.showPath({
  resource: "project/bridge-for-minnd",
  path: { kind: "ifc_alignment", id: "curve:215711", measure: "station" },
  line: { ranges: [{ from: 0, to: 140 }] },
  markers: [
    { range: { from: 0, to: 100 }, every: 10, label: "measure" },
    { range: { from: 100, to_end: true }, every: 50, label: "measure" }
  ]
})
```

Additive follow-ups can send only the new rule and ask the viewer to merge it:

```js
viewer.annotations.showPath({
  resource: "project/bridge-for-minnd",
  mode: "add",
  path: { kind: "ifc_alignment", id: "curve:215711", measure: "station" },
  markers: [{ range: { from: 120, to_end: true }, every: 50, label: "measure" }]
})
```

Use `to_end: true` when a line or marker range should end at the explicit IFC path end. Do not
guess the numeric end from visible bridge geometry or model bounds.

For marker-only additive updates, omit `line`. A default line request such as `line: {}` or
`line: { ranges: [{}] }` means "draw the whole explicit path", which is useful for explicit line
requests but too broad for adding marker rules.

The renderer still receives only a neutral annotation layer.
