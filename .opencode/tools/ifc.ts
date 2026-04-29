import { tool } from "@opencode-ai/plugin";
import { spawnSync } from "node:child_process";
import {
  ifcBridgeStructureSummary,
  ifcElementSearch,
} from "../tool-helpers/ifc_search_helpers.ts";
import {
  ifcScopeInspect,
  ifcScopeSummary,
} from "../tool-helpers/ifc_scope_helpers.ts";
import {
  ifcQuantityTakeoff,
  ifcSectionAtPointOrStation,
} from "../tool-helpers/ifc_quantity_section_helpers.ts";

type ToolContext = {
  worktree?: string;
  directory?: string;
};

const DEFAULT_SCHEMA = "IFC4X3_ADD2";
const DEFAULT_API_BASE = "http://127.0.0.1:8001";
const DEFAULT_VIEWER_TOOL_TIMEOUT_MS = 35_000;
const FALLBACK_API_BASES = ["http://127.0.0.1:8001", "http://localhost:8001"];

function validViewerApiBase(value?: string): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  try {
    const url = new URL(trimmed);
    if (url.protocol === "http:" || url.protocol === "https:") {
      return url.toString();
    }
  } catch {
    // ignore invalid or relative values
  }
  return null;
}

function viewerApiBaseCandidates(apiBase?: string): string[] {
  const candidates = [
    validViewerApiBase(process.env.CC_W_VIEWER_API_BASE),
    validViewerApiBase(apiBase),
    ...FALLBACK_API_BASES.map(validViewerApiBase),
  ].filter((value): value is string => Boolean(value));
  return [...new Set(candidates)];
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function viewerToolTimeoutMs(): number {
  const parsed = Number.parseInt(process.env.CC_W_VIEWER_TOOL_TIMEOUT_MS ?? "", 10);
  if (Number.isFinite(parsed) && parsed > 0) {
    return parsed;
  }
  return DEFAULT_VIEWER_TOOL_TIMEOUT_MS;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function countMatches(value: string, pattern: RegExp): number {
  return Array.from(value.matchAll(pattern)).length;
}

function riskyProjectCypherReason(cypher: string): string | null {
  const normalized = cypher.replace(/\s+/g, " ").trim();
  const optionalMatches = countMatches(normalized, /\bOPTIONAL\s+MATCH\b/gi);
  const startsFromProject = /\bMATCH\s*\(\s*\w+\s*:\s*IfcProject\b[^)]*\)/i.test(normalized);
  const aggregatesAcrossBranches = /\b(count|collect)\s*\(/i.test(normalized);

  if (startsFromProject && aggregatesAcrossBranches && optionalMatches >= 3) {
    return [
      "Project overview queries with several independent OPTIONAL MATCH aggregate branches can explode into a Cartesian product.",
      "Use one entity histogram query instead, for example:",
      "MATCH (n) WHERE n.declared_entity IS NOT NULL RETURN n.declared_entity AS entity, count(*) AS count ORDER BY count DESC LIMIT 20",
      "Or split the requested counts into separate small label-first queries.",
    ].join(" ");
  }

  const bridgePartContainmentCount =
    /\bIfcBridge\b/i.test(normalized) &&
    /\bIfcBridgePart\b/i.test(normalized) &&
    /\bIfcRelContainedInSpatialStructure\b/i.test(normalized) &&
    /\b(count|collect)\s*\(/i.test(normalized) &&
    !/\bid\s*\(\s*part\s*\)\s*=/.test(normalized);

  if (bridgePartContainmentCount) {
    return [
      "Bridge-part containment aggregate queries must be anchored per bridge part; the unanchored all-parts shape is known to be slow.",
      "First list bridge part ids with:",
      "MATCH (bridge:IfcBridge)--(:IfcRelAggregates)-->(part:IfcBridgePart) RETURN id(part) AS part_node_id, part.Name AS part_name LIMIT 20",
      "Then query one part id at a time, for example:",
      "MATCH (part:IfcBridgePart)<--(:IfcRelContainedInSpatialStructure)-->(prod) WHERE id(part) = 123 RETURN prod.declared_entity AS entity, count(*) AS count ORDER BY count DESC LIMIT 24",
    ].join(" ");
  }

  return null;
}

function repoRoot(context: ToolContext): string {
  return context.worktree ?? context.directory ?? process.cwd();
}

function runKnowledgeCli(
  context: ToolContext,
  command: string,
  args: string[],
  schema?: string,
): string {
  const finalSchema = (schema ?? DEFAULT_SCHEMA).trim() || DEFAULT_SCHEMA;
  const result = spawnSync(
    "cargo",
    [
      "run",
      "-q",
      "-p",
      "cc-w-platform-web",
      "--bin",
      "ifc-knowledge",
      "--",
      "--schema",
      finalSchema,
      command,
      ...args,
    ],
    {
      cwd: repoRoot(context),
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "pipe"],
    },
  );

  if (result.status !== 0) {
    const stderr = (result.stderr ?? "").toString().trim();
    const stdout = (result.stdout ?? "").toString().trim();
    return JSON.stringify(
      {
        ok: false,
        command,
        schema: finalSchema,
        stderr,
        stdout,
        error: `knowledge cli exited with status ${result.status ?? "unknown"}`,
      },
      null,
      2,
    );
  }

  return (result.stdout ?? "").toString().trim();
}

async function runReadonlyCypher(
  _context: ToolContext,
  resource: string,
  cypher: string,
  why?: string,
  apiBase?: string,
): Promise<string> {
  return postViewerJson(apiBase, "/api/cypher", { resource, cypher, why });
}

async function runProjectReadonlyCypher(
  _context: ToolContext,
  projectResource: string,
  cypher: string,
  why?: string,
  resourceFilter?: string[],
  apiBase?: string,
): Promise<string> {
  const riskyReason = riskyProjectCypherReason(cypher);
  if (riskyReason) {
    return JSON.stringify(
      {
        ok: false,
        tool: "ifc_project_readonly_cypher",
        resource: projectResource,
        error: riskyReason,
        cypher,
      },
      null,
      2,
    );
  }

  return postViewerJson(apiBase, "/api/cypher", {
    resource: projectResource,
    cypher,
    why,
    resourceFilter: resourceFilter ?? [],
  });
}

async function postViewerJson(
  apiBase: string | undefined,
  path: string,
  body: Record<string, unknown>,
): Promise<string> {
  const bases = viewerApiBaseCandidates(apiBase);
  const failures: string[] = [];
  const toolTimeoutMs = viewerToolTimeoutMs();
  const cypherServerTimeoutMs = Math.max(1_000, toolTimeoutMs - 1_000);
  const requestBody =
    path === "/api/cypher" && body.timeoutMs === undefined
      ? { ...body, timeoutMs: cypherServerTimeoutMs }
      : body;
  const deadline = Date.now() + toolTimeoutMs;

  for (let attempt = 0; attempt < 3; attempt += 1) {
    for (const base of bases) {
      const remainingMs = deadline - Date.now();
      if (remainingMs <= 0) {
        return JSON.stringify(
          {
            ok: false,
            path,
            error: `viewer API request timed out after ${toolTimeoutMs} ms`,
            tried: failures,
          },
          null,
          2,
        );
      }

      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), remainingMs);
      try {
        const response = await fetch(new URL(path, base), {
          method: "POST",
          headers: {
            "content-type": "application/json",
          },
          body: JSON.stringify(requestBody),
          signal: controller.signal,
        });

        const text = await response.text();
        if (!response.ok) {
          return JSON.stringify(
            {
              ok: false,
              path,
              base,
              status: response.status,
              error: text,
            },
            null,
            2,
          );
        }

        return text.trim();
      } catch (error) {
        if (isAbortError(error)) {
          return JSON.stringify(
            {
              ok: false,
              path,
              base,
              error: `viewer API request timed out after ${toolTimeoutMs} ms`,
            },
            null,
            2,
          );
        }
        failures.push(`${base}: ${errorMessage(error)}`);
      } finally {
        clearTimeout(timer);
      }
    }
    await sleep(80 * (attempt + 1));
  }

  return JSON.stringify(
    {
      ok: false,
      path,
      error: "viewer API connection failed",
      tried: failures,
    },
    null,
    2,
  );
}

