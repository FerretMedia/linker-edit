# Your own Linker-Edit web address

Put Linker-Edit on your own Cloudflare address, open it in any browser, sign in with Linker and work on your edit computer from anywhere. The edit computer still does all the work and keeps everything; this address only serves the app's files.

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/FerretMedia/linker-edit/tree/main/apps/web)

1. Click **Deploy to Cloudflare**. Cloudflare asks you to sign in to Cloudflare and GitHub, copies this folder into a new small repository in your GitHub, and deploys it. There's nothing to fill in.
2. You get an address like `https://linker-edit-web.<your-name>.workers.dev`. You can add your own domain to it in Cloudflare.
3. Open it and click **Sign in with Linker**. The first time, Linker asks whether this address may reach your edit computers: **Allow**. Only allow an address you deployed yourself.
4. You're in the app, with the projects of all your edit computers in the sidebar. Each one needs Linker-Edit running, signed in to the same Linker account, with Linker Connect on (Settings → Remote access). A computer that can't be reached shows its projects greyed, as they were when this browser last saw them.

*Prefer a fork?* Fork [FerretMedia/linker-edit](https://github.com/FerretMedia/linker-edit), then in Cloudflare → Workers → **Create** → **Import a repository**, pick your fork and set the root directory to `apps/web`.

## Updating

Redeploy in Cloudflare (your Worker → Deployments). Every deploy fetches the newest Linker-Edit. The page also tells you when your edit computer runs a newer Linker-Edit than the page.

## Tailscale or the tunnel

The page tries your computer's Tailscale address first, then its Linker Connect tunnel. Over Tailscale, your browser may ask once to allow the page to reach devices on your local network: allow it, or the page uses the tunnel. In Linker-Edit's settings you can turn the tunnel off and use Tailscale only.

## What's in this folder

- `get-ui.mjs` downloads `Linker-Edit-<version>-web.tar.gz` from the newest [release](https://github.com/FerretMedia/linker-edit/releases), checks it against its `.sha256`, and unpacks it into `public/`. Cloudflare runs it on every deploy. It needs only Node.js 20 or later; nothing is installed.
- `wrangler.jsonc` serves `public/` as static files. No Worker code runs, so it's free and doesn't count against request limits.

Build variables, all optional (Cloudflare → your Worker → Settings → Build → Variables):

| Variable | What it does |
| --- | --- |
| `LINKER_EDIT_VERSION` | A release to use, such as `2026.11.19`, instead of the newest. |
| `LINKER_EDIT_REPO` | Where the releases come from, instead of `FerretMedia/linker-edit`. |
| `LINKER_URL` | Linker's address, instead of `https://app.linker.cool`. |
| `GITHUB_TOKEN` | Only if a deploy fails because GitHub turned away too many requests. |
