# Linker-Edit for iPad

A native SwiftUI app that drives Linker-Edit on your edit computer from an iPad. The agent still works in DaVinci Resolve on that computer; the iPad shows the chat as it streams in, the tool calls, the Allow/Deny cards in supervised mode, Resolve's status and the preview renders, and sends your messages back. It's a remote for the desktop app ("bridge A" in [the iPad plan](https://github.com/FerretMedia/linker-edit/blob/plan/ipad/docs/ipad-plan.md)), not an editor of its own.

> **Status: first version, never run on a real iPad.** It builds and its tests pass on the iPad simulator in CI, including connecting to a real Linker-Edit server there. Nobody has used it on an iPad, over Tailscale or through Linker Connect yet.

## What it does

- **Connect** to your edit computer by typing its remote access address, such as `https://studio.your-tailnet.ts.net:8443` (Settings → Remote access in Linker-Edit on the computer). The iPad needs Tailscale, logged in with the computer owner's account; Tailscale Serve tells the computer who it is, so the app sends no password. Recent computers are remembered.
- **Or sign in with Linker** (Linker Connect) and pick one of the computers linked to your Linker account. This needs the Linker side (see below) and is off in this build.
- **Projects and threads** in the sidebar; picking a thread opens it on the computer too, as on the desktop. A pencil per project starts a new thread.
- **The chat**: messages stream in, tool calls are one line each and open to show their input and output, and in supervised mode each change waits for **Allow** or **Deny**. **Stop** ends the turn. Full access and Supervised switch in the composer.
- **Resolve's status** in the toolbar: what Resolve has open, or why it can't be reached. Tap it to check again.
- **Preview**: renders the timeline open in Resolve on the computer and plays it here (AVPlayer, seeking with Range requests).

What it leaves out for now: the edit history, the library, settings, adding projects, choosing the agent, compaction and the context meter. The desktop app or a browser does those.

## Layout

```
apps/ipad
├── project.yml                  XcodeGen spec: the app and UI test targets
├── LinkerEdit/                  the app: SwiftUI views, AppModel, sign-in sheet, assets
├── LinkerEditUITests/           UI tests that also take the screenshots
└── Packages/LinkerEditKit/      everything testable without UI
    ├── Sources/LinkerEditKit/
    │   ├── Protocol.swift       Codable mirror of apps/desktop/shared/protocol.ts (what the iPad uses)
    │   ├── Store.swift          applies snapshot/meta/item like the web UI's appState.ts
    │   ├── HostConnection.swift URLSessionWebSocketTask to /ws, reconnecting
    │   ├── LinkerConnect.swift  ConnectAuth: Linker sign-in (PKCE), computers, host sessions
    │   └── …
    └── Tests/
        ├── LinkerEditKitTests/
        └── Fixtures/            real /ws messages recorded from the Node server
```

The server is the same one the desktop app runs. The app connects to its `/ws` with the host's own address as `Origin` (the server checks it) and plays `/preview/…` files from it.

Unknown message types, item kinds, states and fields decode without failing: the app skips what it doesn't know, so an iPad a release behind still works.

## Protocol fixtures

`Tests/Fixtures/server` holds every message the Node server sent in a scripted session (snapshot, meta, streamed items, an approval, a preview render, a refused message, a toast); `Tests/Fixtures/client` holds what the iPad sends, typed against the server's `ClientMessage`. The Swift tests decode and replay all of them. After a change to `apps/desktop/shared/protocol.ts`, record them again from the repository root:

```sh
RECORD_IPAD_FIXTURES=1 npm test -- scripts/ipad-fixtures.test.ts
```

and push: the iPad CI fails if the models can't read the new messages.

## How CI builds it

[`.github/workflows/ipad.yml`](../../.github/workflows/ipad.yml) runs on a macOS runner for pull requests and pushes to `main` that touch `apps/ipad/**` or the workflow, and by hand. It:

1. installs [XcodeGen](https://github.com/yonaskolb/XcodeGen) with Homebrew and runs `xcodegen generate`;
2. builds the app for an iPad simulator, unsigned (`CODE_SIGNING_ALLOWED=NO`);
3. starts the Linker-Edit server from this checkout on port 4390, with its own config (no Resolve or agents there);
4. runs the `LinkerEditKit` tests on the simulator, one of which connects to that server;
5. runs the UI tests, which take screenshots of the connect screen and a fixed demo chat and connect the app to that server, and uploads the screenshots as the `ipad-screenshots` artifact.

There is no Swift toolchain on Windows or Linux, so CI is the only way to check Swift changes without a Mac.

## On a Mac

With Xcode 16 or newer:

```sh
brew install xcodegen
cd apps/ipad
xcodegen generate
open LinkerEdit.xcodeproj
```

Run the `LinkerEdit` scheme on an iPad simulator. `LinkerEdit.xcodeproj` and `LinkerEdit/Info.plist` are generated and not committed; change `project.yml` instead. Launch arguments for trying it: `-demo` shows a fixed chat without a computer, `-host http://127.0.0.1:4317` connects at once (the simulator reaches the Mac's own `127.0.0.1`), `-reset` forgets recent computers.

## Linker Connect

The client side is built and unit tested against a mocked network; nothing on the server side exists yet except step 3's first request. The contract, shared with the server work:

1. **Sign in with Linker**: Clerk OAuth with PKCE in `ASWebAuthenticationSession`, back to `linkeredit://oauth/callback`. The client id and issuer come from the Info.plist keys `LinkerOAuthClientID` and `LinkerOAuthIssuer` (in `project.yml`), empty for now, which keeps sign-in off. Tokens are kept in the Keychain.
2. **Computers**: `GET https://linker.cool/api/edit/computers` with `Authorization: Bearer <Linker access token>`, answering `{ limit, computers: [{ id, name, url, online, linkedAt, lastSeenAt }] }`.
3. **A session on the computer**: `POST <computer>/api/connect/session` with the Linker token and `{ "name": "<device name>" }` answers `{ code }`; then `POST <computer>/api/connect/native` with `{ "code": "<code>" }` answers `{ "session": "<opaque>", "expiresAt": "<ISO 8601>" }`.
4. **Using it**: every request to the computer, the `/ws` WebSocket included, sends `Authorization: Bearer <session>`; AVPlayer gets it through `AVURLAssetHTTPHeaderFieldsKey`. The session is kept in the Keychain per computer. When the computer turns the app away (401), the session is dropped and the next open signs in again.

## Not done yet

- **Signing, TestFlight and the App Store**: no Apple Developer account yet, so the app can't be installed on an iPad. That's thread 7 in the plan.
- **Push notifications** for approvals and finished turns (thread 7).
- **"Send to iPad"** project files for Resolve for iPad (thread 5).
- **Linker Connect's server side**, and Linker's registration of the iPad app at Clerk.
- **Never run on a real iPad**, over Tailscale or Linker Connect, or against a host with Resolve and a real agent.
- The app icon is the desktop app's; it has an alpha channel, which App Store builds will need removed.
