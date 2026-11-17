# Linker Connect: your edit computers from anywhere, like T3 Code

**Goal:** sign in with your Linker account and reach your own edit computers from any of your other computers, without setting up Tailscale. **Each Linker account links up to 3 edit computers** (the hosts), like T3 Connect's 3 machines. The computers you only drive them from don't count.

**Status (2026-11-17):** the host side is merged (`feat/sync`) and the relay is in FerretMedia/Linker#219. The native iPad app's way in is built on the host side (`feat/connect-native`); what Linker must add for it is under "Native clients".

## How T3 Code does it

T3 Code doesn't copy data between computers. The sources are `docs/user/remote-access.md` and `docs/internals/t3-connect.md` in pingdotgg/t3code.

- **The host keeps everything.** The computer that runs the agents holds the threads, projects and settings. Every other device connects to it and streams that state live, so there is only one copy, and it always looks the same.
- **T3 Connect gets you there from anywhere.**
  1. You sign in (Clerk) and switch T3 Connect on for the host.
  2. A relay gives the host its own Cloudflare Tunnel address, which `cloudflared` keeps open.
  3. Devices then talk to the host through the tunnel. The relay never sees the traffic.
- **Each account links up to 3 machines.** Phones and browsers don't count.
- **Other routes keep working:** LAN pairing, Tailscale and SSH.

## What Linker-Edit already has

- **One copy of the state, streamed live.** `apps/desktop/server/app.ts` sends every open window a snapshot plus patches.
- **Remote access over Tailscale** (`apps/desktop/server/remote.ts`). It only lets in the owner's own devices.
- **The computer menu**, which opens another computer in the window.
- **Sign in with Linker** (`apps/desktop/server/linker.ts`), which is Clerk, the same kind of login T3 Connect uses.

## The design

### Linker's side: the relay

The relay is in `app.linker.cool`, next to Linker's other API, in its database. It's called with the person's own Linker sign-in (`Authorization: Bearer <token>`). It isn't tied to the AI connector switch or to a plan.

| Route | What it does |
|---|---|
| `GET /api/edit/computers` | Your linked computers, each with its address and whether it's online now. |
| `PUT /api/edit/computers/:id` `{ name, port }` | Links this computer, or refreshes it. Gives it a Cloudflare Tunnel and an address `edit-<16 hex>.linker.cool`, and returns the tunnel's token. Answers 409 `limit` when 3 others are already linked. |
| `DELETE /api/edit/computers/:id` | Unlinks it. The tunnel and the address go away. |

- **The tunnel only forwards to `http://127.0.0.1:<port>`** on that computer, the port Linker-Edit listens on.
- **One dot below `linker.cool`**, because Cloudflare's free certificate covers `*.linker.cool` only.
- **The relay never sees chat traffic.** It only creates and removes tunnels.

### The edit computer (host)

1. Under Settings → Linker Connect, switching it on links the computer with the owner's Linker sign-in. It gets its own id, kept in `~/LinkerEdit/connect.json`.
2. Linker-Edit downloads a **pinned `cloudflared`**, checked against its hash, and runs it with the tunnel token. It stops with the app. The computer is reachable while Linker-Edit runs on it, like T3's desktop app.
3. **Nothing gets in through the tunnel without a session.** The only exceptions are `/api/hello` and the two sign-in routes below.

### Opening a computer from another one

1. Your other computer signs in with the same Linker account in its own Linker-Edit.
2. Its computer menu lists your linked computers from the relay, next to the Tailscale ones.
3. Picking one makes your Linker-Edit call `POST https://edit-….linker.cool/api/connect/session` with your Linker token.
4. The edit computer asks Linker who that is. If it's the account signed in on the edit computer, it answers with a one-time code that lasts 60 seconds.
5. The window opens `…/api/connect/enter?code=…`. The edit computer sets a session cookie (HttpOnly, Secure, 30 days) and shows the app.

Unlike T3's relay handshake, the Linker token here goes to your own computer, which already holds the same account's sign-in. So there's no extra signing step to copy.

- **Settings → Linker Connect** lists the devices with a session, and you can sign any of them out.
- **Turning Linker Connect off, or signing out of Linker** on the edit computer, ends every session.
- **Tailscale stays** as a second route.

## Native clients

