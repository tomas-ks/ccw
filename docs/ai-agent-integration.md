# AI Agent Integration Plan

## Goal

Add an `AI` terminal to the web viewer that is backed by an external agent runtime such as `opencode`.

The agent's job is to:

- explore the currently selected IFC model
- craft and execute read-only openCypher queries
- return a small, validated set of viewer actions

The agent must **not**:

- write to the database
- switch models by itself
- execute arbitrary frontend code
- execute arbitrary shell commands through the viewer backend

## Trust Boundary

The system is split into three roles:

### Browser

The browser is responsible for:

- rendering the 3D view
- rendering the graph explorer
- rendering the JS and AI terminals
- applying already-validated viewer actions

The browser is **not** the policy authority.

### Rust Web Server

The Rust server is the policy and execution authority.

It is responsible for:

- binding each AI session to the currently selected IFC resource
- executing Cypher against the selected IFC database
- enforcing read-only query policy
- validating all AI-returned actions
- returning transcript events and validated actions to the browser

### External Agent Runtime

The external agent runtime is an untrusted planner.

It may:

- inspect the user request
- decide what read-only Cypher to run
- inspect the results
- propose viewer actions

It may not directly:

- connect to Velr
- talk to the browser
- run arbitrary tools beyond the server-owned tool surface

## Session Model

The AI terminal should be session-based.

Each session carries:

- `session_id`
- `resource`
- transcript history
- optional short-lived working memory

The session is always bound to one current IFC resource such as:

- `ifc/building-architecture`

If the user switches to another IFC resource, the server should update the session binding and emit a terminal notice that the AI context has changed.

## Read-Only Query Policy

The AI is allowed to run openCypher only through a server-owned tool:

- `run_readonly_cypher(query)`

That tool must enforce all of the following:

1. the selected resource must be an IFC resource
2. the query must be a single statement
3. the query must not contain write clauses such as:
   - `CREATE`
   - `MERGE`
   - `SET`
   - `DELETE`
   - `REMOVE`
   - `DROP`
4. the query must execute through the existing rollback-scoped Velr query path

Even for valid read-only queries, execution should stay inside the rollback-only scoped transaction path already used by ad hoc Cypher queries in `cc-w-velr`.

This gives two safety layers:

- policy validation before execution
- rollback-scoped execution during runtime

## Schema-Aware Knowledge Layer

The agent should not guess the IFC schema from entity names alone.

The host already knows the selected model resource and its authoritative IFC schema. That schema
should be carried into the AI session and every turn.

Authoritative schema/runtime assets live under:

- `artifacts/ifc/_graphql/ifc2x3_tc1/`
- `artifacts/ifc/_graphql/ifc4/`
- `artifacts/ifc/_graphql/ifc4x3_add2/`

Those directories contain the generated schema/runtime artifacts:

- `ifc-runtime.graphql`
- `ifc-runtime.mapping.json`
- `feature-queries.graphql`
- `handoff-manifest.json`

The first thing the agent should reach for is a compact per-schema reference:

- `artifacts/ifc/_graphql/<schema>/agent-reference.json`

And the shared human-maintained exploration guidance:

- `docs/agent/ifc-exploration-playbook.md`

The Rust server should expose bounded, schema-aware tools over that knowledge instead of giving the
external agent raw file-system roaming:

- `get_schema_context()`
- `get_entity_reference(entity_names)`

This keeps the schema layer deterministic, small, and safe while still letting the agent reason in
a schema-specific way.

## Native OpenCode Tool Surface

The repo-local OpenCode setup also has a narrow native tool surface that is meant to replace the
older freestyle JSON tool dialect over time. The default repo-local agent is
`ifc-explorer`, and the launcher can override it with `CC_W_OPENCODE_AGENT` when
you want a different repo-local profile.

Current files:

- `.opencode/agents/ifc-explorer.md`
- `.opencode/tools/ifc.ts`
- `crates/cc-w-platform-web/src/bin/ifc-knowledge.rs`

