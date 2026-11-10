// What pages in the desktop window may ask of it: which of the user's computers run Linker-Edit, and switching to one.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("linkerEditDesktop", {
  computers: () => ipcRenderer.invoke("computers"),
  open: (/** @type {string} */ url) => ipcRenderer.invoke("open", url),
});
