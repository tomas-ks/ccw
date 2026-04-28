# Velr IFC Metadata And Graph Edge Contract Memo

Date: 2026-04-27

## Summary

`cc-w` should be able to build render/cache packages from the imported Velr database plus small
authoritative import metadata. It should not need to read `source.ifc` during geometry extraction,
style/color extraction, or normal viewer startup.

The current `cc-w` fallback path still contains older source-prefix reads for metadata:

```text
schema lookup:
  import-log.txt
  source.ifc prefix
  import-bundle.json prefix
```

That source-prefix fallback exists only because schema/unit metadata is not yet available through a
small stable contract. We want to remove that fallback once Velr IFC exposes the required metadata
and graph facts directly.

Separately, IFC style/color extraction must be database-only. We briefly tested a source IFC scan for
surface colors, then rolled it back. If a style path is missing in the DB, `cc-w` should report no DB
color rather than recovering it from `source.ifc`.

## Design Principle

The IFC source file belongs to import and debugging. After import, downstream tools should treat the
Velr DB and a compact import manifest as the source of truth.

Normal downstream consumers should not parse:

- `source.ifc`
- full `import-bundle.json`
- full `import.cypher`

Those artifacts can remain valuable debug/provenance outputs, but they should not be required for
runtime extraction.

## Metadata Ask

Velr IFC should write a small authoritative import metadata artifact or equivalent DB metadata.

Suggested file:

```text
import/import-manifest.json
```

Suggested fields:

```json
{
  "schema": "IFC2X3",
  "source_sha256": "...",
  "source_file": "...",
  "importer": {
    "tool": "ifc-schema-tool",
    "version": "...",
    "runtime_bundle": "..."
  },
  "units": {
    "length": {
      "name": "FOOT",
      "kind": "conversion_based_unit",
      "scale_to_metre": 0.3048
    }
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

Exact names can change. The important contract is:

- schema must be cheap and authoritative
- length unit / source coordinate scale must be cheap and authoritative
- runtime/schema mapping used by the importer must be recorded
- source identity/hash must be recorded
- full debug payloads must not be required to answer these metadata questions

## Unit Metadata / Graph Ask

Length unit detection should not require scanning `source.ifc`.

Either the import manifest should expose the resolved length unit and scale, or the imported DB
should preserve/query the IFC unit graph clearly enough for downstream tools to resolve it.

Relevant IFC shapes include:

```text
IfcProject
  -> IfcUnitAssignment
  -> IfcSIUnit

IfcProject
  -> IfcUnitAssignment
  -> IfcConversionBasedUnit
  -> IfcMeasureWithUnit
  -> IfcSIUnit
```

The DB should preserve role/attribute names for these paths, for example `Units`,
`ConversionFactor`, `ValueComponent`, and `UnitComponent`, plus scalar fields such as unit type,
prefix, name, and conversion value.

## Style / Color Edge Ask

For render colors, `cc-w` needs the IFC presentation style graph preserved in the DB.

The important IFC2X3 path is:

```text
IfcStyledItem
  -> IfcPresentationStyleAssignment
  -> IfcSurfaceStyle
  -> IfcSurfaceStyleRendering / IfcSurfaceStyleShading
  -> IfcColourRgb
```

Concrete asks:

- Preserve the edge from `IfcPresentationStyleAssignment` to `IfcSurfaceStyle`.
- Preserve style role/attribute names, especially `Styles`, `Styles[0]`, `SurfaceColour`, and any
  rendered/shading style roles.
- Preserve `IfcColourRgb.Red`, `IfcColourRgb.Green`, and `IfcColourRgb.Blue` as queryable scalar
  properties.
- Preserve the intermediate style nodes rather than flattening or dropping them.
- Make this path queryable using normal Velr/Cypher traversal.

Without the `IfcPresentationStyleAssignment -> IfcSurfaceStyle` edge, a DB-only renderer cannot
recover surface colors for IFC2X3 models. In that case, returning zero colored instances is the
honest outcome.

## Source Identity / Debug Ask

Downstream tools need enough source identity in the DB to debug graph issues without opening
`source.ifc`.

Recommended:

- preserve STEP entity id / sid on imported nodes when available
- preserve declared IFC entity name
- preserve original property names / role names on generated edges
- expose a tiny import manifest with source hash and importer version

This makes it possible to compare DB facts with importer output when investigating missing edges,
while keeping normal extraction source-free.

## Acceptance Criteria

`cc-w` should be able to:

1. determine schema without reading `source.ifc` or the full import bundle
2. determine length unit / source coordinate scale without reading `source.ifc`
3. extract surface colors using only Velr DB queries
4. explain missing colors as missing DB graph facts, not silently recover them from source text
5. run render cache generation with `source.ifc` absent, as long as the DB and import manifest exist

## Proposed `cc-w` Cleanup After Velr IFC Support

Once Velr IFC provides the above contract, `cc-w` can remove:

- `schema_from_source_ifc_if_exists`
- `ifc_length_unit_from_source_ifc_if_exists`
- source-file fallbacks in render/cache extraction

At that point, `source.ifc` can be treated as an import/debug artifact only.