export const schema_context = tool({
  description: "Load IFC schema context for the active model schema.",
  args: {
    schema: tool.schema.string().optional().describe("IFC schema id, for example IFC4X3_ADD2"),
  },
  async execute(args, context) {
    return runKnowledgeCli(context, "schema-context", [], args.schema);
  },
});

export const entity_reference = tool({
  description: "Look up IFC entity guidance from the schema reference bundle.",
  args: {
    schema: tool.schema.string().optional().describe("IFC schema id, for example IFC4X3_ADD2"),
    entity_names: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("One or more IFC entity names"),
  },
  async execute(args, context) {
    const cliArgs = args.entity_names.flatMap((entityName) => ["--entity", entityName]);
    return runKnowledgeCli(context, "entity-reference", cliArgs, args.schema);
  },
});

export const relation_reference = tool({
  description: "Look up IFC relation guidance from the schema reference bundle.",
  args: {
    schema: tool.schema.string().optional().describe("IFC schema id, for example IFC4X3_ADD2"),
    relation_names: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("One or more IFC relation names"),
  },
  async execute(args, context) {
    const cliArgs = args.relation_names.flatMap((relationName) => [
      "--relation",
      relationName,
    ]);
    return runKnowledgeCli(context, "relation-reference", cliArgs, args.schema);
  },
});

