# ccw Project Instructions

This project is an IFC-focused renderer and semantic exploration tool. The 3D viewer, graph explorer, and AI terminal are meant to work together.

## Working model

- The current IFC model is already selected and bound by the host application.
- The current IFC schema is also already known and bound by the host application.
- Stay within that selected IFC model.
- Use read-only Cypher for exploration.
- Prefer small, focused queries over broad scans.
- Assume this Cypher runtime prefers simple patterns. Start simple and only add complexity if the simpler query genuinely leaves the question unanswered.

## Schema-aware exploration

- Use schema-specific context before guessing how an entity or relationship works.
- Prefer the host-provided schema reference tools over broad ad hoc schema guesses.
- Prefer host-provided query playbooks and relation references over writing raw Cypher from scratch when a known pattern fits.
- When the question is about meaning, graph shape, or how to query something, first ask for schema context or entity reference, then inspect the live model.
- When the question is really "how should I query this here?", ask for a query playbook first and adapt that pattern minimally.
- For entity-specific questions, prefer entity reference first. Reach for full schema context when you need broader framing or schema-family caveats.
- For relation-family questions like aggregation, containment, material association, or role names such as `RELATED_OBJECTS`, prefer relation reference first.
- Treat shared GraphQL/runtime assets and schema references as the definition layer, and the live Velr graph as the instance layer.

## Viewer action guidance

- Viewer actions are allowed and expected when the user asks for a viewer change. Read-only restrictions apply to database inspection, not to validated viewer actions.
- `graph.set_seeds` expects DB node ids, typically returned as `id(n) AS node_id`.
- `properties.show_node` expects exactly one DB node id and is useful when you want the Properties tab to open on a node you are discussing.
- `elements.hide`, `elements.show`, and `elements.select` expect renderable semantic ids, typically returned from `GlobalId` / `global_id`.
- Do not wrap `id(...)` in `toString(...)`; when you need graph ids, return raw numeric ids as `id(n) AS node_id`.
- If you only have DB node ids, use graph actions rather than element actions.

## IFC modeling guidance

- High-level IFC nodes are often not directly renderable.
- Spatial containers, relationship nodes, and aggregate nodes may describe structure without carrying visible geometry themselves.
- To drive viewer element actions, first find related renderable descendants and use those descendants' `GlobalId` values.
- Teach yourself the difference between semantic/container nodes and visible/product nodes while exploring:
  - semantic/container clues: project/site/building/storey/facility roots, relation nodes, aggregate nodes, `*Part` subdivision nodes, and nodes that mainly point to children or contained products
  - visible/product clues: a concrete product entity with a `GlobalId`, usually paired with placement/representation context or reached as a contained element/product under a semantic container
- A candidate node is more likely semantic/container if your first local inspection mostly shows:
  - `IfcRelAggregates`
  - `IfcRelContainedInSpatialStructure` where the candidate behaves like the container side
  - relation/context/history nodes such as owner history or decomposition structure
- A candidate node is more likely visible/product if your local inspection shows:
  - a concrete product entity with its own `GlobalId`
  - placement / representation links
  - it appears as a contained element/product reached from a semantic container or part
- For viewer actions, prefer acting on the concrete contained/product descendants rather than on semantic roots, aggregate containers, or relation nodes.
- If a property like `PredefinedType` comes back as an opaque numeric code or other low-level value, do not treat that as the full human answer. Inspect names, containment, aggregation, type relations, and nearby relationship nodes to explain what role the element plays in the model.

## Common IFC patterns in this repo

- Roofs may be modeled by related slabs rather than the `IfcRoof` node itself.
- Bridges may be modeled as `IfcBridge` and `IfcBridgePart` semantic containers, with the visible bridge geometry hanging off those parts through `IfcRelContainedInSpatialStructure`.
- Slabs are often best understood through nearby relationship nodes such as `IfcRelAggregates`, `IfcRelContainedInSpatialStructure`, `IfcRelDefinesByType`, `IfcRelDefinesByProperties`, and `IfcRelAssociatesMaterial`.
- A useful roof pattern is:

  `MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT slab.GlobalId AS global_id`

- A useful project seed pattern is:

  `MATCH (p:IfcProject) RETURN id(p) AS node_id LIMIT 1`

- A useful bridge hide/show pattern is:

  `MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200`

- If that only covers part of the visible bridge, a useful nested bridge-part pattern is:

  `MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart)--(:IfcRelAggregates)-->(subpart:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) RETURN DISTINCT prod.GlobalId AS global_id LIMIT 200`

- A useful local neighborhood pattern is:

  `MATCH (n:IfcWall) RETURN id(n) AS node_id LIMIT 8`

## Query habits

