# State Management Implementation Plan

## Goal

Bring the web viewer fully in line with the state ownership contract in
[state-management.md](./state-management.md).

The target architecture is:

- app intent is owned by one JavaScript app-state store
- renderer truth is owned by Rust runtime state
- semantic/database truth is fetched from backend APIs
- UI widgets render from committed snapshots and dispatch commands to owners

This plan is intentionally incremental. The current web viewer is productive, so each slice should
leave the app usable and testable.

## Current Status

Implemented:

- `cc-w-runtime::RuntimeSceneState` owns visibility, selection, residency, and default view state
- file-level outliner suppression is now a renderer visibility layer
- resource load completion emits `w-viewer-state-change`
- renderer mutations now emit committed viewer-state events for view mode, visibility,
  suppression, selection, and streaming changes
- a small JavaScript app-state store is exposed as `window.wAppState`
- resource switching now records requested resource intent and reacts to committed viewer state
- graph clearing and AI resource binding follow committed resource changes instead of raw picker
  events
- graph, terminal, and outliner panel visibility are app-state driven
- rotate/pick tool state is app-state driven while still syncing the Rust interaction picker
- AI/JS terminal tab state is app-state driven
- the outliner derives from the committed app-state viewer snapshot, including
  `defaultElementIds` and `suppressedElementIds`
- active semantic focus now flows through app state for graph picks and 3D picks
- property balloon open/close/dismiss and screen anchor state now flow through app state
- the browser now reads project membership from the Rust-owned viewer resource catalog

Remaining gaps:

- graph layout and graph visual selection are still local to `createGraphShell`
- property balloon content is still rendered locally from query results
- resource picker rendering still relies on the Rust-populated select element rather than an
  app-state-rendered control
- some derived UI still reacts directly to DOM or viewer events instead of a single app-state
  subscriber path

The main risk is state drift: two widgets can believe different things because they reacted to
different events.

## Slice 1: Introduce A Small App-State Store

Status: implemented.

Added a tiny store in `crates/cc-w-platform-web/web/index.html`.

Initial state shape:

```js
{
  requestedResource: null,
  committedViewerState: null,
  panels: {
    graph: false,
    terminal: false,
    outliner: false
  },
  tools: {
    orbit: true,
    pick: true
  },
  terminal: {
    activeTool: "ai"
  },
  focus: {
    source: "none",
    resource: null,
    dbNodeId: null,
    graphNodeId: null,
    semanticId: null
  },
  balloon: {
    open: false,
    anchor: null,
    source: "none",
    dismissed: false
  }
}
```

Store API:

- `getState()`
- `subscribe(listener)`
- `dispatch(action)`
- `renderAll()` only through subscribers, not direct cross-calls

Acceptance:

- no behavior change
- existing viewer still boots
- panel/tool state can be inspected in the console through `window.wAppState`
- `git diff --check`
- JS module syntax parse

## Slice 2: Make Resource Switching Commit-Driven

Status: implemented for graph clearing and AI binding; resource picker visual rendering still needs
one more cleanup pass.

Route all resource picker changes through the app store.

Rules:

- the picker dispatches `resource/requested`
- Rust remains responsible for loading and committing renderer state
- `w-viewer-state-change` with reason `resource` dispatches `viewer/committed`
- picker selected value is rendered from app state plus committed viewer state
- AI session binding updates from the committed resource, not from the raw picker event
- graph clear/reset follows committed resource changes

Cleanup:

- remove duplicate `handleResourceSwitch` calls attached directly to the picker
- remove outliner rendering from raw picker events
- keep one resource-switch orchestration path

Acceptance:

- start on `ifc/building-architecture`
- open outliner
- switch to `project/building`
- outliner shows project assets only after committed viewer state
- AI terminal reports the committed resource/project once
- graph clears once
- no stale `ifc/building-architecture` asset state appears after switching

## Slice 3: Move Panel And Tool State Into App State

Status: implemented for graph, terminal, outliner, rotate/pick tools, and AI/JS terminal tabs.

Migrate:

- graph open/closed
- terminal open/closed
- outliner open/closed
- active tools: orbit and pick
- active terminal tool: AI or JS

Rules:

- buttons dispatch app-state actions
- DOM classes and `aria-pressed` are render outputs
- no panel should use DOM class state as its source of truth
- close buttons dispatch the same app-state action as header toggles

Acceptance:

- graph, terminal, and outliner buttons stay in sync with panel state
- closing a panel updates the matching header button
- console `wAppState.getState()` matches the visible UI
- rotate and pick remain independently toggleable
- drag with orbit+pick does not create a click pick

## Slice 4: Normalize Focus And Selection

Status: partly implemented.

Implemented:

- app state now has one `focus` object for graph and pick focus
- graph node selection dispatches `focus/set`
- 3D picks dispatch `focus/set`
- empty picks and graph clear paths dispatch `focus/clear`
- property balloon open/close/dismiss and anchor placement dispatch app-state actions