The native iPad app (`apps/ipad`) reaches your edit computers the same way, without Tailscale. It has no window to hold a cookie, so the host gives it a session it sends as a Bearer token instead. Plain browsers are not supported: a hosted browser sign-in was started and dropped (it needs a confidential OAuth client and new pages on Linker, and a host that bounces Clerk codes is a risk, see below).

### The flow

```
iPad ─ ASWebAuthenticationSession: Clerk /oauth/authorize (PKCE S256), redirect linkeredit://oauth/callback
     ◀ code → POST Clerk /oauth/token (code + verifier, no secret) → Linker access token (+ refresh token)
iPad ─ GET https://app.linker.cool/api/edit/computers   Authorization: Bearer <Linker token> ─▶ Linker
     ◀ { limit, computers: [{ id, name, url, online, linkedAt, lastSeenAt }] }
iPad ─ POST https://edit-x.linker.cool/api/connect/session   Bearer <Linker token>  { "name": "Jane's iPad" } ─▶ host
host:  userinfo(token).sub is the account that linked it and the one signed in here? no → 403
     ◀ { code }   one-time, 60 s, only on this host
iPad ─ POST https://edit-x.linker.cool/api/connect/native   { "code": "<code>" } ─▶ host
     ◀ { session, expiresAt }   random, 30 days; the host keeps only its SHA-256
iPad ─ every request and the /ws upgrade: Authorization: Bearer <session>
```

### The host's side (built, `apps/desktop/server`)

- **`POST /api/connect/native`**, through the tunnel only, takes JSON `{ "code" }` and answers `200 { "session": "<43 base64url chars>", "expiresAt": "<ISO 8601>" }` with `cache-control: no-store`. A wrong, used or older-than-60-s code is `401`; Linker Connect off is `503`; more than 20 tries a minute is `429`; a body that isn't JSON is `400`; any method but POST is `405`. Every refusal is `{ "error": "<words for the user>" }`.
- **Every route and `/ws` take `Authorization: Bearer <session>`** on requests through the tunnel (those with `CF-Connecting-IP`): the app, `/api/*`, and the media routes `/preview/*`, `/media/*`, `/library/*` and `/api/timeline`, Range requests included, so AVPlayer can send the header. `/ws` takes a Bearer session with no `Origin` (native apps send none) or this computer's own; a cookie still needs this computer's own `Origin`.
- **A refused Bearer request is `401`** with `www-authenticate: Bearer error="invalid_token"` and a JSON `{ error }`. The iPad app then signs in again: a new code, a new session.
- **A session works only the way it was given out.** A cookie's token is never taken as a Bearer token, nor a Bearer session as a cookie; a Bearer header that isn't a session doesn't fall back to a cookie sent with it.
- **Exactly as strong as the cookie.** Same 30 days from sign-in (not extended by use), same owner check before the code, and the same ways out: *Sign out* in Settings → Remote access → Signed-in devices (it shows *Native app*, when it signed in and when it was last used), turning Linker Connect off, signing out of Linker on the host, unlinking it, or another Linker account linking it. Signing out closes its open `/ws` at once.
- **Only through the tunnel.** A Bearer header changes nothing for this computer itself or for Tailscale.
- **Secrets stay out of logs and addresses.** The code and the session only travel in bodies and headers; codes and sessions are compared by SHA-256 in constant time.

### What the iPad app must do

