# Frontend / Backend Split for `w`

## Purpose

This note makes one architectural choice explicit:

`w` is not a single blended runtime. It is a thin rendering frontend over a geometry-processing backend.

This matters most for web deployment, where the split cannot stay fuzzy without pushing heavy geometry work, large native dependencies, and backend data access into the browser.

## The Split

### Backend

The backend is the geometry engine.

Responsibilities:

- connect to Velr / Velr-IFC and other external geometry sources
- resolve definitions, instances, placements, and source metadata
- normalize basis and units into `w` world space
- perform geometry processing
  - healing
  - booleans
  - tessellation / triangulation
  - future LOD generation
- produce transport-neutral prepared geometry packages

Primary crates:

- `cc-w-db`
- `cc-w-kernel`
- `cc-w-prepare`
- `cc-w-backend`

### Frontend

The frontend is the rendering client.

Responsibilities:

- request and stream prepared geometry packages
- maintain the active runtime scene projection
- manage visibility, residency, and culling
- upload prepared assets to the GPU
- render with `wgpu`
- support interaction, selection, and highlighting against streamed data

The frontend has two state layers that must not be blurred:

- app state: user/product intent owned by the platform shell
- renderer state: committed scene truth owned by `cc-w-runtime`

The detailed state ownership and event contract is documented in
[state-management.md](./state-management.md).

Primary crates:

- `cc-w-scene`
- `cc-w-runtime`
- `cc-w-render`
- `cc-w-platform-web`
- `cc-w-platform-native`
- `cc-w-platform-headless`

### Shared Boundary

The frontend/backend boundary lives in `cc-w-types`.

In code, the intended shape is:

`GeometryBackend -> GeometryPackageSource -> Engine`

The key payload at this boundary is `PreparedGeometryPackage`, which groups:

- prepared reusable definitions
- prepared instances
- world-space bounds and transforms
- stable external references for scene/runtime bookkeeping

## Deployment Model

### Web production

This is the default deployment shape:

`Velr / adapters -> backend geometry engine -> prepared geometry package -> web frontend`

Rules:

- the web frontend should not depend on runtime tessellation in the normal path
- the web frontend should not connect directly to a native Velr runtime
- heavy kernels such as OCCT should stay backend-side

### Native development

Native builds may compose frontend and backend in one process for faster iteration.

That is a deployment convenience only. The code should still preserve the same ownership split and package boundary used by web production.

## Why This Split Is Worth Locking In Now

- it keeps the web client small and focused on rendering
- it makes direct Velr integration much simpler on the backend side
- it keeps heavyweight kernels and exact-geometry tooling out of browser deployment
- it gives one clean place to cache tessellation and prepared assets
- it makes streaming a first-class model instead of a later retrofit

For the concrete Velr / IFC wiring plan on top of this split, see `docs/velr-ifc-integration.md`.

## Allowed Exceptions

The frontend may still keep a tiny local geometry path for:

- tests
- examples
- procedural debug geometry
- explicit fallback workflows

But that path should stay secondary and should not define the production architecture.

## Immediate Consequences

1. treat `cc-w-db`, `cc-w-kernel`, and `cc-w-prepare` as backend-side crates
2. treat `cc-w-scene`, `cc-w-runtime`, `cc-w-render`, and platform crates as frontend-side crates
3. keep transport-neutral package types in `cc-w-types`
4. design web flows around streaming prepared packages, not browser-side tessellation
5. allow native to compose both sides locally without collapsing the boundary in the codebase
