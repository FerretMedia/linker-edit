// What pages in the desktop window may ask of it: which of the user's computers run Linker-Edit, switching to
// one, the app's own updates, the system's folder dialog, and where the window's own buttons sit. The types are DesktopBridge in src/lib/desktop.ts.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("linkerEditDesktop", {
  // desktop/main.ts hides the system title bar: macOS keeps its traffic lights on the left, Windows and Linux
  // draw their window buttons over the right.
  titleBar: process.platform === "darwin" ? "mac" : "overlay",
  computers: () => ipcRenderer.invoke("computers"),
  open: (/** @type {string} */ url) => ipcRenderer.invoke("open", url),
  update: () => ipcRenderer.invoke("update"),
  onUpdate: (/** @type {(update: unknown) => void} */ listener) => {
    /** @param {unknown} _event @param {unknown} update */
    const forward = (_event, update) => listener(update);
    ipcRenderer.on("update", forward);
    return () => void ipcRenderer.removeListener("update", forward);
  },
  installUpdate: () => ipcRenderer.invoke("install-update"),
  canPickFolder: () => ipcRenderer.invoke("can-pick-folder"),
  pickFolder: (/** @type {string | undefined} */ defaultPath) => ipcRenderer.invoke("pick-folder", defaultPath),
});