The `ifc_*` tools are the only OpenCode tools that should be allowed automatically for the IFC
exploration agent. The repo-local OpenCode config in `tools/opencode/opencode.json` denies by
default and allows only that IFC-prefixed family.

The knowledge-side tools currently cover:

- schema context
- entity reference lookup
- relation reference lookup
- query playbook lookup
- node relation / neighborhood inspection
- renderable descendant discovery

The live read-only Cypher path remains server-owned, but the native tool surface now has a clear
Rust-backed knowledge helper and a single place for the allow-listed OpenCode tool definitions.

## Allowed Viewer Actions

The AI does not return JS source. It returns a validated action list.

Initial action set:

- `graph.set_seeds`
- `elements.hide`
- `elements.show`
- `elements.select`
- `viewer.frame_visible`

Example:

```json
{
  "actions": [
    { "kind": "graph.set_seeds", "dbNodeIds": [395, 396, 397] },
    { "kind": "elements.hide", "semanticIds": ["2iPwJwpPDCSgMheXwk9cBT"] },
    { "kind": "elements.select", "semanticIds": ["12UVOn4wvAJPMUExKdZLb8"] },
    { "kind": "viewer.frame_visible" }
  ]
}
```

The server must reject:

- unknown action kinds
- malformed payloads
- oversized id lists
- actions that no longer match the bound resource

## Server Contract

### Create or resume session

`POST /api/agent/session`

Request:

```json
{
  "resource": "ifc/building-architecture"
}
```

Response:

```json
{
  "sessionId": "agent-1",
  "resource": "ifc/building-architecture",
  "created": true
}
```

### Run one AI turn

`POST /api/agent/turn`

Request:

```json
{
  "sessionId": "agent-1",
  "resource": "ifc/building-architecture",
  "input": "Hide the roof and seed the graph from its related slabs."
}
```

Response:

```json
{
  "sessionId": "agent-1",
  "resource": "ifc/building-architecture",
  "transcript": [
    {
      "kind": "message",
      "role": "assistant",
      "text": "I found the roof aggregate and the related slab elements."
    },
    {
      "kind": "toolCall",
      "name": "run_readonly_cypher",
      "text": "MATCH (:IfcRoof)<--(:IfcRelAggregates)-->(slab:IfcSlab) RETURN DISTINCT slab.GlobalId AS global_id"
    }
  ],
  "actions": [
    {
      "kind": "elements.hide",
      "semanticIds": ["2iPwJwpPDCSgMheXwk9cBT"]
    },
    {
      "kind": "graph.set_seeds",
      "dbNodeIds": [395, 396]
    }
  ]
}
```

For v1, transcript can be returned as a complete batch per turn rather than streaming.

## Browser Execution Model

The browser will:

1. send AI terminal input to `/api/agent/turn`
2. render the returned transcript into the AI xterm
3. apply the validated actions through the existing viewer and graph APIs

The browser should never execute raw JS returned by the AI.

## Adapter Model

The Rust server owns the agent loop.

The adapter interface should look roughly like this:

- `start_session(resource)`
- `run_turn(input, tools) -> AgentTurnResult`

The tool surface exposed to the adapter should initially be:

- `run_readonly_cypher(query)`

The adapter may later expose other read-only helpers, but viewer mutations should still happen only by returning validated actions.

## Initial Implementation Sweep

### Lane A: Contract and docs

- write this architecture note
- define request and response types
- define UI action schema

### Lane B: Backend session and policy

- add `POST /api/agent/session`
- add `POST /api/agent/turn`
- add server-side read-only Cypher validation
- add server-side action validation
- bind sessions to the selected IFC resource

### Lane C: Frontend AI terminal

- replace the echo AI terminal with a backend-backed terminal
- keep the JS terminal unchanged
- render transcript output into the AI xterm
- apply validated actions to the viewer and graph

### Lane D: Agent adapter

- add a stub agent runtime first

