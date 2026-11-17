import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { agentRules, EDITING_RULES, prepareWorkspace } from "./rules.ts";

describe("prepareWorkspace", () => {
  it.each([
    ["creates the rules in a new workspace", null, true],
    ["refreshes rules it wrote before", "<!-- managed by Linker-Edit; edits here are overwritten -->\n\nold rules", true],
    ["leaves an AGENTS.md the user wrote alone", "# My own rules", false],
  ])("%s", (_name, existing, managed) => {
    const workspace = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-ws-")), "nested", "workspace");
    const agentsFile = path.join(workspace, "AGENTS.md");
    if (existing !== null) {
      fs.mkdirSync(workspace, { recursive: true });
      fs.writeFileSync(agentsFile, existing);
    }
    prepareWorkspace(workspace);
    const contents = fs.readFileSync(agentsFile, "utf8");
    expect(contents).toBe(managed ? contents : existing);
    expect(contents.includes(EDITING_RULES)).toBe(managed);
  });

  it("writes the project's own rules when it has them", () => {
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-ws-"));
    prepareWorkspace(workspace, agentRules([{ heading: 'Workspace "Ferret Media"', text: "End on the logo." }]));
    expect(fs.readFileSync(path.join(workspace, "AGENTS.md"), "utf8")).toMatch(/## Workspace "Ferret Media"\n\nEnd on the logo.\n$/);
  });
});

describe("agentRules", () => {
  it("puts each source under its own heading, in the order given, and leaves empty ones out", () => {
    const rules = agentRules([
      { heading: "Workspace", text: "General\n" },
      { heading: "Series", text: "   " },
      { heading: "Project", text: "Specific" },
    ]);
    expect(rules.slice(EDITING_RULES.length)).toMatch(/^\n\n# The user's rules\n\n.+\n\n## Workspace\n\nGeneral\n\n## Project\n\nSpecific$/);
    expect(agentRules([{ heading: "Series", text: "" }])).toBe(EDITING_RULES);
  });
});
