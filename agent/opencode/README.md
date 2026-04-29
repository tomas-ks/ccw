# OpenCode Adapter

The OpenCode adapter consumes materialized files under `.opencode/`.

Run:

```sh
just opencode-sync-agents
```

to copy canonical built-in profiles from `agent/agents/` into `.opencode/agents/`.

This keeps OpenCode launch compatibility while letting the renderer own its agent workflow in a
runtime-neutral place.
