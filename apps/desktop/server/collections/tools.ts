// The brief's tools on the agents' "linker" MCP server: how an agent changes a workspace's or series' rules, adds
// a lesson learned from feedback, and keeps its notes on example videos. The rules and lessons steer every later
// edit, so the user accepts or declines each change, in full access too.
import { COLLECTION_LABEL } from "../../src/lib/labels.ts";

export const SAVE_BRIEF = "save_brief";
export const ADD_LESSON = "add_lesson";
export const SAVE_EXAMPLE = "save_example";

const where = {
  for: {
    type: "string",
    enum: ["workspace", "series"],
    description: `Whose: the ${COLLECTION_LABEL.workspace.lower} (the brand) or a ${COLLECTION_LABEL.series.lower} (one format). Defaults to the most specific one this thread's project is in.`,
  },
  series: {
    type: "string",
    description: `With for "series": the name of a ${COLLECTION_LABEL.series.lower} in this ${COLLECTION_LABEL.workspace.lower}, when it isn't this project's own.`,
  },
};

export const BRIEF_TOOLS = [
  {
    name: SAVE_BRIEF,
    description: `Propose new rules for the ${COLLECTION_LABEL.workspace.lower} or a ${COLLECTION_LABEL.series.lower}: the brief every edit in it follows. Pass the complete new text, not a patch: it replaces the rules. They are kept in its SKILL.md; leave front matter out, Linker-Edit writes it. The user sees what changes and accepts or declines. Naming a ${COLLECTION_LABEL.series.lower} that doesn't exist makes it.`,
    inputSchema: {
      type: "object",
      properties: {
        ...where,
        rules: { type: "string", description: "The complete new rules, in Markdown." },
        summary: { type: "string", description: "One or two sentences on what changes and why, for the user." },
      },
      required: ["rules", "summary"],
    },
    annotations: { readOnlyHint: false, destructiveHint: false },
  },
  {
    name: ADD_LESSON,
    description:
      "Propose one lesson from the user's feedback: a short rule that holds for the next videos too. The user sees it next to their own words and accepts or declines. Only after the user has seen your fix and it is what they meant, or when they ask you to remember something.",
    inputSchema: {
      type: "object",
      properties: {
        ...where,
        rule: { type: "string", description: 'The rule, concrete enough to follow without this chat: "Intro at most 3 seconds."' },
        feedback: { type: "string", description: "The user's own words this comes from, quoted, or what they changed in Resolve themselves." },
      },
      required: ["rule", "feedback"],
    },
    annotations: { readOnlyHint: false, destructiveHint: false },
  },
  {
    name: SAVE_EXAMPLE,
    description:
      "Keep your notes on one example of this brand, a video or a finished project, as a Markdown file next to its rules: what it is, its numbers, its structure with times, what stands out. Saving again under the same name replaces them.",
    inputSchema: {
      type: "object",
      properties: {
        ...where,
        name: { type: "string", description: "The video's file name or title, or the project's name." },
        notes: { type: "string", description: "Your notes, in Markdown." },
      },
      required: ["name", "notes"],
    },
    annotations: { readOnlyHint: false, destructiveHint: false },
  },
];

export const isBriefTool = (name: string) => name === SAVE_BRIEF || name === ADD_LESSON || name === SAVE_EXAMPLE;
