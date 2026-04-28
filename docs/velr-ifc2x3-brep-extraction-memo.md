# IFC2X3 BRep Extraction Benchmark Memo

## Context

`openifcmodel-20210219-architecture` rendered as a badly incomplete blue slice in the viewer.
The source database is not empty or simple; the current render package was incomplete.

Current cached render package before the IFC2X3 BRep prototype:

- 733 definitions
- 730 elements
- 733 instances
- 5,882 triangles
- 0 colored instances

Database shape:

- ~1.97M nodes
- ~3.49M edges
- 19,910 `IfcShapeRepresentation`
- 15,957 `IfcProductDefinitionShape`
- 4,012 `IfcFacetedBrep`
- ~340k `IfcFace`
- many product entities not represented in the old render package, including doors, windows, columns, plates, members, furnishings, walls, slabs, and roofs.

## BRep Storage Shape

For this IFC2X3 model, the BRep geometry path is:

```text
IfcFacetedBrep
  - OUTER -> IfcClosedShell
  - CFS_FACES -> IfcFace
  - BOUNDS -> IfcFaceOuterBound
  - BOUND -> IfcPolyLoop
  - POLYGON -> IfcCartesianPoint
```

The key label/type ids observed in the SQLite-backed Velr store:

- `IfcFacetedBrep`: label type id `1414240`
- `IfcClosedShell`: label type id `1414200`
- `IfcFace`: label type id `1414236`
- `IfcFaceOuterBound`: label type id `1414239`
- `IfcPolyLoop`: label type id `1414292`
- `IfcCartesianPoint`: label type id `1`
- `OUTER`: edge type id `34`
- `CFS_FACES`: edge type id `9`
- `BOUNDS`: edge type id `7`
- `BOUND`: edge type id `6`
- `POLYGON`: edge type id `43`
- `Coordinates`: key id `5`

## Benchmark Findings

The important discovery is that query shape matters enormously.

A naive one-solid query using one large joined pattern took about `3.77s` just to count `40` point rows.
The SQLite query plan chose a bad middle-first join.

The same path with explicit left-to-right traversal was effectively instant in SQLite, and fast enough through Cypher when expressed with `WITH` barriers.

Measured stepwise Cypher:

- `10` BReps: `4,656` point rows in about `0.13s`
- `100` BReps: `21,290` point rows in about `0.11s`
- all BReps: `1,121,024` point rows counted in about `4.37s`
- all BRep rows serialized to TSV: `80MB` in about `8.96s`

This strongly suggests the raw topology size is not inherently fatal. The bad query plan was fatal.

## Current Prototype Path

The current prototype in `crates/cc-w-velr/src/lib.rs` adds an IFC2X3 `IfcFacetedBrep` path beside the existing Body extraction paths.

The high-level body package path is:

1. Resolve `IfcLocalPlacement` transforms.
2. Query existing `IfcTriangulatedFaceSet` body records.
3. Query existing narrow `IfcExtrudedAreaSolid` body records.
4. Query prototype `IfcFacetedBrep` body records.
5. Convert all `IfcBodyRecord` values into `ImportedGeometrySceneResource`.
6. Pass that scene into `GeometryBackend::build_imported_scene_package`.
7. Cache the resulting `PreparedGeometryPackage`.

The prototype BRep extraction currently does two major reads:

1. Geometry read:
   - walks `IfcFacetedBrep -> ... -> IfcCartesianPoint`
   - returns `item_id`, `face_id`, edge ordinals, point ordinals, and `pt.Coordinates`
   - uses stepwise `MATCH ... WITH ... MATCH` to avoid the bad planner shape

2. Metadata read:
   - maps each `IfcFacetedBrep` item back to its product, placement, name, entity, type semantics, classification, and optional style color

The Rust-side geometry assembly then:

1. Converts each Velr cell to a string with `render_cell`.
2. Parses ids and ordinals from strings.
3. Parses `pt.Coordinates` from JSON text into `DVec3`.
4. Groups rows as `item_id -> face_id -> points`.
5. Sorts face points by `POLYGON.ordinal`.
6. Removes duplicate closing points.
7. Creates one `IndexedPolygon` per `IfcFace`.
8. Builds `TessellatedGeometry` per BRep item.
9. Wraps it in `GeometryPrimitive::Tessellated`.

## Diagnostic Slice Results

The next diagnostic slice added an uncached `body-summary --diagnostic` mode with optional `--limit-brep-items`.

Before the BRep benchmark could run, two more query-shape blockers showed up:

- `IfcLocalPlacement` extraction hung with one broad optional query. Splitting it into small relationship queries brought placement resolution to roughly `0.3-0.6s` in the full diagnostic.
- `IfcExtrudedAreaSolid` extraction had the same issue around inline position traversal. Keeping the solid-position id in the main query and resolving `Axis2Placement3D` vectors separately brought the phase down to roughly `2.0s`.
- The BRep geometry query must avoid labels on every intermediate hop after `solid` is bound. The stepwise untyped-intermediate shape stayed fast; the typed-intermediate shape regressed badly.

