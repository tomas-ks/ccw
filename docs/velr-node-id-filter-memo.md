# Velr `id(...)` Filter Memo

Date: 2026-04-21

## Summary

We found a reproducible issue in the current Velr stack around `WHERE id(...)` filtering.

The behavior splits into two failure modes:

1. `WHERE id(n) IN [...]` appears to silently ignore the filter and returns the full node set.
2. `WHERE id(n) = ...` and `WHERE id(source) = ...` panic inside the SQL emitter.

Important nuance: `RETURN id(n)` itself appears to work correctly. The problem is specifically in `WHERE id(...)` filter handling.

This matters for graph exploration because we need exact neighborhood expansion from a seed set of internal DB node ids. A wrong or unstable `id(...)` filter makes the graph explorer unreliable.

## Repro Context

Repo:

- `/Users/tomas/cartesian/codex/cc-renderer-w`

Model:

- `building-architecture`

CLI used:

- `cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model building-architecture --query '...'`

The repro is running against the local aligned Velr checkout used by this repo, not a mixed-version stack.

## Minimal Repro

### 1. Seed query works

Command:

```bash
cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model building-architecture --query 'MATCH (w:IfcWall) RETURN id(w) AS node_id LIMIT 40'
```

Observed output:

```text
node_id
395
396
397
398
```

This is the expected seed set for later neighborhood expansion.

### 2. `IN [...]` filter is wrong

Command:

```bash
cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model building-architecture --query 'MATCH (n) WHERE id(n) IN [395,396,397,398] RETURN id(n) AS node_id ORDER BY id(n)'
```

Expected result:

```text
395
396
397
398
```

Observed result:

- returns the full node range `1..404`
- this looks like the filter is ignored or miscompiled

This is a silent correctness failure, which is more dangerous than a hard error.

### 3. Equality filter on `id(n)` panics

Command:

```bash
cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model building-architecture --query 'MATCH (n) WHERE id(n) = 395 RETURN id(n) AS node_id'
```

Observed failure:

```text
thread '<unnamed>' panicked at rust/velr-core/src/backends/default/sql/emitters.rs:5508:14:
unsupported local filter lhs: Expr(Graph(NodeId { var: 0 }))
```

### 4. Equality filter on edge endpoint also panics

Command:

```bash
cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model building-architecture --query 'MATCH (source)-[rel]->(target) WHERE id(source) = 395 RETURN id(source) AS source_db_node_id, id(target) AS target_db_node_id, type(rel) AS relationship_type LIMIT 20'
```

Observed failure:

```text
thread '<unnamed>' panicked at rust/velr-core/src/backends/default/sql/emitters.rs:5508:14:
unsupported local filter lhs: Expr(Graph(NodeId { var: 0 }))
```

## Why This Matters

Our graph viewer flow is:

1. run a seed query that returns internal DB node ids
2. fetch those exact nodes and their incident edges
3. build a bounded neighborhood for graph exploration

That means we depend on queries like:

```cypher
MATCH (n) WHERE id(n) IN [...]
```

and

```cypher
MATCH (source)-[rel]->(target) WHERE id(source) = ...
```

If those are not correct, graph exploration either:

- returns the wrong neighborhood
- crashes the query engine

## Temporary Workaround Used in `cc-w`

To keep the graph explorer stable for now, we stopped relying on filtered Cypher for DB-id neighborhood expansion.

Instead we do:

1. unfiltered bounded snapshot of nodes
2. unfiltered bounded snapshot of edges
3. local BFS / filtering in Rust

Relevant code:

- `/Users/tomas/cartesian/codex/cc-renderer-w/crates/cc-w-platform-web/src/bin/server.rs`

The snapshot-based graph builder starts here:

- `build_graph_subgraph_response(...)`

And the snapshot loaders are:

- `fetch_graph_snapshot_nodes(...)`
- `fetch_graph_snapshot_edges(...)`

This is good enough for phase 1, but it is a workaround, not a real fix in Velr.

## Likely Issue Area

The equality panic points at:

- `rust/velr-core/src/backends/default/sql/emitters.rs:5508`

The panic message:

```text
unsupported local filter lhs: Expr(Graph(NodeId { var: 0 }))
```

suggests that local filter emission does not support `NodeId` expressions on the left-hand side.

The `IN [...]` case is even more concerning because it does not panic; it appears to produce a wrong result instead.

So there may be two related bugs:

1. unsupported local filter emission for `id(...) = ...`
2. incorrect compilation or dropped predicate for `id(...) IN [...]`

## Expected Behavior

All of these should work:

```cypher
MATCH (n) WHERE id(n) = 395 RETURN id(n) AS node_id
MATCH (n) WHERE id(n) IN [395,396,397,398] RETURN id(n) AS node_id ORDER BY id(n)
MATCH (source)-[rel]->(target) WHERE id(source) = 395 RETURN id(source), id(target), type(rel)
```

At minimum, if a shape is unsupported, Velr should return a normal query error rather than panic.

## Suggested Acceptance Checks

For this same model, these should pass:

1. `id(n) = 395` returns exactly one row: `395`
2. `id(n) IN [395,396,397,398]` returns exactly four rows: `395, 396, 397, 398`
3. `id(source) = 395` returns only edges incident from source `395`
4. none of the above panic

## Notes

- This is not the earlier GraphQL version-skew issue. That was already resolved.
- This repro is on the current aligned local Velr setup.
- `RETURN id(...)` appears healthy; the breakage is in `WHERE id(...)`.
