# Agent Workflow

`agent/` is the canonical source for repo-owned AI agent behavior.

The viewer may run through OpenCode today, but the product-level agent workflow should not be
defined by OpenCode's directory layout. Keep durable prompts, workflow rules, and future
user-facing agent definitions here; materialize runtime-specific adapter files from this source.

## Layout

- `agent/agents/` contains built-in agent profiles. These files currently keep OpenCode-compatible
  frontmatter because OpenCode is the active runtime adapter.
- `agent/ifc/` contains durable IFC schema/query guidance consumed by the host-provided AI tools.
- `agent/opencode/` documents the OpenCode adapter layer.
- `.opencode/agents/` is the materialized OpenCode runtime copy. Keep it synchronized with
  `just opencode-sync-agents` or by using the `just web-viewer-opencode*` launch recipes.

## Rules

- Treat `agent/agents/` as source of truth for repo-owned profiles.
- Treat `.opencode/` as runtime adapter/configuration, not as the product model for agents.
- Do not put IFC model data, generated query artifacts, or user project data in `agent/`.
- Future user-defined agents should live in project/app data and be merged through the host's
  permission policy. They should not bypass the deny-by-default OpenCode configuration.
