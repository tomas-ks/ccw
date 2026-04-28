# IFC Exploration Playbook

This playbook is for the AI agent that explores IFC-backed Velr graphs in `ccw`.
It is intentionally practical. The goal is not to restate the whole IFC schema.
The goal is to help the agent choose the right schema-aware query habits and turn
graph facts into good answers or viewer actions.

## 1. Start with the bound model and schema

- The host already binds the current IFC model and its schema.
- Do not guess the schema from entity names alone.
- Prefer the schema-aware reference bundle for the active model:
  - `artifacts/ifc/_graphql/ifc2x3_tc1/agent-reference.json`
  - `artifacts/ifc/_graphql/ifc4/agent-reference.json`
  - `artifacts/ifc/_graphql/ifc4x3_add2/agent-reference.json`

Use the active schema reference first when:
- the user asks what an entity means
- the user asks how the graph is shaped
- the user asks for a Cypher starting point
- the same concept differs across IFC2X3, IFC4, and IFC4X3

## 2. Keep the two id systems straight

- Use `id(n) AS node_id` when you want graph seeds or graph reasoning.
- Use `GlobalId AS global_id` when you want viewer element actions like
  hide, show, or select.
- High-level IFC nodes are often not directly renderable. If the user wants a
  viewer action, first find the renderable descendants or related products that
  actually carry visible geometry.

## 2.5. Learn to separate semantic/container nodes from visible/product nodes

This matters more than memorizing every IFC entity family one by one.

When you encounter an unfamiliar node and the user wants a viewer action, ask:

1. Does this node behave like semantic structure?
2. Or does it behave like a visible product?

### Common semantic/container clues

- facility / project / site / building / storey roots
- relation nodes such as `IfcRelAggregates` or `IfcRelContainedInSpatialStructure`
- subdivision/group nodes such as many `*Part` entities
- the node mostly points to children, containers, or relationship structure
- the first neighborhood sample is dominated by aggregation, containment, owner
  history, or other context nodes

These nodes are useful for explanation and graph navigation, but often not the
thing to hide/show/select directly.

### Common visible/product clues

- a concrete product entity with its own `GlobalId`
- placement / representation links in the local neighborhood
- the node is reached as a contained element/product underneath a semantic root
  or part
- the node looks like a physical carrier such as a wall, slab, column, member,
  footing, furnishing element, sign, or fill product

These nodes are much better candidates for viewer element actions.

### A good default renderability check

Before emitting a viewer action for an unfamiliar entity family:

1. inspect the candidate node's local relations
2. decide whether it behaves like semantic/container structure or like a
   visible/product node
3. if it looks semantic/container, descend through aggregates or containment
4. act on the descendant products' `GlobalId` values, not the container ids

This heuristic is often more reliable than asking "is this entity type usually
renderable?" in the abstract.

## 3. Explore locally before concluding

Prefer a small sequence of focused reads over one large speculative query.

Good pattern:
1. Find one or a few candidate nodes.
2. Inspect names, `declared_entity`, and `GlobalId`.
3. Inspect nearby relationship nodes and neighboring products.
4. Only then answer or emit viewer actions.

Use `LIMIT` unless the request clearly needs a full scan.

The Velr Cypher path in this app is happiest with plain shapes. Prefer:
- direct label matches like `MATCH (n:IfcFurniture)`
- simple traversals like `MATCH (n)-[r]-(other)`
- explicit `RETURN` columns

Only reach for more dynamic filtering if the simple pass really is not enough.
In particular, avoid leading with:
- `any(...)` across many properties
- `coalesce(...)`/`toLower(...)` chains in `WHERE`
- `labels(n)` filtering instead of a direct label match
- complex `UNION` exploration just to discover candidates
- unbounded or unconstrained `[*]` / `[*..]` style walks

Bounded variable-length traversals are fine when they stay disciplined:
- keep the range small, usually `*1..3` or `*0..2`
- constrain the relation family if you can
- use them to discover candidate descendants or relation context
- once you find promising candidates, switch back to a simpler follow-up query

This is usually better:

```cypher
MATCH (root)-[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]-(n)
RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name
LIMIT 40
```

than this:

```cypher
MATCH (root)-[*1..3]-(n)
RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name
LIMIT 40
```

The bare walk tends to pull in placements, owner history, and other context
nodes before it tells you which visible products actually matter.

## 4. Common graph shapes to expect

### Spatial structure

`IfcProject -> IfcSite -> IfcBuilding -> IfcBuildingStorey`

This usually appears through aggregation and containment:
- `IfcRelAggregates`
- `IfcRelContainedInSpatialStructure`

