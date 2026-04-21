# IFC Reference View Implementation Plan

## Purpose

This document turns the current IFC Reference View geometry strategy into an implementation roadmap for `w`.

It answers three practical questions:

1. what exists today
2. what is still missing before `w` can claim meaningful IFC RV body-geometry support
3. what order we should implement the next steps in

## Current Status

The current state is best described as "primitive foundation in place, IFC RV pipeline not yet wired through."

Implemented today:

- shared world-frame and source-space normalization types
- generic geometry-definition and geometry-instance IDs
- generic primitive IR in `cc-w-types`
- `TessellatedGeometry`, `SweptSolid`, and `CircularProfileSweep` value types
- 2D/3D polycurve and profile IR that can preserve arcs
- generic import normalization in `cc-w-db` for tessellated geometry, swept solids, circular sweeps, and repeated instances
- kernel entry point over `GeometryPrimitive`
- tessellation for triangle, concave polygon, and holey tessellated face sets
- generic `TessellationRequest` controls at the kernel boundary
- transport-neutral `PreparedGeometryPackage` boundary types for backend/frontend delivery
- package-first runtime wiring for prepared geometry sources
- repeated-instance demo resource proving one definition can drive multiple instances
- scene nodes can now reference geometry definition IDs
- native, headless, and web demo entrypoints all consume the package boundary

Still true today:

- native/headless still compose backend and frontend in-process for development, even though they now cross the package boundary
- `MeshDocument` still exists as an adapter/import envelope inside `cc-w-db`
- `cc-w-scene` still carries a `MeshHandle` placeholder because render/runtime reuse is not fully definition-driven yet
- `cc-w-kernel` does not yet tessellate sweeps
- there is no real IFC adapter yet
- there is no render submission/cache path for repeated mapped geometry yet

## Support Status by IFC RV Geometry Case

### `IfcTriangulatedFaceSet`

Status: partial foundation, not full support.

What we have:

- generic `TessellatedGeometry`
- generic source-space normalization path in `cc-w-db` that can carry imported tessellated resources
- direct kernel path for triangle faces
- prepared mesh path
- headless render example through the new primitive API

What is missing:

- IFC adapter parsing into `TessellatedGeometry`
- optional normals import
- style/color handling
- runtime use of definition/instance separation

### `IfcPolygonalFaceSet`

Status: partial foundation, not full support.

What we have:

- polygon-face representation through `IndexedPolygon`
- generic source-space normalization path in `cc-w-db` for imported tessellated polygon resources
- triangulation in the kernel for concave polygons and polygon faces with holes

What is missing:

- robust planarity and winding normalization
- IFC adapter parsing

### `IfcExtrudedAreaSolid`

Status: IR only.

What we have:

- generic `Profile2`
- generic `SweepPath::Linear`
- generic `SweptSolid`
- generic source-space normalization path in `cc-w-db` for imported swept-solid resources

What is missing:

- profile lowering from IFC
- extrusion tessellation in the kernel
- normals/edge generation through prepare
- runtime/render integration through reusable definitions

### `IfcRevolvedAreaSolid`

Status: IR only.

What we have:

- generic `SweepPath::Revolved`
- profile IR shared with extrusion
- generic source-space normalization path in `cc-w-db` for imported revolved swept-solid resources

What is missing:

- revolved solid tessellation
- tolerance strategy for angular subdivision
- IFC adapter lowering

### `IfcSweptDiskSolid`

Status: IR only.

What we have:

- generic `CircularProfileSweep`
- generic 3D directrix curve representation
- generic source-space normalization path in `cc-w-db` for imported circular sweep resources

What is missing:

- swept-disk tessellation
- trim handling
- join/frame behavior along the path
- IFC adapter lowering

### `IfcMappedItem` / `IfcRepresentationMap`

Status: partial foundation, not full support.

What we have:

- generic geometry-definition and geometry-instance IDs
- primitive-first runtime flow for a single reusable definition
- repeated-instance demo resource proving one definition can produce multiple scene instances
- docs and architecture direction for reusable definitions plus many instances

