import { tool } from "@opencode-ai/plugin";

type ToolContext = {
  sessionID: string;
  directory: string;
  worktree: string;
  abort: AbortSignal;
  client: any;
};

const DEFAULT_API_BASE = "http://127.0.0.1:8001";
const FALLBACK_API_BASES = ["http://127.0.0.1:8001", "http://localhost:8001"];
let activeClient: any = null;

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

async function loadSessionTranscript(context: ToolContext): Promise<string> {
  const client = activeClient ?? context.client;
  if (!client?.session?.messages) {
    return "";
  }
  const response = await client.session.messages(
    {
      sessionID: context.sessionID,
      directory: context.directory,
      workspace: context.worktree,
      limit: 8,
    },
    { throwOnError: true },
  );
  const data = (response as any).data ?? response;
  return JSON.stringify(data);
}

function extractBoundValue(transcriptJson: string, label: string): string | null {
  const pattern = new RegExp(
    `${label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*:\\s*([^\\n.]+)`,
    "gi",
  );
  let raw: string | null = null;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(transcriptJson)) !== null) {
    raw = match[1]?.trim() ?? null;
  }
  return raw && raw.length ? raw.replace(/\s*\([^)]*\)\s*$/, "").trim() : null;
}

function extractBoundResource(transcriptJson: string): string | null {
  return extractBoundValue(transcriptJson, "Bound IFC resource for this turn");
}

function extractBoundSchema(transcriptJson: string): string | null {
  return extractBoundValue(transcriptJson, "Bound IFC schema for this turn");
}

function normalizeKeywords(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map((entry) => String(entry || "").trim())
      .filter((entry) => entry.length > 0);
  }
  if (typeof value === "string") {
    return value
      .split(/[,\n]/)
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
  }
  return [];
}

function asText(value: unknown): string {
  return String(value ?? "").trim();
}

function searchRows(rows: Array<Record<string, unknown>>, keywords: string[], entityType?: string) {
  const loweredKeywords = keywords.map((keyword) => keyword.toLowerCase());
  const loweredEntityType = entityType?.trim().toLowerCase() || "";

  return rows.filter((row) => {
    const haystack = [
      row.entity,
      row.name,
      row.object_type,
      row.description,
      row.global_id,
    ]
      .map((value) => asText(value).toLowerCase())
      .join(" ");

    if (loweredEntityType && !haystack.includes(loweredEntityType)) {
      return false;
    }
    if (!loweredKeywords.length) {
      return true;
    }
    return loweredKeywords.some((keyword) => haystack.includes(keyword));
  });
}

