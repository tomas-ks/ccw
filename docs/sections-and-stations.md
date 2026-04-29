# Sections And Stations

This document defines the contract for station-based section views.

The goal is to support requests such as:

```text
show me a cross section at station 120
```

without guessing geometry, placement, stationing, or alignment intent.

## Design Rules

- A station value is meaningful only when it resolves through explicit IFC facts.
- The AI must not infer stationing from model bounds, the longest curve, object names, terrain
  shape, visible bridge centerlines, or screen-space placement.
- If multiple alignments can own the requested station, the user must choose an alignment or the
  tool must return a ranked candidate list with provenance.
- If the active model does not expose enough facts to resolve a station pose, the system must fail
  loudly with diagnostics rather than drawing a plausible section plane.
- Section visualization is renderer state, not imported model data. A default section width, color,
  or translucency is a presentation policy and must not be treated as IFC source truth.

## Layering

Station section support has two separate layers.

Semantic resolution:

- discovers explicit alignment/station facts in the active IFC or project
- resolves station values to world-space poses
- reports provenance and unsupported facts

Renderer section state:

- stores one active section
- renders a 3D overlay and, later, optional clipping or 2D section views
- does not decide what station 120 means

## Shared Runtime Types

The frontend/runtime contract starts with these transport-neutral shapes in `cc-w-types`:

```rust
SectionPose {
    origin: DVec3,
    tangent: DVec3,
    normal: DVec3,
    up: DVec3,
}

SectionState {
    resource: String,
    alignment_id: Option<String>,
    station: Option<f64>,
    pose: SectionPose,
    width: f64,
    height: f64,
    thickness: f64,
    mode: SectionDisplayMode,
    clip: SectionClipMode,
    provenance: Vec<String>,
}
```

All vectors are in the engine world frame: right-handed, Z-up, metric.

The expected pose meaning is:

- `origin`: resolved world point at the requested station
- `tangent`: alignment tangent at the station
- `normal`: cross-section horizontal axis, usually perpendicular to the tangent
- `up`: vertical section axis

The runtime stores this as committed viewer state. The renderer consumes it. The semantic tools
produce it.

## Semantic Tool Contracts

### `ifc_alignment_catalog`

Purpose:

- list explicit alignment or station-capable entities in the active IFC/project

Returns:

- `resource`
- `db_node_id`
- `global_id`
- `declared_entity`
- `name`
- known station/range/unit evidence when available
- provenance strings
- diagnostics for unsupported or incomplete alignment structures

Allowed evidence includes explicit IFC graph facts such as:

- `IfcAlignment`
- `IfcLinearPlacement`
- `IfcAxis2PlacementLinear`
- `IfcPointByDistanceExpression`
- explicit curve or gradient entities that the importer exposes with enough topology to evaluate

### `ifc_station_resolve`

Purpose:

- resolve `alignment_id + station` to a `SectionPose`

Inputs:

- resource
- alignment id or DB node id
- station value
- requested orientation, defaulting to cross section

Returns:

- resolved pose
- unit handling
- provenance
- diagnostics

Failure is valid and expected when required facts are missing. The tool must not silently switch to
a different alignment, use model bounds, or approximate from a visible product path.

### `ifc_section_intersections`

Purpose:

- summarize what a resolved section cuts through

V1 may return only explicitly station-related candidates. True geometric plane intersections are a
later renderer/geometry feature and must be marked as not implemented until it exists.

## Viewer API Contract

The JS API should expose a small bridge over Rust-controlled state:

```js
viewer.section.set({
  resource,
  alignmentId,
  station,
  pose,
  width,
  height,
  thickness,
  mode: "3d-overlay",
  clip: "none",
  provenance,
})

viewer.section.clear()
viewer.section.state()
```

Corresponding backend surface:

- `POST /api/viewer/section/set`
- `POST /api/viewer/section/clear`
- `GET /api/viewer/section/state`

The web side should not compute or guess section poses. It forwards resolved state to the Rust
viewer/runtime layer.

## Renderer Contract

The first rendering slice is intentionally small:

- render a translucent section slab/plane at the resolved pose
- render subtle orientation axes
- keep Diffuse, BIM, and Architectural profiles compatible
- do not clip geometry yet
- do not generate 2D linework yet

Later slices:

- one-sided clipping
- cut cap rendering
- true mesh/plane intersection curves
- 2D orthographic section panel
- labels and quantity summaries grouped by semantic id/material/source IFC

## AI Flow

For a request like "show me a cross section at station 120":

1. Call `ifc_alignment_catalog`.
2. If exactly one strong alignment candidate exists, call `ifc_station_resolve`.
3. If multiple candidates exist, ask which alignment.
4. If station resolution succeeds, call `viewer.section.set`.
5. Optionally call `ifc_section_intersections` for a summary.
6. If resolution fails, explain the missing explicit IFC facts and do not draw a section.

## Parallel Implementation Lanes

1. Semantic tools can implement catalog/resolve/intersection diagnostics against this contract.
2. Runtime can store and expose `SectionState`.
3. Renderer can implement the overlay pass against `SectionState`.
4. Web can expose the JS/API bridge without owning resolution logic.
5. Agent prompts can route station requests through the semantic tools before viewer actions.
6. Tests can independently validate runtime state, tool refusal behavior, and API shape.
