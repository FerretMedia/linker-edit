# Your own Linker-Edit web address

**Goal:** anyone can put Linker-Edit's UI on their own Cloudflare account, open it in any browser, sign in with Linker, and drive one of their own edit computers over Tailscale or the Linker Connect tunnel. The edit computer still does all the work and keeps all the state; the web address only serves the UI's files.

**Status (2026-11-19):** a plan. Nothing is built yet. It builds on Linker Connect (`docs/sync-plan.md`) and needs its prerequisites first.

## What the person does

1. Click **Deploy to Cloudflare** in the README. It points at `apps/web` in this repo.
2. Cloudflare asks them to sign in to Cloudflare and GitHub, copies just that folder into a new small repo in their GitHub, and deploys it. There's nothing to fill in. They get `https://linker-edit-web.<name>.workers.dev`, and can add their own domain if they want.

   *Prefer a fork?* Fork `FerretMedia/linker-edit`, then in Cloudflare use **Import a repository** with root directory `apps/web`.
3. Open that address and click **Sign in with Linker**. The first time, Linker asks *"Let linker-edit-web.jane.workers.dev reach your edit computers?"* → **Allow**.
4. Pick a computer. They're in.

**Updating:** redeploy in Cloudflare. The deploy always fetches the newest UI. The page also shows a hint when your computer runs a newer Linker-Edit than the page.

On the edit computer, nothing new: Linker-Edit runs, it's signed in with Linker, and Tailscale and/or Linker Connect is on, as today.

## How it works

```
Browser (your address, e.g. edit.jane.com)          files only: HTML/JS/CSS from Cloudflare
  │
  ├─ Sign in with Linker ──────────────▶ app.linker.cool/edit/web/authorize
  │    ◀ web token (only for addresses your account allowed)
  │
  ├─ GET app.linker.cool/api/edit/computers ─▶ [{ name, url (tunnel), tailnetUrl, online }]
  │
  ├─ try tailnetUrl first (https://pc.tailnet.ts.net:8443), else the tunnel (https://edit-….linker.cool)
  │    POST /api/connect/session   Bearer <web token>  → host asks Linker who it is → one-time code
  │    POST /api/connect/native    { code }            → session (Bearer, 30 days)
  │
  └─ /ws and /api/* go straight to that computer with the session. Linker never sees the traffic.
```

It's the iPad app's way in (`docs/sync-plan.md` → Native clients), opened up to browsers. The only differences are how the browser gets its Linker token, and that every request is cross-origin.

## Decisions

### 1. Your copy fetches the UI from the app's own release

The release workflow already uploads the installers to GitHub Releases. It also uploads **`Linker-Edit-<version>-web.tar.gz`**: just the app's built UI folder (`apps/desktop/build`), a few MB, plus its `.sha256`. These are the same files the Electron app ships and serves, with no Electron and no server.

