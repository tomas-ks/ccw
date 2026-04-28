# Velr IFC2X3 BRep Query Planner Memo

Date: 2026-04-27

## Summary

While investigating incomplete rendering for the IFC2X3 model
`openifcmodel-20210219-architecture`, we found a large query-performance swing caused by Cypher
query shape / join ordering.

The underlying model data is present in the Velr database. The problematic part is traversing the
stored BRep topology efficiently enough for geometry extraction diagnostics.

The key finding:

- a naive, single-pattern BRep traversal can take seconds for one tiny solid
- the same logical traversal expressed as explicit left-to-right steps with `WITH` barriers is fast
- raw SQLite confirms that forced left-to-right traversal over the same indexed edge tables is also
  fast

This looks like a planner/join-order issue for typed path traversals where the left side is already
selective or bound.

## Repo / Environment

Repo:

```text
/Users/tomas/cartesian/codex/cc-renderer-w
```

Model:

```text
artifacts/ifc/openifcmodel-20210219-architecture/model.velr.db
```

CLI used:

```bash
target/release/cc-w-velr-tool cypher --model openifcmodel-20210219-architecture --query '...'
```

Database shape:

- ~1.97M nodes
- ~3.49M edges
- 4,012 `IfcFacetedBrep`
- ~340k `IfcFace`
- 19,910 `IfcShapeRepresentation`
- 15,957 `IfcProductDefinitionShape`

## IFC2X3 BRep Storage Shape

The BRep geometry path in this imported model is:

```text
IfcFacetedBrep
  - OUTER -> IfcClosedShell
  - CFS_FACES -> IfcFace
  - BOUNDS -> IfcFaceOuterBound
  - BOUND -> IfcPolyLoop
  - POLYGON -> IfcCartesianPoint
```

Observed Velr SQLite ids:

```text
label_type:
  IfcFacetedBrep      1414240
  IfcClosedShell      1414200
  IfcFace             1414236
  IfcFaceOuterBound   1414239
  IfcPolyLoop         1414292
  IfcCartesianPoint   1

edge_type:
  OUTER               34
  CFS_FACES           9
  BOUNDS              7
  BOUND               6
  POLYGON             43
```

## Slow Shape

This direct path pattern was slow even when anchored to one known BRep node:

```cypher
MATCH (solid:IfcFacetedBrep)-[:OUTER]->(shell:IfcClosedShell)-[face_edge:CFS_FACES]->(face:IfcFace)-[:BOUNDS]->(:IfcFaceOuterBound)-[:BOUND]->(loop:IfcPolyLoop)-[point_edge:POLYGON]->(pt:IfcCartesianPoint)
WHERE id(solid) = 1114113
RETURN count(pt) AS point_rows
```

Observed:

```text
point_rows = 40
time ~= 3.77s
```

SQLite `EXPLAIN QUERY PLAN` for the analogous join showed the planner starting in the middle of the
chain rather than walking outward from the bound `solid`:

```text
SEARCH e1 USING COVERING INDEX idx_edge_type_from_to_id (type_id=? AND from_node=?)
SEARCH e3 USING COVERING INDEX idx_edge_type_to_from_id (type_id=?)
SEARCH e2 USING COVERING INDEX idx_edge_type_to_from_id (type_id=? AND to_node=? AND from_node=?)
SEARCH e5 USING COVERING INDEX idx_edge_type_to_from_id (type_id=?)
SEARCH e4 USING COVERING INDEX idx_edge_type_to_from_id (type_id=? AND to_node=? AND from_node=?)
```

The problematic part is that `e3` and `e5` are searched primarily by edge type across a large set,
even though the traversal is logically anchored by `solid`.

## Fast Shape

The same logical traversal expressed with explicit `WITH` barriers is much faster:

```cypher
MATCH (solid)
WHERE id(solid) = 1114113
MATCH (solid)-[:OUTER]->(shell)
WITH solid, shell
MATCH (shell)-[face_edge:CFS_FACES]->(face)
WITH solid, face, face_edge
MATCH (face)-[:BOUNDS]->(bound)
WITH solid, face, face_edge, bound
MATCH (bound)-[:BOUND]->(loop)
WITH solid, face, face_edge, loop
MATCH (loop)-[point_edge:POLYGON]->(pt)
RETURN count(pt) AS point_rows
```

