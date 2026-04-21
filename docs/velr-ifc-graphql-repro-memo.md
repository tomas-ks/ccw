# Velr IFC GraphQL Version-Alignment Memo

## Summary

The previously observed IFC GraphQL runtime failure:

```text
no such column: np0.value
```

was caused by version skew in the local Velr development stack, not by the current `velr-ifc`
import/runtime pipeline itself.

For `w`, there is one extra operational consequence:

- any model database imported through the old skewed stack should be discarded and reimported

After reimporting `building-architecture`, the local `cc-w-velr` GraphQL smoke path returned to the
expected GraphQL execution path.

## Failure Shape

The earlier failure showed up both in upstream verification and in `w`'s local wrapper as:

```text
resolver error: velr error (code -4): no such column: np0.value
```

## Root Cause

Upstream diagnosis: `velr-ifc` had been running with a live path checkout of `velr-graphql-core`
while another part of the stack still resolved `velr` from a mismatched crates.io version.

That meant the GraphQL layer and the active Velr storage/runtime contract were not guaranteed to
match.

In practice, the GraphQL layer expected the newer single-value property shape
`node_property.value`, while the mismatched Velr path did not line up with that expectation.

## What Changed

Upstream fixed this by forcing the stack to resolve `velr` from the sibling local checkout.

We mirrored that guardrail in `w` by adding:

```toml
[patch.crates-io]
velr = { path = "../../../velr/codex/velr-repo/rust/velr-rust-driver" }
```

to `/Users/tomas/cartesian/codex/cc-renderer-w/Cargo.toml`.

## Reimport Requirement For `w`

Even after dependency alignment is fixed, a database imported through the old skewed stack should
be treated as stale.

For `w`, the safe rule is:

- if a model was imported before the Velr alignment fix, delete its `model.velr.db*` artifacts and
  reimport it

The `cc-w-velr` import path now has an explicit replace mode for that:

```bash
cd /Users/tomas/cartesian/codex/cc-renderer-w
cargo run -p cc-w-velr --bin cc-w-velr-tool -- import --fixture building-architecture --replace-existing
```

## Verified Current State In `w`

After removing the old database artifacts and reimporting `building-architecture`, the local query
path succeeded through GraphQL again:

```bash
cd /Users/tomas/cartesian/codex/cc-renderer-w
cargo run -p cc-w-velr --bin cc-w-velr-tool -- query-projects --model building-architecture
```

Observed result:

```text
query_source: graphql
projects: 1
- 215 [IfcProjectInstance] global_id=- name=- long_name=- phase=-
```

## Notes

- The earlier failure memo was useful as a repro record, but the root-cause conclusion there is now
  superseded by the upstream version-alignment finding.
- The family-specific runtime bundle selection in `velr-ifc` remains the cleaner pattern, but it
  was not the reason `building-architecture` failed in this case because the top-level and
  `ifc4x3_add2` runtime bundle files currently match byte-for-byte for this fixture set.
