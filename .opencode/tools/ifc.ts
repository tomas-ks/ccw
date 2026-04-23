import { tool } from "@opencode-ai/plugin";
import { spawnSync } from "node:child_process";

type ToolContext = {
  worktree?: string;
  directory?: string;
};

const DEFAULT_SCHEMA = "IFC4X3_ADD2";
const DEFAULT_API_BASE = "http://127.0.0.1:8001";

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
  const resolvedApiBase = (apiBase ?? DEFAULT_API_BASE).trim() || DEFAULT_API_BASE;
  const response = await fetch(new URL("/api/cypher", resolvedApiBase), {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify({
      resource,
      cypher,
      why,
    }),
  });

  const text = await response.text();
  if (!response.ok) {
    return JSON.stringify(
      {
        ok: false,
        resource,
        cypher,
        why: why ?? null,
        status: response.status,
        error: text,
      },
      null,
      2,
    );
  }

  return text.trim();
}

async function postViewerJson(
  apiBase: string | undefined,
  path: string,
  body: Record<string, unknown>,
): Promise<string> {
  const resolvedApiBase = (apiBase ?? DEFAULT_API_BASE).trim() || DEFAULT_API_BASE;
  const response = await fetch(new URL(path, resolvedApiBase), {
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
        status: response.status,
        error: text,
      },
      null,
      2,
    );
  }

  return text.trim();
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
