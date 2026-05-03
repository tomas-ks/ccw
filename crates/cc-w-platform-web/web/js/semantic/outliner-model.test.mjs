import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_SEMANTIC_OUTLINER_FACETS,
  normalizeSemanticOutliner,
  semanticGroupDeclaredCount,
  semanticGroupInspectionOperation,
  semanticGroupInspectionState,
  semanticGroupOutlinerState,
  semanticGroupVisibilityOperation,
  semanticGroupViewerBuckets,
} from "./outliner-model.mjs";

test("default semantic facets start with workspace and omit IFC project", () => {
  assert.equal(DEFAULT_SEMANTIC_OUTLINER_FACETS[0].id, "workspace");
  assert.equal(DEFAULT_SEMANTIC_OUTLINER_FACETS[0].label, "Workspace");
  assert.equal(
    DEFAULT_SEMANTIC_OUTLINER_FACETS.some((facet) => facet.id === "project"),
    false
  );
});

test("normalizes backend semantic groups with provenance and element counts", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/ifcroad-wg",
    facets: [
      {
        id: "layers",
        label: "Layers",
        groups: [
          {
            id: "layers:triangoli-post-operam",
            label: "TRIANGOLI - post operam",
            provenance: "ifc_graph",
            elementCount: 1,
            semanticIds: ["0wcS47SZn6nw7tZqMTZ0u$"],
          },
        ],
      },
    ],
  });

  const group = outliner.facets.find((facet) => facet.id === "layers").groups[0];
  assert.equal(group.sourceKind, "ifc_graph");
  assert.equal(group.sourceDetail, "ifc_graph");
  assert.equal(semanticGroupDeclaredCount(group), 1);
  assert.deepEqual(group.semanticIds, ["0wcS47SZn6nw7tZqMTZ0u$"]);
});

test("keeps facet diagnostics for empty semantic facets", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/ifcroad-wg",
    facets: [
      {
        id: "materials",
        diagnostics: [
          {
            severity: "info",
            code: "no_material_groups",
            message: "No IfcRelAssociatesMaterial product groups were found.",
          },
        ],
        groups: [],
      },
    ],
  });

  const materials = outliner.facets.find((facet) => facet.id === "materials");
  assert.deepEqual(materials.diagnostics, [
    "No IfcRelAssociatesMaterial product groups were found.",
  ]);
});

test("semantic group state respects project-scoped ids and visibility", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "project/roads",
    facets: [
      {
        id: "classes",
        groups: [
          {
            id: "classes:ifccourse",
            label: "IfcCourse",
            sourceResource: "ifc/road-a",
            semanticIds: ["course-a", "ifc/road-b::course-b"],
          },
        ],
      },
    ],
  });
  const group = outliner.facets.find((facet) => facet.id === "classes").groups[0];
  const state = semanticGroupOutlinerState(
    group,
    {
      listElementIds: ["ifc/road-a::course-a", "ifc/road-b::course-b"],
      visibleElementIds: ["ifc/road-a::course-a"],
    },
    "project/roads"
  );

  assert.deepEqual(state.ids, ["ifc/road-a::course-a", "ifc/road-b::course-b"]);
  assert.equal(state.checked, true);
  assert.equal(state.indeterminate, true);
  assert.equal(state.enabledCount, 1);
});

test("semantic group state can toggle non-default renderable classes", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/building-architecture",
    facets: [
      {
        id: "classes",
        groups: [
          {
            id: "classes:ifcbuildingelementproxy",
            label: "IfcBuildingElementProxy",
            semanticIds: ["helper-a"],
          },
        ],
      },
    ],
  });
  const group = outliner.facets.find((facet) => facet.id === "classes").groups[0];
  const state = semanticGroupOutlinerState(
    group,
    {
      listElementIds: ["helper-a"],
      defaultElementIds: [],
      visibleElementIds: [],
    },
    "ifc/building-architecture"
  );

  assert.deepEqual(state.ids, ["helper-a"]);
  assert.equal(state.disabled, false);
  assert.equal(state.checked, false);
  assert.equal(state.enabledCount, 0);
});

