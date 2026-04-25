# State Management Contract

## Purpose

The web viewer, graph explorer, AI terminal, and renderer all react to the same project. They must
not each keep their own competing version of the truth.

This document defines the state ownership model for the app. The rule is:

**One source of truth per domain, with derived UI rendering from snapshots.**

There should not be one global mega-state object. Instead, each kind of state has one owner. Other
parts of the app send commands to that owner and render from the committed snapshot it publishes.

The cleanup plan for bringing the current web viewer fully in line with this contract lives in
[state-management-implementation-plan.md](./state-management-implementation-plan.md).

## State Domains

### App State

App state owns user and product intent. In the web viewer this currently lives in JavaScript.

Examples:

- active resource or project requested by the user
- panel visibility: graph, terminal, outliner
- active tools: rotate, pick
- terminal tab: AI or JS
- graph viewport open/closed
- property balloon open/closed
- balloon anchor source and screen placement
- graph layout state
- selected graph node or active semantic focus
- active AI session binding
- panel sizes and other UI-only preferences

App state does not own GPU draw state, geometry residency, IFC properties, or semantic graph truth.

### Rendering State

Rendering state owns scene truth for the active renderer. This belongs in Rust, primarily
`cc-w-runtime`, with platform entrypoints in `cc-w-platform-web` and `cc-w-platform-native`.

Examples:

- loaded resource or project package
- start view request: default, all, minimal, or explicit element set
- default visible element set derived from the loaded package
- current visible element set
- visibility overrides: hidden and shown
- outliner/file-level suppression
- selected elements for 3D highlight
- resident geometry instances and definitions
- missing stream plans
- pickable/renderable instance IDs
- camera framing targets derived from renderable bounds

JavaScript may call renderer commands and read renderer snapshots. It must not duplicate renderer
truth in local UI state.

### Database State

Database state owns semantic truth. IFC properties, relationships, classifications, and graph
topology live in Velr/IFC databases and are queried through backend APIs.

Examples:

- IFC entity properties
- semantic graph relationships
- project/IFC source membership
- Cypher query results
- graph expansion neighborhoods
- property balloon data fetched after a pick

The renderer should carry stable references into this state, not mirror the full database.

### Derived UI State

Derived UI is not a source of truth. It renders from app state, renderer state, and database query
results.

Examples:

- outliner checkbox state
- footer triangle/draw/visible counts
- graph panel summaries
- property balloon labels and property rows
- tool button active styling
- model/resource picker selected option

If a derived UI component needs to change state, it dispatches a command to the owner. It does not
mutate a private copy and hope the rest of the app catches up.

## Resource Switching Contract

Resource switching is asynchronous and must be commit-driven.

Flow:

1. The user chooses a resource or project.
2. App state records a pending resource intent.
3. The Rust viewer begins loading the requested resource.
4. The Rust viewer commits the loaded `RuntimeSceneState`.
5. The Rust viewer emits a committed `w-viewer-state-change` event.
6. Derived UI rerenders from the committed renderer snapshot.

The outliner, footer, graph, and AI binding must not treat the picker value alone as proof that the
renderer has switched. The picker can change before the runtime scene is ready.

## Visibility Contract

Visibility has layered ownership:

- start view owns the base/default visible set
- user and AI element actions own explicit `hidden` / `shown` overrides
- the outliner owns file-level `suppressed` state
- the renderer computes the final visible set

Final visibility is renderer-owned and should be read from `viewer.viewState().visibleElementIds`.

Outliner rules:

- the outliner lists project IFC assets
- the outliner acts only on each IFC member's default-view elements
- toggling a file off suppresses those default-view elements
- toggling a file on clears suppression only
- toggling a file on must not force-show elements hidden by user or AI actions
- checkbox state is derived from suppression on the default-view elements, not from element-level
  hidden/shown overrides

This is why outliner state belongs in the renderer as a visibility layer, not as a private browser
list of hidden files.

## Selection Contract

Selection has one current semantic focus at a time unless a feature explicitly introduces
multi-selection.

Current intended behavior:

- picking a 3D surface selects the corresponding semantic element
- selecting a graph node selects/highlights the corresponding 3D element when it is renderable
- clicking outside selection surfaces clears selection
- graph node clicks may show a property balloon only when the pick tool is active
- rotate/orbit behavior must not be blocked by graph or balloon state

The renderer owns 3D highlight state. The graph owns graph layout and graph visual selection. App
state coordinates the active focus so both can reflect the same user intent.

## Event Contract

Commands should flow to owners. Events should announce committed state.

Recommended events:

- `w-viewer-state-change`: renderer state committed
- `w-viewer-pick`: a pick operation completed
- `w-viewer-anchor`: projected anchor changed for a visible picked element
- `w-resource-catalog-change`: available resources/projects changed
- `w-terminal-visibility-change`: terminal panel visibility changed

Do not use raw DOM control events as authoritative state. A `change` event on a picker is only an
intent signal until the viewer state commit arrives.

Renderer event delivery rule:

- Rust renderer commands mutate renderer state first
- they build any viewer-state or anchor event payloads while they still own the state snapshot
- they release the mutable wasm state borrow
- only then do they dispatch DOM events

This keeps event listeners free to call `viewer.viewState()`, `viewer.currentResource()`, or other
read-only viewer APIs without reentering a mutably borrowed Rust state cell.

## Current Web Mapping

Current implementation shape:

- `cc-w-runtime::RuntimeSceneState` owns render state
- `cc-w-platform-web` exposes renderer commands and snapshots through wasm exports
- `window.wViewer` is the JavaScript bridge to those exports
- `window.wViewer.resourceCatalog()` exposes the resource/project catalog that Rust loaded at boot
- `window.wAppState` owns web UI intent: panels, tools, pending resource, focus, and balloon anchor
- `viewer.viewState()` is the renderer snapshot used by derived UI; it includes stable
  `defaultElementIds` plus committed visible/hidden/suppressed sets
- the resource picker requests resource changes; it is not the source of truth
- the project outliner uses explicit project membership from the resource catalog
- the outliner renders from the committed app-state copy of `viewer.viewState()`
- the outliner mutates renderer suppression through `viewer.suppress()` / `viewer.unsuppress()`
- AI element actions and JS shell helpers both use the same `window.wViewer` element action API
- the graph panel fetches DB neighborhoods and renders them separately from the renderer scene
- property balloon visibility and anchor placement come from app state
- property balloon content is fetched from semantic DB properties after picks or graph interactions

## Anti-Patterns

Avoid:

- rendering UI directly from pending picker values after an async resource switch
- storing duplicate visibility state in the outliner
- force-showing an element to undo a higher-level UI toggle
- letting graph state imply renderer state without sending a renderer command
- letting renderer state imply database truth beyond stable IDs and bounds
- using terminal/API helpers that mutate JS-only state without calling the owning Rust/API layer

## Testing Expectations

When touching state flow, test at least one path in each affected domain:

- renderer unit test for visibility, selection, or residency rules
- web lib test when wasm-facing state shape changes
- JS syntax/build check when browser bridge code changes
- manual smoke path for async resource switching when the change affects picker/project/outliner

Useful manual smoke for project state:

1. Start on `ifc/building-architecture`.
2. Open the outliner.
3. Switch to `project/building`.
4. Confirm the outliner waits for the project state and lists all project IFC files.
5. Hide one IFC file from the outliner.
6. Hide one element via API or AI.
7. Re-enable the IFC file from the outliner.
8. Confirm the API/AI-hidden element remains hidden.
