# Linker in Linker-Edit

**Goal:** sign in with your Linker account, then work on your Linker projects from Linker-Edit: the videos in Linker's content planner that are at the Edit step. Signing in stays optional at every step. Linker-Edit without an account works exactly as before.

**Status (2026-11-11):** part 1, the sign-in, is built. It needs one small Linker change to work in production (see "Shipping part 1").

## How Linker signs people in

Checked against Linker's code and its live Clerk on 2026-11-11:

- **Clerk is Linker's OAuth server**, at `clerk.linker.cool`. Linker's AI connector (`mcp.linker.cool`) already uses it: Claude and ChatGPT sign in there and get an OAuth token, and `app.linker.cool/api/*` accepts that token for the endpoints that opt in (`api(run, { connector: 'pro' })`).
- **An app identifies itself to Clerk** in one of two ways:
  - Production has **Client ID Metadata Documents (CIMD)** on: the client id is the address of a JSON file that describes the app.
  - **Dynamic Client Registration is off.** Clerk answers "dynamic client registration is not enabled".
  - Staging's Clerk (the development instance) has neither switched on.
- **`GET /api/connector` answers on any plan** with the plan's name. It is the one endpoint a token can call today without being on Pro or Max. It answers 403 when the person switched Linker's AI connector off.

## Part 1: sign in (this PR)

| Piece | Where |
|---|---|
| OAuth code flow with PKCE, refresh, sign-out | `server/linker.ts` |
| `GET /api/linker/sign-in` redirects to Linker; `GET /api/linker/callback` finishes | `server/http.ts` |
| Account row at the bottom of the sidebar | `src/lib/components/LinkerAccount.svelte` |
| The client file Clerk reads | Linker: `apps/app/src/routes/oauth/linker-edit.json` (separate PR) |

- **The browser comes back to this server**, at `http://127.0.0.1:4317/api/linker/callback` (RFC 8252). So signing in works only on the edit machine itself. Other computers using it over Tailscale see "Sign in with Linker on <computer>".
- **Scopes:** `openid profile email offline_access`. The name, email and picture come from Clerk's `/oauth/userinfo`, and the plan from `/api/connector`.
- **The sign-in is kept in `~/LinkerEdit/linker-account.json`** (mode 0600). The token refreshes itself. Linker refusing the refresh signs you out with a message; being offline doesn't.
- **Which client is used**, in this order:
  1. `linkerClientId` from the config.
  2. Linker's CIMD, when it is live and lists this server's callback.
  3. Registering itself, which only works if Linker ever turns Dynamic Client Registration on.
- **Nothing depends on being signed in.**

### Shipping part 1

1. Merge the Linker PR into `dev`. The nightly release takes it to production, and then `https://app.linker.cool/oauth/linker-edit.json` answers.
2. Until then, pressing Sign in says "Linker doesn't accept Linker-Edit's sign-in yet."
3. Then try it once for real. Two things are still unknown:
   - What Clerk's consent screen shows for a CIMD client.
   - Whether Clerk ignores the port of a loopback callback. If it does, users who change `port` can sign in too, and the port check in `linker.ts` can go.
4. To test on staging, switch on "Publish CIMD support" in the development instance. Then set `linkerAuthServer: "https://normal-giraffe-32.clerk.accounts.dev"` and `linkerAppUrl: "https://app.staging.linker.cool"`.

## Part 2: Linker-Edit as a Linker app, not an AI connector

Today Linker treats every OAuth token as an AI connector:

- the plan decides access: the page from Pro, numbers and clients from Max
- the person's "AI connector" switch can turn it off
- only the endpoints in `connector.spec.ts` are allowed

That fits Claude acting on your page. It doesn't fit your own editor opening your planner.

**Recommendation:** Linker recognizes Linker-Edit's tokens by their client id (`…/oauth/linker-edit.json`). It gives them their own short allowlist, named `app: 'linker-edit'` next to `connector`, and keeps them out of the AI connector switch. The plan rules stay the app's, as for the dashboard.

The alternative is to opt the planner endpoints into the connector at `'pro'`. That's quicker, but then turning off AI apps would also cut off the editor.

## Part 3: Linker projects in Linker-Edit

Linker's planner moves each video through **Idea → Script → Film → Edit → Publish → Published**. Linker-Edit is the Edit step.

1. **"From Linker" in Add project:** list the planner items at Film or Edit, from `GET /api/planner/items`. Items assigned to you come first.
2. **Starting one makes a Linker-Edit project linked to the item.** `linker-edit.json` gets a portable `linker: { itemId, orgId }` next to the Resolve link, so a teammate's clone knows it too.
3. **The project shows the item:** its title, publish date and when the edit is due (red when late), plus a button that opens the script (Obsidian note or Google Doc).
4. **"Ready to publish"** moves the item to Publish (`POST /api/planner/move`). Later it could attach the render or its link.
5. **The agents get the item as context:** the script and the notes. "Cut this to the script" then needs no copy and paste.

Linker endpoints part 3 needs, behind part 2's allowlist:

| Endpoint | For |
|---|---|
| `GET planner/items` | the list in Add project |
| `GET`/`PATCH planner/items/[id]` | the item's card |
| `POST planner/move` | Ready to publish |

## Part 4: teams

Linker teams are Clerk organizations, with the roles Admin, Editor and Viewer. A token says who, not where:

- Linker-Edit uses the organization picked in Linker's settings, as the connector does.
- Or Linker-Edit gets its own picker, with the `user:org:read` scope.

A Viewer reads and can't move items. This meets the multi-user edit history goal: a team's project could share its Git history through Linker later. That's an open question, not part of this plan.

## Part 5: Linker tools for the agents

When you're signed in, attach Linker's MCP server (`https://mcp.linker.cool/mcp`) to Claude and Codex with your token. Then "put the new video on my Linker page" or "make a tracked link for the upload" just work.

Linker writes go through the same Allow/Deny as Resolve writes (`server/policy.ts`). This follows the connector's plan rules (Pro and Max), which is right here because it is an AI acting.

## Later, if real use asks for it

- **Keep the token in the system keychain** (Electron `safeStorage`) instead of a 0600 file.
- **Sign in from a remote computer:** Clerk supports the device code grant. Its code can be typed on any device.
- **Give the consent screen a logo** (`logo_uri` in the client file).
