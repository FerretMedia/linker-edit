// How this window's sidebar looks: a browser's choice, like the theme, so each of your computers can have its own.

const FOLDERS_KEY = "linker-edit.sidebar.project-folders";

function read(): boolean {
  try {
    return localStorage.getItem(FOLDERS_KEY) === "true";
  } catch {
    return false;
  }
}

class SidebarPrefs {
  /** T3 Code's boring sidebar: threads in a folder per project instead of the inbox's shelves. Off by default. */
  folders = $state(read());

  setFolders(folders: boolean) {
    this.folders = folders;
    try {
      if (folders) localStorage.setItem(FOLDERS_KEY, "true");
      else localStorage.removeItem(FOLDERS_KEY);
    } catch {
      // Private windows: the choice lasts until the page closes.
    }
  }
}

export const sidebarPrefs = new SidebarPrefs();
