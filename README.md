# ccw

`ccw` is a Rust rendering stack for web and native viewing, with a split between:

- a thin frontend/rendering client (`wgpu`, native viewer, web viewer, headless renderer)
- a backend-side geometry and IFC ingestion path

Today the repo includes:

- generic geometry/kernel/rendering crates
- native and web viewers
- headless PNG rendering for tests
- IFC import/query/render integration through Velr

## Prerequisites

- Rust `1.90+`
- `cargo`
- [`just`](https://github.com/casey/just)
- `git-lfs`
- `wasm-bindgen-cli`
- Node.js + npm

Recommended setup:

```bash
cargo install just wasm-bindgen-cli
rustup target add wasm32-unknown-unknown
git lfs install
```

## Clone And Checkout

Clone the repo and fetch LFS-backed fixtures:

```bash
git clone https://github.com/tomas-ks/ccw.git
cd ccw
git lfs pull
```

The largest IFC fixture is tracked through Git LFS, so `git-lfs` needs to be installed if you want the full local test corpus.

## Local Velr Checkouts

This workspace currently uses local path dependencies for Velr-related crates.

By default it expects sibling checkouts here:

```text
../velr-repo
../velr-graphql
../velr-ifc
```

If you keep those repos elsewhere, update the path dependencies in `Cargo.toml`.

## First Build

Quick sanity check:

```bash
cargo check --workspace
```

List the available helper commands:

```bash
just
```

## Run The Viewers

Native viewer:

```bash
just native-viewer resource="demo/pentagon"
```

Native IFC viewer:

```bash
just ifc-native-viewer model="building-architecture"
```

Web viewer:

```bash
just web-viewer
```

This builds the wasm target, starts the Rust static server, and prints the local URL.

Stop stray web viewer servers:

```bash
just web-viewer-stop
```

## Project-Local OpenCode Install

Install the official `opencode` CLI and expose it through a repo-local launcher:

```bash
just opencode-install
```

Verify the launcher and the repo-local writable state setup:

```bash
just opencode-check
```

Authenticate the repo-local OpenCode runtime:

```bash
just opencode-login
```

Run the web viewer with the `AI` terminal backed by the repo-local `ifc-explorer`
OpenCode agent and the allow-listed `ifc_*` tools:

```bash
just web-viewer-opencode
```

Start the repo-local OpenCode server outside the viewer:

```bash
just opencode-smoke
```

This gives the project a stable launcher at:

```text
.tools/opencode/bin/opencode
```

and keeps writable cache/config/data/state under `.tools/opencode/`.

The web viewer now talks to the repo-local OpenCode server directly from the Rust server.
The Rust adapter:

- keeps OpenCode state under `.tools/opencode/`
- uses the locked-down config in
  [tools/opencode/opencode.json](/Users/tomas/cartesian/codex/cc-renderer-w/tools/opencode/opencode.json)
- defaults to the repo-local `ifc-explorer` agent in
  [.opencode/agents/ifc-explorer.md](/Users/tomas/cartesian/codex/cc-renderer-w/.opencode/agents/ifc-explorer.md)
- exposes only the allow-listed `ifc_*` tools from
  [.opencode/tools/ifc.ts](/Users/tomas/cartesian/codex/cc-renderer-w/.opencode/tools/ifc.ts)
- creates a native OpenCode session per viewer AI session and streams progress from OpenCode's server events

The launcher defaults to `CC_W_OPENCODE_AGENT=ifc-explorer` for OpenAI-like
models and to `CC_W_OPENCODE_AGENT=ifc-playbook-cypher-only` for Gemma-like
models. Set `CC_W_OPENCODE_AGENT` before `just web-viewer-opencode` if you want
a different repo-local agent profile.

For a strict Gemma-friendly profile that uses only the canonical `ifc_*`
tool names, run:

```bash
just web-viewer-opencode-strict
```

That launcher defaults to `CC_W_OPENCODE_AGENT=ifc-explorer-strict` and
`CC_W_OPENCODE_MODEL=ollama/gemma4:e4b`, while still allowing overrides from
your shell.

For a one-line instruction-delivery smoke test that should reply only `42`,
run:

```bash
just web-viewer-opencode-42
```

That launcher defaults to `CC_W_OPENCODE_AGENT=ifc-answer-42` and
`CC_W_OPENCODE_MODEL=ollama/gemma4:e4b`.

For a one-tool smoke test that only allows `ifc_readonly_cypher`, run:

```bash
just web-viewer-opencode-cypher-only
```

That launcher defaults to `CC_W_OPENCODE_AGENT=ifc-readonly-cypher-only` and
`CC_W_OPENCODE_MODEL=ollama/gemma4:e4b`.

For the Gemma-friendly two-tool debug profile that allows `ifc_query_playbook`
and `ifc_readonly_cypher`, run:

```bash
just web-viewer-opencode-playbook-cypher
```

That launcher defaults to `CC_W_OPENCODE_AGENT=ifc-playbook-cypher-only` and
`CC_W_OPENCODE_MODEL=ollama/gemma4:e4b`.

By default, `just web-viewer-opencode` now pins the provider/model seed and lets
OpenCode/provider metadata describe the available models and real
provider-supported reasoning variants. The viewer only shows providers listed
in:

- [tools/opencode/provider-whitelist.json](/Users/tomas/cartesian/codex/cc-renderer-w/tools/opencode/provider-whitelist.json)

Add `cloudflare` to that JSON list later if you want Cloudflare models surfaced too.

To override the default model seed for one run:

```bash
CC_W_OPENCODE_MODEL=openai/gpt-5.4 just web-viewer-opencode
```

To use the Gemma-friendly default profile explicitly:

```bash
just web-viewer-opencode-playbook-cypher
```

You can still override `CC_W_OPENCODE_MODEL` in your shell before launching the
viewer. Set `CC_W_OPENCODE_VARIANT` only when you want to force a specific
provider-supported variant. If no explicit variant is set, the viewer starts on a
middle available level.

If you want to inspect the repo-local OpenCode server directly, use:

```bash
just opencode-acp
```

Headless PNG render:

```bash
just ifc-headless-render model="building-architecture" output="/tmp/ccw-ifc.png"
```

## Working With IFC Fixtures

Repo-local IFC mirrors live under `fixtures/ifc/`.

Refresh the local mirror from `velr-ifc/testdata`:

```bash
just ifc-sync-fixtures
```

Import one IFC fixture into Velr artifacts:

```bash
just ifc-import fixture="building-architecture"
```

Import the curated set:

```bash
just ifc-import-all
```

Show available curated fixtures:

```bash
just ifc-list-fixtures
```

## IFC Geometry And Runtime Maintenance

Rebuild prepared geometry from the existing Velr DB:

```bash
just ifc-rebuild-geometry model="building-architecture"
```

Clear only prepared geometry cache:

```bash
just ifc-clear-geometry-cache model="building-architecture"
```

Provision shared GraphQL runtime assets per IFC schema:

```bash
just ifc-refresh-runtime-schema schema="IFC2X3_TC1"
just ifc-refresh-runtime-schema schema="IFC4"
just ifc-refresh-runtime-schema schema="IFC4X3_ADD2"
```

Legacy per-model runtime folders can be removed with:

```bash
just ifc-clear-legacy-runtime-all
```

## Querying

Simple IFC summary:

```bash
just ifc-summary model="building-architecture"
```

Project query:

```bash
just ifc-projects model="building-architecture"
```

Raw Cypher:

```bash
just ifc-cypher model="building-architecture" query="MATCH (n:IfcProject) RETURN id(n) AS id ORDER BY id"
```

## Tests

Run the Velr/IFC unit tests:

```bash
cargo test -p cc-w-velr
```

Run the web server/viewer tests:

```bash
cargo test -p cc-w-platform-web --features native-server
```

Run the visual e2e artifact test:

```bash
just visual-e2e
```

Run the strict headless snapshot suite (exact PNG identity, including the smaller IFC fixtures):

```bash
just headless-snapshots
```

That command writes a static review page to:

```text
crates/cc-w-platform-headless/artifacts/snapshot-review/index.html
```

Each case in the report shows:

- the accepted baseline
- the current render
- the diff image when the render changed
- the exact `just` command to accept or invalidate that case

Accept a new snapshot baseline after an intentional rendering change:

```bash
just headless-accept-snapshot case="ifc-building-architecture"
```

Invalidate a snapshot baseline so the next strict run treats it as missing:

```bash
just headless-invalidate-snapshot case="ifc-building-architecture"
```

The snapshot suite compares rendered PNGs at exact pixel identity with zero tolerance. If a render changes, the strict run fails until the changed artifact is explicitly accepted one case at a time.

## Repo Layout

Main workspace crates:

- `cc-w-types`: shared geometry/render types
- `cc-w-kernel`: tessellation and primitive handling
- `cc-w-backend`: backend orchestration
- `cc-w-render`: `wgpu` renderer
- `cc-w-platform-native`: native viewer
- `cc-w-platform-web`: wasm viewer + Rust server
- `cc-w-platform-headless`: offscreen renderer / PNG output
- `cc-w-velr`: IFC import/query/geometry bridge

Useful directories:

- `crates/`
- `docs/`
- `fixtures/ifc/`

## Notes

- Generated render/import artifacts under `artifacts/` are ignored by Git.
- Web asset `node_modules/` is ignored by Git.
- Shared IFC GraphQL runtime assets now live under `artifacts/ifc/_graphql/<schema>/`.
- The normal rendering path is Cypher/DB-first; GraphQL is optional and schema-shared.
