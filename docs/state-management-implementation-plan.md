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
- graph shell ownership is extracted to `web/js/graph/graph-shell.js`
- property balloon content/query orchestration is extracted to
  `web/js/ui/balloon-controller.js` and `web/js/semantic/properties.js`
- resource and render-profile picker rendering are extracted to dedicated viewer controllers

Remaining gaps:

- some derived UI still reacts directly to DOM or viewer events instead of a single app-state
  subscriber path

The main risk is state drift: two widgets can believe different things because they reacted to
different events.

## Post-Refactor Module Ownership Target

The end state should make each extracted module's ownership narrow enough that it can be reasoned
about without opening the full app entrypoint.

### Graph Shell

Target owner: a graph-shell/controller module under the graph UI boundary.

Owns:

- Graphology/Sigma instance lifetime
- graph layout positions, camera state, and visual selected-node styling
- graph command surface such as `reset`, `expand`, `select`, `clear`, and `frame`
- graph status text that describes graph-controller readiness or query failures

Does not own:

- panel visibility, which remains `appState.panels.graph`
- semantic focus, which remains `appState.focus`
- renderer selection/highlight truth, which remains the renderer committed state
- graph/database facts, which remain backend query results
- property balloon open state or content

Inputs:

- committed resource changes from `viewer/committed`
- app-state focus snapshots
- backend graph query results
- explicit terminal or AI graph commands

Outputs:

- app-state `focus/set` or `focus/clear` actions for semantic focus changes
- renderer selection commands only when a graph node maps to a renderable semantic id
- graph-local render updates for layout and visual node selection

Graph clicks must not directly update sibling widgets. A click can set focus, optionally command
renderer selection, and let subscribers decide what to render.

### Balloon Controller

Target owner: `js/ui/balloon.js`.

Owns:

- property balloon DOM lookup
- screen positioning, arrow placement, and pick-anchor marker placement
- close/dismiss button wiring
- rendering the open/closed/anchored visual state from app state
- rendering an already prepared property view into the balloon DOM

Does not own:

- semantic focus selection
- pick-tool policy
- property query/fetch ownership
- renderer selection

Inputs:

- `appState.balloon` for `open`, `source`, `anchor`, and `dismissed`
- focus-keyed property query results from the semantic/data owner
- viewport/canvas measurements for positioning

Outputs:

- `balloon/open`, `balloon/close`, `balloon/dismiss`, and `balloon/anchor` actions
- DOM positioning side effects only inside the balloon/marker elements

Content requests should be keyed to the current focus/resource. If a stale property query resolves
after focus changes, it must be ignored instead of replacing the current balloon content.

### Resource And Profile Pickers

Target owners: small picker controllers, separate from graph, outliner, terminal, and app entrypoint
orchestration.

Resource picker contract:

- the Rust resource catalog owns available resources and projects
- app state owns pending `requestedResource` intent
- renderer committed state owns the active loaded resource
- the picker renders options from the catalog and selected/pending state from app and renderer
  snapshots
- `change` dispatches `resource/requested` and invokes the renderer load path
- graph clearing, outliner rendering, and chat binding must wait for `viewer/committed`

Profile picker contract:

- the renderer owns available profiles and the committed active profile
- the picker may record pending profile intent only as UI intent, not as renderer truth
- `change` invokes the renderer profile command
- selected profile display updates from the committed renderer/profile snapshot
- a profile mutation that changes renderer state must emit one committed viewer-state event

Raw picker values are never authoritative outside the picker controller. They are intent signals
until the owning state domain commits.

### Chat Readiness

Target owner: an AI/chat readiness controller near the terminal/agent boundary.

Owns:

- chat readiness derivation and status text
- AI model/level control enablement
- active chat session binding for the committed resource/project
- stale-session invalidation when committed resource or profile changes

Does not own:

- resource or profile truth
- terminal panel visibility or active terminal tab
- renderer state
- backend semantic/database facts

Inputs:

- `appState.terminal.activeTool` and terminal panel visibility
- committed viewer resource/profile snapshots
- resource catalog project membership
- backend agent capability/session state
- selected AI model and level

Outputs:

- readiness UI state such as disabled controls and status copy
- one session bind/update after the viewer commits the resource it will answer against
- no chat bind or prompt dispatch for a pending picker value

When a resource/profile change is pending, chat should report a pending/not-ready state instead of
answering against the previous committed context or the raw picker value.

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

Status: implemented for graph clearing, AI binding, and resource picker rendering.

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

Status: implemented for focus ownership; graph layout remains intentionally owned by the graph
shell module.

Implemented:

- app state now has one `focus` object for graph and pick focus
- graph node selection dispatches `focus/set`
- 3D picks dispatch `focus/set`
- empty picks and graph clear paths dispatch `focus/clear`
- property balloon open/close/dismiss and anchor placement dispatch app-state actions
- property balloon content is populated by the balloon controller from app focus plus semantic
  readback results
- renderer highlight remains a renderer-owned effect commanded by graph selection

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
- graph layout and graph visual selection render from the graph shell's graph snapshot; app state
  owns semantic focus, not graph coordinates

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
- no-browser state contract smoke added at
  `crates/cc-w-platform-web/web/smoke/state-contract-smoke.mjs`
- no-browser smoke covers app-state dispatch semantics, resource catalog helpers, balloon helper
  behavior, stable DOM anchors, and this plan's post-refactor ownership headings

Remaining:

- run the smoke harness against a live viewer once Playwright is installed/resolvable in the local
  Node environment
- add graph selection/balloon browser smoke to cover the moduleized controller path

Add or preserve tests at each layer.

Rust:

- visibility/suppression separation
- start-view switching preserves user overrides
- source-scoped project IDs remain stable
- resource commit snapshot includes expected view-state fields

JavaScript/browser smoke:

- module syntax parse
- no-browser state-contract smoke
- outliner resource-switch smoke
- panel toggle smoke
- resource/profile picker smoke
- chat readiness smoke
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
13. Change render profile and verify footer/viewer state update from one committed state event.
14. During resource/profile pending state, verify AI chat controls are not ready for the raw picker
    context.
15. After commit, verify chat readiness binds to the committed resource/project once.

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
- resource and profile pickers render from catalog/app/renderer snapshots rather than local truth
- chat readiness follows committed resource/profile state rather than raw picker values
- no UI component stores renderer truth locally
- no UI component renders from raw picker values as authoritative state
- all renderer mutations produce one committed state event
- outliner, footer, graph, terminal, and balloons can all be explained as renderers of app state,
  renderer state, or database query results
- the manual smoke script passes
