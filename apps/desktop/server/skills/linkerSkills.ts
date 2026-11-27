// Linker's skills API, /api/edit/skills, with the Linker sign-in's token: the person's own skills and their orgs'.
import { SKILL_NAME } from "./skills.ts";
import { SkillsRefusal, type RemoteSkill, type SkillFiles, type SkillsApi } from "./sync.ts";

const NETWORK_TIMEOUT = 15_000;
// A path as Linker keeps one: parts of letters, digits, dots, dashes and underscores, never "." or "..".
const PATH_PART = /^[A-Za-z0-9._-]+$/;

/** A skill in Linker's answer, if it is one that can be written here. */
export function parseSkill(value: unknown): RemoteSkill | null {
  const skill = value as Partial<RemoteSkill> | null;
  if (typeof skill?.owner !== "string" || typeof skill.name !== "string" || !SKILL_NAME.test(skill.name)) return null;
  if (typeof skill.revision !== "number" || !skill.files || typeof skill.files !== "object") return null;
  const files: SkillFiles = {};
  for (const [file, text] of Object.entries(skill.files)) {
    const safe = file.split("/").every((part) => PATH_PART.test(part) && part !== "." && part !== "..");
    if (!safe || typeof text !== "string") return null;
    files[file] = text;
  }
  return "SKILL.md" in files ? { owner: skill.owner, name: skill.name, files, revision: skill.revision } : null;
}

export function createLinkerSkills(options: { appUrl: string; accessToken: () => Promise<string | null>; fetch?: typeof fetch }): SkillsApi {
  const fetchFn = options.fetch ?? fetch;
  const base = `${options.appUrl.replace(/\/$/, "")}/api/edit/skills`;
  const at = (owner: string, name: string) => `${base}/${encodeURIComponent(owner)}/${encodeURIComponent(name)}`;

  async function call(method: string, url: string, headers: Record<string, string> = {}, body?: string): Promise<Response> {
    const token = await options.accessToken().catch(() => {
      throw new SkillsRefusal("offline");
    });
    if (!token) throw new SkillsRefusal("signed_out");
    const res = await fetchFn(url, {
      method,
      headers: { authorization: `Bearer ${token}`, accept: "application/json", ...(body ? { "content-type": "application/json" } : {}), ...headers },
      body,
      signal: AbortSignal.timeout(NETWORK_TIMEOUT),
    }).catch(() => {
      throw new SkillsRefusal("offline");
    });
    if (res.ok || res.status === 304) return res;
    const refusal = (await res.json().catch(() => ({}))) as { error?: unknown; message?: unknown; limit?: unknown };
    // A 503 or 401 from in front of Linker has no code of Linker's.
    const code = res.status === 503 ? "unavailable" : res.status === 401 ? "unauthorized" : typeof refusal.error === "string" ? refusal.error : `http_${res.status}`;
    throw new SkillsRefusal(
      code,
      typeof refusal.message === "string" ? refusal.message : `Linker answered ${res.status}.`,
      typeof refusal.limit === "string" ? refusal.limit : null,
    );
  }

  return {
    async list(etag) {
      const res = await call("GET", base, etag ? { "if-none-match": etag } : {});
      if (res.status === 304) return null;
      const skills = ((await res.json().catch(() => null)) as { skills?: unknown } | null)?.skills;
      if (!Array.isArray(skills)) throw new SkillsRefusal("invalid_answer", "Linker's list of skills didn't make sense.");
      return { etag: res.headers.get("etag") ?? "", skills: skills.flatMap((skill) => parseSkill(skill) ?? []) };
    },

    async put(owner, name, files, ifMatch) {
      const res = await call("PUT", at(owner, name), ifMatch === null ? {} : { "if-match": String(ifMatch) }, JSON.stringify({ files }));
      const skill = parseSkill(((await res.json().catch(() => null)) as { skill?: unknown } | null)?.skill);
      if (!skill) throw new SkillsRefusal("invalid_answer", "Linker's answer didn't make sense.");
      return skill;
    },

    async remove(owner, name) {
      await call("DELETE", at(owner, name));
    },
  };
}