What is missing:

- repository API that returns full definition collections separately from instances
- runtime caching keyed by definition
- repeated submission / later instancing in render

### `IfcLocalPlacement` and placement chains

Status: conceptual only.

What we have:

- source-space normalization and transform carriers

What is missing:

- real IFC placement-chain resolution
- composition of product placement, mapped-item transform, and item-local transform
- tests for nested placements and repeated mapped use

### Auxiliary RV representations: `FootPrint`, `Box`, `CoG`, `Reference`

Status: not implemented.

What we have:

- docs and primitive-family direction only

What is missing:

- storage model in definitions or instances
- parser support
- any overlay or debug rendering path

## What Is Left Before We Can Say "IFC RV Body Geometry Works"

For a practical first claim of IFC RV support, `w` still needs all of the following:

1. primitive-first runtime wiring
2. full definition/instance flow through `db`, `scene`, and `runtime`
3. real `IfcTriangulatedFaceSet` import
4. real `IfcMappedItem` handling
5. real placement-chain handling
6. polygon-face support good enough for `IfcPolygonalFaceSet`
7. extrusion tessellation
8. revolve tessellation
9. swept-disk tessellation
10. enough render/runtime support to draw many definitions and many instances in one scene

That is the minimum path for meaningful IFC RV body geometry.

## Next Slice: Semantic Element IDs and Query-Driven Viewer Control

Before building richer selection, hide/show, and metadata-driven analysis tools, `w` needs an
explicit semantic element layer in the prepared-package contract.

This is the next contract step because IFC rendering control should target IFC elements, not
package-local draw instances.

### Contract Goal

The boundary should distinguish between:

- reusable geometry definitions
- render instances
- semantic elements

For IFC-backed packages:

- the semantic element ID must come from IFC identity carried by the source model
- the public viewer control ID should be `IfcProduct.GlobalId`
- the encoded transport form should be the raw `GlobalId` string itself

This must become the ID used for:

- query results
- hide / show
- selection
- frame / center
- later analytical appearance overrides

The renderer should still draw geometry instances, but higher-level viewer tools should operate on
semantic elements that can own one or more render instances.

### Required Boundary Additions

Add to the shared prepared-package contract:

- a semantic element ID type
- semantic element metadata entries
- a link from each prepared geometry instance to its semantic element ID
- default render-class metadata such as `physical`, `space`, `zone`, or `helper`

Recommended first semantic metadata payload:

- `element_id`
- `label`
- `declared_entity`
- `default_render_class`
- world-space bounds

### Parallel Execution Lanes

#### Lane A: Shared Boundary Contract

Primary crates:

- `cc-w-types`
- `cc-w-runtime` tests

Tasks:

- add a semantic element ID type to the shared boundary
- add a `PreparedGeometryElement` metadata type
- extend `PreparedGeometryInstance` with `element_id`
- update package validation and fixture/test helpers

Definition of done:

- the prepared package can represent many render instances belonging to one semantic element
- tests prove the contract can carry stable semantic IDs independently of draw-instance IDs

#### Lane B: IFC Extraction and Element Metadata

Primary crates:

- `cc-w-velr`

Tasks:

- populate semantic element metadata from `IfcProduct`
- set the public element ID from `IfcProduct.GlobalId`
- aggregate multiple body items under the same semantic element when one IFC product lowers to many
  prepared instances
- preserve default render-class hints for `physical`, `space`, `zone`, and `helper`

Definition of done:

- IFC-backed prepared packages expose semantic element IDs derived from IFC, not from package-local
  instance numbering

#### Lane C: Frontend Element Index and State

Primary crates:

- `cc-w-runtime`
- `cc-w-scene`

Tasks:

- build a frontend-side element index from the prepared package
- store per-element override state
- add runtime helpers for `show`, `hide`, `reset_visibility`, `select`, `clear_selection`,
  `frame_elements`, and `center_elements`