Important detail: after the first anchor, the fast shape intentionally avoids adding labels to every
intermediate node. Reintroducing labels such as `(shell:IfcClosedShell)`, `(face:IfcFace)`, and
`(pt:IfcCartesianPoint)` caused the limited BRep geometry query to regress badly again. The labels are
semantically true, but they appear to give the planner enough freedom to choose a worse join shape.

Observed:

```text
point_rows = 40
time ~= immediate / interactive
```

For larger subsets using the same stepwise shape:

```cypher
MATCH (solid:IfcFacetedBrep)
WITH solid
LIMIT 100
MATCH (solid)-[:OUTER]->(shell)
WITH solid, shell
MATCH (shell)-[face_edge:CFS_FACES]->(face)
WITH solid, face, face_edge
MATCH (face)-[:BOUNDS]->(bound)
WITH solid, face, face_edge, bound
MATCH (bound)-[:BOUND]->(loop)
WITH solid, face, face_edge, loop
MATCH (loop)-[point_edge:POLYGON]->(pt)
RETURN count(pt) AS point_rows
```

Observed timings:

```text
10 BReps:       4,656 point rows      ~0.13s
100 BReps:     21,290 point rows      ~0.11s
all BReps:  1,121,024 point rows      ~4.37s
```

Serializing all BRep point rows to TSV:

```text
1,121,024 rows
80MB TSV
~8.96s
```

## Raw SQLite Cross-Check

The same edge traversal can be fast in raw SQLite when the join order is forced left-to-right:

```sql
SELECT count(*)
FROM edge e1 INDEXED BY idx_edge_from_type_to_id
CROSS JOIN edge e2 INDEXED BY idx_edge_from_type_to_id
CROSS JOIN edge e3 INDEXED BY idx_edge_from_type_to_id
CROSS JOIN edge e4 INDEXED BY idx_edge_from_type_to_id
CROSS JOIN edge e5 INDEXED BY idx_edge_from_type_to_id
WHERE e1.from_node = 1114113
  AND e1.type_id = 34
  AND e2.from_node = e1.to_node
  AND e2.type_id = 9
  AND e3.from_node = e2.to_node
  AND e3.type_id = 7
  AND e4.from_node = e3.to_node
  AND e4.type_id = 6
  AND e5.from_node = e4.to_node
  AND e5.type_id = 43;
```

Observed:

```text
40 rows
~0.00s
```

The unforced/raw SQL version can pick a poor order, mirroring the slow Cypher shape.

## Additional Problematic Shapes Found In `cc-w`

The later phase-timed extraction pass found three more query-shape issues worth carrying to Velr.
These were not theoretical. Each one blocked or distorted the renderer-side IFC2X3 extraction
benchmark before any real backend preparation happened.

### Broad Optional Placement Query

This all-in-one shape hung past the useful diagnostic window on the large IFC2X3 model:

```cypher
MATCH (lp:IfcLocalPlacement)
OPTIONAL MATCH (lp)-[:PLACEMENT_REL_TO]->(parent:IfcLocalPlacement)
OPTIONAL MATCH (lp)-[:RELATIVE_PLACEMENT]->(relative:IfcAxis2Placement3D)
OPTIONAL MATCH (relative)-[:LOCATION]->(location:IfcCartesianPoint)
OPTIONAL MATCH (relative)-[:AXIS]->(axis:IfcDirection)
OPTIONAL MATCH (relative)-[:REF_DIRECTION]->(ref_direction:IfcDirection)
RETURN id(lp), id(parent), id(relative), location.Coordinates, axis.DirectionRatios, ref_direction.DirectionRatios
```

Splitting the same work into small relationship-specific reads completed quickly:

```cypher
MATCH (lp:IfcLocalPlacement) RETURN id(lp) AS placement_id
MATCH (lp:IfcLocalPlacement)-[:PLACEMENT_REL_TO]->(parent:IfcLocalPlacement) RETURN id(lp), id(parent)
MATCH (lp:IfcLocalPlacement)-[:RELATIVE_PLACEMENT]->(relative:IfcAxis2Placement3D) RETURN id(lp), id(relative)
MATCH (relative:IfcAxis2Placement3D)-[:LOCATION]->(location:IfcCartesianPoint) RETURN id(relative), location.Coordinates
MATCH (relative:IfcAxis2Placement3D)-[:AXIS]->(axis:IfcDirection) RETURN id(relative), axis.DirectionRatios
MATCH (relative:IfcAxis2Placement3D)-[:REF_DIRECTION]->(ref_direction:IfcDirection) RETURN id(relative), ref_direction.DirectionRatios
```

