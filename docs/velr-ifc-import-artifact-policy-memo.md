# Velr IFC Import Artifact Policy Memo

Date: 2026-04-27

## Summary

While benchmarking `openifcmodel-20210219-architecture`, `cc-w` found that the Velr IFC importer
writes very large debug/provenance artifacts by default:

```text
artifacts/ifc/openifcmodel-20210219-architecture/import/import-bundle.json  1.2GB
artifacts/ifc/openifcmodel-20210219-architecture/import/import.cypher       637MB
artifacts/ifc/openifcmodel-20210219-architecture/import/import-log.txt        3KB
artifacts/ifc/openifcmodel-20210219-architecture/import/import-timing.json    4KB
```

Those artifacts are useful for import debugging, but they should not be part of normal runtime
metadata lookup or viewer startup.

`cc-w` accidentally used `import-bundle.json` as a schema fallback in the hot path. On this model,
that meant reading/parsing a `1.2GB` JSON file just to discover `"schema": "IFC2X3"`. The resulting
cache-hit path took roughly `10s` before we fixed schema lookup to prefer tiny metadata files.

After avoiding full bundle parsing:

```text
cache hit body-summary: ~0.54s
cache diagnostic:
  cache_read_text:             64ms for 187MB prepared geometry cache
  cache_json_parse:           202ms
  cache_validate:               0ms
  cache_into_prepared_package:  1ms
```

The large import bundle was not needed for that runtime path.

## Why The Bundle Exists

The full import bundle is still valuable as a debugging and reproducibility artifact. It can help
answer questions such as:

- what graph data did the IFC importer generate before DB import?
- did the generated bundle include a relation or property that the live DB is missing?
- how did the generated graph compare with `import.cypher` and the imported Velr DB?
- can a Velr IFC issue be reproduced without re-parsing the source STEP file?

That makes it a good debug artifact. It does not make it a good runtime artifact.

## Observed Cost

For `openifcmodel-20210219-architecture`, import timing reported:

```text
bundle_build_ms:          6924
debug_cypher_render_ms:   8233
debug_artifact_write_ms: 830754
velr_import_ms:         621099
total_ms:              1477858
```

The debug artifact write alone was larger than the actual Velr DB import time.

That suggests the default importer path is paying a very large cost for artifacts that are mostly
useful after something goes wrong.

## Problem

The current artifact layout blurs two separate concerns:

1. **Runtime metadata**
   - schema
   - source file path/hash
   - importer/runtime version
   - import timing
   - imported node/edge counts
   - warnings/issues summary

2. **Debug/provenance payloads**
   - full generated import bundle
   - full rendered import Cypher
   - verbose per-node/per-edge import handoff data

Runtime metadata should be tiny, cheap, and always safe to read. Debug payloads can be huge and
should be explicit.

## Recommendation

Velr IFC should split import output into a small always-written manifest and optional debug
artifacts.

Suggested layout:

```text
generated/step-import/<model>/
  import-manifest.json
  import-timing.json
  issues.json
  debug/
    import-bundle.json
    import.cypher
```

`import-manifest.json` should be the runtime/provenance contract. Suggested fields:

```json
{
  "schema": "IFC2X3",
  "source_sha256": "...",
  "source_file": "...",
  "projection": "opencypher-property-graph",
  "subset": "full-ifc2x3-runtime-v1",
  "importer": {
    "tool": "ifc-schema-tool",
    "version": "...",
    "runtime_bundle": "..."
  },
  "counts": {
    "nodes": 1970000,
    "edges": 3490000
  },
  "debug_artifacts": {
    "bundle": "debug/import-bundle.json",
    "cypher": "debug/import.cypher"
  }
}
```

The exact shape can change, but the key point is that schema/source/count metadata should not require
opening the full bundle.

## Proposed Importer Flags

Recommended default:

```text
write manifest: yes
write timing/issues/log: yes
write full import bundle: no
write rendered Cypher: no
```

Suggested flags:

```text
--debug-artifacts                 write debug/import-bundle.json and debug/import.cypher
--debug-import-bundle             write only debug/import-bundle.json
--debug-import-cypher             write only debug/import.cypher
--debug-artifact-dir <path>       override debug output location
--no-debug-artifacts              explicit default
```

This lets the normal import path stay fast while preserving a way to produce complete forensic
artifacts when investigating importer or DB mismatches.

## `cc-w` Side Adjustment

`cc-w` should also treat the full bundle as optional:

- runtime schema lookup should use `import-log.txt`, `source.ifc` header, or a future
  `import-manifest.json`
- `import-bundle.json` should only be a fallback and should only be prefix-read
- artifact copy should eventually copy debug outputs only when requested

The immediate `cc-w` fix already changed schema lookup to avoid full bundle parsing. The cleaner
long-term fix is for Velr IFC to provide a small manifest as the authoritative runtime metadata
source.

