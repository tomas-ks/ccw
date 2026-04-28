import { tool } from "@opencode-ai/plugin";
import { spawnSync } from "node:child_process";

type ToolContext = {
  worktree?: string;
  directory?: string;
};

const DEFAULT_SCHEMA = "IFC4X3_ADD2";
const DEFAULT_API_BASE = "http://127.0.0.1:8001";
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
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
  for (let attempt = 0; attempt < 3; attempt += 1) {
    for (const base of bases) {
      try {
        const response = await fetch(new URL(path, base), {
          method: "POST",
          headers: {
            "content-type": "application/json",
          },
          body: JSON.stringify(body),
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
        failures.push(`${base}: ${errorMessage(error)}`);
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
