# Rust port: conventions for the agents doing it

Temporary: this file goes when the port merges. The plan is `docs/rust-gpui-plan.md` on
branch `plan/rust-gpui` (PR #31).

## Decisions (the user's, 2026-11-18)

- The Rust server is a web backend: axum, the same `/ws` and `/api`, behind the same
  Cloudflare Tunnel and Tailscale Serve. It serves the Svelte UI (apps/desktop/src) to browsers
  and remote computers. The GPUI window (crates/ui) is the desktop window.
- npm stays: a launcher package plus one package per platform with the binary.
- `claude` and `codex` are bundled with the app; from source, PATH.
- One PR. The TypeScript server and Electron are deleted at the end, not by the module ports.

## Layout

| Crate | What |
|---|---|
| `crates/protocol` | Everything on `/ws` and `/api`. `apps/desktop/shared/protocol.ts` is generated from it (`UPDATE_PROTOCOL_TS=1 cargo test -p linker-edit-protocol --test typescript`). Never redefine these types. |
| `crates/server` | The port of `apps/desktop/server`. Modules mirror its files: `server/resolve/run.ts` → `crates/server/src/resolve/run.rs`. |
| `crates/ui` | The GPUI window; see `crates/ui/README.md`. |
| `crates/app` | The `linker-edit` binary: window + server, `serve`, `mcp`. |
| `crates/drp` | .drp/.drt files. |

## Server code

- Port 1:1: same behaviour, names in snake_case, the same messages to users and agents.
  Port the module's Vitest tests next to it; they are the proof.
- In `lib.rs` (or the parent `mod.rs`) add only your own `pub mod` lines, in alphabetical
  order, so merges stay trivial.
- Async is tokio. A trait used as a trait object uses the `async-trait` crate. Where the TS
  takes an interface to inject (a store, a connection, a session), make it a trait with the
  same methods; where it takes a factory function, a constructor taking the dependencies.
- Callbacks (`onProgress`) are `Arc<dyn Fn(..) + Send + Sync>`; an `AbortSignal` is a
  `tokio_util::sync::CancellationToken`.
- Child processes: `tokio::process::Command`, with `CREATE_NO_WINDOW` on Windows (the TS's
  `windowsHide: true`), killed on drop where the TS kills them on close.
- Errors: `anyhow` for plumbing; `thiserror` where the TS has error kinds callers branch on.
- Config is `linker_edit_server::config::AppConfig`; the version is `updates::VERSION`.
  Files the TS read next to the app (Resolve scripts, the built UI) come from `assets.rs`.
- Add dependencies in your crate's `Cargo.toml`; `workspace = true` for those in the root.
  Don't run `cargo rm` (it rewrote the root `Cargo.toml` once).
- `crates/protocol` is shared. If a type is missing or wrong, make the smallest fix,
  regenerate `protocol.ts`, check `npm run typecheck`, and say so in your report.

## Checks before you report

```sh
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=4
cargo fmt --all --check
cargo clippy -p <your crate> --all-targets -- -D warnings
cargo test -p <your crate>
```

Several agents build at once on a 16-core, 16 GB machine: keep `CARGO_BUILD_JOBS=4` and build
only your crate (`-p`), not the whole workspace.