Observed result after splitting:

```text
local placements: 16,199
placement phase in full diagnostic: ~0.3-0.6s
```

### Inline Axis2Placement Traversal Inside Extruded Body Extraction

The extruded solid query was fast until it also walked `solid -> POSITION -> LOCATION/AXIS/REF_DIRECTION`
inline. The isolated extrusion shape was around `0.10s`; adding inline position traversal pushed that
probe to around `4.23s`, and the full record query measured around `6.47s`.

The better shape kept the solid-position id in the main extrusion query:

```cypher
OPTIONAL MATCH (solid)-[:POSITION]->(solid_position)
RETURN id(solid_position) AS solid_position_id
```

Then it reused the same small `IfcAxis2Placement3D` vector maps used by local placements. After that,
the full extruded phase measured around `2.0s`.

This still leaves a fixed cost worth optimizing, but it removed the worst planner behavior without
changing semantics.

### Labels On Already-Bound Intermediate BRep Nodes

For BRep geometry, labels on intermediate nodes were semantically correct but planner-hostile:

```cypher
MATCH (solid:IfcFacetedBrep)
WITH solid LIMIT 10
MATCH (solid)-[:OUTER]->(shell:IfcClosedShell)
WITH solid, shell
MATCH (shell)-[face_edge:CFS_FACES]->(face:IfcFace)
...
MATCH (loop)-[point_edge:POLYGON]->(pt:IfcCartesianPoint)
```

The same traversal stayed fast when only the root used a label and subsequent nodes were reached by
typed relationships:

```cypher
MATCH (solid:IfcFacetedBrep)
WITH solid LIMIT 10
MATCH (solid)-[:OUTER]->(shell)
WITH solid, shell
MATCH (shell)-[face_edge:CFS_FACES]->(face)
...
MATCH (loop)-[point_edge:POLYGON]->(pt)
```

This is a subtle but important ergonomics issue: from a user perspective, adding true labels should
normally feel like making a query more selective, not dramatically less predictable.

## Why This Matters

`cc-w` uses Cypher primarily for semantic exploration. For geometry extraction we can eventually use
a more specialized path, but during diagnostics and import/export work we still need typed topology
traversals to be predictable.

In this case, the difference between bad and good query shape was large enough to change the
engineering conclusion:

- bad shape suggests "Cypher cannot handle this geometry topology"
- good shape suggests "the topology is traversable, but the planner needs help"

That distinction matters for deciding whether to invest in Cypher ergonomics, planner hints,
or a separate geometry-export API.

## Requests / Discussion Points For Velr

1. Can Velr's Cypher planner prefer bound-left traversal for typed edge chains?

   A chain like:

   ```cypher
   MATCH (a)-[:T1]->(b)-[:T2]->(c)-[:T3]->(d)
   WHERE id(a) = ...
   ```

   should generally not start from `:T2` or `:T3` if `a` is already selective.

2. Can Cypher `EXPLAIN` expose the lowered edge-table join order?

   For this class of issue, seeing the SQL-like join order is enough to diagnose the problem.

3. Should Velr document `WITH` barriers as the recommended way to force traversal order?

   If this is expected behavior, we can encode that in `cc-w` query playbooks and importer code.

4. Is there or should there be a lower-overhead row API for bulk numeric/JSON extraction?

   Even with a good query shape, geometry extraction currently pays for row materialization,
   string rendering, and JSON coordinate parsing in `cc-w`.

5. Would a typed traversal helper make sense for importer/exporter workloads?

   This is not a replacement for semantic Cypher, but a helper for dense graph-to-geometry walks.

## Suggested Acceptance Checks

For this model, compare these two shapes:

1. Direct one-pattern traversal anchored to `id(solid) = 1114113`
2. Stepwise traversal using `WITH` barriers

Acceptance criteria:

- both return the same `point_rows = 40`
- the direct shape should not choose a pathologically slow middle-first plan
- if planner cannot optimize the direct shape, `EXPLAIN` should make the bad join order obvious
- the all-BRep stepwise traversal should remain in the seconds range for count-only execution

## Related `cc-w` Note

Renderer-side context and current prototype extraction notes live in:

```text
docs/velr-ifc2x3-brep-extraction-memo.md
```