test("semantic container primary state controls default descendants only", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/building-architecture",
    facets: [
      {
        id: "classes",
        groups: [
          {
            id: "classes:ifcbuilding",
            label: "IfcBuilding",
            semanticIds: ["wall-a", "zone-a"],
          },
        ],
      },
    ],
  });
  const group = outliner.facets.find((facet) => facet.id === "classes").groups[0];
  const viewState = {
    listElementIds: ["wall-a", "zone-a"],
    defaultElementIds: ["wall-a"],
    visibleElementIds: ["wall-a"],
  };

  const buckets = semanticGroupViewerBuckets(
    group,
    viewState,
    "ifc/building-architecture"
  );
  assert.deepEqual(buckets.defaultIds, ["wall-a"]);
  assert.deepEqual(buckets.hiddenIds, ["zone-a"]);

  const primary = semanticGroupOutlinerState(
    group,
    viewState,
    "ifc/building-architecture"
  );
  assert.deepEqual(primary.ids, ["wall-a"]);
  assert.equal(primary.bucket, "default");
  assert.equal(primary.checked, true);
  assert.equal(primary.allEnabledCount, 1);
  assert.equal(primary.allTotalCount, 2);
  assert.equal(primary.hiddenCount, 1);

  const hidden = semanticGroupOutlinerState(
    group,
    viewState,
    "ifc/building-architecture",
    { bucket: "hidden" }
  );
  assert.deepEqual(hidden.ids, ["zone-a"]);
  assert.equal(hidden.bucket, "hidden");
  assert.equal(hidden.checked, false);
  assert.equal(hidden.allEnabledCount, 1);
  assert.equal(hidden.allTotalCount, 2);
});

test("semantic visibility operation keeps default and hidden buckets distinct", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/building-architecture",
    facets: [
      {
        id: "classes",
        groups: [
          {
            id: "classes:ifcbuilding",
            label: "IfcBuilding",
            semanticIds: ["wall-a", "zone-a"],
          },
        ],
      },
    ],
  });
  const group = outliner.facets.find((facet) => facet.id === "classes").groups[0];
  const viewState = {
    listElementIds: ["wall-a", "zone-a"],
    defaultElementIds: ["wall-a"],
    visibleElementIds: ["wall-a"],
  };

  assert.deepEqual(
    semanticGroupVisibilityOperation(
      group,
      viewState,
      "ifc/building-architecture",
      false
    ),
    {
      action: "hide",
      ids: ["wall-a"],
      state: semanticGroupOutlinerState(
        group,
        viewState,
        "ifc/building-architecture"
      ),
    }
  );

  const restoreDefault = semanticGroupVisibilityOperation(
    group,
    viewState,
    "ifc/building-architecture",
    true
  );
  assert.equal(restoreDefault.action, "reset");
  assert.deepEqual(restoreDefault.ids, ["wall-a"]);

  const showHidden = semanticGroupVisibilityOperation(
    group,
    viewState,
    "ifc/building-architecture",
    true,
    { bucket: "hidden" }
  );
  assert.equal(showHidden.action, "reveal");
  assert.deepEqual(showHidden.ids, ["zone-a"]);

  const resetHidden = semanticGroupVisibilityOperation(
    group,
    viewState,
    "ifc/building-architecture",
    false,
    { bucket: "hidden" }
  );
  assert.equal(resetHidden.action, "reset");
  assert.deepEqual(resetHidden.ids, ["zone-a"]);
});

test("semantic inspection state is independent from visibility state", () => {
  const outliner = normalizeSemanticOutliner({
    resource: "ifc/building-architecture",
    facets: [
      {
        id: "classes",
        groups: [
          {
            id: "classes:ifcbuilding",
            label: "IfcBuilding",
            semanticIds: ["wall-a", "zone-a"],
          },
        ],
      },
    ],
  });
  const group = outliner.facets.find((facet) => facet.id === "classes").groups[0];
  const viewState = {
    listElementIds: ["wall-a", "zone-a"],
    defaultElementIds: ["wall-a"],
    visibleElementIds: ["wall-a"],
    inspectedElementIds: ["zone-a"],
  };

  const defaultInspection = semanticGroupInspectionState(
    group,
    viewState,
    "ifc/building-architecture"
  );
  assert.deepEqual(defaultInspection.ids, ["wall-a"]);
  assert.equal(defaultInspection.checked, false);

  const hiddenInspection = semanticGroupInspectionState(
    group,
    viewState,
    "ifc/building-architecture",
    { bucket: "hidden" }
  );
  assert.deepEqual(hiddenInspection.ids, ["zone-a"]);
  assert.equal(hiddenInspection.checked, true);

  const addDefault = semanticGroupInspectionOperation(
    group,
    viewState,
    "ifc/building-architecture",
    true
  );
  assert.equal(addDefault.action, "add");
  assert.deepEqual(addDefault.ids, ["wall-a"]);

  const removeHidden = semanticGroupInspectionOperation(
    group,
    viewState,
    "ifc/building-architecture",
    false,
    { bucket: "hidden" }
  );
  assert.equal(removeHidden.action, "remove");
  assert.deepEqual(removeHidden.ids, ["zone-a"]);
});
