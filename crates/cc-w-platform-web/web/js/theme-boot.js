(() => {
  const stored = window.localStorage?.getItem("w-viewer-theme");
  if (stored === "light" || stored === "dark") {
    document.documentElement.dataset.theme = stored;
  }
  const electronShell = window.ccwElectron;
  if (electronShell?.isElectron) {
    document.documentElement.classList.add("electron-shell");
    document.documentElement.dataset.electronPlatform = electronShell.platform || "unknown";
  }
})();