Useful questions:
- what storey contains this product
- what project/building/storey structure exists

### Product typing and properties

Products often reach their richer meaning through:
- `IfcRelDefinesByType`
- `IfcRelDefinesByProperties`
- `IfcRelAssociatesMaterial`

If a type enum or numeric-looking value feels opaque, inspect those relations
before answering in human terms.

### Semantic roots vs visible leaves

Many IFC questions have the same two-layer shape:

- a semantic root or grouping layer
- one or more visible product leaves underneath it

Examples:
- `IfcRoof` -> aggregated `IfcSlab` products
- `IfcBridge` / `IfcBridgePart` -> contained bridge products
- storey/building containers -> contained walls, doors, furniture, and slabs

If the user's request is a viewer action, bias toward the visible leaves.
If the user's request is explanation or hierarchy, the semantic roots are often
exactly what you want.

### Roofs and slabs

Do not assume `IfcRoof` is the renderable thing the user sees.

In this repo, roofs are often better understood as:
- a semantic roof aggregate
- related `IfcSlab` products that carry the visible roof geometry

If "hide the roof" fails directly:
- inspect `IfcRelAggregates`
- inspect related `IfcSlab` nodes
- use slab `GlobalId` values for viewer actions if those are the visible parts

### Bridges and bridge parts

Do not assume `IfcBridge` or `IfcBridgePart` is the visible thing the user sees.

In infrastructure models, bridge exploration is often better understood as:
- a semantic `IfcBridge` facility root
- one or more `IfcBridgePart` subdivision/container nodes
- visible products hanging off those parts through `IfcRelContainedInSpatialStructure`

Bridge support/substructure language:
- `IfcFooting`, foundation-like products, piers, and abutments under a bridge part are strong bridge substructure/support signals
- explain that classification from the live containment/type relations, not from a display name alone

If "hide the rail bridge" does nothing:
- first list `IfcBridge` roots and choose the one whose name/object type matches rail/railway/road/girder/arched wording
- anchor follow-up descendant queries to that chosen bridge root; do not descend from every `IfcBridge` when the user asked for one specific bridge
- inspect `IfcRelAggregates` to find the bridge parts
- inspect `IfcRelContainedInSpatialStructure` from those parts to find visible products
- if the first bridge-part containment step only covers part of the bridge, descend one more aggregate hop for nested parts such as piers
- use the contained products' `GlobalId` values for viewer actions

## 5. Query recipes

### Seed from the project root

```cypher
MATCH (p:IfcProject)
RETURN id(p) AS node_id
LIMIT 1
```

### Seed from a handful of walls

```cypher
MATCH (w:IfcWall)
RETURN id(w) AS node_id
LIMIT 8
```

### Seed from roof-related slabs

```cypher
MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab)
RETURN DISTINCT id(slab) AS node_id
LIMIT 16
```

### Hide roof-related slabs

```cypher
MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab)
RETURN DISTINCT slab.GlobalId AS global_id
LIMIT 32
```

### Hide products contained by bridge parts

```cypher
MATCH (bridge:IfcBridge)
RETURN id(bridge) AS bridge_node_id, bridge.Name AS name, bridge.ObjectType AS object_type
LIMIT 24
```

Then choose the bridge root that matches the user wording and anchor the product query:

```cypher
MATCH (bridge:IfcBridge)
WHERE id(bridge) = <bridge_node_id>
MATCH (bridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod)
RETURN DISTINCT prod.GlobalId AS global_id
LIMIT 200
```

### Hide products contained by nested bridge parts

```cypher
MATCH (bridge:IfcBridge)
WHERE id(bridge) = <bridge_node_id>
MATCH (bridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)--(:IfcRelAggregates)-->(subpart:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod)
RETURN DISTINCT prod.GlobalId AS global_id
LIMIT 200
```

### Show sewer manholes

In the infrastructure sample project, sewer manholes are modeled as renderable
`IfcElementAssembly` products, not as `IfcDistributionChamberElement` nodes.
Use label-first assembly queries before broad text scans.

```cypher
MATCH (n:IfcElementAssembly)
RETURN id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type
LIMIT 40
```

Then narrow if needed:

```cypher
MATCH (n:IfcElementAssembly)
WHERE n.Name CONTAINS 'manhole'
RETURN id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type
LIMIT 40
```

If the type relation is useful:

```cypher
MATCH (n:IfcElementAssembly)--(:IfcRelDefinesByType)--(t:IfcElementAssemblyType)
WHERE t.Name CONTAINS 'manhole'
RETURN DISTINCT id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type
LIMIT 40
```

Avoid starting this lookup with broad `MATCH (n)` plus `toLower(...)` text
filters; those scans can become noisy in this runtime and return many
non-renderable helpers.

