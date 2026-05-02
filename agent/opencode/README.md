# OpenCode Adapter

The OpenCode adapter consumes materialized files under `.opencode/`.

Run:

```sh
just opencode-sync-agents
```

to copy canonical built-in profiles from `agent/agents/` into `.opencode/agents/`.

This keeps OpenCode launch compatibility while letting the renderer own its agent workflow in a
runtime-neutral place.

## Tool Naming

Runtime tools live under `.opencode/tools/`.

OpenCode publishes tools from a file under that file's namespace. The repo-owned IFC tools
therefore live in `.opencode/tools/ifc.ts` with unprefixed export suffixes:

```ts
export const readonly_cypher = tool(...)
```

The public tool name seen by agents is:

```text
ifc_readonly_cypher
```

Do not prefix exports inside `ifc.ts` with `ifc_`; that would double-prefix the public name. Also
do not add non-IFC compatibility tool files for production agents. The allow-list is intentionally
the single `ifc_*` family.
