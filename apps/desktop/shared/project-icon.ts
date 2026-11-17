// Project icons as T3 Code has them (apps/web/src/projectIdentity.ts, packages/contracts/src/project.ts; MIT, see NOTICE).

import { PROJECT_ICON_COLORS, type ProjectIcon, type ProjectIconColor } from "./protocol.ts";

const LUCIDE_NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const MONOGRAM = /^[\p{L}\p{N}][\p{L}\p{N}\p{M}‌‍]*$/u;

const isColor = (value: unknown): value is ProjectIconColor => PROJECT_ICON_COLORS.includes(value as ProjectIconColor);

/** One or two letters or numbers, upper case; null when the text can't be a monogram. */
export function monogramText(value: string): string | null {
  const text = value.normalize("NFKC").trim().toUpperCase();
  if (!MONOGRAM.test(text) || Array.from(new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(text)).length > 2) {
    return null;
  }
  return text;
}

/** A saved or sent icon, checked; anything malformed is null, so it falls back to the monogram. */
export function parseProjectIcon(value: unknown): ProjectIcon | null {
  if (!value || typeof value !== "object") return null;
  const icon = value as Record<string, unknown>;
  switch (icon.kind) {
    case "lucide":
      return typeof icon.name === "string" && icon.name.length <= 64 && LUCIDE_NAME.test(icon.name) && isColor(icon.color)
        ? { kind: "lucide", name: icon.name, color: icon.color }
        : null;
    case "emoji": {
      const emoji = typeof icon.emoji === "string" ? firstEmoji(icon.emoji) : null;
      return emoji ? { kind: "emoji", emoji } : null;
    }
    case "monogram": {
      const text = typeof icon.text === "string" ? monogramText(icon.text) : null;
      return text && isColor(icon.color) ? { kind: "monogram", text, color: icon.color } : null;
    }
    default:
      return null;
  }
}

/** The first emoji in the text, or null. */
export function firstEmoji(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const segment = new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(trimmed)[Symbol.iterator]().next().value?.segment;
  const isFlag = /^\p{Regional_Indicator}{2}$/u.test(segment ?? "");
  const isKeycap = /^[#*0-9]️?⃣$/u.test(segment ?? "");
  if (!segment || (!/\p{Extended_Pictographic}/u.test(segment) && !isFlag && !isKeycap)) return null;
  return segment;
}

/** The monogram and color a project gets from its name until an icon is picked. */
export function deriveProjectIdentity(projectName: string): { monogram: string; color: ProjectIconColor } {
  const name = projectName.normalize("NFKC").trim();
  const words = name.match(/[\p{L}\p{N}]+/gu) ?? [];
  let monogram = "PR";
  const firstWord = words[0];
  if (firstWord) {
    const glyphs = Array.from(firstWord);
    const first = glyphs[0] ?? "P";
    const second =
      glyphs.slice(1).find((glyph) => /\p{N}/u.test(glyph)) ??
      (words.length > 1 ? Array.from(words.at(-1) ?? "")[0] : glyphs.at(-1)) ??
      first;
    monogram = Array.from(`${first}${second}`.toUpperCase()).slice(0, 2).join("");
  }
  const seed = name.toLocaleLowerCase("en-US") || "project";
  let index = 0;
  for (const glyph of seed) index = (index * 31 + (glyph.codePointAt(0) ?? 0)) % PROJECT_ICON_COLORS.length;
  return { monogram, color: PROJECT_ICON_COLORS[index] ?? "blue" };
}