- Sign in with the Clerk OAuth app below: `ASWebAuthenticationSession`, `callbackURLScheme: "linkeredit"`, PKCE S256, a random `state` checked on return. Discovery: `https://clerk.linker.cool/.well-known/openid-configuration` (`authorization_endpoint`, `token_endpoint`). Keep the tokens in the Keychain.
- List computers at **`https://app.linker.cool/api/edit/computers`** (Linker's API lives on `app.`, not on `linker.cool`).
- **Send the Linker token only to a computer from that list,** whose `url` matches `^https://edit-[0-9a-f]{16}\.linker\.cool$` (staging: `edit-staging-`). Any Linker user can link a computer, so an address from anywhere else may be someone else's, and the Linker token is good for the whole account.
- Send a `name` to `/api/connect/session` (at most 64 characters, such as the device name); it's what Settings shows. Without one it shows "Another computer".
- Keep one session per computer (by `id`), in the Keychain. On `401` from any route or a refused `/ws` upgrade, drop it and sign in to that computer again. Don't wait for `expiresAt`; a session can end sooner.

### Linker's part (FerretMedia/Linker, `apps/app`)

**1. A Clerk OAuth application for the iPad app** (Clerk dashboard → Configure → OAuth applications), one per instance (production `clerk.linker.cool`, and staging):

| Setting | Value |
|---|---|
| Name | `Linker-Edit for iPad`, shown on the consent screen |
| Public client | **On**: no client secret, the app can't keep one. PKCE (S256) is required. |
| Redirect URI | exactly `linkeredit://oauth/callback`. No others, no wildcards. |
| Scopes | `openid profile email offline_access`, as the desktop app asks: `openid` for `sub` (what the host checks), `profile email` to show who's signed in, `offline_access` for a refresh token so the iPad doesn't sign in again every time the access token runs out. |
| Consent screen | On or off; it's Linker's own app. |

Its client id goes into the iPad app's build settings (it's public). If Clerk refuses a custom-scheme redirect URI, the fallback is a universal link, `https://app.linker.cool/oauth/ipad/callback`, with `/.well-known/apple-app-site-association` on `app.linker.cool` naming the app, and `ASWebAuthenticationSession`'s `.https(host:path:)` callback (iOS 17.4+). Check this first.

**2. Let that client list computers, and only list them.** `EditComputers.ownerOf` checks `EDIT_CONNECT_CLIENT_IDS` for every route. Add a second list, `EDIT_CONNECT_VIEW_CLIENT_IDS`, for clients that may only `GET /api/edit/computers`: the iPad's client id goes there, never in `EDIT_CONNECT_CLIENT_IDS`, so a token from it can't link or unlink computers (`PUT`/`DELETE` answer 401 `unauthorized`). While `EDIT_CONNECT_CLIENT_IDS` is unset any token may do everything, as today; set both lists when the desktop's OAuth app exists. Tests: the iPad's client id lists; it gets 401 on `PUT` and `DELETE`; an unknown client id gets 401 on all three once the lists are set.

**3. What `GET /api/edit/computers` must answer** (it already does, keep it): `200 { "limit": 3, "computers": [ { "id", "name", "url", "online", "linkedAt", "lastSeenAt" } ] }`, the person's own computers only, `url` the full `https://edit-<16 hex>.linker.cool` with no path, `online` `true`/`false`/`null`, `cache-control: no-store`; `401 { "error": "unauthorized" }` for a missing or refused token, which the iPad takes as "sign in to Linker again"; `503 { "error": "unavailable" }` while Linker Connect isn't set up. No CORS is needed: a native app isn't a page.

**4. Keep the AI connector out of it.** `modules/connector` takes any Clerk OAuth token. Refuse the iPad's client id there (the same check by `clientId` the edit routes use), so its token only lists computers and proves who you are to your own computers.

Nothing else changes on Linker: the code and the session are made and checked on the host, and Linker never sees them.

**Why not let the host do the OAuth itself** (with a fixed Linker redirect that bounces the code back to the host): the token would reach whichever host started the sign-in before anyone checks the computer is yours. Any Linker user can link a computer, run a modified Linker-Edit and send you its address. The iPad app signs in with Clerk itself and only shows its token to computers on its own list.

### Not tested yet

Nothing here has run against real Clerk or a real tunnel: the OAuth app doesn't exist, Linker's sign-in doesn't work in production yet (below), and #219 isn't deployed. The host's side is covered by unit tests with a stand-in for Linker.

## Prerequisites before it works for real

1. **Sign in with Linker has to work in production.** Clerk can't read the CIMD (see `docs/linker-plan.md`). The fix is a public OAuth app in Clerk's dashboard, with its client id built in as the default.
2. **A Cloudflare API token on Linker's account** that can edit Cloudflare Tunnels and the `linker.cool` DNS. It's kept as a Worker secret and never put in the app.
3. **Check tunnel cost and the video terms.** T3 reclaims idle tunnels because Cloudflare bills them. Preview videos would also stream through the tunnel.

## Later

- Approving new devices with a code (T3's optional device approval).
- Phones and plain browsers, through a hosted sign-in page.
- A background service, so the edit computer stays reachable without the window open.
