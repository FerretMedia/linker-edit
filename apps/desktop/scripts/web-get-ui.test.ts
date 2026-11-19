import { execFile, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

/**
 * apps/web/get-ui.mjs, which Cloudflare runs to fetch the UI for someone's own web address, against a fake GitHub:
 * the newest release has the web tarball the release workflow packs, an older one doesn't.
 */
const SCRIPT = path.resolve(import.meta.dirname, "../../web/get-ui.mjs");
const REPO = "FerretMedia/linker-edit";
const NAME = "Linker-Edit-2026.11.19-web.tar.gz";

const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-get-ui-"));
// The built UI, packed as release.yml packs apps/desktop/build: its contents at the archive's root.
const build = path.join(root, "build");
fs.mkdirSync(path.join(build, "_app", "immutable"), { recursive: true });
fs.writeFileSync(path.join(build, "index.html"), "<html>Linker-Edit</html>");
fs.writeFileSync(path.join(build, "_app", "immutable", "entry.js"), "start()");
execFileSync("tar", ["-czf", `../${NAME}`, "."], { cwd: build });
const tarball = fs.readFileSync(path.join(root, NAME));
const sha256 = createHash("sha256").update(tarball).digest("hex");

let server: http.Server;
let base: string;
/** What the fake download of the .sha256 answers; tests change it. */
let checksum = `${sha256}  ${NAME}\n`;
let rateLimited = false;
const seen: string[] = [];

beforeAll(async () => {
  server = http.createServer((req, res) => {
    seen.push(`${req.url} ${req.headers["user-agent"]} ${req.headers.authorization ?? ""}`);
    const json = (status: number, body: unknown) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    if (rateLimited) return json(403, { message: "API rate limit exceeded" });
    const asset = (name: string) => ({ name, browser_download_url: `${base}/download/${name}` });
    switch (req.url) {
      case `/repos/${REPO}/releases/latest`:
      case `/repos/${REPO}/releases/tags/v2026.11.19`:
        return json(200, { tag_name: "v2026.11.19", assets: [asset("Linker-Edit-2026.11.19-win-x64.exe"), asset(NAME), asset(`${NAME}.sha256`)] });
      case `/repos/${REPO}/releases/tags/v2026.11.14`:
        return json(200, { tag_name: "v2026.11.14", assets: [asset("Linker-Edit-2026.11.14-win-x64.exe")] });
      case `/download/${NAME}`:
        return res.writeHead(200).end(tarball);
      case `/download/${NAME}.sha256`:
        return res.writeHead(200).end(checksum);
      default:
        return json(404, { message: "Not Found" });
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

afterAll(() => {
  server.close();
});

/** Runs a copy of get-ui.mjs in its own folder, as Cloudflare does in the copied repository. */
async function run(env: Record<string, string> = {}): Promise<{ code: number; output: string; dir: string }> {
  const dir = fs.mkdtempSync(path.join(root, "web-"));
  fs.copyFileSync(SCRIPT, path.join(dir, "get-ui.mjs"));
  const { LINKER_EDIT_VERSION: _v, LINKER_EDIT_REPO: _r, LINKER_URL: _l, GITHUB_TOKEN: _t, ...rest } = process.env;
  return new Promise((resolve) => {
    execFile(
      process.execPath,
      [path.join(dir, "get-ui.mjs")],
      { cwd: dir, env: { ...rest, LINKER_EDIT_GITHUB_API: base, ...env } },
      (error, stdout, stderr) => resolve({ code: error ? Number(error.code ?? 1) : 0, output: stdout + stderr, dir }),
    );
  });
}

const read = (dir: string, file: string) => fs.readFileSync(path.join(dir, "public", file), "utf8");

describe("get-ui.mjs", () => {
  it("unpacks the newest release's UI into public/, checked against its .sha256", async () => {
    const { code, output, dir } = await run();
    expect(output).toContain("Linker-Edit 2026.11.19 is in public/");
    expect(code).toBe(0);
    expect(read(dir, "index.html")).toBe("<html>Linker-Edit</html>");
    expect(read(dir, "_app/immutable/entry.js")).toBe("start()");
    expect(fs.existsSync(path.join(dir, "public", "linker-edit-web.json"))).toBe(false);
    expect(fs.readdirSync(dir).sort()).toEqual(["get-ui.mjs", "public"]);
    expect(seen.at(-3)).toMatch(/^\/repos\/FerretMedia\/linker-edit\/releases\/latest linker-edit-web $/);
  });

  it("takes a release and Linker's address from the build variables", async () => {
    const { code, dir } = await run({ LINKER_EDIT_VERSION: "2026.11.19", LINKER_URL: "https://linker.example/", GITHUB_TOKEN: "ghp_x" });
    expect(code).toBe(0);
    expect(JSON.parse(read(dir, "linker-edit-web.json"))).toEqual({ linkerUrl: "https://linker.example" });
    expect(seen.at(-3)).toBe(`/repos/${REPO}/releases/tags/v2026.11.19 linker-edit-web Bearer ghp_x`);
    // The token only goes to GitHub's API, not to the downloads.
    expect(seen.at(-1)).not.toContain("ghp_x");
  });

  it.each([
    [{ LINKER_EDIT_VERSION: "2026.11.14" }, "has no Linker-Edit-2026.11.14-web.tar.gz"],
    [{ LINKER_EDIT_VERSION: "2099.1.1" }, "has no release v2099.1.1"],
    [{ LINKER_EDIT_REPO: "someone/else" }, "someone/else has no releases"],
    [{ LINKER_EDIT_REPO: "not a repo" }, "must look like owner/name"],
    [{ LINKER_URL: "javascript:alert(1)" }, "LINKER_URL must be an https:// address"],
  ])("says what's wrong: %j", async (env, message) => {
    const { code, output, dir } = await run(env);
    expect(code).not.toBe(0);
    expect(output).toContain(message);
    expect(fs.existsSync(path.join(dir, "public"))).toBe(false);
  });

  it("refuses a tarball that doesn't match its .sha256", async () => {
    checksum = `${"0".repeat(64)}  ${NAME}\n`;
    try {
      const { code, output, dir } = await run();
      expect(code).not.toBe(0);
      expect(output).toContain(`${NAME} doesn't match`);
      expect(fs.existsSync(path.join(dir, "public"))).toBe(false);
    } finally {
      checksum = `${sha256}  ${NAME}\n`;
    }
  });

  it("says when GitHub turns it away for asking too often", async () => {
    rateLimited = true;
    try {
      const { code, output } = await run();
      expect(code).not.toBe(0);
      expect(output).toContain("GITHUB_TOKEN");
    } finally {
      rateLimited = false;
    }
  });
});
