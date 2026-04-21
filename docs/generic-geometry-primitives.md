# Generic Geometry Primitives for `w`

## Purpose

`w` core should name geometry by engine behavior, not by IFC entities. IFC- and STEP-specific class names belong in adapters in `cc-w-db`; `cc-w-kernel`, `cc-w-prepare`, `cc-w-scene`, `cc-w-runtime`, and `cc-w-render` should only see generic primitives.

This follows the current architecture:

- `docs/architecture.md` already requires basis/unit normalization at the adapter boundary and a split between geometry definitions and instances.
- `docs/ifc-reference-view-geometry.md` already identifies the first useful IFC geometry slice.
- the runnable frontend/backend path now crosses `PreparedGeometryPackage`, while `MeshDocument` remains only a temporary adapter/import envelope inside `cc-w-db`.

## Core Model

The core body-geometry model should start with three primitive families plus a separate auxiliary channel:

```rust
enum GeometryDefinitionKind {
    FaceSet(FaceSet3),
    ProfileSweep(ProfileSweep3),
    PathSweep(PathSweep3),
}

struct GeometryDefinition {
    id: GeometryDefId,
    local_bounds: Bounds3,
    kind: GeometryDefinitionKind,
    auxiliary: Vec<AuxiliaryGeometry>,
}

struct GeometryInstance {
    definition: GeometryDefId,
    world_from_instance: DMat4,
    external_id: ExternalId,
}
```

Recommended family names:

- `FaceSet3`: explicit indexed surface faces
- `ProfileSweep3`: a closed planar profile plus a simple sweep operator
- `PathSweep3`: a section swept along a 3D directrix
- `AuxiliaryGeometry`: footprint/bounds/reference data that is not default shaded body geometry

Do not add core enum variants like `IfcExtrudedAreaSolid` or `IfcSweptDiskSolid`. Those names should stop at the adapter boundary.

## Primitive Families

| Family | Core meaning | First useful contents |
| --- | --- | --- |
| `FaceSet3` | explicit surface definition | triangles first, then polygons with holes, optional normals |
| `ProfileSweep3` | 2D profile swept by a simple operator | linear extrusion and revolve |
| `PathSweep3` | section swept along a 3D path | circular or annular tube first |
| `AuxiliaryGeometry` | non-body representations carried with the definition or instance | footprint curves, boxes, centers/reference points |

`GeometryDefinition` and `GeometryInstance` are cross-cutting and should apply to every family. Reuse is not its own primitive family.

## Mapping IFC Reference View to Generic Primitives

| IFC RV concept | Generic primitive in `w` | Notes |
| --- | --- | --- |
| `IfcTriangulatedFaceSet` | `FaceSet3` | direct indexed-triangle path |
| `IfcPolygonalFaceSet` | `FaceSet3` | kernel triangulates polygons and holes |
| `IfcExtrudedAreaSolid` | `ProfileSweep3` | linear sweep of a normalized `Profile2` |
| `IfcRevolvedAreaSolid` | `ProfileSweep3` | revolve operator on the same profile IR |
| `IfcSweptDiskSolid` | `PathSweep3` | first path-sweep case with circular or annular section |
| `IfcRepresentationMap` + `IfcMappedItem` | `GeometryDefinition` + `GeometryInstance` | reuse mechanism, not a new primitive |
| `FootPrint`, `Box`, `CoG`, `Reference` | `AuxiliaryGeometry` | carried beside the body path |

This keeps the IFC adapter specific while letting the rest of the engine stay schema-neutral.

## How STEP-Style Geometry Fits the Same Model

STEP should map to the same families instead of introducing a parallel core model:

- tessellated or display-ready STEP geometry maps to `FaceSet3`
- swept, extruded, and revolved STEP solids map to `ProfileSweep3`
- tubular or wire-following STEP shapes map to `PathSweep3`
- assembly reuse maps to shared `GeometryDefinition` values plus many `GeometryInstance` placements

The awkward case is exact STEP shell/B-Rep data. The first STEP path should lower that data to one of the families above, usually `FaceSet3`, at the adapter/kernel boundary. If exact topology retention later proves necessary, that should be a new deferred family such as `BoundaryRep3`, not a reason to let STEP names leak into the current core model.

## Handling Through the Pipeline

In every case, `cc-w-db` is responsible for:

- source basis and unit normalization into `w` world conventions
- lowering IFC/STEP entities into generic primitive families
- separating reusable definition data from occurrence transforms

After that, handling should look like this:

