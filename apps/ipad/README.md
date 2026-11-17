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

## Design

The app needs **iPadOS 26** and uses Apple's Liquid Glass. Following Apple's guidance, glass is for the controls that float above the content, not the content:

- **The system's own parts** get glass by themselves: the split view's sidebar, the toolbar (sidebar button, Resolve's status, Preview), the inspector, popovers and the video player's controls. The app paints no backgrounds over them.
- **Floating controls** are glass: the composer at the bottom of the chat, its send button (which turns into Stop while the agent works), toasts, Allow and Deny, and the buttons on the connect screen and in the preview.
- **Content stays plain and readable**: messages, tool calls, the approval card (with an orange border while it waits for you) and code are on the system's ordinary backgrounds.

It follows the iPad's light or dark mode, with the system's colours and Linker-Edit's blue as the accent.

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

The server is the same one the desktop app runs. Over Tailscale the app connects to its `/ws` with the host's own address as `Origin` (the server checks it), and it plays `/preview/…` files from it.

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
5. runs the UI tests, which take landscape screenshots of the connect screen and a fixed demo chat (light and dark) and connect the app to that server, and uploads the screenshots as the `ipad-screenshots` artifact.

It picks the runner's newest Xcode 26 and an iOS 26 iPad simulator, since the app needs the iOS 26 SDK.

There is no Swift toolchain on Windows or Linux, so CI is the only way to check Swift changes without a Mac.

## On a Mac

With Xcode 26 or newer:

```sh
brew install xcodegen
cd apps/ipad
xcodegen generate
open LinkerEdit.xcodeproj
```

Run the `LinkerEdit` scheme on an iPad simulator. `LinkerEdit.xcodeproj` and `LinkerEdit/Info.plist` are generated and not committed; change `project.yml` instead. Launch arguments for trying it: `-demo` shows a fixed chat without a computer, `-host http://127.0.0.1:4317` connects at once (the simulator reaches the Mac's own `127.0.0.1`), `-reset` forgets recent computers, `-uiTestAppearance dark` (or `light`) overrides the system's appearance for screenshots.

## Linker Connect

The client side is built and unit tested against a mocked network. The host side is PR #28 (`feat/connect-native`); the two have never talked to each other. The contract:

1. **Sign in with Linker**: Clerk OAuth, a public client with PKCE (S256), in `ASWebAuthenticationSession`, back to `linkeredit://oauth/callback`, with the scopes `openid profile email offline_access`. The client id and issuer come from the Info.plist keys `LinkerOAuthClientID` and `LinkerOAuthIssuer` (in `project.yml`), empty for now, which keeps sign-in off. Tokens are kept in the Keychain.
2. **Computers**: `GET https://app.linker.cool/api/edit/computers` with `Authorization: Bearer <Linker access token>`, answering `{ limit, computers: [{ id, name, url, online, linkedAt, lastSeenAt }] }`.
3. **Where the Linker token goes**: only to a computer in that list whose address is exactly `https://edit-<16 hex digits>.linker.cool`. The app refuses any other address before sending anything.
4. **A session on the computer**: `POST <computer>/api/connect/session` with the Linker token and `{ "name": "<this iPad's name, at most 64 characters>" }` answers `{ code }` (the computer's settings list the iPad by that name); then `POST <computer>/api/connect/native` with `{ "code": "<code>" }` answers `{ "session": "<opaque>", "expiresAt": "<ISO 8601>" }`.
5. **Using it**: every request to the computer, the `/ws` WebSocket included, sends `Authorization: Bearer <session>` and no `Origin`; AVPlayer gets it through `AVURLAssetHTTPHeaderFieldsKey`. The session is kept in the Keychain per computer. When the computer refuses the WebSocket (401), the app drops that session and signs in to the computer again, once.

## Not done yet

- **Signing, TestFlight and the App Store**: no Apple Developer account yet, so the app can't be installed on an iPad. That's thread 7 in the plan.
- **Push notifications** for approvals and finished turns (thread 7).
- **"Send to iPad"** project files for Resolve for iPad (thread 5).
- **Linker's registration of the iPad app at Clerk**, so sign-in can be switched on. A 401 on a preview file isn't noticed yet; only the WebSocket's is.
- **Never run on a real iPad**, over Tailscale or Linker Connect, or against a host with Resolve and a real agent.
- The app icon is the desktop app's; it has an alpha channel, which App Store builds will need removed.
