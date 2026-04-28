# Rendering Profiles

## Purpose

A rendering profile is a named visual contract for drawing the currently visible scene.

Profiles let us experiment with different presentation styles without changing model loading,
streaming, selection ownership, picking identity, or IFC semantics. The same project, runtime scene,
camera, visibility overrides, and resident geometry should be drawable through multiple profiles.

The stable user-facing profiles are:

- `diffuse`: the current baseline renderer
- `bim`: the default lightweight BIM renderer, based on the old `architectural-v2` screen-space
  outline profile
- `architectural`: the richer architectural renderer, based on the old `bim` inspection-capable
  profile

The older numbered styles are retained as experimental comparison profiles:

- `architectural-v1`: solid shaded geometry plus geometry-derived crease and boundary lines
- `architectural-v3`: solid shaded geometry plus screen-space outlines and selective crease lines
- `architectural-v4`: experimental `architectural-v3` plus normal-aware screen-space ambient
  occlusion

The old `architectural-v2` name remains accepted as a compatibility alias for `bim`. The old
`architectural-v3-inspection` and `architectural-v3-inspect` names remain accepted as compatibility
aliases for `architectural`.

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
- selection, inspection, and highlight style
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

1. Matte solid shaded geometry pass.
2. Geometry-derived edge pass for boundary edges.
3. Geometry-derived edge pass for crease edges.

This version intentionally does not include screen-space silhouettes yet. It is meant to establish
the profile architecture and a deterministic mesh-edge overlay before we add post-processing.

Boundary edges are triangle edges used by exactly one triangle.

Crease edges are triangle edges shared by two triangles whose adjacent face normals differ more than
the profile threshold. The initial threshold should be conservative, around 30 degrees, and kept as a
profile parameter so it can be tuned without changing the geometry payload contract.

### `bim`

The default lightweight BIM presentation profile. It started as `architectural-v2`, and that old
profile name remains accepted as a compatibility alias.

Pass stack:

1. Matte solid shaded geometry pass.
2. Visible object-id pass into an offscreen `Rgba8Uint` target.
3. Fullscreen screen-space outline pass that samples:
   - final depth, for visible silhouette and depth-discontinuity edges
   - visible object id, for semantic/object boundary edges

This version intentionally does not use mesh crease lines. It is meant to compare v1's
geometry-derived linework against a view-dependent architectural illustration style.

Object boundaries are not the same as pick identity. The first implementation uses per-instance
outline ids with semantic class suppression for terrain, water, vegetation cover, and surface decals
so terrain tiles, water surfaces, and road markings do not create noisy object seams. A later pass
should promote this to an explicit `outline_group_id` carried by the prepared instance payload.

Picking remains profile-independent. The profile's object-id target is presentation-only and does
not replace the ID-color pick pass.

This profile is the default for the web app and the renderer because it is lighter than the richer
architectural inspection profile and has proven good enough for bridge-scale work.

### `architectural-v3`

The first hybrid architectural experiment.

Pass stack:

1. Matte solid shaded geometry pass.
2. Selective geometry-derived crease/detail line pass.
3. Visible object-id pass into an offscreen `Rgba8Uint` target.
4. Fullscreen screen-space outline pass that samples final depth and visible object id.

This version keeps v2's clean view-dependent silhouettes and semantic/object boundaries while
bringing back v1's useful crease/detail lines. The crease pass intentionally ignores boundary edges
because object boundaries are already handled by the screen-space pass.

Crease/detail lines are controlled by semantic render class. For example, physical BIM objects and
terrain features can draw creases, while tree crowns, vegetation cover, water, and surface decals do
not expose tessellation or z-fighting noise as detail lines.

This profile should become the main comparison point for a more professional BIM/architecture
presentation style.

The architectural profiles intentionally use a softer matte surface shader than `diffuse`: the
directional light is still present, but dark-facing surfaces keep a stronger ambient fill so models
read more like architectural presentation geometry and less like game-lit assets.

### `architectural`

The richer architectural presentation profile. It started as `bim`, and the old
`architectural-v3-inspection` and `architectural-v3-inspect`
profile name remains accepted as a compatibility alias.

Inspection is runtime render state, similar to selection. It does not change geometry residency,
visibility overrides, picking ids, IFC ids, graph ids, or the active project. Runtime state marks
instances with a `PreparedRenderRole`:

- `Normal`: regular visible geometry
- `Selected`: selected regular geometry
- `Inspected`: the current inspection focus
- `InspectionContext`: visible geometry outside the current focus

Pass stack:

1. X-ray context pass for `InspectionContext` instances, drawn with alpha blending and no depth
   writes.
