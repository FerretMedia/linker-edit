# Releasing

Each release ships Linker-Edit two ways:

- **Desktop installers** on the GitHub release: Windows (NSIS `.exe`), macOS (`.dmg`, plus the `.zip` updates use) for Apple silicon and Intel, and Linux (`.AppImage`). Installed apps update themselves from the newest GitHub release.
- **The npm package** `linker-edit`, which users run with `npx linker-edit@latest`.

## Make a release

GitHub → Actions → **Release** → Run workflow, on `main`.

The workflow runs CI, then:

1. Picks the version from today's date (UTC): `YYYY.MDD.N`. `2026.1108.0` is the first release on 8 November 2026 and `2026.1108.1` the second. npm and the app updater need three numbers without leading zeros, which this keeps in order.
2. Tags the commit `v<version>`. A run that fails after this skips that version; the next run takes the next number.
3. Builds the installers, each on its own platform: the agents' programs ship inside the app, and npm installs only the current platform's.
4. Once every installer has built, creates the GitHub release with them and notes generated from the merged PRs. The `latest*.yml` files are what installed apps check for updates; the two Mac builds' lists are merged into one `latest-mac.yml`.
5. Builds and publishes the npm package with provenance. While the repository is private this step is skipped: npm refuses provenance from a private repository.

The repository's `package.json` stays at `0.0.0`; the version is only stamped into what's built. Running apps check once a few hours (desktop) or once a day (npm) and show the update in the sidebar.

Before a release that changes how Linker-Edit drives Resolve, run `npm run test:live` against a scratch project.

## Building an installer locally

```sh
npm run dist:desktop
```

builds the installer for the computer you're on into `release/`. `node scripts/build-desktop.ts --dir` after `npm run build` only lays out the unpacked app in `release/<platform>-unpacked`, which is quicker to try.

The build stages the app in `.desktop-stage` with its own `package.json` and only the packages it runs with, then runs electron-builder there. The app ships without an asar archive, because the agents' programs (`claude`, `codex`), ffmpeg and the Python scripts have to be real files to run. They are also why the installer is about 400 MB.

In the installed app, Electron starts the server as Node (`ELECTRON_RUN_AS_NODE`) with `linker-edit --serve`, and its log goes to the app's log folder (`%APPDATA%\Linker-Edit\logs\server.log` on Windows, `~/Library/Logs/Linker-Edit` on macOS, `~/.config/Linker-Edit/logs` on Linux). Helper processes the agents start (the `linker` MCP server, the Resolve guard) run the same way.

## Signing

Releases build unsigned for now: Windows shows SmartScreen on first launch, and Mac builds are signed ad hoc, so macOS asks once and can't install updates by itself (the sidebar links to the download instead). To sign, add these repository secrets and the next release uses them:

| Secret | For |
| --- | --- |
| `CSC_LINK`, `CSC_KEY_PASSWORD` | The code signing certificate (`.p12`, base64) and its password. A Developer ID Application certificate on macOS, or an Authenticode one on Windows. |
| `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID` | Notarizing the Mac app, so macOS opens it without asking. |

With a Developer ID, Mac apps can update themselves too: set `autoDownload` to true on macOS in `desktop/main.ts`.

## One-time npm setup

The first publish needs a token, because npm only lets you set up trusted publishing on a package that exists.

1. On npmjs.com, create a granular access token that can publish packages, and add it as the `NPM_TOKEN` secret of this repository.
2. Run the Release workflow.
3. On npmjs.com → `linker-edit` → Settings → Trusted publishing, add GitHub Actions with owner `FerretMedia`, repository `linker-edit` and workflow `release.yml`.
4. Delete the `NPM_TOKEN` secret and the token. Later releases publish through GitHub's OIDC token, with no stored secret.

## Updating davinci-resolve-mcp

The version Linker-Edit installs is `RESOLVE_MCP_VERSION` in `server/setup.ts`. To move it, update a local checkout to the new tag, run `npm test` and review the changed list of actions that run without approval, then run `npm run test:live`. Setup only installs davinci-resolve-mcp when it's missing, so existing installs keep their copy until `~/LinkerEdit/davinci-resolve-mcp` is deleted.

## Merging

The CI job `Check` passes only when every other CI job passed: typecheck, unit tests on Linux and Windows, e2e, installing and starting the packed npm package, and building and starting the Linux desktop app. It's the one check to require for `main`, so adding or splitting jobs never needs a change to the merge rules.
