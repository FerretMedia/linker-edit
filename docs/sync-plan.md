# Linker Connect: your edit computers from anywhere, like T3 Code

**Goal:** sign in with your Linker account and reach your own edit computers from any of your other computers, without setting up Tailscale. **Each Linker account links up to 3 edit computers** (the hosts), like T3 Connect's 3 machines. The computers you only drive them from don't count.

**Status (2026-11-16):** being built. The Linker side is in a FerretMedia/Linker PR and this side is in `feat/sync`.

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

- **One copy of the state, streamed live.** `server/app.ts` sends every open window a snapshot plus patches.
- **Remote access over Tailscale** (`server/remote.ts`). It only lets in the owner's own devices.
- **The computer menu**, which opens another computer in the window.
- **Sign in with Linker** (`server/linker.ts`), which is Clerk, the same kind of login T3 Connect uses.

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

## Prerequisites before it works for real

1. **Sign in with Linker has to work in production.** Clerk can't read the CIMD (see `docs/linker-plan.md`). The fix is a public OAuth app in Clerk's dashboard, with its client id built in as the default.
2. **A Cloudflare API token on Linker's account** that can edit Cloudflare Tunnels and the `linker.cool` DNS. It's kept as a Worker secret and never put in the app.
3. **Check tunnel cost and the video terms.** T3 reclaims idle tunnels because Cloudflare bills them. Preview videos would also stream through the tunnel.

## Later

- Approving new devices with a code (T3's optional device approval).
- Phones and plain browsers, through a hosted sign-in page.
- A background service, so the edit computer stays reachable without the window open.
