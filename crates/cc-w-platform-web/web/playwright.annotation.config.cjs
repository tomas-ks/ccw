const { defineConfig } = require("playwright/test");

const baseURL = process.env.CC_W_PLAYWRIGHT_BASE_URL || "http://127.0.0.1:8123";
const useExternalServer = Boolean(process.env.CC_W_PLAYWRIGHT_BASE_URL);

module.exports = defineConfig({
  testDir: "./tests",
  timeout: 120_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    viewport: { width: 1280, height: 800 },
    launchOptions: {
      args: ["--enable-unsafe-webgpu", "--use-angle=swiftshader"],
    },
    trace: "retain-on-failure",
  },
  webServer: useExternalServer
    ? undefined
    : {
        command:
          "cd ../../.. && just web-viewer-build && cargo build -p cc-w-platform-web --bin cc-w-platform-web-server --bin cc-w-platform-web-cypher-worker --features native-server && CC_W_AGENT_BACKEND=stub CC_W_CYPHER_WORKER_BINARY=target/debug/cc-w-platform-web-cypher-worker target/debug/cc-w-platform-web-server --host 127.0.0.1 --port 8123 --root crates/cc-w-platform-web/artifacts/viewer --ifc-artifacts-root artifacts/ifc",
        url: baseURL,
        reuseExistingServer: true,
        timeout: 180_000,
      },
});
