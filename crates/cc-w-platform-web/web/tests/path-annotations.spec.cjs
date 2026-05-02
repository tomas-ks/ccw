const { expect, test } = require("playwright/test");

const bridgePath = {
  kind: "ifc_alignment",
  id: "curve:215711",
  measure: "station",
};

test("path annotations compose line and marker ranges in the browser", async ({ page }) => {
  await page.route("**/__webgpu_probe", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>WebGPU probe</title>",
    })
  );
  await page.goto("/__webgpu_probe");
  const hasWebGpuAdapter = await page.evaluate(async () => {
    if (!globalThis.navigator?.gpu?.requestAdapter) {
      return false;
    }
    const adapter = await globalThis.navigator.gpu.requestAdapter().catch(() => null);
    return Boolean(adapter);
  });
  test.skip(!hasWebGpuAdapter, "No WebGPU adapter is available for the browser viewer.");

  await page.goto("/");
  await page.waitForFunction(
    () =>
      Boolean(globalThis.viewer?.annotations?.showPath) &&
      Boolean(document.querySelector("#resource-picker option[value='project/bridge-for-minnd']"))
  );
  await page.selectOption("#resource-picker", "project/bridge-for-minnd");
  await page.waitForFunction(
    () => globalThis.viewer?.currentResource?.() === "project/bridge-for-minnd"
  );
  await page.waitForTimeout(500);

  const canvas = page.locator("#viewer-canvas");
  await expect(canvas).toBeVisible();
  await page.evaluate(() => {
    globalThis.viewer.annotations.clear();
    globalThis.viewer.allView();
  });
  await page.waitForTimeout(250);
  const before = await canvas.screenshot();

  await page.evaluate(async ({ path }) => {
    globalThis.viewer.annotations.clear();
    await globalThis.viewer.annotations.showPath({
      resource: "ifc/bridge-for-minnd",
      path,
      line: { ranges: [{ from: 100, from_offset: 100, to: 200, to_offset: 200 }] },
    });
    await globalThis.viewer.annotations.showPath({
      mode: "add",
      resource: "ifc/bridge-for-minnd",
      path,
      line: { ranges: [{ from: 300, to: 400 }] },
      markers: [{ range: { from: 300, to: 400 }, every: 20, label: "measure" }],
    });
    await globalThis.viewer.annotations.showPath({
      mode: "add",
      resource: "ifc/bridge-for-minnd",
      path,
      line: { ranges: [{ from: 400, to_end: true }] },
      markers: [{ range: { from: 400, to_end: true }, every: 50, label: "measure" }],
    });
  }, { path: bridgePath });
  await page.waitForTimeout(250);

  const annotationState = await page.evaluate(() => globalThis.viewer.annotations.state());
  const layer = annotationState.layers.find(
    (candidate) => candidate.id === "path-annotations-215711"
  );
  expect(layer).toBeTruthy();

  const lineIds = layer.primitives
    .filter((primitive) => primitive.type === "polyline" && primitive.id.startsWith("path-line-"))
    .map((primitive) => primitive.id)
    .sort();
  expect(lineIds).toEqual([
    "path-line-line0-100-200",
    "path-line-line0-300-400",
    expect.stringMatching(/^path-line-line0-400-/),
  ]);
  expect(lineIds).not.toContain("path-line-line0-0-500");
  expect(lineIds.find((id) => id.startsWith("path-line-line0-400-"))).not.toBe(
    "path-line-line0-400-400"
  );

  const labels = layer.primitives
    .filter((primitive) => primitive.type === "text")
    .map((primitive) => primitive.text)
    .sort((left, right) => Number(left) - Number(right));
  expect(labels).toEqual(
    expect.arrayContaining(["300", "320", "340", "360", "380", "400", "450", "500"])
  );

  const after = await canvas.screenshot();
  expect(Buffer.compare(before, after)).not.toBe(0);
});
