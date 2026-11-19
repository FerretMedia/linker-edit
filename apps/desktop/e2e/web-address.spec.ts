import { expect, test, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as path from "node:path";

// The built UI served the way someone's own Cloudflare address serves it: files only, and the page itself for
// every other path (/api/hello included). Linker and the edit computer are stand-ins on made-up addresses.
const build = path.resolve("build");
const LINKER = "https://linker.test";
const TAILNET = "https://studio.tail1.ts.net:8443";
const TUNNEL = "https://edit-a.linker.test";

let site: http.Server;
let origin: string;

test.beforeAll(async () => {
  site = http.createServer((req, res) => {
    const urlPath = decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname);
    if (urlPath === "/linker-edit-web.json") {
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ linkerUrl: LINKER }));
      return;
    }
    const file = path.join(build, urlPath);
    const found = file.startsWith(build) && fs.existsSync(file) && fs.statSync(file).isFile() ? file : path.join(build, "index.html");
    const type = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml" }[path.extname(found)];
    res.writeHead(200, { "content-type": type ?? "application/octet-stream" }).end(fs.readFileSync(found));
  });
  await new Promise<void>((resolve) => site.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${(site.address() as AddressInfo).port}`;
});

test.afterAll(() => new Promise<void>((resolve) => site.close(() => resolve())));

const cors = { "access-control-allow-origin": "*", "access-control-allow-headers": "authorization, content-type" };
const answer = (route: Route, body: unknown, status = 200) =>
  route.request().method() === "OPTIONS"
    ? route.fulfill({ status: 204, headers: cors })
    : route.fulfill({ status, headers: cors, contentType: "application/json", body: JSON.stringify(body) });

test("a web address signs in with Linker and lists the computers, Tailscale first, then the tunnel", async ({ page }) => {
  const tokenRequests: unknown[] = [];
  await page.route(`${LINKER}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/edit/web/authorize") {
      expect(url.searchParams.get("origin")).toBe(origin);
      expect(url.searchParams.get("code_challenge_method")).toBe("S256");
      const back = `${origin}/#linker_code=c1&state=${url.searchParams.get("state")}`;
      await route.fulfill({ contentType: "text/html", body: `<script>location.replace(${JSON.stringify(back)})</script>` });
    } else if (url.pathname === "/api/edit/web/token") {
      if (route.request().method() === "POST") tokenRequests.push(route.request().postDataJSON());
      await answer(route, { token: "lew_test", expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    } else if (url.pathname === "/api/edit/computers") {
      await answer(route, {
        computers: [
          { id: "a", name: "Studio-PC", url: TUNNEL, tailnetUrl: TAILNET, online: true },
          { id: "b", name: "Old-Laptop", url: null, tailnetUrl: null, online: null },
        ],
      });
    } else {
      await route.abort();
    }
  });
  // Tailscale isn't reachable from here; the tunnel is.
  await page.route(`${TAILNET}/**`, (route) => route.abort());
  await page.route(`${TUNNEL}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/hello") await answer(route, { app: "linker-edit", name: "Studio-PC", version: "2026.11.14", protocol: 1 });
    else if (url.pathname === "/api/connect/session") await answer(route, { code: "one-time" });
    else if (url.pathname === "/api/connect/native") await answer(route, { session: "s1", expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    else if (url.pathname === "/api/connect/media-ticket") await answer(route, { ticket: "t1", expiresAt: new Date(Date.now() + 3_600_000).toISOString() });
    else await route.abort();
  });

  await page.goto(origin);
  await page.getByRole("button", { name: "Sign in with Linker" }).click();

  // Back from Linker: the code is swapped for a token, and gone from the address.
  await expect(page.getByRole("heading", { name: "Your computers" })).toBeVisible();
  expect(new URL(page.url()).hash).toBe("");
  expect(tokenRequests).toEqual([{ code: "c1", code_verifier: expect.stringMatching(/^[\w-]{43}$/), origin }]);

  // Before the first try over Tailscale, it says why the browser may ask about the local network.
  await expect(page.getByText("may ask to let this page reach devices on your local network")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();

  const studio = page.getByRole("button", { name: /Studio-PC/ });
  await expect(studio).toContainText("Through Linker Connect");
  await expect(page.getByRole("button", { name: /Old-Laptop/ })).toContainText("offline");

  // Picking it opens the app on that computer.
  await studio.click();
  await expect(page.getByText("Connecting to the Linker-Edit server…")).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("linker-edit:web-sessions") ?? "{}"))).toMatchObject({ a: { session: "s1" } });
});
