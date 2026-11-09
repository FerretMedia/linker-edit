# Releasing

Linker-Edit ships to npm as `linker-edit`, and users run it with `npx linker-edit@latest`.

## Make a release

GitHub → Actions → **Release** → Run workflow, on `main`.

The workflow runs CI, then:

1. Picks the version from today's date (UTC): `YYYY.MDD.N`. `2026.1108.0` is the first release on 8 November 2026 and `2026.1108.1` the second. npm needs three numbers without leading zeros, which this keeps in order.
2. Tags the commit `v<version>`. A run that fails after this skips that version; the next run takes the next number.
3. Builds and publishes the package to npm with provenance.
4. Creates the GitHub release with notes generated from the merged PRs.

The repository's `package.json` stays at `0.0.0`; the version is only stamped into the published package. Running apps check npm once a day and show the update in the sidebar.

Before a release that changes how Linker-Edit drives Resolve, run `npm run test:live` against a scratch project.

## One-time npm setup

The first publish needs a token, because npm only lets you set up trusted publishing on a package that exists.

1. On npmjs.com, create a granular access token that can publish packages, and add it as the `NPM_TOKEN` secret of this repository.
2. Run the Release workflow.
3. On npmjs.com → `linker-edit` → Settings → Trusted publishing, add GitHub Actions with owner `FerretMedia`, repository `linker-edit` and workflow `release.yml`.
4. Delete the `NPM_TOKEN` secret and the token. Later releases publish through GitHub's OIDC token, with no stored secret.

## Updating davinci-resolve-mcp

The version Linker-Edit installs is `RESOLVE_MCP_VERSION` in `server/setup.ts`. To move it, update a local checkout to the new tag, run `npm test` and review the changed list of actions that run without approval, then run `npm run test:live`. Setup only installs davinci-resolve-mcp when it's missing, so existing installs keep their copy until `~/LinkerEdit/davinci-resolve-mcp` is deleted.