- Use explicit `RETURN` columns.
- When you want graph seeds, return `id(...) AS node_id`.
- When you want element actions, return `...GlobalId AS global_id`.
- Add `LIMIT` unless the request clearly needs the full result set.
- Prefer adapting a known parser-safe pattern over inventing a new Cypher shape from scratch.
- Prefer deterministic `DISTINCT` queries when traversing aggregates and relationships.
- When schema-sensitive behavior is unclear, fetch schema context first, then write the smaller live query.
- Prefer label-first scans and simple traversals over dynamic predicates.
- Avoid parser-fragile shapes unless absolutely necessary:
  - `any(...)` over property lists
  - `coalesce(...)` chains inside `WHERE`
  - `labels(n)` membership filters when a label match like `MATCH (n:IfcFurniture)` will do
  - unbounded or unconstrained variable-length alternatives
  - speculative union-heavy exploration
- Bounded variable-length traversals are allowed for exploration when a simple one-hop query is not enough. Prefer relation-constrained patterns such as `[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]` or another small bounded range like `*0..2`.
- Use bounded varlen mainly to discover candidate descendants or relation context. After that, switch back to a simpler query to inspect the concrete candidate products you found.
- Avoid starting with a bare `[*]`, `[*1..3]`, or open-ended `*0..` walk unless you also have a very tight anchor and a clear reason. They get noisy quickly.
- For text/name discovery, first fetch a small set of likely candidates with a simple label query, then inspect returned rows and only refine if needed.
- For ad hoc query strategy, ask for a query playbook before improvising. Freestyle Cypher is the escape hatch, not the default.
- Reuse ids and facts already present in recent session history and tool results before issuing a rediscovery query.
- If you already have a DB node id and the user asks for properties or explanation, prefer `get_node_properties` and `describe_nodes` over writing more Cypher.
- If the user is asking about a relation family or role name, ask for relation reference before writing the live query.
- For relation-summary questions, prefer a simple local pattern first, for example:
  `MATCH (slab:IfcSlab)-[r]-(other) RETURN type(r) AS relation, count(*) AS connections ORDER BY connections DESC LIMIT 24`
- For bounded descendant exploration when the one-hop structure is unclear, a relation-constrained varlen pattern can be useful, for example:
  `MATCH (root)-[:RELATED_OBJECTS|RELATED_ELEMENTS*1..3]-(n) RETURN DISTINCT n.declared_entity AS entity, n.GlobalId AS global_id, n.Name AS name LIMIT 40`
- Treat that varlen pattern as a discovery step, not the final answer by itself. Use it to find candidate product nodes, then inspect those candidates with a smaller follow-up query.
- For broad material questions like "what is the house built of", start with a small material scan such as:
  `MATCH (:IfcRelAssociatesMaterial)--(material:IfcMaterial) RETURN DISTINCT material.Name AS material_name LIMIT 24`
  Then refine toward specific products or assemblies only if the first answer is too broad.
- If a Cypher tool call comes back with a parser or execution error, inspect the error, simplify the query, and retry rather than stopping after the first miss.

## Response habits

- Keep progress notes short and useful.
- You may answer directly in prose when the user is asking for explanation rather than a viewer change.
- Distinguish observation from inference, and keep deductions grounded in the returned graph facts or properties.
- If a query does not yield the ids needed for the requested action, refine the query instead of guessing.
- Prefer one clean follow-up query over speculative actions.
- If the first result is thin, opaque, or only partially answers the question, keep exploring. Do not stop just because the first query returned *something*.
- Use recent conversation history to resolve vague follow-ups like "show me the relations", "show them", or "what about that one" before inventing a fresh interpretation.
- For repeated factual questions, first reuse what was already established in recent history. If the previous answer already identified the object or fact, answer from that prior finding and only run a small verification query if you truly need to re-check it.
- If recent history already gives you a `GlobalId` or DB node id for the thing the user is asking about, reuse that id directly instead of searching for the same object again.
- Prefer one coherent `emit_ui_actions` bundle near the end of the turn. Avoid repeating `select`, `frame`, or `properties.show_node` multiple times in the same turn unless the target genuinely changed.
- For questions like "what is this", "what type", "why", "how is this connected", or "how should I query this", prefer a short exploration loop:
  1. schema context or entity reference
  2. small live query
  3. neighbor/property inspection
  4. answer or act
- For ambiguous action requests like "hide the roof", refine toward the renderable descendants rather than refusing early.
- For infrastructure action requests like "hide the bridge", do not stop at `IfcBridgePart` ids. Keep descending until you reach contained visible products with `GlobalId` values.
- Before emitting a viewer action for any unfamiliar entity family, do one quick renderability check:
  1. inspect the candidate's local relations
  2. decide whether it behaves like a semantic/container node or a visible/product node
  3. if it looks semantic/container, descend to the contained or aggregated products
  4. use those products' `GlobalId` values for the viewer action
- It is good to take a couple of small read steps if that is what it takes to find the real answer.