Remaining:

- graph layout and Sigma visual selected-node state still live locally in `createGraphShell`
- renderer highlight remains an imperative effect from graph selection
- property balloon content is still populated directly from graph/pick query results

Introduce one app-level semantic focus object while keeping 3D highlight in the renderer.

Focus inputs:

- 3D pick
- graph node click
- AI graph/select actions
- explicit console/API selection
- click outside selection targets

Rules:

- renderer owns 3D selected/highlighted element IDs
- graph owns graph layout and graph visual selected node rendering
- app state owns the active semantic focus
- property balloon opens only from a surface pick or graph node click when pick is active
- graph node click with rotate-only highlights without opening a balloon
- graph selection should command renderer selection when the graph node maps to a renderable element

Acceptance:

- selecting graph node highlights matching 3D element when renderable
- clicking empty space clears focus and renderer selection
- graph click with pick off does not open balloon
- graph click with pick on opens balloon
- dragging/orbiting is not interrupted by balloon controls

## Slice 5: Make Derived UI Pure Renderers

Refactor widgets to render from snapshots:

- outliner renders from app state plus the committed renderer snapshot; it does not call live
  viewer state APIs while painting rows
- footer renders from `viewer.viewState()`
- header buttons render from app state
- resource picker renders from resource catalog plus committed/pending resource state
- property balloon renders from app focus, anchor, and property query results

Rules:

- derived UI must not store visibility, resource, or selection truth
- if derived UI needs a change, it dispatches an action or renderer command
- direct DOM event handlers may dispatch commands, but should not update sibling widgets directly

Acceptance:

- suppressing/unsuppressing via console updates outliner counts
- hiding/showing via console or AI leaves file-level outliner enablement intact
- outliner toggles update footer visible counts
- resource switching updates footer, outliner, graph, and AI from one committed state path
- no component calls another component's `render()` directly except through the app-state subscriber path

## Slice 6: Renderer Event Consistency

Status: mostly implemented.

Make renderer state commits consistent.

Options:

1. Preferred: Rust dispatches `w-viewer-state-change` after every committed renderer mutation.
2. Acceptable interim: all JS-facing renderer commands go through `window.wViewer`, and `wViewer`
   dispatches after every successful command.

Do not mix untracked renderer mutation paths.

Renderer mutations to cover:

- resource load
- start view change
- hide/show/reset visibility
- suppress/unsuppress
- select/clear selection
- pick if it changes renderer selection
- stream-visible completion if residency/UI stats change

Acceptance:

- every renderer mutation path has one state event
- no duplicate state events for one logical command
- console APIs still return useful immediate results
- footer/outliner update after streaming and visibility changes

## Slice 7: Test And Smoke Harness

Status: partly implemented.

Implemented:

- JS smoke harness added at `crates/cc-w-platform-web/web/smoke/state-smoke.mjs`
- harness covers resource commit, outliner project rows, and suppression persistence

Remaining:

- run the smoke harness against a live viewer once Playwright is installed/resolvable in the local
  Node environment
- add graph selection/balloon browser smoke once the selection flow is fully centralized

Add or preserve tests at each layer.

Rust:

- visibility/suppression separation
- start-view switching preserves user overrides
- source-scoped project IDs remain stable
- resource commit snapshot includes expected view-state fields

JavaScript/browser smoke:

- module syntax parse
- outliner resource-switch smoke
- panel toggle smoke
- graph selection/balloon smoke

Manual smoke script:

1. Open `?resource=ifc/building-architecture`.
2. Open outliner.
3. Switch to `project/building`.
4. Confirm all project IFC assets appear after load.
5. Toggle one IFC off, then on.
6. Hide one element by console/API.
7. Toggle its IFC off, then on.
8. Confirm the element remains hidden.
9. Open graph, select a node, verify 3D highlight.
10. Turn pick off, click graph node, verify no balloon.
11. Turn pick on, click graph node, verify balloon.
12. Rotate while balloon is visible, verify no focus steal.

## Implementation Order

Recommended order:

1. App-state store scaffold.
2. Resource switching through app state.
3. Panel/tool state through app state.
4. Focus/selection state through app state.
5. Derived UI cleanup.
6. Renderer event consistency cleanup.
7. Browser smoke harness.

This order reduces risk because resource switching is the most visible source of stale state, while
selection/focus cleanup touches more UI behavior and should happen after the store is stable.

## Definition Of Done

We can consider the design fully followed when:

- every mutable state domain has a named owner
- all resource switching is commit-driven
- no UI component stores renderer truth locally
- no UI component renders from raw picker values as authoritative state
- all renderer mutations produce one committed state event
- outliner, footer, graph, terminal, and balloons can all be explained as renderers of app state,
  renderer state, or database query results
- the manual smoke script passes
