#!/usr/bin/env node

import process from "node:process";

const DEFAULT_URL = "http://127.0.0.1:8001/?resource=ifc/building-architecture";
const DEFAULT_TO_RESOURCE = "project/building";

function usage() {
  return `Usage:
  node crates/cc-w-platform-web/web/smoke/state-smoke.mjs [--url URL] [--to RESOURCE] [--headed] [--timeout MS]

Runs a browser smoke against an already-started web viewer.

Defaults:
  --url ${DEFAULT_URL}
  --to  ${DEFAULT_TO_RESOURCE}

Example:
  just web-viewer
  node crates/cc-w-platform-web/web/smoke/state-smoke.mjs

Requires the playwright package to be resolvable by Node.`;
}

function parseArgs(argv) {
  const options = {
    url: DEFAULT_URL,
    toResource: DEFAULT_TO_RESOURCE,
    headed: false,
    timeoutMs: 30_000,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    if (arg === "--headed") {
      options.headed = true;
      continue;
    }
    if (arg === "--url") {
      options.url = argv[++index];
      continue;
    }
    if (arg === "--to") {
      options.toResource = argv[++index];
      continue;
    }
    if (arg === "--timeout") {
      options.timeoutMs = Number(argv[++index]);
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }

  if (!options.url) {
    throw new Error("--url requires a value");
  }
  if (!options.toResource) {
    throw new Error("--to requires a value");
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("--timeout must be a positive number of milliseconds");
  }

  return options;
}

async function loadPlaywright() {
  try {
    return await import("playwright");
  } catch (error) {
    throw new Error(
      [
        "playwright is not installed or not resolvable by Node.",
        "Install it in your local test environment, then rerun this smoke.",
        `Original import error: ${error.message}`,
      ].join("\n")
    );
  }
}

async function expectPageValue(page, description, callback, arg) {
  const result = await page.evaluate(callback, arg);
  if (!result?.ok) {
    throw new Error(`${description}: ${result?.message || "failed"}`);
  }
  console.log(`ok - ${description}`);
  return result.value;
}

async function waitForPagePredicate(page, description, callback, timeoutMs, arg) {
  try {
    await page.waitForFunction(callback, arg, { timeout: timeoutMs });
  } catch (error) {
    throw new Error(`${description}: ${error.message}`);
  }
  console.log(`ok - ${description}`);
}

async function run() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    console.log(usage());
    return;
  }

  const { chromium } = await loadPlaywright();
  const browser = await chromium.launch({ headless: !options.headed });
  const page = await browser.newPage();
  const browserErrors = [];

  page.on("pageerror", (error) => {
    browserErrors.push(error.message);
  });
  page.on("console", (message) => {
    if (message.type() === "error") {
      browserErrors.push(message.text());
    }
  });

  try {
    await page.goto(options.url, { waitUntil: "domcontentloaded", timeout: options.timeoutMs });

    await waitForPagePredicate(
      page,
      "viewer globals are installed",
      () => Boolean(window.wAppState?.getState && window.wViewer?.viewState),
      options.timeoutMs
    );

    await expectPageValue(page, "app state exposes committed viewer resource", () => {
      const state = window.wAppState.getState();
      const resource = state?.committedViewerState?.resource || window.wViewer.viewState()?.resource;
      return resource
        ? { ok: true, value: resource }
        : { ok: false, message: "missing committed viewer resource" };
    });

    await page.click("#outliner-toggle-button");
    await expectPageValue(page, "outliner panel is app-state driven", () => {
      const state = window.wAppState.getState();
      const panel = document.querySelector("#project-outliner");
      const button = document.querySelector("#outliner-toggle-button");
      if (!state?.panels?.outliner) {
        return { ok: false, message: "state.panels.outliner is false" };
      }
      if (panel?.hidden) {
        return { ok: false, message: "outliner panel is hidden" };
      }
      if (button?.getAttribute("aria-pressed") !== "true") {
        return { ok: false, message: "outliner button aria-pressed is not true" };
      }
      return { ok: true };
    });

    await expectPageValue(page, "resource picker contains target resource", (toResource) => {
      const picker = document.querySelector("#resource-picker");
      if (!picker) {
        return { ok: false, message: "missing #resource-picker" };
      }
      const values = Array.from(picker.options).map((option) => option.value);
      return values.includes(toResource)
        ? { ok: true, value: values }
        : { ok: false, message: `${toResource} not found in resource picker` };
    }, options.toResource);

    await page.selectOption("#resource-picker", options.toResource);

    await waitForPagePredicate(
      page,
      `viewer commits ${options.toResource}`,
      (toResource) => {
        const state = window.wAppState?.getState?.();
        return (
          state?.committedViewerState?.resource === toResource &&
          state?.requestedResource !== toResource &&
          window.wViewer?.viewState?.()?.resource === toResource
        );
      },
      options.timeoutMs,
      options.toResource
    );

    await expectPageValue(page, "project outliner renders member rows after commit", () => {
      const rows = Array.from(document.querySelectorAll("#outliner-body .outliner-row"));
      if (!rows.length) {
        return { ok: false, message: "no outliner rows rendered" };
      }
      const rowSummaries = rows.map((row) => row.textContent.trim()).filter(Boolean);
      const hasCheckboxes = rows.every((row) => row.querySelector('input[type="checkbox"]'));
      return hasCheckboxes
        ? { ok: true, value: rowSummaries }
        : { ok: false, message: "one or more outliner rows has no checkbox" };
    });

    await expectPageValue(page, "outliner checkboxes preserve renderer ownership", () => {
      const checkbox = Array.from(
        document.querySelectorAll('#outliner-body .outliner-row input[type="checkbox"]')
      ).find((input) => !input.disabled && input.checked);
      const before = window.wViewer.viewState();
      if (!checkbox || checkbox.disabled) {
        return { ok: false, message: "no enabled checked outliner checkbox found" };
      }
      const member = String(checkbox.getAttribute("aria-label") || "").replace(/^Toggle\s+/, "");
      const currentResource = window.wViewer.viewState()?.resource || "";
      const defaultIds = window.wViewer.defaultElementIds();
      const memberIds = currentResource.startsWith("project/")
        ? defaultIds.filter((id) => id.startsWith(`${member}::`))
        : defaultIds.filter((id) => !id.includes("::"));
      if (!member || !memberIds.length) {
        return { ok: false, message: "could not resolve checked outliner member ids" };
      }
      const countVisible = (state) => {
        const visible = new Set(state.visibleElementIds || []);
        return memberIds.filter((id) => visible.has(id)).length;
      };
      const beforeCount = countVisible(before);
      checkbox.click();
      const suppressed = window.wViewer.viewState();
      checkbox.click();
      const restored = window.wViewer.viewState();
      if (!Array.isArray(before.visibleElementIds) || !Array.isArray(restored.visibleElementIds)) {
        return { ok: false, message: "view state does not expose visibleElementIds" };
      }
      const suppressedCount = countVisible(suppressed);
      const restoredCount = countVisible(restored);
      if (suppressedCount >= beforeCount) {
        return {
          ok: false,
          message: `suppression did not reduce visible count (${beforeCount} -> ${suppressedCount})`,
        };
      }
      if (restoredCount !== beforeCount) {
        return {
          ok: false,
          message: `unsuppression did not restore visible count (${beforeCount} -> ${restoredCount})`,
        };
      }
      return { ok: true };
    });

    if (browserErrors.length) {
      throw new Error(`browser emitted errors:\n${browserErrors.join("\n")}`);
    }
  } finally {
    await browser.close();
  }
}

run().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