- rebuild visible render-instance submission from element state instead of mutating backend data

Definition of done:

- the frontend can apply state changes over lists of semantic element IDs without changing the
  package contract or query layer

#### Lane D: Local Query Service Surface

Primary crates:

- `cc-w-platform-web` server path
- `cc-w-velr`

Tasks:

- extend the local web server with a dev query endpoint such as `POST /api/cypher`
- execute queries backend-side against Velr
- return structured rows and a convenience path for semantic element IDs
- keep the browser as a thin client

Current status:

- done: the local Rust web server now exposes `POST /api/cypher`
- done: Cypher executes backend-side against `cc-w-velr`
- done: the response includes structured rows plus `semanticElementIds`
- done: the local Rust web server now exposes `GET /api/resources` and `POST /api/package`
- done: the web viewer can now fetch prepared IFC packages through the same server boundary

Definition of done:

- the web viewer can submit a Cypher query and receive rows or semantic element IDs without
  embedding Velr in the browser

#### Lane E: Web Viewer API and Terminal

Primary crates:

- `cc-w-platform-web`

Tasks:

- expose a small JS viewer API that accepts lists of semantic element IDs
- add a browser-side terminal panel with a JavaScript REPL bound to that API
- add convenience helpers such as `queryCypher` and `queryIds`

Current status:

- done: the web viewer now exposes the semantic-id viewer API and a browser-side JS REPL
- done: the browser API now has `queryCypher` / `queryIds` backed by the local Rust web server
- done: IFC resources are now listed in the web viewer and loaded through the Rust server boundary
- pending: use query results to drive richer viewer commands over larger IFC scenes and evolve the
  current package fetch into proper streaming

Definition of done:

- a user can run a query in the web viewer and immediately hide, show, select, or frame the
  returned IFC elements

### Parallelization and Merge Order

Recommended order:

1. land Lane A first, because it fixes the shared contract
2. run Lane B and Lane C in parallel against that contract
3. run Lane D in parallel with Lane C once the frontend knows what semantic IDs look like
4. land Lane E after the viewer API and query surface are stable enough to script against

This gives a clean vertical slice:

- IFC-backed prepared packages carry real IFC semantic IDs
- the frontend can apply per-element state
- the web viewer can query Velr and act on returned semantic IDs

### First-Slice Non-Goals

This semantic-ID slice does not need to solve:

- picking from screen-space ray casts
- permanent saved view states
- rich material pipelines
- partial GPU-side selection outlines
- final production auth or remote service hardening

The goal is narrower:

- stable IFC semantic IDs in the package boundary
- frontend element-state control
- query-driven hide/show/select/frame through the web viewer

## Recommended Implementation Order

### Phase 1: Primitive-First Runtime Migration

Goal:

- complete the transition from the legacy polygon document seam to the definition/instance primitive model
- make the frontend/backend package boundary explicit even when native runs both sides locally

Tasks:

- expand repository output from the current single-definition demo resource into real definition plus instance collections
- build backend-produced prepared packages as the normal handoff to the frontend
- keep `MeshDocument` out of runtime-facing demo/resource flows
- replace placeholder mesh-handle wiring with definition-driven prepared-asset lookup
- keep the old legacy bridge only where older adapter scaffolding still needs it

Definition of done:

- runtime can load one or more generic definitions plus one or more instances without using `MeshDocument.polygon`
- frontend code can consume a prepared package without owning tessellation as a normal production responsibility

### Phase 2: IFC RV Tessellated Geometry

Goal:

- make `FaceSet3` the first real IFC RV body-geometry slice

Tasks:

- add an IFC adapter path for `IfcTriangulatedFaceSet -> TessellatedGeometry`
- import optional normals if available
- add basic style/material carrier data at least far enough for body color
- extend the current package-fed tessellated demos into repository-fed tessellated definitions from real adapters

Current note:

