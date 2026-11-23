// A video started from a brief: what the New video form sends, and the files it becomes in the project's folder.
import * as fs from "node:fs";
import * as path from "node:path";
import type { VideoBrief } from "../../shared/protocol.ts";
import { expandHome } from "../folders.ts";
import { PROJECT_NOTES_FILE, readProjectFile, SCRIPT_FILE, writeProjectFile } from "../rules.ts";

const LIMITS: Record<keyof VideoBrief, { label: string; max: number }> = {
  title: { label: "title", max: 120 },
  footage: { label: "footage folder", max: 1_000 },
  concept: { label: "concept", max: 4_000 },
  script: { label: "script", max: 100_000 },
};

/** The brief as sent by a page, checked against this computer; or why a video can't start from it. */
export function parseBrief(value: unknown): VideoBrief | { refusal: string } {
  const sent = (value && typeof value === "object" ? value : {}) as Record<string, unknown>;
  const brief = { title: "", footage: "", concept: "", script: "" };
  for (const key of Object.keys(LIMITS) as (keyof VideoBrief)[]) {
    const text = sent[key];
    brief[key] = typeof text === "string" ? text.replaceAll("\r\n", "\n").trim() : "";
    const { label, max } = LIMITS[key];
    if (brief[key].length > max) return { refusal: `The ${label} can be up to ${max.toLocaleString("en-US")} characters.` };
  }
  brief.title = brief.title.replace(/\s+/g, " ");
  if (!brief.title) return { refusal: "Give the video a title." };
  if (!brief.footage) return { refusal: "Choose the folder with the footage." };
  brief.footage = expandHome(brief.footage);
  try {
    if (!path.isAbsolute(brief.footage) || !fs.statSync(brief.footage).isDirectory()) return { refusal: `"${brief.footage}" isn't a folder on this computer.` };
  } catch {
    return { refusal: `There's no folder "${brief.footage}" on this computer.` };
  }
  return brief;
}

/** Why a video can't start from a brief in this folder: it already has one. */
export function briefRefusal(dir: string, brief: VideoBrief): string | null {
  if (readProjectFile(dir).trim()) return `This project already has a ${PROJECT_NOTES_FILE}. Write to the agent instead, or make a project for the new video.`;
  if (brief.script && fs.existsSync(path.join(dir, SCRIPT_FILE))) return `This project already has a ${SCRIPT_FILE}. Move it away first.`;
  return null;
}

/** Writes PROJECT.md, and SCRIPT.md when there is a script. Returns why it couldn't, or null. */
export function writeBrief(dir: string, brief: VideoBrief): string | null {
  const refusal = briefRefusal(dir, brief);
  if (refusal) return refusal;
  const script = path.join(dir, SCRIPT_FILE);
  if (brief.script) {
    try {
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(script, `${brief.script}\n`, { flag: "wx" });
    } catch (error) {
      return `Couldn't save ${SCRIPT_FILE}: ${error instanceof Error ? error.message : String(error)}`;
    }
  }
  const about = [brief.concept, brief.script ? `Script: ${SCRIPT_FILE} in the working folder, ${brief.script.split(/\s+/).length.toLocaleString("en-US")} words.` : ""].filter(Boolean);
  const problem = writeProjectFile(dir, [`# ${brief.title}`, ...(about.length ? [`## Brief\n${about.join("\n\n")}`] : []), `## Footage\n- ${brief.footage}`].join("\n\n"));
  if (problem && brief.script) fs.rmSync(script, { force: true });
  return problem;
}
