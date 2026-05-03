import { tryGetFirst } from "../util/object.js";

export function agentSemanticIds(action) {
  const candidates = tryGetFirst(action, [
    "semanticIds",
    "semantic_ids",
    "ids",
    "elementIds",
    "element_ids",
  ]);
  if (!Array.isArray(candidates)) {
    return [];
  }
  return Array.from(
    new Set(
      candidates
        .map((value) => String(value || "").trim())
        .filter(Boolean)
    )
  );
}

export function agentDbNodeIds(action) {
  const candidates = tryGetFirst(action, [
    "dbNodeIds",
    "db_node_ids",
    "dbNodeId",
    "db_node_id",
    "nodeIds",
    "node_ids",
    "nodeId",
    "id",
    "ids",
  ]);
  const values = Array.isArray(candidates) ? candidates : candidates === undefined ? [] : [candidates];
  if (!values.length) {
    return [];
  }
  return Array.from(
    new Set(
      values
        .map((value) => Number.parseInt(String(value ?? ""), 10))
        .filter((value) => Number.isFinite(value))
    )
  );
}

function agentPathAnnotation(action, resource) {
  const path = tryGetFirst(action, ["path"]);
  if (!path || typeof path !== "object" || Array.isArray(path)) {
    return null;
  }
  const annotation = {
    resource,
    path,
  };
  for (const [target, keys] of [
    ["line", ["line", "line_range", "lineRange", "line_ranges", "lineRanges", "ranges"]],
    ["markers", ["markers", "marker_groups", "markerGroups"]],
    ["mode", ["mode", "operation", "update", "behavior"]],
    ["max_samples", ["max_samples", "maxSamples"]],
  ]) {
    const value = tryGetFirst(action, keys);
    if (value !== undefined && value !== null && value !== "") {
      annotation[target] = value;
    }
  }
  return annotation;
}