- backend side: `cc-w-db` -> `cc-w-kernel` -> `cc-w-prepare`
- frontend side: `cc-w-scene` -> `cc-w-runtime` -> `cc-w-render`

| Family | `cc-w-kernel` | `cc-w-prepare` | `cc-w-runtime` / `cc-w-scene` / `cc-w-render` |
| --- | --- | --- | --- |
| `FaceSet3` | validate indices, winding, planarity; triangulate non-triangle faces; compute bounds in `f64` | emit `PreparedMesh`, local origin, normals, later chunk metadata | store one reusable definition handle plus instance transforms; upload once; draw many times |
| `ProfileSweep3` | validate profile loops; tessellate extrusion/revolution by tolerance; compute bounds in local definition space | same `PreparedMesh` path as meshes; cache by definition and tessellation settings | identical runtime/render contract to `FaceSet3` after preparation |
| `PathSweep3` | validate directrix, trims, and section; tessellate tube/annulus adaptively | same mesh-preparation path; optionally preserve centerline as auxiliary data later | identical runtime/render contract to other body primitives after preparation |
| `AuxiliaryGeometry` | validate and store; derive bounds/debug meshes only when needed | usually no default shaded preparation; later overlay/debug preparation paths | attach beside the main body instance; render only in explicit overlay/debug modes |

This is intentionally close to the current repo shape: `cc-w-kernel` stays CPU-side and `f64`, `cc-w-prepare` still targets `PreparedMesh`, and `cc-w-render` remains schema-agnostic. The new explicit split is that the backend owns geometry evaluation and package building, while the frontend consumes streamed prepared packages rather than owning tessellation as a normal production responsibility.

## Tessellation Policy

The kernel boundary should treat tessellation as explicit policy, not an invisible implementation detail. The current generic control surface is `TessellationRequest`, which carries:

- quality intent (`Draft`, `Balanced`, `Fine`)
- chord tolerance in world meters
- normal tolerance in radians
- optional maximum edge length in world meters
- normal generation mode
- path-frame mode for directrix sweeps

This keeps all future primitive families on one contract:

- `FaceSet3` can use the request for robust polygon triangulation and future edge/normal policy
- `ProfileSweep3` can use the same request for extrusion and revolve subdivision
- `PathSweep3` can use it for tube density and frame choice along the path

The renderer should still receive prepared triangle meshes as the canonical v1 payload. Any later shader-specialized analytic path should be derived from the same generic primitive plus tessellation-policy contract, not replace it.

At the deployment boundary, those prepared triangle meshes should travel inside `PreparedGeometryPackage` so web frontends can stay thin and transport-neutral.

## First Milestone

The first milestone should be the smallest step that unlocks both IFC RV and later STEP import work:

1. add `GeometryDefinition` and `GeometryInstance` as real shared types
2. keep runtime-facing `cc-w-db` output on generic definitions plus instances, with `MeshDocument` limited to adapter/import scaffolding
3. make `cc-w-scene` reference reusable definitions instead of only a `MeshHandle`
4. implement `FaceSet3` first, starting with triangles and then polygonal faces
5. keep `cc-w-prepare` and `cc-w-render` on one prepared-triangle-mesh path for now

That milestone covers:

- IFC RV tessellated body geometry
- IFC mapped geometry reuse
- STEP tessellated import
- STEP assembly/occurrence reuse

## Deferred

Defer these until the definition/instance split and `FaceSet3` path are working:

- `ProfileSweep3`
- `PathSweep3`
- richer `AuxiliaryGeometry` consumers beyond simple storage/bounds
- exact `BoundaryRep3`, CSG, and general freeform surfaces
- GPU instancing, line/edge passes, and material-system expansion

## Recommended Implementation Order

1. Introduce the generic definition/instance model in `cc-w-types`, `cc-w-db`, and `cc-w-scene`.
2. Replace the convex-polygon-only import path with `FaceSet3` triangles.
3. Extend `FaceSet3` to polygonal faces and holes.
4. Add definition-keyed kernel/prepare caching and repeated instance submission.
5. Add `ProfileSweep3` with linear sweep first, using the small profile IR already sketched in `docs/ifc-reference-view-geometry.md`.
6. Extend `ProfileSweep3` to revolve.
7. Add `PathSweep3`.
8. Add overlay/debug consumers for `AuxiliaryGeometry`, then re-evaluate whether exact boundary-representation support is needed as a first-class family.

That order matches the immediate-next-step direction in `docs/architecture.md`: first separate definitions from instances, then make the imported geometry path real, then expand analytic coverage.