## Schema-Aware Evaluation Matrix

We should evaluate the same intent across at least one model for each supported schema family:

- IFC2X3_TC1
- IFC4
- IFC4X3_ADD2

Prompts should check both **definition awareness** and **exploration behavior**.

Good evaluation prompts:

- "What schema is this model using, and what does that imply for roof/slab reasoning?"
- "What does `IfcRoof` mean in this schema, and is it likely to be directly renderable here?"
- "What relations are slabs connected to in this model?"
- "Find one `IfcWall`, explain its role, and show its properties."
- "Show me the project/building/storey structure in the graph."
- "Why didn't hiding the roof work directly in this model?"

Success should look like:

- the agent names the active schema correctly
- the agent prefers schema context or entity reference before guessing
- the agent uses `node_id` for graph actions and `global_id` for viewer actions
- the agent distinguishes observation from inference
- the agent adjusts its explanation when the same concept differs across IFC2X3, IFC4, and IFC4X3
- add a real `opencode` adapter later

### Lane E: Hardening tests

- reject write Cypher
- reject malformed or unknown actions
- reject stale-resource actions
- accept valid read-only query and action flows

## Important Current Constraint

`opencode` is not currently installed on this machine, so the first end-to-end implementation should use a backend stub adapter that already matches the final server contract.

That lets the browser and Rust server stabilize before the real adapter is introduced.

## Phase 2 Backend Selection

The Rust web server now supports a backend selection split:

- default: `stub`
- optional: `opencode`

Current selection environment variables:

- `CC_W_AGENT_BACKEND=stub|opencode`
- `CC_W_AGENT_MAX_READONLY_QUERIES_PER_TURN=<n>`
- `CC_W_AGENT_MAX_ROWS_PER_QUERY=<n>`

When `CC_W_AGENT_BACKEND=opencode`, the local child-process adapter reads:

- `CC_W_OPENCODE_EXECUTABLE`
- `CC_W_OPENCODE_ARGS`
- `CC_W_OPENCODE_AGENT`
- `CC_W_OPENCODE_MODEL`
- `CC_W_OPENCODE_WORKDIR`
- `CC_W_OPENCODE_TIMEOUT_MS` for the no-progress timeout
- `CC_W_OPENCODE_MAX_STDOUT_BYTES`
- `CC_W_OPENCODE_MAX_STDERR_BYTES`
- `CC_W_OPENCODE_MAX_STEPS_PER_TURN`

The current `opencode` path uses the repo-local `ifc-explorer` agent by default and keeps the Rust
server as the policy authority. The server still owns:

- read-only Cypher validation
- current-resource binding
- per-turn query and row caps
- final UI action validation

For local development, `just opencode-install` installs the official OpenCode CLI, exposes a
repo-local launcher at `.tools/opencode/bin/opencode`, and redirects writable cache/config/data/state
into `.tools/opencode/`. That gives us a stable binary path for the project without depending on
shell profile edits.

The current project integration speaks to the repo-local OpenCode server directly from the Rust server.

The Rust adapter:

- starts a native `opencode serve` process with the locked-down repo-local config
- creates one OpenCode session per viewer AI session
- submits turns through `POST /session/{sessionID}/prompt_async`
- streams progress from the OpenCode `/event` feed
- uses the locked-down config at
  [tools/opencode/opencode.json](/Users/tomas/cartesian/codex/cc-renderer-w/tools/opencode/opencode.json)
- keeps the repo-local `ifc-explorer` agent and `ifc_*` tools as the only allow-listed interface

`CC_W_OPENCODE_AGENT` is optional. If it is unset, the launcher defaults to
`ifc-explorer`.

`CC_W_OPENCODE_MODEL` is optional. If it is set, the Rust adapter passes `--model`.
If it is unset, the adapter lets OpenCode choose the default model for the
current authenticated account.

This keeps the Rust server policy boundary intact while avoiding the older one-shot bridge runtime.