Release-mode diagnostic timings on `openifcmodel-20210219-architecture`:

| Limit | Total | BRep point rows | BRep geometry query/parse/group | BRep geometry build | BRep metadata query | Backend prepare |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | `3.20s` | 4,656 | `40ms` | `0ms` | `140ms` | `2ms` |
| 100 | `2.89s` | 21,290 | `152ms` | `1ms` | `310ms` | `3ms` |
| 1000 | `8.76s` | 564,102 | `3.882s` | `36ms` | `2.391s` | `9ms` |
| all | `21.47s` | 1,121,024 | `7.424s` | `76ms` | `11.247s` | `52ms` |

Full uncached diagnostic output:

```text
definitions: 4019
elements: 1118
instances: 5667
triangles: 236334
brep_geometry_items: 4012
brep_geometry_faces: 310573
brep_geometry_point_rows: 1121024
brep_metadata_rows: 4934
phase.placement_transforms.ms: 286
phase.triangulated_body_records.ms: 120
phase.extruded_body_records.ms: 2151
phase.brep_geometry_query_parse_group.ms: 7424
phase.brep_geometry_build.ms: 76
phase.brep_metadata_query.ms: 11247
phase.brep_metadata_parse_records.ms: 12
phase.imported_scene_assembly.ms: 61
phase.backend_prepare_package.ms: 52
```

This removes backend preparation, scene assembly, and BRep face assembly as primary bottlenecks. The remaining cost is dominated by Velr query execution / row materialization / per-cell string rendering / JSON coordinate parsing, plus BRep metadata query shape.

## Cache Write / Read Diagnostic

The diagnostic mode also supports writing the complete diagnostic package to the normal prepared
geometry cache:

```bash
target/release/cc-w-velr-tool body-summary \
  --model openifcmodel-20210219-architecture \
  --diagnostic \
  --write-cache
```

Partial cache writes are intentionally rejected. `--write-cache` cannot be combined with
`--limit-brep-items`, because a partial BRep diagnostic package is not a valid app cache.

An initial cache-write/read diagnostic looked slow because cache validation called
`authoritative_schema()`, which read and parsed the `1.2GB` `import-bundle.json` before checking the
tiny import log or source header. After changing schema detection to prefer cheap metadata and only
prefix-read the import bundle fallback, the cache path is no longer a meaningful bottleneck.

Release-mode full diagnostic with cache write after that fix:

```text
geometry_cache_status: diagnostic_uncached
brep_limit_items: all
definitions: 4019
elements: 1118
instances: 5667
triangles: 236334
brep_geometry_items: 4012
brep_geometry_faces: 310573
brep_geometry_point_rows: 1121024
brep_metadata_rows: 4934
cache_written: true
phase.placement_transforms.ms: 436
phase.triangulated_body_records.ms: 158
phase.extruded_body_records.ms: 2034
phase.brep_geometry_query_parse_group.ms: 7478
phase.brep_geometry_build.ms: 73
phase.brep_metadata_query.ms: 11400
phase.brep_metadata_parse_records.ms: 11
phase.imported_scene_assembly.ms: 63
phase.backend_prepare_package.ms: 52
phase.cache_write.ms: 343
real 22.11
```

The prepared JSON package written by this run was `187MB`.

The following normal production path hit the cache:

```bash
target/release/cc-w-velr-tool body-summary --model openifcmodel-20210219-architecture
```

Observed:

```text
geometry_cache_status: cache_hit
definitions: 4019
elements: 1118
instances: 5667
triangles: 236334
real 0.54
```

Dedicated cache diagnostics showed the corrected cache-hit path is small:

```text
phase.cache_read_text.ms: 64
phase.cache_json_parse.ms: 202
phase.cache_validate.ms: 0
phase.cache_into_prepared_package.ms: 1
real 0.59
```

So the cache works and is not currently the scaling blocker. The next optimization question returns
to the measured extraction hot phases: BRep metadata query, BRep geometry query/materialization, and
the fixed extruded-solid baseline.

## Runtime Render Sanity Finding: Source Units Matter

The first viewer test of the cached IFC2X3 package still looked structurally wrong. One concrete
issue was unit handling: `cc-w` hard-coded IFC body extraction as millimeters, but
`openifcmodel-20210219-architecture` declares a conversion-based length unit of `FOOT`.

That meant source coordinates were normalized with `0.001` meters per unit instead of `0.3048`
meters per unit, a `304.8x` scale error. The resulting prepared package had tiny bounds such as
wall centers around `(-0.039, 0.200, 0.028)`.

After detecting the source IFC length unit and rebuilding the cache, the same sample instances are
meter-scaled, for example wall centers around `(-11.781, 60.906, 8.586)` with sizes around
`(1.079, 2.020, 3.554)`.

This fixes a real transform/scale bug, but it does not prove the whole BRep render is correct yet.
Remaining structural suspects include mapped-item transforms and exact face-bound handling.

