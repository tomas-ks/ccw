# IFC Agent Knowledge

This directory holds repo-owned IFC knowledge used by AI tools.

`schemas/<schema>/` contains the small schema-aware JSON files consumed by:

- `ifc_schema_context`
- `ifc_entity_reference`
- `ifc_relation_reference`
- `ifc_query_playbook`

These files are not imported IFC model data. They are durable agent guidance for known IFC schema
families and should stay separate from generated model artifacts under `artifacts/`.

For a normal viewer/server run, the runtime lookup order is:

1. `CC_W_AGENT_KNOWLEDGE_ROOT/<schema>/...`, when explicitly configured
2. `agent/ifc/schemas/<schema>/...` from the current repo/workdir
3. the same path relative to the compiled crate
4. legacy generated artifact locations, for migration compatibility only

If none of those files exist, or if the caller deliberately passes a non-existent artifacts root in
tests, the server falls back to its built-in minimal schema guidance.
