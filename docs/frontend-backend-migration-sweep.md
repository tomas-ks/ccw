# Frontend / Backend Migration Sweep

## Purpose

This note turns the frontend/backend split into a concrete implementation sweep for the current codebase.

Status as of `2026-04-19`:

- the runnable demo/runtime path now uses `backend -> PreparedGeometryPackage -> frontend`
- `cc-w-runtime` no longer owns built-in demo geometry or backend composition
- the remaining work is richer package contents, real scene packaging, and a remote web transport

The goal is to move from the current in-process demo composition:

`repository -> kernel -> prepare -> runtime -> render`

to the new architectural split:

- backend: `cc-w-db` + `cc-w-kernel` + `cc-w-prepare` + backend orchestration
- frontend: `cc-w-scene` + `cc-w-runtime` + `cc-w-render` + platform shells
- shared boundary: `cc-w-types`

The intended production path becomes:

`backend -> PreparedGeometryPackage -> frontend`

Native development may still compose both sides in one process, but only through that same boundary.

## Sweep Strategy

The migration should happen in one sweep with:

1. one short serial boundary-freeze step
2. parallel backend and frontend refactors with disjoint write scopes
3. local composition integration for native/headless
4. cleanup of the old direct path after the new one is runnable

## Phase 0: Boundary Freeze

Goal:

- freeze the first version of the package boundary before broader refactoring starts

Current boundary types:

- `PreparedGeometryDefinition`
- `PreparedGeometryInstance`
- `PreparedGeometryPackage`

Current assumptions for v1:

- prepared triangle meshes are the only production geometry payload
- instances are flat and refer to reusable definitions
- a single implicit root is enough for the current demo/runtime path
- web clients should consume packages rather than own tessellation

Done when:

- `cc-w-types` contains the first stable package boundary
- docs point to packages as the normal backend/frontend handoff

## Parallel Lanes

### Lane 1: Backend Orchestrator

Owns:

- `crates/cc-w-backend/**`

Responsibilities:

- compose `cc-w-db`, `cc-w-kernel`, and `cc-w-prepare`
- build `PreparedGeometryPackage` from the current demo resource flow
- preserve repeated-definition behavior
- provide one narrow public API for package building

Suggested public API:

```rust
pub struct GeometryBackend<R, K, P> { ... }

impl GeometryBackend {
    pub fn build_demo_package(&self) -> Result<PreparedGeometryPackage, GeometryBackendError>;
    pub fn build_demo_package_for(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryBackendError>;
}
```

Done when:

- backend code can emit a complete package for `demo/pentagon`
- backend code can emit a repeated-instance package for `demo/mapped-pentagon-pair`

### Lane 2: Frontend Runtime Refactor

Owns:

- `crates/cc-w-runtime/**`

Responsibilities:

- make runtime consume `PreparedGeometryPackage`
- introduce a frontend-facing package source trait
- remove direct runtime dependencies on backend crates
- preserve the existing `DemoAsset` / `DemoFrame` ergonomics where practical

Suggested trait shape:

```rust
pub trait GeometryPackageSource {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError>;
}
```

Done when:

- `cc-w-runtime` depends only on frontend-side crates plus `cc-w-types`
- runtime tests can build assets from a synthetic package source

### Lane 3: Native / Headless Local Composition

Owns:

- `crates/cc-w-platform-native/**`
- `crates/cc-w-platform-headless/**`

Responsibilities:

- compose backend and frontend locally through the package boundary
- keep current native/headless workflows fast
- preserve CLI and snapshot behavior

Done when:

- native still renders the demo asset
- headless still renders PNGs and passes snapshot tests
- neither platform crate needs direct repository/kernel/prepare composition

### Lane 4: Scene Packaging

Owns:

- follow-up changes to `cc-w-types`
- backend/runtime package touchpoints

Responsibilities:

- move enough scene projection data into backend-produced packages so frontend can stay thin
- start with flat instances and implicit root
- grow toward explicit projection payload later if needed

Done when:

- frontend builds the active scene from package contents, not backend-facing concepts

### Lane 5: Web Package Consumption

Owns:

- `crates/cc-w-platform-web/**`
- optional future wire/transport crate if needed

Responsibilities:

- make web consume prepared packages as its normal path
- keep runtime tessellation out of the browser production path
- establish the first transport-neutral fetch/deserialize seam

Done when:

- web code can consume a package-shaped payload without linking to backend crates

### Lane 6: Legacy Cleanup

Owns:

- cleanup across `cc-w-db`, `cc-w-kernel`, docs, and tests

Responsibilities:

- retire the runtime’s old direct repo/kernel/prepare path
- remove remaining helpers and demos that bypass the package boundary
- keep adapter-only scaffolding clearly outside the runnable frontend/backend path

Done when:

- there is one real runnable frontend/backend path
- legacy helpers are either deleted or explicitly isolated

## Critical Path

The shortest path to a real split is:

1. package boundary frozen
2. backend emits `PreparedGeometryPackage`
3. runtime consumes `PreparedGeometryPackage`
4. native/headless compose both sides locally

Everything else is important, but that is the minimum line that turns the architecture from aspirational into real.

## Recommended Execution Order

1. Phase 0 boundary freeze
2. run Lane 1 and Lane 2 in parallel
3. integrate Lane 3 on top of their new boundary
4. run Lane 4 and Lane 5 as the next widening pass
5. finish with Lane 6 cleanup

## File Ownership Guidance

To reduce merge collisions during the sweep:

- only one owner should touch `cc-w-types` boundary types during a given pass
- backend orchestrator work should stay inside `cc-w-backend`
- runtime refactor work should stay inside `cc-w-runtime`
- platform integration should stay inside platform crates
- cleanup should happen only after the new path works

## Definition of Done

The migration sweep is complete when:

- `cc-w-runtime` no longer depends on `cc-w-db`, `cc-w-kernel`, or `cc-w-prepare`
- backend code can emit `PreparedGeometryPackage`
- frontend code can render from packages alone
- native/headless still run by composing backend and frontend locally
- web production is designed around streamed prepared packages
- the old direct runtime path is removed or clearly isolated as legacy

## Immediate Next Steps

1. widen package contents for richer scene projection
2. replace the web stub source with a real fetch/deserialize transport
3. make definition-keyed render submission and caching real
4. continue removing placeholder scene/runtime wiring that still assumes one uploaded mesh path