export const query_playbook = tool({
  description: "Look up a schema-aware Cypher playbook for an IFC question.",
  args: {
    schema: tool.schema.string().optional().describe("IFC schema id, for example IFC4X3_ADD2"),
    goal: tool.schema.string().describe("Short description of the question or task"),
    entity_names: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC entity names that matter for the question"),
  },
  async execute(args, context) {
    const cliArgs = ["--goal", args.goal, ...args.entity_names.flatMap((entityName) => ["--entity", entityName])];
    return runKnowledgeCli(context, "query-playbook", cliArgs, args.schema);
  },
});

export const readonly_cypher = tool({
  description: "Run a read-only Cypher query against the currently selected IFC model.",
  args: {
    resource: tool.schema.string().describe("Selected IFC resource, for example ifc/building-architecture"),
    cypher: tool.schema.string().describe("Single read-only Cypher statement"),
    why: tool.schema.string().optional().describe("Short reason for the query"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args, context) {
    return runReadonlyCypher(
      context,
      args.resource,
      args.cypher,
      args.why,
      args.api_base,
    );
  },
});

export const project_readonly_cypher = tool({
  description:
    "Run the same read-only Cypher query across every IFC model in the active project, returning source provenance for each row.",
  args: {
    resource: tool.schema.string().describe("Selected project resource, for example project/infra"),
    cypher: tool.schema.string().describe("Single read-only Cypher statement"),
    why: tool.schema.string().optional().describe("Short reason for the query"),
    resource_filter: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC resources within the project to query, for example ifc/infra-bridge"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args, context) {
    return runProjectReadonlyCypher(
      context,
      args.resource,
      args.cypher,
      args.why,
      args.resource_filter,
      args.api_base,
    );
  },
});

export const element_search = tool({
  description:
    "Find IFC elements or candidate products using small label-first searches with source provenance. Use this before broad custom Cypher when looking for named or typed bridge/model content.",
  args: {
    resource: tool.schema.string().describe("Selected IFC or project resource, for example ifc/infra-road or project/infra"),
    text: tool.schema.string().optional().describe("Optional free-text term to look for in name, object type, description, tag, or GlobalId"),
    keywords: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional search terms"),
    entity_names: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC entity labels to search first, for example IfcBeam or IfcElementAssembly"),
    renderable_only: tool.schema
      .boolean()
      .optional()
      .describe("When true, only return candidates with GlobalId values suitable for viewer element actions"),
    bridge_part_node_ids: tool.schema
      .array(tool.schema.number())
      .default([])
      .describe("Optional local IfcBridgePart DB node ids to anchor contained-product searches"),
    material_names: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional material names to anchor material-associated product searches"),
    limit: tool.schema.number().optional().describe("Maximum rows per focused query"),
    all_matches: tool.schema
      .boolean()
      .optional()
      .describe(
        "When true, return every match for a focused action query instead of a preview. Requires entity_names, bridge_part_node_ids, or material_names; do not use for broad text-only scans.",
      ),
    resource_filter: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC members to query when resource is a project"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    return ifcElementSearch(
      {
        resource: args.resource,
        text: args.text,
        keywords: args.keywords,
        entityNames: args.entity_names,
        renderableOnly: args.renderable_only,
        bridgePartNodeIds: args.bridge_part_node_ids,
        materialNames: args.material_names,
        limit: args.limit,
        allMatches: args.all_matches,
        resourceFilter: args.resource_filter,
        apiBase: args.api_base,
      },
      postViewerJson,
    );
  },
});