### Bounded descendant discovery

Use this when you know you need to descend, but the exact one-hop structure is
still unclear.

```cypher
MATCH (root)-[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]-(n)
RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name
LIMIT 40
```

Use it as a discovery pass. Then follow up with a smaller query against the
concrete products you found rather than answering from the varlen walk alone.

### Inspect a product's nearby relation types

```cypher
MATCH (n)-[r]-(m)
WHERE id(n) = $node_id
RETURN type(r) AS rel, m.declared_entity AS entity, m.Name AS name
LIMIT 24
```

### Inspect slabs through relation context

```cypher
MATCH (slab:IfcSlab)<-[r]-(other)
RETURN type(r) AS rel, other.declared_entity AS entity, other.Name AS name
LIMIT 24
```

### Summarize slab relation types

```cypher
MATCH (slab:IfcSlab)-[r]-(other)
RETURN type(r) AS relation, count(*) AS connections
ORDER BY connections DESC
LIMIT 24
```

If a more complex relation query fails to parse, simplify toward this shape and
retry before giving up.

### Start broad on material questions

For a question like "what is the house built of", do not begin with a large
schema detour or a speculative multi-join query. Start by listing the materials
actually attached in the model:

```cypher
MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial)
RETURN DISTINCT material.Name AS material_name
LIMIT 24
```

If that answer is too broad, then narrow toward the relevant products or
assemblies.

### Find a named furnishing candidate

When the user asks for something like "kitchen unit", do not start with a giant
dynamic predicate. Start by pulling a few likely furnishing candidates and
inspect the returned names and object types in the tool result.

```cypher
MATCH (n:IfcFurniture)
RETURN id(n) AS node_id, n.GlobalId AS global_id, n.Name AS name, n.ObjectType AS object_type
LIMIT 25
```

If that is not enough, then try the adjacent likely entity or a slightly more
focused second query.

## 6. Schema-specific cautions that matter

### IFC2X3_TC1

- `IfcRoof` exposes `ShapeType` instead of the later `PredefinedType` field.
- `IfcRoof` and `IfcSlab` sit under `IfcBuildingElement`.
- The schema bundle is full coverage, but some runtime notes still call out
  partial roadmap history for materials, presentation, and geometry slices.

### IFC4

- `IfcProject` lives under `IfcContext`, not directly under `IfcObject`.
- `IfcRoof` uses `PredefinedType`.
- `IfcSlabElementedCase` and `IfcSlabStandardCase` appear as useful slab
  variants in addition to `IfcSlab`.

### IFC4X3_ADD2

- `IfcRoof` and `IfcSlab` live under `IfcBuiltElement`.
- The schema surface expands beyond building-only concerns and includes
  infrastructure entities such as `IfcAlignment`, `IfcBridge`, and `IfcRoad`.
- Do not assume that every model using IFC4X3 is building-centric.

## 7. Answering questions well

When the user asks a general question:
- answer in prose if no viewer change is needed
- distinguish observation from inference
- name the relation or property that supports the answer

When the user asks a follow-up like "show it" or "tell me more about the
properties":
- reuse the ids discovered in the prior step if they are already in session
- prefer `get_node_properties` and `properties.show_node`
- avoid rediscovering the same node unless the prior turn truly did not identify it

Better:
- "The roof is represented semantically by `IfcRoof`, but the visible geometry is
  likely on related `IfcSlab` nodes under `IfcRelAggregates`."

Worse:
- "The roof is weird."

When the user asks "what type/kind is this?":
- if `PredefinedType` is clean and human-readable, use it
- if the value is opaque or unhelpful, explain the role through containment,
  aggregation, type, property, or material relations instead

## 8. Use the schema reference bundle, not the full raw schema, by default

The generated GraphQL SDL and mapping files are authoritative, but they are large.
The first thing to reach for should be the compact per-schema `agent-reference.json`
files. Drop down to raw runtime SDL or mapping detail only when the compact
reference is not enough.

## 9. Prefer playbooks over invention

When the main uncertainty is not entity meaning but query shape, prefer a
query playbook first. A good workflow is:

1. `get_entity_reference` or `get_relation_reference`
2. `get_query_playbook`
3. one small live Cypher query adapted from that playbook
4. node/property/neighbor inspection if needed

The playbook should be treated as the safe starting posture for this runtime.
Raw freestyle Cypher is the fallback when no playbook fits.

When a bounded varlen traversal fits, keep it disciplined:
- prefer `*1..3` or `*0..2`
- prefer relation-constrained forms over bare `[*]`
- use it to find candidates, then tighten the next step
