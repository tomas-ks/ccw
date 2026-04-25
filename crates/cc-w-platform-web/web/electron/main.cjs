const { app, BrowserWindow, Menu, ipcMain, nativeTheme } = require("electron");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const ELECTRON_DIR = __dirname;
const WEB_DIR = path.resolve(ELECTRON_DIR, "..");
const REPO_ROOT = path.resolve(WEB_DIR, "..", "..", "..");
const SERVER_READY_PATTERN = /^open\s+(https?:\/\/[^\s]+)\/?$/m;

let serverProcess = null;
let mainWindow = null;
let shuttingDown = false;
let serverUrl = null;

function resolveServerBinary() {
  if (process.env.CC_W_WEB_SERVER_BINARY) {
    return path.resolve(process.env.CC_W_WEB_SERVER_BINARY);
  }
  const executable =
    process.platform === "win32"
      ? "cc-w-platform-web-server.exe"
      : "cc-w-platform-web-server";
  return path.join(REPO_ROOT, "target", "debug", executable);
}

function resolveViewerRoot() {
  if (process.env.CC_W_VIEWER_ROOT) {
    return path.resolve(process.env.CC_W_VIEWER_ROOT);
  }
  return path.join(REPO_ROOT, "crates", "cc-w-platform-web", "artifacts", "viewer");
}

function electronResourceArg() {
  const index = process.argv.indexOf("--resource");
  if (index >= 0 && process.argv[index + 1]) {
    return process.argv[index + 1];
  }
  return process.env.CC_W_ELECTRON_RESOURCE || "project/building";
}

function startViewerServer() {
  const serverBinary = resolveServerBinary();
  const viewerRoot = resolveViewerRoot();
  const host = process.env.CC_W_ELECTRON_HOST || "127.0.0.1";
  const port = process.env.CC_W_ELECTRON_PORT || "8001";

  if (!fs.existsSync(serverBinary)) {
    throw new Error(
      `Missing web viewer server binary: ${serverBinary}\nRun \`just web-viewer-electron-build\` first.`
    );
  }
  if (!fs.existsSync(path.join(viewerRoot, "index.html"))) {
    throw new Error(
      `Missing web viewer artifact: ${path.join(viewerRoot, "index.html")}\nRun \`just web-viewer-build\` first.`
    );
  }

  return new Promise((resolve, reject) => {
    const args = [
      "--host",
      host,
      "--port",
      port,
      "--root",
      viewerRoot,
      "--ifc-artifacts-root",
      path.join(REPO_ROOT, "artifacts", "ifc"),
    ];
    const child = spawn(serverBinary, args, {
      cwd: REPO_ROOT,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    serverProcess = child;

    let stdout = "";
    let settled = false;
    const failTimer = setTimeout(() => {
      if (settled) {
        return;
      }
      settled = true;
      reject(new Error("Timed out waiting for the web viewer server to start."));
    }, 15_000);

    const settleReady = (url) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(failTimer);
      resolve({ url, process: child });
    };

    child.stdout.on("data", (chunk) => {
      const text = chunk.toString();
      stdout += text;
      process.stdout.write(text);
      const match = stdout.match(SERVER_READY_PATTERN);
      if (match) {
        settleReady(match[1]);
      }
    });
    child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
    });
    child.on("error", (error) => {
      if (!settled) {
        settled = true;
        clearTimeout(failTimer);
        reject(error);
      }
    });
    child.on("exit", (code, signal) => {
      if (!shuttingDown && !settled) {
        settled = true;
        clearTimeout(failTimer);
        reject(new Error(`Web viewer server exited before startup: code=${code} signal=${signal}`));
      }
    });
  });
}

function viewerUrl(baseUrl) {
  const url = new URL(baseUrl);
  const resource = electronResourceArg();
  if (resource) {
    url.searchParams.set("resource", resource);
  }
  url.searchParams.set("shell", "electron");
  return url.toString();
}

function createWindow(baseUrl) {
  const isMac = process.platform === "darwin";
  mainWindow = new BrowserWindow({
    width: 1480,
    height: 980,
    minWidth: 980,
    minHeight: 640,
    backgroundColor: "#fafbfb",
    frame: isMac,
    titleBarStyle: isMac ? "hiddenInset" : undefined,
    trafficLightPosition: isMac ? { x: 14, y: 12 } : undefined,
    webPreferences: {
      preload: path.join(ELECTRON_DIR, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  mainWindow.loadURL(viewerUrl(baseUrl));
  syncWindowState(mainWindow);
  mainWindow.on("maximize", () => syncWindowState(mainWindow));
  mainWindow.on("unmaximize", () => syncWindowState(mainWindow));
  const autoQuitMs = Number(process.env.CC_W_ELECTRON_AUTO_QUIT_MS || 0);
  if (Number.isFinite(autoQuitMs) && autoQuitMs > 0) {
    setTimeout(() => app.quit(), autoQuitMs).unref();
  }
  return mainWindow;
}

function syncWindowState(window) {
  window.webContents.send("ccw:window-state", {
    maximized: window.isMaximized(),
    fullscreen: window.isFullScreen(),
  });
}

function installIpc() {
  ipcMain.handle("ccw:reload", (event) => {
    BrowserWindow.fromWebContents(event.sender)?.reload();
  });
  ipcMain.handle("ccw:toggle-devtools", (event) => {
    BrowserWindow.fromWebContents(event.sender)?.webContents.toggleDevTools();
  });
  ipcMain.handle("ccw:quit", () => {
    app.quit();
  });
  ipcMain.handle("ccw:window-minimize", (event) => {
    BrowserWindow.fromWebContents(event.sender)?.minimize();
  });
  ipcMain.handle("ccw:window-toggle-maximize", (event) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window) {
      return;
    }
    if (window.isMaximized()) {
      window.unmaximize();
    } else {
      window.maximize();
    }
    syncWindowState(window);
  });
  ipcMain.handle("ccw:window-close", (event) => {
    BrowserWindow.fromWebContents(event.sender)?.close();
  });
  ipcMain.on("ccw:theme", (_event, theme) => {
    nativeTheme.themeSource = theme === "light" ? "light" : "dark";
    const color = theme === "light" ? "#fafbfb" : "#0f1320";
    for (const window of BrowserWindow.getAllWindows()) {
      window.setBackgroundColor(color);
    }
  });
}

function stopServer() {
  shuttingDown = true;
  if (!serverProcess || serverProcess.killed) {
    return;
  }
  serverProcess.kill("SIGTERM");
  setTimeout(() => {
    if (serverProcess && !serverProcess.killed) {
      serverProcess.kill("SIGKILL");
    }
  }, 2_000).unref();
}

app.setName("W Viewer");
Menu.setApplicationMenu(null);
installIpc();

app.whenReady().then(async () => {
  try {
    const server = await startViewerServer();
    serverUrl = server.url;
    createWindow(server.url);
  } catch (error) {
    console.error(error);
    app.quit();
  }
});

app.on("before-quit", stopServer);
app.on("window-all-closed", () => {
  stopServer();
  if (process.platform !== "darwin") {
    app.quit();
  }
});
app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0 && serverUrl) {
    createWindow(serverUrl);
  }
});
