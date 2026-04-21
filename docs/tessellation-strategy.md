# Tessellation Strategy for `w`

## Purpose

This note records the current generic tessellation strategy for `w` without tying it to IFC- or STEP-specific adapter types.

The goal is to make sure the engine grows around a stable geometry contract that can support the primitives we need for IFC Reference View body geometry while staying reusable for STEP-style frontends later.

## Canonical Path

For v1, the canonical backend geometry path should stay:

`GeometryPrimitive` -> `cc-w-kernel` tessellation -> `TriangleMesh` -> `PreparedMesh` -> `PreparedGeometryPackage`

That means:

- the kernel is the source of truth for converting higher-order primitives into renderable surfaces
- the frontend renderer stays triangle-native
- headless image tests, culling, bounds, picking, and later sectioning all depend on the same CPU-side geometry result

Any future shader-specialized analytic path should be an optimization layered on top of this contract, not a replacement for it.

On the frontend side, the normal production path should be:

`PreparedGeometryPackage` -> scene/runtime projection -> GPU upload -> render

For web deployment, tessellation should therefore happen on the backend by default, not in the browser.

## Tessellation Policy Surface

The kernel boundary now has a generic `TessellationRequest`:

- `quality`
- `chord_tolerance`
- `normal_tolerance_radians`
- `max_edge_length`
- `normal_mode`
- `path_frame_mode`

These values live in the internal `w` world frame, so lengths are meters and angles are radians.

This is the shared contract for:

- polygon face triangulation quality
- extrusion and revolve subdivision
- swept-disk density and path framing
- future cache keys for prepared geometry reuse

## Generic Primitive Strategy

### 1. Face sets

Current state:

- triangle faces work
- convex polygon fan triangulation works
- polygon holes do not work yet
- robust non-convex triangulation does not work yet

Recommended canonical path:

1. validate planarity, duplicate vertices, and winding in `f64`
2. build a stable local face plane for each polygon
3. project the exterior ring and hole rings into 2D
4. triangulate in 2D
5. map triangles back to the original 3D vertex indices

Recommended crate direction:

- `spade` is the strongest candidate for the canonical kernel triangulation path
  - current docs say it provides exact geometric predicate evaluation and a 2D `ConstrainedDelaunayTriangulation`
  - it also exposes refinement controls and can exclude outer faces, including holes, which matches polygon-with-hole workflows well
- `earcut` is a good benchmark and possible fast path for simple coplanar polygons
  - current docs mention reusable buffers, an experimental 3D coplanar projection helper, and strong benchmark numbers
  - current `earcutr` docs also make clear that the ear-slicing approach aims for acceptable practical output but does not guarantee correctness on degeneracies and self-intersections
- `lyon_tessellation` is useful, but it looks better suited to 2D path/fill work than to being the core 3D polygon kernel
  - its API is path/fill oriented and `f32`-leaning

Recommendation:

- use a constrained triangulation path as the canonical generic face-set tessellator
- keep Earcut-class tessellation as an optional comparison or fallback path if it proves materially faster on clean polygon data

This recommendation is an inference from the current crate documentation, not yet from local implementation experience in this repo.

### 2. Profile sweeps

Current state:

- generic profile and sweep IR exists
- extrusion tessellation is not implemented
- revolve tessellation is not implemented

Recommended path:

- tessellate on the CPU from the preserved profile IR
- flatten arc segments by `TessellationRequest`
- generate caps from the same profile loops used for side walls
- cache results by definition ID plus tessellation request

Why CPU-first here:

- it keeps bounds, snapshots, and later edge extraction deterministic
- it avoids splitting behavior between web and native render backends
- it gives us one mesh-preparation path for both IFC RV and STEP frontends

### 3. Circular path sweeps

Current state:

- generic circular sweep IR exists
- swept-disk tessellation is not implemented

Recommended path:

- tessellate a tube or annular tube in the kernel
- use `ParallelTransport` as the default path-frame mode
- keep `Frenet` available only as an explicit alternate mode
- support trims as part of kernel tessellation, not as a render-side afterthought

`ParallelTransport` is the better default because it is usually less twist-prone on piecewise-linear and gently curved directrices.

## Why Not Make Higher-Order Shader Primitives Canonical Yet

We should keep the option open for future GPU-specialized paths, especially for:

- repeated circular sweeps
- analytic caps
- instanced tubes or profile sweeps

But those should not be the canonical representation in v1, because the engine still needs:

- deterministic headless image output
- CPU-side bounds for culling and streaming
- unified behavior across web and native
- geometry usable for selection, diagnostics, and later clipping/section work

So the practical rule is:

- preserve higher-order generic primitives in the IR
- tessellate them in the kernel for the canonical runtime/render path
- add shader-specialized acceleration only after the CPU result is already correct and cacheable

## Status Snapshot

| Generic primitive area | Current status | Still missing |
| --- | --- | --- |
| Tessellated face sets | triangles and convex fan triangulation work | holes, robust non-convex triangulation, stronger validation |
| Extrusion | IR exists | kernel tessellation |
| Revolve | IR exists | kernel tessellation |
| Circular path sweep | IR exists | kernel tessellation, trims, frame behavior |
| Definition/instance reuse | one-definition repeated-instance demo works | runtime/render caching and repeated submission keyed by definition |

## Suggested Next Implementation Order

1. implement robust polygon triangulation with holes for generic face sets
2. implement extrusion tessellation on the shared request contract
3. implement revolve tessellation on the same contract
4. add definition-keyed prepare/render caching and repeated submission
5. implement circular path sweeps

## Sources

- [spade crate docs](https://docs.rs/spade/latest/spade/)
- [spade refinement parameters](https://docs.rs/spade/latest/spade/struct.RefinementParameters.html)
- [lyon_tessellation `FillTessellator`](https://docs.rs/lyon_tessellation/latest/lyon_tessellation/struct.FillTessellator.html)
- [earcut crate docs](https://docs.rs/crate/earcut/latest)
- [earcutr crate docs](https://docs.rs/crate/earcutr/latest)
