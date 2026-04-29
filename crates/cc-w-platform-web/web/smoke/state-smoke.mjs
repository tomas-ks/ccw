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

async function expectPanelState(
  page,
  description,
  { panel, visible, bodyHiddenClass, buttonSelector, panelSelector }
) {
  return expectPageValue(
    page,
    description,
    ({ panel, visible, bodyHiddenClass, buttonSelector, panelSelector }) => {
      const state = window.wAppState?.getState?.();
      const button = document.querySelector(buttonSelector);
      const panelElement = document.querySelector(panelSelector);
      const style = panelElement ? getComputedStyle(panelElement) : null;
      const panelIsRendered = Boolean(
        panelElement &&
          !panelElement.hidden &&
          style?.display !== "none" &&
          style?.visibility !== "hidden" &&
          panelElement.getClientRects().length > 0
      );
      const headerVisible =
        panel === "graph"
          ? window.wHeader?.graphVisible?.()
          : panel === "terminal"
            ? window.wHeader?.terminalVisible?.()
            : undefined;

      if (!state) {
        return { ok: false, message: "window.wAppState is unavailable" };
      }
      if (state.panels?.[panel] !== visible) {
        return {
          ok: false,
          message: `state.panels.${panel} is ${JSON.stringify(state.panels?.[panel])}`,
        };
      }
      if (headerVisible !== undefined && headerVisible !== visible) {
        return {
          ok: false,
          message: `window.wHeader ${panel} visible is ${JSON.stringify(headerVisible)}`,
        };
      }
      if (!button) {
        return { ok: false, message: `missing ${buttonSelector}` };
      }
      if (button.disabled || button.hasAttribute("disabled")) {
        return { ok: false, message: `${buttonSelector} is disabled` };
      }
      if (button.getAttribute("aria-pressed") !== String(visible)) {
        return {
          ok: false,
          message: `${buttonSelector} aria-pressed is not ${visible}`,
        };
      }
      if (button.classList.contains("active") !== visible) {
        return {
          ok: false,
          message: `${buttonSelector} active class is not ${visible}`,
        };
      }
      if (document.body.classList.contains(bodyHiddenClass) !== !visible) {
        return {
          ok: false,
          message: `body ${bodyHiddenClass} class does not match visibility`,
        };
      }
      if (panelIsRendered !== visible) {
        return {
          ok: false,
          message: `${panelSelector} rendered visibility is ${panelIsRendered}`,
        };
      }

      if (panel === "terminal" && visible) {
        const activeTool = state.terminal?.activeTool;
        const activeHostId =
          activeTool === "ai" ? "agent-terminal" : activeTool === "js" ? "repl-terminal" : null;
        const activeHost = activeHostId ? document.getElementById(activeHostId) : null;
        const activeTab = activeTool
          ? document.querySelector(`[data-terminal-tool="${activeTool}"]`)
          : null;
        if (!activeHost || activeHost.hidden || !activeHost.classList.contains("active")) {
          return { ok: false, message: "active terminal host is not visible" };
        }
        if (!activeTab || activeTab.getAttribute("aria-selected") !== "true") {
          return { ok: false, message: "active terminal tab is not selected" };
        }
      }

      return { ok: true };
    },
    { panel, visible, bodyHiddenClass, buttonSelector, panelSelector }
  );
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
  const externalRequests = [];
  const aiTurnRequests = [];
  const viewerOrigin = new URL(options.url).origin;

  page.on("pageerror", (error) => {
    browserErrors.push(error.message);
  });
  page.on("console", (message) => {
    if (message.type() === "error") {
      const text = message.text();
      if (/Failed to load resource: the server responded with a status of 404/.test(text)) {
        return;
      }
      browserErrors.push(text);
    }
  });
  page.on("request", (request) => {
    const requestUrl = new URL(request.url());
    if (requestUrl.protocol !== "http:" && requestUrl.protocol !== "https:") {
      return;
    }
    if (requestUrl.origin !== viewerOrigin) {
      externalRequests.push(request.url());
      return;
    }
    if (
      requestUrl.pathname === "/api/agent/turn-start" ||
      requestUrl.pathname === "/api/agent/turn-poll"
    ) {
      aiTurnRequests.push(`${request.method()} ${requestUrl.pathname}`);
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

    await expectPanelState(page, "graph panel starts hidden", {
      panel: "graph",
      visible: false,
      bodyHiddenClass: "graph-hidden",
      buttonSelector: "#graph-toggle-button",
      panelSelector: ".side-panel",
    });

    await page.click("#graph-toggle-button");
    await expectPanelState(page, "header graph toggle opens graph panel", {
      panel: "graph",
      visible: true,
      bodyHiddenClass: "graph-hidden",
      buttonSelector: "#graph-toggle-button",
      panelSelector: ".side-panel",
    });

    await page.click("#graph-toggle-button");
    await expectPanelState(page, "header graph toggle closes graph panel", {
      panel: "graph",
      visible: false,
      bodyHiddenClass: "graph-hidden",
      buttonSelector: "#graph-toggle-button",
      panelSelector: ".side-panel",
    });

    await expectPanelState(page, "terminal panel starts hidden", {
      panel: "terminal",
      visible: false,
      bodyHiddenClass: "terminal-hidden",
      buttonSelector: "#terminal-toggle-button",
      panelSelector: ".terminal",
    });

    await page.evaluate(() => {
      window.__stateSmokeTerminalVisibilityEvents = [];
      window.addEventListener("w-terminal-visibility-change", (event) => {
        window.__stateSmokeTerminalVisibilityEvents.push(Boolean(event.detail?.visible));
      });
    });

    await page.click("#terminal-toggle-button");
    await expectPanelState(page, "header terminal toggle opens terminal panel", {
      panel: "terminal",
      visible: true,
      bodyHiddenClass: "terminal-hidden",
      buttonSelector: "#terminal-toggle-button",
      panelSelector: ".terminal",
    });
    await waitForPagePredicate(
      page,
      "terminal toggle emits visible event",
      () => window.__stateSmokeTerminalVisibilityEvents?.includes(true),
      options.timeoutMs
    );

    await page.click("#terminal-toggle-button");
    await expectPanelState(page, "header terminal toggle closes terminal panel", {
      panel: "terminal",
      visible: false,
      bodyHiddenClass: "terminal-hidden",
      buttonSelector: "#terminal-toggle-button",
      panelSelector: ".terminal",
    });
    await waitForPagePredicate(
      page,
      "terminal toggle emits hidden event",
      () => window.__stateSmokeTerminalVisibilityEvents?.includes(false),
      options.timeoutMs
    );

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

    await expectPageValue(page, "resource picker reflects committed resource", (toResource) => {
      const state = window.wAppState.getState();
      const picker = document.querySelector("#resource-picker");
      if (picker?.value !== toResource) {
        return { ok: false, message: `picker value is ${JSON.stringify(picker?.value)}` };
      }
      if (state?.committedViewerState?.resource !== toResource) {
        return {
          ok: false,
          message: `committed resource is ${JSON.stringify(state?.committedViewerState?.resource)}`,
        };
      }
      if (state?.requestedResource) {
        return { ok: false, message: `requested resource is still ${state.requestedResource}` };
      }
      return { ok: true };
    }, options.toResource);

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

    const outlinerToggleTarget = await expectPageValue(page, "outliner checkbox target resolves", () => {
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
      const row = checkbox.closest(".outliner-row");
      const rows = Array.from(document.querySelectorAll("#outliner-body .outliner-row"));
      const rowIndex = rows.indexOf(row);
      if (rowIndex < 0) {
        return { ok: false, message: "could not resolve outliner row index" };
      }
      const countVisible = (state) => {
        const visible = new Set(state.visibleElementIds || []);
        return memberIds.filter((id) => visible.has(id)).length;
      };
      const beforeCount = countVisible(before);
      if (!Array.isArray(before.visibleElementIds)) {
        return { ok: false, message: "view state does not expose visibleElementIds" };
      }
      return { ok: true, value: { rowIndex, member, memberIds, beforeCount } };
    });

    const outlinerCheckboxSelector =
      `#outliner-body .outliner-row:nth-of-type(${outlinerToggleTarget.rowIndex + 1}) ` +
      'input[type="checkbox"]';
    await page.click(outlinerCheckboxSelector);
    await waitForPagePredicate(
      page,
      "outliner checkbox suppresses default member visibility",
      ({ memberIds, beforeCount }) => {
        const state = window.wViewer?.viewState?.();
        const visible = new Set(state?.visibleElementIds || []);
        const count = memberIds.filter((id) => visible.has(id)).length;
        return count < beforeCount;
      },
      options.timeoutMs,
      outlinerToggleTarget
    );
    await page.click(outlinerCheckboxSelector);
    await waitForPagePredicate(
      page,
      "outliner checkbox restores default member visibility",
      ({ memberIds, beforeCount }) => {
        const state = window.wViewer?.viewState?.();
        const visible = new Set(state?.visibleElementIds || []);
        const count = memberIds.filter((id) => visible.has(id)).length;
        return count === beforeCount;
      },
      options.timeoutMs,
      outlinerToggleTarget
    );

    if (externalRequests.length) {
      throw new Error(
        `browser made non-local requests:\n${Array.from(new Set(externalRequests)).join("\n")}`
      );
    }
    if (aiTurnRequests.length) {
      throw new Error(
        `browser made AI turn requests:\n${Array.from(new Set(aiTurnRequests)).join("\n")}`
      );
    }
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