**`apps/web/` in this monorepo** holds no UI at all, only:
- **`get-ui.mjs`:** about 40 lines with no dependencies. It asks GitHub for the latest release (or `LINKER_EDIT_VERSION`, if set in Cloudflare's build variables), downloads the web tarball, checks its SHA-256 and unpacks it into `public/`.
- **`wrangler.jsonc`:** static assets from `public/` only, no Worker code, `"not_found_handling": "single-page-application"`, and `build.command: "node get-ui.mjs"`. Cloudflare runs that command on every deploy, so nobody fills anything in.
- **A README** with the 4 steps above.

It has no `package.json`, so it isn't an npm workspace and nothing gets installed. Its copy in someone's GitHub never needs syncing, because the deploy fetches the UI. Redeploying is the update. Static-only Workers are free and don't count against request limits.

**Why in the monorepo and not its own repo:** there's one place to change it, CI tests `get-ui.mjs` next to the release step that makes the tarball, and the Deploy button gives people a small repo anyway. **Why not build the UI in Cloudflare:** that would install the whole monorepo with Electron, which is slow and one more thing that can break. Downloading a finished tarball can't break that way.

### 2. Same UI, the same build

There's no separate web build. The UI decides when it starts:
- If the page's own origin answers `GET /api/hello` as Linker-Edit, a computer is serving it, and it works exactly as today.
- If not, it's on a web address, and it shows **a start screen:** Sign in with Linker, then a list of your computers. The last one picked opens again the next time.

For that to work, the UI stops assuming the server is the page's own address. Today it uses `location.host` (`src/lib/connection.svelte.ts`) and relative `/api`, `/preview`, `/media` and `/library` URLs in about 9 places. They all go through one `server.url(path)` and `server.ws()`.

The host keeps serving the same files, so the desktop app, Tailscale in a browser and the computer menu work as today.

### 3. Sign in with Linker for web addresses: Linker's own small sign-in

A Clerk OAuth app needs every redirect address registered up front, and every copy has its own address. So Linker hands out its own **web token**:

1. The page opens `app.linker.cool/edit/web/authorize?origin=…&state=…&code_challenge=…` (PKCE S256).
2. Linker uses its normal Clerk sign-in. The first time for that address, it asks to allow it, and remembers the address on the account.
3. Linker sends the page back to `<origin>/#code=…&state=…`. The page posts `{ code, code_verifier }` to `app.linker.cool/api/edit/web/token`, which allows CORS only for that origin, and gets `{ token: "lew_…", expiresAt }`. The token is good for 30 days.

The web token can **only** list your computers and prove who you are to them. It doesn't work for the AI connector, the page tools or anything else on Linker. **Linker → Settings → Linker-Edit → Web addresses** lists the allowed addresses. Removing one ends its tokens.

The consent screen says to allow only addresses you deployed yourself. Someone else's copy is code they control, just like any site you sign in to.

### 4. Tailscale first, the tunnel as fallback

- **The host tells Linker its Tailscale address when it links.** `PUT /api/edit/computers/:id` gets `{ tailnetUrl, tunnel: boolean }`. A computer can link **without a tunnel** (Tailscale only) and still shows in the list and counts toward the 3.
- **The page tries `tailnetUrl` first**, with `GET /api/hello` and a 2-second timeout. It's direct, fast and stays on your own network. If that fails, it uses the tunnel `url`.
- **Chrome asks once** on Tailscale, because 100.x addresses count as "local network" (Local Network Access, Chrome 142+): *"Allow … to access other apps and services on this device / local network"* → Allow. The start screen explains this before the prompt shows. If it's refused, the page uses the tunnel.

### 5. The same way in on both routes

One handshake for Tailscale and the tunnel: web token → code → Bearer session, as in the native flow that's already built. On Tailscale the host also still checks that `Tailscale-User-Login` is the owner. Because it's one flow, there's no second auth path to keep safe.

## What changes where

### Linker (FerretMedia/Linker, `apps/app`)

| Route | What it does |
|---|---|
| `GET /edit/web/authorize` | The sign-in page and the consent for a new address, then sends you back with a code. |
| `POST /api/edit/web/token` | Code + verifier → web token. CORS for that origin only. |
| `GET /api/edit/whoami` | `Bearer lew_…` → `{ sub }`. The host uses it to check web tokens, like Clerk userinfo for normal ones. |
| `GET /api/edit/web/origins` | The host's own Linker sign-in → your allowed addresses, so the host knows which origins to trust. |
| `GET /api/edit/computers` | Also takes web tokens, adds `tailnetUrl`, and allows CORS for your allowed addresses. |
| `PUT /api/edit/computers/:id` | Takes `tailnetUrl` and `tunnel: false`. |
| Settings → Web addresses | List and remove addresses. |

Tokens are kept as SHA-256 hashes, with rate limits on `/token` and `/whoami`.

### The edit computer (`apps/desktop/server`)

- **CORS for your allowed addresses:** the host fetches them from Linker (cached 5 minutes) and answers preflight `OPTIONS` without a session. It never allows credentials: Bearer only, no cookies.
- **Bearer sessions on Tailscale too,** not only through the tunnel, for those origins.
- **The `/ws` session goes in `Sec-WebSocket-Protocol`** (`linker-edit, bearer.<session>`), because a browser can't set headers on a WebSocket. The Origin must be one of your allowed addresses.
- **Media tickets:** `<video>` and `<img>` can't send headers, so the host sends a short-lived signed `?t=` ticket over `/ws` for `/preview`, `/media` and `/library`. It's renewed before it runs out. The session itself never goes in a URL.
- **`linker.whoIs`** sends `lew_…` tokens to Linker's `/api/edit/whoami` and others to Clerk userinfo, as today.
- **Linking** sends the Tailscale Serve address, and lets you link without a tunnel.
- **`/api/hello`** adds `protocol: <number>`, raised only for a breaking change. The page shows *"Redeploy your web address in Cloudflare"* or *"Update Linker-Edit on Studio-PC"*.

### The UI (`apps/desktop/src`)

`server.url` / `server.ws`, the check at start for whether a computer is serving the page, the web start screen, the web token and sessions kept in `localStorage` per computer, and a `401` → sign in to that computer again.

### The release and the template

- **`release.yml`** packs `apps/desktop/build` as `Linker-Edit-<version>-web.tar.gz` plus `.sha256` next to the installers.
- **`apps/web/`** (`get-ui.mjs`, `wrangler.jsonc`, README with the Deploy button). `get-ui.mjs` gets a unit test against a fake release. The main README links the button too.

## Order

| PR | Repo | Needs |
|---|---|---|
| **0. Live checks** (below) | none | Linker Connect prerequisites |
| **1. Web sign-in, tokens, origins, `tailnetUrl`** | Linker | none |
| **2. Host: CORS, Bearer on Tailscale, WS subprotocol, media tickets, `protocol`** | linker-edit | none (in parallel with 1) |
| **3. UI: `server.url`, the start check, the start screen** | linker-edit | 2 |
| **4. Web tarball in releases + `apps/web`** | linker-edit | 3 (the release step can land any time) |

Subagents in their own worktrees, as usual.

## Check live first (step 0)

1. Linker Connect's own prerequisites (`docs/sync-plan.md`): Linker sign-in working in production, and #219 deployed.
2. A test page on `*.workers.dev` opening `wss://<pc>.ts.net:8443/ws` from Chrome, Edge and Safari/iPad: does the Local Network Access prompt show, and does it work after Allow?
3. Tailscale Serve and the Cloudflare tunnel both pass `OPTIONS`, `Origin` and `Sec-WebSocket-Protocol` through unchanged.
4. Cloudflare's **Deploy button** with `…/tree/main/apps/web` copies only that folder. Both it and **Import a repository** with root directory `apps/web` must:
   - run `build.command` from an assets-only `wrangler.jsonc`
   - not try to install the monorepo's packages
   - reach `api.github.com` and the release downloads during the build

## Not in this plan

- Hosting it ourselves at an address like `edit.linker.cool`. It's the same template, so that's easy later.
- Running anything on Cloudflare besides files: no relay, no proxying of chat or video.
- Cookies across sites. Browsers block third-party cookies, so it's Bearer only.