## Runtime Render Sanity Finding: Mapped Items Are Required

The next viewer test still looked incomplete and structurally wrong, with disconnected-looking stair
geometry and a sparse building shell. The important discovery is that the model relies heavily on
`IfcMappedItem` reuse:

```text
IfcMappedItem count: 12,673
mapped source geometry:
  IfcExtrudedAreaSolid      18,238 rows
  IfcFacetedBrep             5,799 rows
  IfcFaceBasedSurfaceModel     501 rows
  IfcBooleanClippingResult     126 rows
  IfcBooleanResult              21 rows
```

The previous IFC2X3 body package only included direct product body items. It did not expand mapped
source representations into product occurrences, so the viewer was rendering a subset rather than
the model. For this file, all observed `IfcMappedItem` targets shared a single identity
`IfcCartesianTransformationOperator3D`, but the extraction path now still treats the mapped item as
an occurrence transform:

```text
instance transform = product local placement * mapped item target * inverse(mapping origin) * item position
definition id      = mapped source geometry item id
occurrence id      = IfcMappedItem db node id
```

After adding mapped extrusion and mapped BRep occurrences, the rebuilt prepared package changed from
the direct-only cache:

```text
definitions: 4043
elements: 1131
instances: 5691
triangles: 248934
```

to:

```text
definitions: 5169
elements: 4503
instances: 18072
triangles: 528422
```

Release-mode full diagnostic with mapped occurrences:

```text
geometry_cache_status: diagnostic_uncached
brep_limit_items: all
definitions: 5169
elements: 4503
instances: 18072
triangles: 528422
brep_geometry_items: 4012
brep_geometry_faces: 310573
brep_geometry_point_rows: 1121024
brep_metadata_rows: 10733
cache_written: true
phase.placement_transforms.ms: 286
phase.triangulated_body_records.ms: 119
phase.extruded_body_records.ms: 1975
phase.mapped_extruded_body_records.ms: 57322
phase.brep_geometry_query_parse_group.ms: 7427
phase.brep_geometry_build.ms: 85
phase.brep_metadata_query.ms: 11127
phase.brep_metadata_parse_records.ms: 11
phase.mapped_brep_metadata_query.ms: 13711
phase.mapped_brep_metadata_parse_records.ms: 98
phase.imported_scene_assembly.ms: 300
phase.backend_prepare_package.ms: 103
phase.cache_write.ms: 844
real 100.08
```

The mapped path improved semantic completeness but exposed a new query-shape hotspot:
`mapped_extruded_body_records` is about `57s`, mostly because product occurrence metadata and mapped
source geometry are still resolved through broad Cypher joins. The important correctness lesson is
that mapped geometry must be part of the export; the important performance lesson is that mapped
definition extraction and mapped occurrence extraction need the same stepwise, definition/instance
split that made BRep viable.

The new cache is larger but still usable:

```text
cache_bytes: 440345158
phase.cache_read_text.ms: 94
phase.cache_json_parse.ms: 440
phase.cache_validate.ms: 0
phase.cache_into_prepared_package.ms: 6
```

## What To Lift With Velr

The most actionable Velr-level observation is query planning:

- A chained graph traversal can become very slow if the planner starts in the middle of the path.
- Explicit `WITH` barriers force left-to-right traversal and make the same logical query much faster.
- The planner may need better cost estimation for bound variables and selective edge traversals.
- It would be useful to expose or document a recommended Cypher shape for long, typed, anchored IFC traversals.

Potential Velr improvements to discuss:

- Better join ordering for typed edge chains where the left side is already bound.
- An `EXPLAIN`/diagnostic story at the Cypher level that exposes when a pattern is planned middle-first.
- A lower-overhead row API for bulk numeric/JSON-like geometry extraction, avoiding string rendering for every cell.
- Optional typed traversal helpers for importer/exporter workloads, separate from semantic exploration Cypher.

This does not mean geometry extraction must leave Cypher immediately. The benchmark says the first thing to do is respect query shape and add phase timing.

## Implemented Benchmark Slice

The diagnostic body extraction mode supports BRep subsets:

- `--limit-brep-items 10`
- `--limit-brep-items 100`
- `--limit-brep-items 1000`
- all

It reports timings around:

- placement transform resolution
- triangulated body query
- extruded body query
- BRep geometry query
- BRep row parse and grouping
- BRep metadata query
- imported scene assembly
- backend preparation
- cache write or cache skip

Next optimization candidates should be chosen from the measured hot phases, not guessed:

1. BRep metadata query: currently the largest phase at full scale.
2. BRep geometry query/parse/group: includes Velr row materialization, `render_cell`, JSON coordinate parsing, and HashMap grouping.
3. Extruded body records: still a fixed ~2s baseline on this model after query-shape cleanup.
4. Cache validation: fixed by avoiding full `import-bundle.json` parsing for schema detection. Cache
   hit is now roughly `0.5s`, so it is not the next bottleneck.
