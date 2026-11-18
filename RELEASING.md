# Releasing

Each release ships Linker-Edit two ways:

- **Desktop installers** on the GitHub release, made by [Velopack](https://velopack.io): Windows (`Linker-Edit-win-x64-Setup.exe`), macOS for Apple silicon and Intel (`Linker-Edit-osx-<arch>-Setup.pkg`), Linux (`Linker-Edit-linux-x64.AppImage`), and portable zips for Windows and macOS. Installed apps update themselves from the newest GitHub release.
- **The npm packages**: `linker-edit`, which users run with `npx linker-edit@latest`, and `@linker-edit/win32-x64`, `@linker-edit/darwin-arm64`, `@linker-edit/darwin-x64` and `@linker-edit/linux-x64`, one per platform, which `linker-edit` depends on optionally so npm installs only the user's.

Both carry the same thing: the `linker-edit` program (`crates/app`, with the Svelte UI built into it) and, next to it, the programs it runs: `claude`, `codex/` and `ffmpeg`.

## Make a release

GitHub → Actions → **Release** → Run workflow, on `main`.

The workflow runs CI, then:

1. Names the release after today's date (UTC), like Linker's releases: `2026.11.08` is the release of 8 November 2026. That name is on the tag (`v2026.11.08`), the GitHub release and in the app. A day has one release: running the workflow again that day replaces it. npm and the updater need three numbers without leading zeros that sort in order, so underneath, the release's version is `YYYY.MDD.HHMM`, the UTC time it was made: `2026.1108.1430` (`apps/desktop/scripts/release-version.ts`). A release that replaces the day's earlier one has a later time, so installed apps still update to it. `crates/protocol/src/release_name.rs` turns a version into its name.
2. Writes the release notes, GitHub's list of the pull requests merged since the last release. The installed apps show them as what's new.
3. Builds each platform on its own runner, from the commit the workflow ran on:
   - stamps the version into the Cargo workspace (`apps/desktop/scripts/stamp-version.ts`), where it is the app's version (`CARGO_PKG_VERSION`); the checkout stays at `0.0.0`;
   - builds the Svelte UI (`npm run build`) and the program (`cargo build --release -p linker-edit`), which carries the UI inside it;
   - lays out the app (`node scripts/package.ts app`): the program, and next to it Claude Code's native binary, Codex's platform build and ffmpeg, which `apps/desktop/scripts/bundle.ts` takes from npm at the versions in `crates/server/src/providers/cli.rs` and `package-lock.json`;
   - packs it with `vpk` (`node scripts/package.ts installer`), on the channel of its platform (`win-x64`, `osx-arm64`, `osx-x64`, `linux-x64`), with the notes;
   - packs `@linker-edit/<platform>` (`node scripts/package.ts npm`).

   Nothing is tagged yet, so a run that fails leaves the releases as they were.
4. Once every platform has built, deletes the day's earlier release and its tag if there is one, then tags the commit and creates the GitHub release with every platform's Velopack files. `releases.<channel>.json` and the `.nupkg` packages are what installed apps update from.
5. Publishes the npm packages with provenance: the four platform packages first, then `linker-edit` (`node scripts/package.ts launcher`). While the repository is private this step is skipped: npm refuses provenance from a private repository.

`vpk` is a .NET tool, pinned to the version of the `velopack` crate in `crates/app/Cargo.toml`: upgrade both together.

## How the desktop app updates

- `crates/app/src/updater.rs` drives Velopack's update manager against the GitHub releases. `crates/app/src/update_state.rs` moves its state from status to status, like T3 Code's update machine: checking, available, downloading, downloaded, up to date, error. It looks 15 seconds after the app starts, then every hour.
- The window runs the server in the same process and hands it the updater (`linker_edit_server::app_update`); the server shows the state to every page. A page on another of your computers shows that computer's update and can restart it.
- *Restart to update* waits until nothing is busy with Resolve: no turn, render or open. Then the app starts Velopack's updater, which waits for the app to quit, replaces it (on Windows it first stops whatever still runs from the app's folder: the agents, ffmpeg, the helpers running the app's own binary), and opens it again. Closing the app with an update downloaded installs it too, without opening it again.
- Before installing, it writes `update-install.json` in the data folder (`~/LinkerEdit`). If the app comes back on the old version, the sidebar says the update didn't install and offers to try again.
- Only a copy Velopack installed updates itself. From npm or a build from source there is no updater, and the server looks on npm for a newer version once a day instead.
- Its log is `~/LinkerEdit/logs/linker-edit.log`, with the server's.

Before a release that changes any of this, run the update smoke test (see the README's Testing section). CI runs it on Linux.

Before a release that changes how Linker-Edit drives Resolve (`crates/server/src/resolve/`), run `npm run test:live` against a scratch project.

## From the Electron app

Releases up to 2026.11.14 were Electron apps updated by electron-updater, which reads `latest.yml`, `latest-mac.yml` and `latest-linux.yml` from the newest release. Velopack can't take over an Electron install, so the publish job writes those files once more (`node scripts/package.ts bridge`), pointing at the new release:

- **macOS**: the old app only ever offered a download link, so it shows the new version and links to the release. The user installs the new app over the old one; the bundle id (`com.ferretmedia.linkeredit`) is the same.
- **Linux**: the old AppImage downloads the new one and swaps itself for it, as any update. The new AppImage is a Velopack install and updates itself from then on.
- **Windows**: no `latest.yml`. Running Velopack's installer from the old app would leave the Electron install next to the new one, and uninstalling that would take the new app's shortcuts. The old app says it couldn't check for updates; the README asks Windows users to uninstall the old app and run the new installer once.

Everyone keeps their projects, threads and settings: they are in `~/LinkerEdit`, not in the app. Drop the bridge step once nobody runs an Electron build.

## Building a release locally

With [.NET 9](https://dotnet.microsoft.com) and `dotnet tool install -g vpk --version 1.2.161`:

```sh
node apps/desktop/scripts/stamp-version.ts 2026.1118.1500   # vpk refuses 0.0.0; don't commit the stamp
npm run build
cargo build --release -p linker-edit
cd apps/desktop
node scripts/package.ts app         # release/app-<platform>
node scripts/package.ts installer   # release/velopack
node scripts/package.ts npm         # release/npm/linker-edit-<platform>-<version>.tgz
node scripts/package.ts launcher    # release/npm/linker-edit-<version>.tgz
```

`release/app-<platform>` runs as it is (`linker-edit` from there), but doesn't update itself. The portable zip in `release/velopack` does: unpack it and run `current/linker-edit`, with `LINKER_EDIT_UPDATE_FEED` pointing at a folder `vpk pack` wrote a newer version into to try an update. The bundle is large: Claude Code and Codex are about 330 and 480 MB on Windows.

## Signing

Releases build unsigned for now: Windows shows SmartScreen on first launch, and macOS asks once under Privacy & Security. To sign, add these repository secrets and the next release uses them:

| Secret | For |
| --- | --- |
| `CSC_LINK`, `CSC_KEY_PASSWORD` | The code signing certificate (`.p12`/`.pfx`, base64) and its password. On Windows an Authenticode certificate, which `vpk` signs with through `signtool` (`--signParams`). On macOS a Developer ID Application certificate, imported into a keychain of the build's own (`--signAppIdentity`); if the same `.p12` has a Developer ID Installer certificate, the `.pkg` is signed with it (`--signInstallIdentity`). |
| `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID` | Notarizing the Mac app: stored as a `notarytool` profile for `vpk` (`--notaryProfile`), so macOS opens it without asking. |

Signing has never run yet: check the first signed release's build logs.

## One-time npm setup

The first publish needs a token, because npm only lets you set up trusted publishing on a package that exists.

1. On npmjs.com, create the `linker-edit` organization (for the `@linker-edit/*` packages) and a granular access token that can publish `linker-edit` and the organization's packages, and add it as the `NPM_TOKEN` secret of this repository.
2. Run the Release workflow.
3. On npmjs.com, for `linker-edit` and each `@linker-edit/<platform>` package → Settings → Trusted publishing, add GitHub Actions with owner `FerretMedia`, repository `linker-edit` and workflow `release.yml`.
4. Delete the `NPM_TOKEN` secret and the token. Later releases publish through GitHub's OIDC token, with no stored secret.

## After a Resolve update

Linker-Edit drives Resolve through the MCP server that comes with Resolve Studio, so a Resolve update can change how it behaves. After one:
- run `npm run test:live -- --project "<scratch project>"`;
- check the Resolve status in the app.

A newer Resolve may add tools to its MCP server. Agents don't get a new tool until it's added to `crates/server/src/policy.rs`.

## Merging

The CI job `Check` passes only when every other CI job passed: Rust on Linux, Windows and macOS, the Svelte UI, e2e, installing and starting the npm packages, and packing, starting and updating the Linux desktop app. It's the one check to require for `main`, so adding or splitting jobs never needs a change to the merge rules.