export const scope_summary = tool({
  description:
    "Summarize a known scope of IFC DB node ids or renderable semantic ids by entity, materials, and lightweight geometry catalog facts.",
  args: {
    resource: tool.schema
      .string()
      .optional()
      .describe("Selected IFC or project resource. Required for DB node ids and unscoped semantic ids"),
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Renderable semantic ids, optionally source-scoped as ifc/resource::GlobalId"),
    db_node_ids: tool.schema
      .array(tool.schema.number())
      .default([])
      .describe("Local DB node ids to summarize"),
    include_materials: tool.schema
      .boolean()
      .optional()
      .describe("Include explicit IfcRelAssociatesMaterial facts when available"),
    include_geometry: tool.schema
      .boolean()
      .optional()
      .describe("Include lightweight geometry catalog counts. Does not infer missing geometry quantities"),
    limit: tool.schema.number().optional().describe("Maximum ids/rows to inspect"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    return ifcScopeSummary({
      resource: args.resource,
      semanticIds: args.semantic_ids,
      dbNodeIds: args.db_node_ids,
      includeMaterials: args.include_materials,
      includeGeometry: args.include_geometry,
      limit: args.limit,
      apiBase: args.api_base,
      postViewerJson,
    });
  },
});

export const scope_inspect = tool({
  description:
    "Update the viewer inspection focus for known renderable semantic ids. Use replace for a new focus, add for additive wording, and remove for subtractive wording.",
  args: {
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the semantic ids came from, for example ifc/infra-road"),
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("Renderable semantic ids to inspect. In project mode these may be source-scoped as ifc/resource::GlobalId"),
    mode: tool.schema
      .string()
      .optional()
      .describe("Inspection update mode: replace, add, or remove"),
    select: tool.schema
      .boolean()
      .optional()
      .describe("Whether to also prepare a selection note for these ids"),
    frame_visible: tool.schema
      .boolean()
      .optional()
      .describe("Whether to also prepare a frame-visible note"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    return ifcScopeInspect({
      resource: args.resource,
      semanticIds: args.semantic_ids,
      mode: args.mode,
      select: args.select,
      frameVisible: args.frame_visible,
      why: args.why,
    });
  },
});

export const bridge_structure_summary = tool({
  description:
    "Summarize bridge roots, bridge parts, nested parts, and contained product-family counts using anchored per-part queries.",
  args: {
    resource: tool.schema.string().describe("Selected IFC or project resource, for example project/bridge-for-minnd"),
    limit: tool.schema.number().optional().describe("Maximum rows for bridge root/part discovery queries"),
    max_parts: tool.schema.number().optional().describe("Maximum bridge parts to inspect with anchored queries"),
    resource_filter: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC members to query when resource is a project"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    return ifcBridgeStructureSummary(
      {
        resource: args.resource,
        limit: args.limit,
        maxParts: args.max_parts,
        resourceFilter: args.resource_filter,
        apiBase: args.api_base,
      },
      postViewerJson,
    );
  },
});

export const quantity_takeoff = tool({
  description:
    "Create a truthful count/material/BOM-style takeoff with provenance. Geometry-derived quantities are reported unsupported unless explicit facts exist.",
  args: {
    resource: tool.schema.string().describe("Selected IFC or project resource"),
    group_by: tool.schema
      .string()
      .optional()
      .describe("Grouping: entity, material, bridge_part, or source_resource"),
    entity_names: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional IFC entity labels to constrain the takeoff"),
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional renderable semantic ids to constrain the takeoff"),
    source: tool.schema
      .string()
      .optional()
      .describe("Source preference: count_only, ifc_quantities, or geometry"),
    limit: tool.schema.number().optional().describe("Maximum rows"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    return ifcQuantityTakeoff(
      {
        resource: args.resource,
        group_by: args.group_by,
        entity_names: args.entity_names,
        semantic_ids: args.semantic_ids,
        source: args.source,
        limit: args.limit,
        api_base: args.api_base,
      },
      postViewerJson,
    );
  },
});

export const section_at_point_or_station = tool({
  description:
    "Prepare a section query from an explicit station, point, or semantic ids. Does not invent section geometry when alignment/plane facts are missing.",
  args: {
    resource: tool.schema.string().describe("Selected IFC or project resource"),
    station: tool.schema
      .union([tool.schema.string(), tool.schema.number()])
      .optional()
      .describe("Explicit alignment station when available"),
    point: tool.schema
      .array(tool.schema.number())
      .optional()
      .describe("Explicit world-space point as [x,y] or [x,y,z]"),
    orientation: tool.schema.string().optional().describe("Requested section orientation, for example cross or longitudinal"),
    width: tool.schema.number().optional().describe("Requested section width"),
    depth: tool.schema.number().optional().describe("Requested section depth"),
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional semantic ids to constrain section candidate discovery"),
    limit: tool.schema.number().optional().describe("Maximum rows"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    const point = Array.isArray(args.point)
      ? args.point.length === 2
        ? ([args.point[0], args.point[1]] as [number, number])
        : args.point.length >= 3
          ? ([args.point[0], args.point[1], args.point[2]] as [number, number, number])
          : undefined
      : undefined;
    return ifcSectionAtPointOrStation(
      {
        resource: args.resource,
        station: args.station,
        point,
        orientation: args.orientation,
        width: args.width,
        depth: args.depth,
        semantic_ids: args.semantic_ids,
        limit: args.limit,
        api_base: args.api_base,
      },
      postViewerJson,
    );
  },
});

export const node_relations = tool({
  description: "Inspect the properties and local relations for a graph node.",
  args: {
    resource: tool.schema.string().describe("Selected IFC resource, for example ifc/building-architecture"),
    db_node_id: tool.schema.number().describe("Database node id to inspect"),
    max_relations: tool.schema.number().optional().describe("Maximum number of relations to return"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    return postViewerJson(args.api_base, "/api/graph/node-properties", {
      resource: args.resource,
      dbNodeId: args.db_node_id,
      maxRelations: args.max_relations,
    });
  },
});

export const renderable_descendants = tool({
  description: "Find visible descendant products for a semantic or grouping node.",
  args: {
    resource: tool.schema.string().describe("Selected IFC resource, for example ifc/building-architecture"),
    db_node_id: tool.schema.number().describe("Database node id to start from"),
    hops: tool.schema.number().optional().describe("Traversal depth, usually 1 to 3"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args) {
    const hops = Math.max(1, Math.min(args.hops ?? 3, 3));
    const cypher = [
      `MATCH (seed) WHERE id(seed) = ${Math.trunc(args.db_node_id)}`,
      `MATCH (seed)-[:RELATED_OBJECTS|RELATED_ELEMENTS*1..${hops}]-(n)`,
      "RETURN DISTINCT id(n) AS node_id, n.GlobalId AS global_id, labels(n) AS labels, n.Name AS name",
      "LIMIT 64",
    ].join("\n");
    return postViewerJson(args.api_base, "/api/cypher", {
      resource: args.resource,
      cypher,
    });
  },
});

export const graph_set_seeds = tool({
  description: "Ask the host viewer to seed the graph from one or more DB node ids.",
  args: {
    db_node_ids: tool.schema
      .array(tool.schema.number())
      .min(1)
      .describe("Database node ids to seed in the graph"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the DB node ids came from, for example ifc/infra-road"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const count = args.db_node_ids.length;
    const reason = args.why?.trim();
    return [
      `Prepared graph.set_seeds for ${count} node${count === 1 ? "" : "s"}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const properties_show_node = tool({
  description: "Ask the host viewer to open the Properties panel for one DB node.",
  args: {
    db_node_id: tool.schema.number().describe("Database node id to inspect"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the DB node id came from, for example ifc/infra-road"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const reason = args.why?.trim();
    return [
      `Prepared properties.show_node for node ${Math.trunc(args.db_node_id)}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const elements_hide = tool({
  description: "Ask the host viewer to hide one or more renderable semantic ids.",
  args: {
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("Renderable semantic ids to hide. In project mode these may be source-scoped as ifc/resource::GlobalId"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the semantic ids came from, for example ifc/infra-road"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const count = args.semantic_ids.length;
    const reason = args.why?.trim();
    return [
      `Prepared elements.hide for ${count} element${count === 1 ? "" : "s"}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const elements_show = tool({
  description: "Ask the host viewer to show one or more renderable semantic ids.",
  args: {
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("Renderable semantic ids to show. In project mode these may be source-scoped as ifc/resource::GlobalId"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the semantic ids came from, for example ifc/infra-road"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const count = args.semantic_ids.length;
    const reason = args.why?.trim();
    return [
      `Prepared elements.show for ${count} element${count === 1 ? "" : "s"}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const elements_select = tool({
  description: "Ask the host viewer to select one or more renderable semantic ids.",
  args: {
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("Renderable semantic ids to select. In project mode these may be source-scoped as ifc/resource::GlobalId"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the semantic ids came from, for example ifc/infra-road"),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const count = args.semantic_ids.length;
    const reason = args.why?.trim();
    return [
      `Prepared elements.select for ${count} element${count === 1 ? "" : "s"}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const elements_inspect = tool({
  description:
    "Ask the host viewer to update the inspection focus for one or more renderable semantic ids. Use mode replace for a new/only focus, add for additive wording like also/include/plus, and remove for subtractive wording like remove/exclude/subtract.",
  args: {
    semantic_ids: tool.schema
      .array(tool.schema.string())
      .min(1)
      .describe("Renderable semantic ids to inspect. In project mode these may be source-scoped as ifc/resource::GlobalId"),
    resource: tool.schema
      .string()
      .optional()
      .describe("IFC resource the semantic ids came from, for example ifc/infra-road"),
    mode: tool.schema
      .string()
      .optional()
      .describe("Inspection update mode: replace, add, or remove. replace sets the focus, add preserves the current focus and adds ids, remove subtracts ids from the current focus."),
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const count = args.semantic_ids.length;
    const reason = args.why?.trim();
    const mode = args.mode ?? "replace";
    return [
      `Prepared elements.inspect ${mode} for ${count} element${count === 1 ? "" : "s"}.`,
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const viewer_frame_visible = tool({
  description: "Ask the host viewer to frame the visible scene.",
  args: {
    why: tool.schema.string().optional().describe("Short reason for the viewer action"),
  },
  async execute(args) {
    const reason = args.why?.trim();
    return [
      "Prepared viewer.frame_visible.",
      reason ? `Why: ${reason}` : null,
    ]
      .filter(Boolean)
      .join("\n");
  },
});

export const viewer_clear_inspection = tool({
  description:
    "Ask the host viewer to clear the current inspection focus and return the scene to normal rendering.",
  args: {
    why: tool.schema.string().optional().describe("Short reason for clearing inspection"),
  },
  async execute(args) {
    const reason = args.why?.trim();
    return ["Prepared viewer.clear_inspection.", reason ? `Why: ${reason}` : null]
      .filter(Boolean)
      .join("\n");
  },
});
