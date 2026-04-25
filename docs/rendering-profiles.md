# Rendering Profiles

## Purpose

A rendering profile is a named visual contract for drawing the currently visible scene.

Profiles let us experiment with different presentation styles without changing model loading,
streaming, selection ownership, picking identity, or IFC semantics. The same project, runtime scene,
camera, visibility overrides, and resident geometry should be drawable through multiple profiles.

The first profiles are:

- `diffuse`: the current baseline renderer
- `architectural-v1`: solid shaded geometry plus geometry-derived crease and boundary lines

Later experimental profiles may include screen-space silhouettes, object-id outlines, hidden-line
views, x-ray display, analysis colors, or other technical illustration styles.

## Ownership

Rendering profiles live in the shared Rust rendering layer:

- owner crate: `cc-w-render`
- consumers: `cc-w-platform-web`, `cc-w-platform-native`, and `cc-w-platform-headless`
- input: `PreparedRenderScene`, camera, viewport, and renderer state
- output: GPU draw passes on the target surface or offscreen target

Profiles do not own or decide:

- which IFC resource or project is active
- which elements are loaded or resident
- which elements are visible
- which elements are selected
- database query behavior
- semantic graph expansion

Those domains are owned by app state, runtime state, and the semantic database as documented in
`state-management.md`.

## Contract

A profile may define:

- surface shader and lighting model
- clear color
- depth and culling behavior
- pass ordering
- geometry edge overlays
- screen-space post-processing overlays
- selection/highlight style
- tunable visual constants such as line color, opacity, crease angle, and depth bias

A profile must preserve:

- renderer identity semantics
- pick pass correctness
- current visibility and suppression state
- current camera and viewport behavior
- streaming and residency behavior

Picking is profile-independent unless a future profile explicitly documents a different picking
contract. The ID-color pick pass should continue to draw the same visible instances with the same
camera and depth behavior regardless of whether the visible profile is `diffuse` or an architectural
variant.

## Current Profiles

### `diffuse`

The existing renderer. It draws opaque triangles with per-instance material color, simple directional
lighting, depth testing, and back-face culling.

This profile is the baseline regression target. Refactors should keep `diffuse` visually equivalent
unless a change explicitly says otherwise.

### `architectural-v1`

The first architectural experiment.

Pass stack:

1. Solid shaded geometry pass.
2. Geometry-derived edge pass for boundary edges.
3. Geometry-derived edge pass for crease edges.

This version intentionally does not include screen-space silhouettes yet. It is meant to establish
the profile architecture and a deterministic mesh-edge overlay before we add post-processing.

Boundary edges are triangle edges used by exactly one triangle.

Crease edges are triangle edges shared by two triangles whose adjacent face normals differ more than
the profile threshold. The initial threshold should be conservative, around 30 degrees, and kept as a
profile parameter so it can be tuned without changing the geometry payload contract.

## Future Profiles

### `architectural-v2`

Likely next experiment:

- solid shaded geometry
- screen-space depth/object-id silhouette and visible object boundary pass
- optional screen-space normal discontinuity linework

Screen-space silhouettes are view-dependent and should be derived from the final visible image using
depth, normal, and/or object-id buffers.

### `architectural-v3`

Likely hybrid:

- solid shaded geometry
- screen-space silhouettes and object boundaries
- subtle geometry crease lines

This lets us compare pure mesh linework against screen-space visible outlines without collapsing the
experiments into one hardcoded architectural mode.

## Enabling Profiles

The shared renderer should expose profile APIs:

```rust
renderer.profile();
renderer.available_profiles();
renderer.set_profile(&device, color_format, RenderProfileId::ArchitecturalV1);
```

Changing a profile may rebuild pipelines and profile-specific GPU buffers, but it should not reload
the active project, reset visibility, clear selection, or change the runtime scene.

The web shell should expose the same capability through the viewer bridge:

```js
viewer.renderProfile()
viewer.renderProfiles()
viewer.setRenderProfile("architectural-v1")
```

The native shell should expose a profile selector because native is the rendering debug interface.
The web shell can expose a smaller selector later, but the JS API should exist first so experiments
can run from the console.

## Adding A Profile

To add a new profile:

1. Add a stable profile id and string name in `cc-w-render`.
2. Add the profile to the renderer registry.
3. Define its pass stack and tunable constants.
4. Reuse existing shared passes where possible.
5. Add profile-specific pipelines only where the pass genuinely differs.
6. Keep picking profile-independent unless the new profile documents why it differs.
7. Add a renderer unit test or headless smoke test.
8. Expose the profile through web/native selectors only after the shared renderer path works.

Avoid adding ad hoc `if architectural` branches in platform shells. Platform shells should select a
profile. The renderer should decide which passes and pipelines that profile implies.

## Implementation Plan

1. Add `RenderProfileId` and a small profile registry in `cc-w-render`.
2. Refactor the current pipeline as the `diffuse` profile with no visual change.
3. Add profile query and mutation APIs to `MeshRenderer`.
4. Add web and native bindings for selecting a profile.
5. Add mesh edge extraction during renderer upload:
   - collect undirected triangle edges
   - mark one-face edges as boundary edges
   - compare adjacent face normals for crease edges
6. Add GPU line buffers per uploaded mesh definition.
7. Add a line rendering pipeline with depth testing enabled and depth writes disabled.
8. Implement `architectural-v1` as solid shaded triangles plus boundary/crease line passes.
9. Add tests for edge extraction:
   - one triangle produces three boundary edges
   - two coplanar triangles do not produce their shared edge as a crease
   - two angled triangles produce their shared edge as a crease
10. Add render smoke coverage:
    - `diffuse` still draws the current baseline
    - `architectural-v1` draws with an additional edge pass
    - picking returns the same ids under both profiles

## Open Decisions

- Whether edge extraction should remain renderer-side or move into `PreparedMesh` once the linework
  behavior settles.
- Whether `architectural-v1` selection should keep the current material override or use an
  edge/outline highlight.
- What crease threshold best fits IFC sample models without exposing too much tessellation noise.
- Whether terrain and faceted organic meshes need per-render-class edge suppression.
