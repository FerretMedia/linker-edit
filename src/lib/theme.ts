// The look is this browser's choice, not the server's: one computer can be open on a light laptop and a dark
// desktop at once. src/app.html applies it before the first paint, from the same key.
export type Theme = "system" | "light" | "dark";

export const THEME_KEY = "linker-edit.theme";

export function loadTheme(): Theme {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    return saved === "light" || saved === "dark" ? saved : "system";
  } catch {
    return "system";
  }
}

export function applyTheme(theme: Theme) {
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
}

export function saveTheme(theme: Theme) {
  try {
    if (theme === "system") localStorage.removeItem(THEME_KEY);
    else localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Private windows: the choice lasts until the page closes.
  }
  applyTheme(theme);
}
