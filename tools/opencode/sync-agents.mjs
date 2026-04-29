#!/usr/bin/env node

import { copyFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const sourceDir = join(repoRoot, "agent", "agents");
const destDir = join(repoRoot, ".opencode", "agents");

mkdirSync(destDir, { recursive: true });

const files = readdirSync(sourceDir)
  .filter((name) => name.endsWith(".md"))
  .sort();

if (files.length === 0) {
  throw new Error(`no canonical agent profiles found in ${sourceDir}`);
}

let copied = 0;
for (const file of files) {
  const source = join(sourceDir, file);
  if (!statSync(source).isFile()) {
    continue;
  }
  copyFileSync(source, join(destDir, file));
  copied += 1;
}

console.log(
  `synced ${copied} agent profile${copied === 1 ? "" : "s"} from agent/agents to .opencode/agents`,
);