- the minimal milestone now exists as a per-draw fragment-stage material color in the renderer
- the renderer now keeps geometry definitions in object space and carries instance model matrices separately instead of collapsing repeated instances into one mesh bucket
- camera and transform composition now stay in `f64` on the CPU side and only convert to `f32` at the GPU upload boundary
- repeated instances of one definition now batch into one GPU instance buffer and one instanced draw for that mesh definition
- current shading assumes instance transforms are rigid-body or uniform-scale; if non-uniform scaling/shear enters later, normals need a stricter transform path again
- next style steps should stay intelligence-oriented: selection overrides, heat maps, x-ray display, and similar analytical views before any richer photoreal material model

Definition of done:

- real IFC triangulated body geometry can render through the primitive-first runtime path

### Phase 3: Mapped Geometry and Placements

Goal:

- support reuse and repeated placement, which is central to IFC RV

Tasks:

- add definition reuse in `cc-w-db`
- add instance transform composition from placement chains and mapped-item transforms
- add runtime caching keyed by geometry definition
- support repeated draw submission of the same prepared geometry

Definition of done:

- one geometry definition can appear many times in the scene with distinct transforms

### Phase 4: Polygonal Face Sets

Goal:

- complete the mesh-native side of IFC RV body geometry

Tasks:

- harden polygon triangulation and validation beyond the current earcut-based baseline
- add stronger face validation and diagnostics

Definition of done:

- `IfcPolygonalFaceSet` can lower to prepared triangle meshes robustly enough for real import cases

### Phase 5: Profile Sweeps

Goal:

- make the first analytic RV solids work

Tasks:

- implement kernel tessellation for linear extrusion
- implement kernel tessellation for revolve
- add profile-lowering helpers for IFC profile usage
- preserve tolerance settings at the kernel boundary

Definition of done:

- `IfcExtrudedAreaSolid` and `IfcRevolvedAreaSolid` render through the same prepared-mesh path as face sets

### Phase 6: Circular Path Sweeps

Goal:

- support `IfcSweptDiskSolid`

Tasks:

- implement tube / annular tube tessellation along a 3D directrix
- support start/end trims
- choose an initial frame strategy for path sweep orientation

Definition of done:

- reinforcing-style RV content can render through `CircularProfileSweep`

### Phase 7: Auxiliary RV Geometry

Goal:

- preserve and expose the useful non-body representations

Tasks:

- add `AuxiliaryGeometry` storage
- parse `FootPrint`, `Box`, `CoG`, and `Reference`
- use `Box` for fallback bounds/debug
- keep overlays optional and out of the default body path

Definition of done:

- auxiliary RV representations survive import and are queryable or renderable in explicit debug/overlay modes

## Near-Term Next Sprint

The next sprint should stay narrow and focus on the highest-leverage gap:

1. implement robust polygon triangulation for generic face sets, including holes
2. implement kernel tessellation for extrusion and revolve on the shared tessellation-request contract
3. add runtime/render caching and repeated submission for reusable geometry definitions

Why this sprint first:

- it hardens the generic kernel before adapter work starts
- it unlocks both IFC RV body geometry and STEP-style import on the same primitive families
- it finishes the most important missing foundation before we widen the frontend surface area

## Rough Milestone Checklist

- [x] runtime no longer depends on `MeshDocument.polygon`
- [x] scene nodes reference reusable geometry definitions
- [x] repeated demo instances can reuse one geometry definition
- [ ] repository API can return full definition collections and instances separately
- [ ] `IfcTriangulatedFaceSet` import works
- [ ] mapped geometry reuse works
- [ ] placement chains work
- [ ] polygonal faces with holes work
- [ ] extrusion works
- [ ] revolve works
- [ ] swept disk works
- [ ] auxiliary RV geometry is preserved

## Practical Summary

The project is in a good place structurally:

- the primitive vocabulary is now generic
- the kernel has a primitive entry point
- the primitive-first render example is real

But we are not yet in "IFC RV supported" territory.

The biggest remaining gap is not a specific primitive. It is the fact that the runtime path still has not fully crossed over to the new definition/instance primitive model.

That should be the next implementation step.
