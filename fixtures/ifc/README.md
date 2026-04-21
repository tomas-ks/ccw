# Curated IFC Fixtures

This directory is the repo-local mirror of the curated IFC fixtures we use for `w` Velr/IFC integration work.

Current curated set:

- `building-architecture.ifc`
- `building-hvac.ifc`
- `building-landscaping.ifc`
- `building-structural.ifc`
- `infra-bridge.ifc`
- `infra-landscaping.ifc`
- `infra-plumbing.ifc`
- `infra-rail.ifc`
- `infra-road.ifc`
- `openifcmodel-20210219-architecture.ifc`
- `fzk-haus.ifc`

Repository note:

- `openifcmodel-20210219-architecture.ifc` is tracked through Git LFS because it exceeds GitHub's normal 100 MB Git object limit
- use `just ifc-sync-fixtures` if you need to refresh the local mirror from `velr-ifc/testdata`

Provenance:

- source checkout: `/Users/tomas/velr/codex/velr-ifc`
- original source tree: `testdata/buildingSMART/IFC4X3_ADD2/PCERT-Sample-Scene`, `testdata/buildingSMART/IFC4X3_ADD2/openifcmodel`, and `testdata/openifcmodel/ifc4`

Sync path:

- Rust CLI: `cargo run -p cc-w-velr --bin cc-w-velr-tool -- sync-fixtures`
- Just wrapper: `just ifc-sync-fixtures`

Policy:

- keep provenance clear and slugs stable so imports and artifacts stay predictable
- treat these files as stable smoke/e2e assets for import, query, and render integration
- prefer the sync recipe over ad hoc copies so the repo mirror stays aligned with `velr-ifc/testdata`