2. Matte solid shaded opaque pass for normal/focused geometry.
3. Surface decal pass for normal/focused decals.
4. Selective geometry-derived crease/detail line pass.
5. Visible object-id pass for normal/focused geometry.
6. Fullscreen screen-space outline pass that samples final depth and visible object id.

This profile is the everyday inspection-capable architectural mode for semantic workflows. It is not
the default because `bim` is lighter and keeps bridge navigation crisp.

### `architectural-v4`

An experimental ambient-occlusion variant, kept for comparison rather than as the preferred
architectural profile.

Pass stack:

1. Matte solid shaded geometry pass.
2. View-space normal pass into an offscreen `Rgba16Float` target.
3. Depth/normal screen-space ambient occlusion overlay.
4. Selective geometry-derived crease/detail line pass.
5. Visible object-id pass into an offscreen `Rgba8Uint` target.
6. Fullscreen screen-space outline pass that samples final depth and visible object id.

This version keeps v3's clean BIM linework but adds screen-space contact depth under overhangs,
bridge decks, roof/wall junctions, curbs, rails, and other tight geometry. In practice, this is a
tradeoff rather than a clear improvement: the AO can help locally, but it can also read as imprecise
on sparse IFC presentation geometry. `architectural-v3` remains the cleaner architectural baseline.

The AO pass uses the reverse-Z depth buffer plus view-space normals, so flat surfaces and shallow
depth variation are less likely to become dirty than they would be with a depth-only AO pass.
Samples whose depth jumps outside a small connected-surface window are ignored, so thin standalone
objects such as signs do not receive or cast broad fake occlusion from unrelated background pixels.

The AO is intentionally an overlay, not a replacement lighting model. It should add depth without
undoing the matte fill used by the architectural profiles.

## Future Profiles

Future hybrids may add:

- normal-buffer discontinuity edges
- explicit `outline_group_id` semantics instead of per-instance outline ids
- hidden-line or x-ray modes
- class-specific line palettes and weights

These should stay separate profiles until the visual contract settles.

## Enabling Profiles

The shared renderer should expose profile APIs:

```rust
renderer.profile();
renderer.available_profiles();
renderer.set_profile(RenderProfileId::ArchitecturalV4);
```

Changing a profile may rebuild pipelines and profile-specific GPU buffers, but it should not reload
the active project, reset visibility, clear selection, or change the runtime scene.

The web shell should expose the same capability through the viewer bridge:

```js
viewer.profile()
viewer.profiles()
viewer.setProfile("bim")
```

The native shell should expose a profile selector because native is the rendering debug interface.
The web shell exposes the stable selector (`diffuse`, `bim`, `architectural`) by default. The JS API
may still accept experimental profile ids for comparison work.

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
11. Implement `bim` as solid shaded triangles plus screen-space outline passes:
    - keep the main depth target sampleable
    - draw visible outline ids into an offscreen `Rgba8Uint` target
    - composite depth and object-id discontinuities in a fullscreen pass
    - suppress noisy object-id outlines for semantic classes such as terrain, water, vegetation
      cover, and surface decals
12. Add render smoke coverage for `bim` so the fullscreen pass is validated by an
    actual GPU render call.
13. Implement `architectural-v3` as the hybrid profile:
    - reuse the v2 screen-space outline stack
    - add a crease-only mesh edge pass before the fullscreen outline composite
    - keep boundary edges out of the v3 mesh pass to avoid double-drawing object boundaries
14. Add render smoke coverage for `architectural-v3`.
15. Implement `architectural` as the inspection-capable architectural presentation profile:
    - add `PreparedRenderRole` to prepared render instances
    - let runtime inspection focus mark `Inspected` and `InspectionContext`
    - draw context geometry through a translucent no-depth-write pass only in the architectural
      profile
    - keep picking profile-independent and ID-correct
16. Add runtime and renderer smoke coverage for `architectural`.
17. Implement `architectural-v4` as the normal-aware SSAO profile:
    - render visible opaque geometry into a view-space normal target
    - sample normal plus reverse-Z depth in a fullscreen AO overlay
    - draw v3 crease/detail lines and screen-space outlines after AO
18. Add render smoke coverage for `architectural-v4`.

## Open Decisions

- Whether edge extraction should remain renderer-side or move into `PreparedMesh` once the linework
  behavior settles.
- Whether `architectural-v1` selection should keep the current material override or use an
  edge/outline highlight.
- What crease threshold best fits IFC sample models without exposing too much tessellation noise.
- Whether `architectural-v3` should keep its crease-only mesh pass or eventually use a normal buffer
  for detail edges.
- Whether `bim` should keep a single x-ray context pass or evolve into a separate
  front/back/depth-peeled style.
- Which semantic classes should receive explicit `outline_group_id` values once terrain tile seams
  and aggregate objects need finer control than per-instance outline suppression.
