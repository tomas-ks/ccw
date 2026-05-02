# Ontology-Neutral Renderer Refactor Note

The renderer should stay ontology-neutral. It should be reusable for IFC, STEP, and future
Velr-backed model adapters where the ontology-specific meaning lives in the database, agents, and
tools rather than in the 3D rendering layer.

This note captures a future cleanup pass. It is not a request to remove IFC-focused workflows from
the web viewer or agents. The web app can stay pragmatic where IFC is the active product use case,
but the Rust runtime/renderer boundary should prefer geometric and source-neutral vocabulary.

## Target Boundary

The renderer/runtime should understand:

- geometry definitions
- geometry instances
- transforms
- bounds
- display colors
- neutral render classes
- selection, visibility, inspection, and section state

The renderer/runtime should not interpret:

- IFC entity names
- STEP entity names
- `GlobalId`
- alignment ids
- stations/chainage
- project/model ontology
- semantic relationship topology

Those concepts belong in adapter, database, semantic-tool, agent, or web UI layers. Those layers may
resolve ontology-specific requests into explicit renderer commands.

## Section State Cleanup

Current section support has the main boundary leak:

- `SectionState` carries `resource`, `alignment_id`, and `station`.
- the web section request mirrors these fields.

The neutral target is:

```text
SectionState {
  frame: SectionFrame,
  extent: SectionExtent,
  display: SectionDisplayMode,
  clip: SectionClipMode,
  provenance: Option<opaque metadata>
}
```

Where:

- `SectionFrame` is explicit world-space geometry: origin plus axes/normal.
- `SectionExtent` is width, height, and thickness.
- `provenance` can contain ontology-specific context, but the renderer must store and echo it only.
  It must never use provenance to compute placement or behavior.

Example IFC provenance:

```json
{
  "ontology": "ifc",
  "resource": "ifc/bridge-for-minnd",
  "kind": "alignment-station",
  "alignmentId": "...",
  "station": 120,
  "unit": "m"
}
```

Example STEP provenance could later refer to a datum plane, face, edge, or assembly context using
the same renderer command shape.

## Current Soft Leaks

These are acceptable for the current IFC-focused web product, but should be watched:

- Graph UI grouping and colors currently know about names such as `IfcRel`, `IfcMaterial`, `IfcSlab`,
  and spatial roots.
- Property balloons use IFC wording and `GlobalId` lookup.
- Web resource helpers use `ifc/` and `project/` prefixes directly.
- `PreparedGeometryElement` exposes `declared_entity`; this is acceptable as opaque metadata, but
  renderer behavior should branch on neutral render classes instead.

## Refactor Principle

Semantic tools resolve meaning. Renderer commands consume explicit geometry and neutral ids.

Good flow:

```text
ifc_station_resolve
  -> explicit section frame + extent + opaque provenance
viewer.section.set
  -> render section frame
```

Future STEP flow:

```text
step_datum_resolve
  -> explicit section frame + extent + opaque provenance
viewer.section.set
  -> render section frame
```

## Acceptance Criteria

- `cc-w-render` has no IFC or STEP vocabulary.
- `cc-w-runtime` section state is geometric and source-neutral.
- ontology-specific section fields move into opaque provenance or web/agent adapters.
- `viewer.section.set` requires an explicit frame and extent.
- old IFC-shaped section payloads are either rejected clearly or converted only above the runtime
  boundary when an explicit frame is already present.
- tests cover that provenance round-trips without affecting renderer behavior.

