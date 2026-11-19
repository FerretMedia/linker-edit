// Puts Linker-Edit's UI from a GitHub release into public/, for Cloudflare to serve. Cloudflare runs this on every
// deploy (build.command in wrangler.jsonc), so redeploying updates it. Node 20+ built-ins only: there is no
// package.json here, so nothing gets installed.
//
// Build variables, all optional:
//   LINKER_EDIT_VERSION  a release such as 2026.11.19; the newest when unset
//   LINKER_EDIT_REPO     where the releases are, FerretMedia/linker-edit when unset
//   LINKER_URL           Linker, when it isn't https://app.linker.cool
//   GITHUB_TOKEN         only if GitHub turns the build away for asking too often
//   LINKER_EDIT_GITHUB_API  GitHub's API, for tests
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(here, "public");
const repo = process.env.LINKER_EDIT_REPO || "FerretMedia/linker-edit";
const wanted = (process.env.LINKER_EDIT_VERSION || "").trim().replace(/^v/, "");
const api = (process.env.LINKER_EDIT_GITHUB_API || "https://api.github.com").replace(/\/$/, "");
const token = process.env.GITHUB_TOKEN;

function fail(message) {
  console.error(`get-ui: ${message}`);
  process.exit(1);
}

async function get(url, what, headers = {}) {
  let res;
  try {
    res = await fetch(url, { headers: { "user-agent": "linker-edit-web", ...headers } });
  } catch (error) {
    fail(`Couldn't download ${what} from ${new URL(url).host}: ${error.cause?.message ?? error.message}`);
  }
  if (res.status === 403 || res.status === 429) {
    fail(`GitHub turned away the request for ${what} (${res.status}), most likely for asking too often. Deploy again in an hour, or add a GITHUB_TOKEN build variable.`);
  }
  if (res.status === 404 && what === "the release") {
    fail(wanted ? `${repo} has no release v${wanted}. Fix LINKER_EDIT_VERSION, or remove it for the newest.` : `${repo} has no releases, or isn't public.`);
  }
  if (!res.ok) fail(`Downloading ${what} failed: ${url} answered ${res.status}.`);
  return res;
}

if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) fail(`LINKER_EDIT_REPO must look like owner/name, not "${repo}".`);
let linkerUrl = null;
if (process.env.LINKER_URL) {
  let url = null;
  try {
    url = new URL(process.env.LINKER_URL);
  } catch {
    // Said below.
  }
  const local = url?.protocol === "http:" && ["localhost", "127.0.0.1"].includes(url.hostname);
  if (!url || (url.protocol !== "https:" && !local)) fail(`LINKER_URL must be an https:// address, not "${process.env.LINKER_URL}".`);
  linkerUrl = url.origin;
}

const release = await (
  await get(`${api}/repos/${repo}/releases/${wanted ? `tags/v${encodeURIComponent(wanted)}` : "latest"}`, "the release", {
    accept: "application/vnd.github+json",
    ...(token ? { authorization: `Bearer ${token}` } : {}),
  })
).json();
const version = String(release.tag_name ?? "").replace(/^v/, "");
const file = `Linker-Edit-${version}-web.tar.gz`;
const asset = (name) => (Array.isArray(release.assets) ? release.assets.find((entry) => entry.name === name) : undefined);
const tarball = asset(file);
const checksum = asset(`${file}.sha256`);
if (!tarball || !checksum) {
  fail(`Release v${version} of ${repo} has no ${file}: it's older than the web UI. Remove LINKER_EDIT_VERSION, or set a newer release.`);
}

const data = Buffer.from(await (await get(tarball.browser_download_url, file)).arrayBuffer());
const expected = (await (await get(checksum.browser_download_url, `${file}.sha256`)).text()).trim().split(/\s+/)[0]?.toLowerCase() ?? "";
if (!/^[0-9a-f]{64}$/.test(expected)) fail(`${file}.sha256 has no SHA-256 in it.`);
const actual = createHash("sha256").update(data).digest("hex");
if (actual !== expected) fail(`${file} doesn't match its ${file}.sha256 (${actual}, expected ${expected}). Deploy again; if it keeps happening, the release is broken.`);

fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });
const temp = path.join(here, ".linker-edit-web.tar.gz");
fs.writeFileSync(temp, data);
// A path relative to public/, with forward slashes, which every tar reads (GNU tar takes "C:" for a remote host).
const tar = spawnSync("tar", ["-xzf", "../.linker-edit-web.tar.gz"], { cwd: out, stdio: "inherit" });
fs.rmSync(temp, { force: true });
if (tar.error) fail(`Couldn't run tar to unpack ${file}: ${tar.error.message}`);
if (tar.status !== 0) fail(`tar couldn't unpack ${file}.`);
if (!fs.existsSync(path.join(out, "index.html"))) fail(`${file} has no index.html.`);
if (linkerUrl) fs.writeFileSync(path.join(out, "linker-edit-web.json"), `${JSON.stringify({ linkerUrl })}\n`);

console.log(`get-ui: Linker-Edit ${version} is in public/${linkerUrl ? `, signing in at ${linkerUrl}` : ""}.`);
