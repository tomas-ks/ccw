const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("ccwElectron", {
  isElectron: true,
  platform: process.platform,
  reload: () => ipcRenderer.invoke("ccw:reload"),
  toggleDevTools: () => ipcRenderer.invoke("ccw:toggle-devtools"),
  quit: () => ipcRenderer.invoke("ccw:quit"),
  minimize: () => ipcRenderer.invoke("ccw:window-minimize"),
  toggleMaximize: () => ipcRenderer.invoke("ccw:window-toggle-maximize"),
  close: () => ipcRenderer.invoke("ccw:window-close"),
  setTheme: (theme) => ipcRenderer.send("ccw:theme", theme),
  onWindowState: (callback) => {
    if (typeof callback !== "function") {
      return () => {};
    }
    const listener = (_event, state) => callback(state);
    ipcRenderer.on("ccw:window-state", listener);
    return () => ipcRenderer.removeListener("ccw:window-state", listener);
  },
});