export const entity_search = tool({
  description:
    "Exact fallback for the older tool name `entity_search`, used to search the current IFC model by keywords or entity type.",
  args: {
    resource: tool.schema.string().optional().describe("Selected IFC resource, for example ifc/building-architecture"),
    entity_type: tool.schema.string().optional().describe("Optional IFC entity family or label to focus on"),
    keywords: tool.schema
      .union([tool.schema.string(), tool.schema.array(tool.schema.string())])
      .optional()
      .describe("Search terms that should appear in the entity name, object type, or description"),
    why: tool.schema.string().optional().describe("Short reason for the search"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args, context) {
    const transcript = await loadSessionTranscript(context);
    const resource = args.resource?.trim() || extractBoundResource(transcript) || "";
    const schema = extractBoundSchema(transcript);
    if (!resource) {
      return JSON.stringify(
        {
          ok: false,
          error:
            "Could not resolve the active IFC resource for entity_search. Use the exact resource string from the turn context.",
          entity_type: args.entity_type ?? null,
          keywords: normalizeKeywords(args.keywords),
        },
        null,
        2,
      );
    }

    const query = [
      "MATCH (n)",
      "WHERE n.declared_entity IS NOT NULL",
      "RETURN n.declared_entity AS entity, n.Name AS name, n.ObjectType AS object_type, n.Description AS description, n.GlobalId AS global_id",
      "LIMIT 200",
    ].join("\n");
    const resultText = await postViewerJson(args.api_base, "/api/cypher", {
      resource,
      cypher: query,
      why: args.why?.trim() || "search the current IFC model for likely matching entities",
    });

    const parsed = (() => {
      try {
        return JSON.parse(resultText);
      } catch {
        return null;
      }
    })();

    if (!parsed || parsed.ok === false || !Array.isArray(parsed.rows)) {
      return resultText;
    }

    const rows = parsed.rows.map((row: unknown) => {
      const tuple = Array.isArray(row) ? row : [];
      return {
        entity: tuple[0],
        name: tuple[1],
        object_type: tuple[2],
        description: tuple[3],
        global_id: tuple[4],
      };
    }) as Array<Record<string, unknown>>;
    const keywords = normalizeKeywords(args.keywords);
    const filtered = searchRows(rows, keywords, args.entity_type?.trim() || undefined).slice(0, 12);

    return JSON.stringify(
      {
        ok: true,
        resource,
        schema,
        entity_type: args.entity_type?.trim() || null,
        keywords,
        matches: filtered,
        total_candidates: rows.length,
        matched: filtered.length,
      },
      null,
      2,
    );
  },
});

export const properties = tool({
  description:
    "Exact fallback for the older tool name `properties`, used for a compact model overview or selected node properties.",
  args: {
    resource: tool.schema.string().optional().describe("Selected IFC resource, for example ifc/building-architecture"),
    db_node_id: tool.schema.number().optional().describe("Optional database node id to inspect"),
    properties: tool.schema
      .array(tool.schema.string())
      .default([])
      .describe("Optional property names the model is interested in"),
    why: tool.schema.string().optional().describe("Short reason for the inspection"),
    api_base: tool.schema.string().optional().describe("Viewer API base URL"),
  },
  async execute(args, context) {
    const transcript = await loadSessionTranscript(context);
    const resource = args.resource?.trim() || extractBoundResource(transcript) || "";
    const schema = extractBoundSchema(transcript);
    const requestedProperties = (args.properties ?? [])
      .map((value) => String(value).trim())
      .filter((value) => value.length > 0);

    if (!resource) {
      return JSON.stringify(
        {
          ok: false,
          error:
            "Could not resolve the active IFC resource for properties. Use the exact resource string from the turn context.",
          db_node_id: args.db_node_id ?? null,
          properties: requestedProperties,
        },
        null,
        2,
      );
    }

    if (typeof args.db_node_id === "number" && Number.isFinite(args.db_node_id)) {
      return postViewerJson(args.api_base, "/api/graph/node-properties", {
        resource,
        dbNodeId: Math.trunc(args.db_node_id),
      });
    }

    const overviewQuery = [
      "MATCH (p:IfcProject)",
      "OPTIONAL MATCH (s:IfcSite)",
      "OPTIONAL MATCH (b:IfcBuilding)",
      "OPTIONAL MATCH (st:IfcBuildingStorey)",
      "OPTIONAL MATCH (sp:IfcSpace)",
      "OPTIONAL MATCH (w:IfcWall)",
      "OPTIONAL MATCH (sl:IfcSlab)",
      "OPTIONAL MATCH (r:IfcRoof)",
      "OPTIONAL MATCH (f:IfcFurniture)",
      "RETURN p.Name AS project_name, count(DISTINCT s) AS site_count, count(DISTINCT b) AS building_count, count(DISTINCT st) AS storey_count, count(DISTINCT sp) AS space_count, count(DISTINCT w) AS wall_count, count(DISTINCT sl) AS slab_count, count(DISTINCT r) AS roof_count, count(DISTINCT f) AS furniture_count",
      "LIMIT 1",
    ].join("\n");

    const resultText = await postViewerJson(args.api_base, "/api/cypher", {
      resource,
      cypher: overviewQuery,
      why: args.why?.trim() || "give a quick model overview when the model asked for properties",
    });

    return JSON.stringify(
      {
        ok: true,
        resource,
        schema,
        requested_properties: requestedProperties,
        overview: resultText,
      },
      null,
      2,
    );
  },
});

export async function server(ctx: ToolContext) {
  activeClient = ctx.client;
  return {
    tool: {
      entity_search,
      properties,
    },
  };
}

export default server;