export function createAgentActionApplier({
  viewer,
  graph,
  isKnownResource,
  isIfcResource,
  safeViewerCurrentResource,
  mapGraphSubgraphResponse,
  revealGraphPanel,
}) {
  const currentResource = () => safeViewerCurrentResource(viewer);

  const agentActionResource = (action) => {
    const raw = tryGetFirst(action, [
      "resource",
      "sourceResource",
      "source_resource",
      "ifcResource",
    ]);
    const resource = raw === null ? "" : String(raw).trim();
    return isKnownResource(resource) ? resource : currentResource();
  };

  const ensureGraphPanelVisible = async () => {
    if (typeof revealGraphPanel === "function") {
      await revealGraphPanel();
    }
  };

  const apply = async (action) => {
    if (!action || typeof action !== "object") {
      return null;
    }
    const kind = String(tryGetFirst(action, ["kind", "type", "name"]) || "").toLowerCase();
    if (!kind) {
      return null;
    }

    if (kind === "graph.set_seeds" || kind === "graph.setseeds") {
      const dbNodeIds = agentDbNodeIds(action);
      const resource = agentActionResource(action);
      if (!dbNodeIds.length || !isIfcResource(resource)) {
        return null;
      }
      await ensureGraphPanelVisible();
      const snapshot = await graph.snapshot();
      const payload = await viewer.queryGraphSubgraph(
        dbNodeIds,
        {
          hops: 1,
          maxNodes: 120,
          maxEdges: 240,
          mode: snapshot?.mode || "semantic",
        },
        resource
      );
      graph.setData(
        mapGraphSubgraphResponse(payload, {
          status:
            tryGetFirst(action, ["status", "message"]) ||
            `Graph set from AI with ${dbNodeIds.length} seed node${dbNodeIds.length === 1 ? "" : "s"} in ${resource}${payload?.truncated ? " (truncated)" : ""}.`,
        })
      );
      return "graph.set_seeds";
    }

    if (kind === "graph.expand") {
      const dbNodeIds = agentDbNodeIds(action);
      const resource = agentActionResource(action);
      if (!dbNodeIds.length || !isIfcResource(resource)) {
        return null;
      }
      await ensureGraphPanelVisible();
      const snapshot = await graph.snapshot();
      const payload = await viewer.queryGraphSubgraph(
        dbNodeIds,
        {
          hops: 1,
          maxNodes: 120,
          maxEdges: 240,
          mode: snapshot?.mode || "semantic",
        },
        resource
      );
      graph.mergeData(
        mapGraphSubgraphResponse(payload, {
          selectedNodeId: String(dbNodeIds[0]),
          status:
            tryGetFirst(action, ["status", "message"]) ||
            `Graph expanded from AI around ${dbNodeIds.length} node${dbNodeIds.length === 1 ? "" : "s"} in ${resource}${payload?.truncated ? " (truncated)" : ""}.`,
        })
      );
      return "graph.expand";
    }

    if (kind === "properties.show_node" || kind === "properties.shownode") {
      const dbNodeIds = agentDbNodeIds(action);
      const resource = agentActionResource(action);
      if (!dbNodeIds.length || !isIfcResource(resource)) {
        return null;
      }
      const dbNodeId = dbNodeIds[0];
      let selected = graph.setSelectedNode(String(dbNodeId), { revealProperties: true });
      if (!selected) {
        const snapshot = await graph.snapshot();
        const payload = await viewer.queryGraphSubgraph(
          [dbNodeId],
          {
            hops: 1,
            maxNodes: 32,
            maxEdges: 64,
            mode: snapshot?.mode || "semantic",
          },
          resource
        );
        graph.mergeData(
          mapGraphSubgraphResponse(payload, {
            selectedNodeId: String(dbNodeId),
            status:
              tryGetFirst(action, ["status", "message"]) ||
              `Graph loaded around node ${dbNodeId} for property inspection in ${resource}${payload?.truncated ? " (truncated)" : ""}.`,
          })
        );
      }
      const details = await viewer.queryGraphNodeProperties(dbNodeId, {}, resource);
      graph.setNodeDetails(dbNodeId, {
        properties: {
          extraProperties: details?.properties || {},
        },
        relations: Array.isArray(details?.relations)
          ? details.relations.map((relation) => {
              const other = relation?.other || {};
              return {
                type: relation.relationshipType || relation.type || "RELATION",
                target: String(
                  tryGetFirst(other, ["dbNodeId", "db_node_id", "id"]) || ""
                ),
                targetLabel:
                  tryGetFirst(other, [
                    "displayLabel",
                    "display_label",
                    "name",
                    "declaredEntity",
                    "declared_entity",
                  ]) || "",
                description: relation.direction || "",
              };
            })
          : [],
      });
      graph.setSelectedNode(String(dbNodeId), { revealProperties: true });
      return "properties.show_node";
    }

    if (kind === "elements.hide") {
      const semanticIds = agentSemanticIds(action);
      if (!semanticIds.length) {
        return null;
      }
      viewer.hide(semanticIds, { sourceResource: agentActionResource(action) });
      return "elements.hide";
    }

    if (kind === "elements.show") {
      const semanticIds = agentSemanticIds(action);
      if (!semanticIds.length) {
        return null;
      }
      viewer.show(semanticIds, { sourceResource: agentActionResource(action) });
      return "elements.show";
    }

    if (kind === "elements.set_visible" || kind === "elements.setvisible") {
      const semanticIds = agentSemanticIds(action);
      if (!semanticIds.length || typeof viewer.setVisible !== "function") {
        return null;
      }
      viewer.setVisible(semanticIds, Boolean(action.visible), {
        sourceResource: agentActionResource(action),
      });
      return "elements.set_visible";
    }

    if (kind === "elements.select") {
      const semanticIds = agentSemanticIds(action);
      if (!semanticIds.length) {
        return null;
      }
      viewer.select(semanticIds, { sourceResource: agentActionResource(action) });
      return "elements.select";
    }

    if (kind === "elements.inspect") {
      const semanticIds = agentSemanticIds(action);
      if (!semanticIds.length) {
        return null;
      }
      viewer.inspect(semanticIds, {
        sourceResource: agentActionResource(action),
        mode: tryGetFirst(action, ["mode", "inspection_mode", "inspectionMode"]) || "replace",
      });
      return "elements.inspect";
    }

    if (kind === "viewer.frame_visible" || kind === "viewer.framevisible") {
      viewer.frameVisible();
      return "viewer.frame_visible";
    }

    if (kind === "viewer.clear_inspection" || kind === "viewer.clearinspection") {
      viewer.clearInspection();
      return "viewer.clear_inspection";
    }

    if (
      kind === "viewer.reset_default_view" ||
      kind === "viewer.resetdefaultview" ||
      kind === "reset_default_view"
    ) {
      if (typeof viewer.resetDefaultView === "function") {
        viewer.resetDefaultView();
      } else {
        viewer.clearInspection();
        viewer.resetAllVisibility();
        viewer.defaultView();
      }
      return "viewer.reset_default_view";
    }

    if (
      kind === "viewer.section.set" ||
      kind === "viewer.sectionset" ||
      kind === "section.set" ||
      kind === "sectionset"
    ) {
      const section = tryGetFirst(action, ["section", "spec"]);
      if (!section || typeof section !== "object" || Array.isArray(section)) {
        return null;
      }
      viewer.section.set(section);
      return "viewer.section.set";
    }

    if (
      kind === "viewer.section.clear" ||
      kind === "viewer.sectionclear" ||
      kind === "section.clear" ||
      kind === "sectionclear"
    ) {
      viewer.section.clear();
      return "viewer.section.clear";
    }

    if (kind === "viewer.annotations.show_path" || kind === "viewer.annotations.showpath") {
      const resource = agentActionResource(action);
      if (!isIfcResource(resource)) {
        return null;
      }
      const annotation = agentPathAnnotation(action, resource);
      if (!annotation) {
        return null;
      }
      const annotations = viewer.annotations;
      if (typeof annotations?.showPath === "function") {
        await annotations.showPath(annotation);
        return "viewer.annotations.show_path";
      }
      if (typeof annotations?.show_path === "function") {
        await annotations.show_path(annotation);
        return "viewer.annotations.show_path";
      }
      return null;
    }

    if (
      kind === "viewer.annotations.clear" ||
      kind === "viewer.annotationsclear" ||
      kind === "annotations.clear"
    ) {
      const resource = agentActionResource(action);
      const annotations = viewer.annotations;
      if (typeof annotations?.clear === "function") {
        annotations.clear({ resource });
        return "viewer.annotations.clear";
      }
      return null;
    }

    return null;
  };

  return { apply };
}
