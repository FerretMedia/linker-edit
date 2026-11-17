# linker-edit-ui: the GPUI window

The desktop window of Linker-Edit, drawn with [GPUI](https://www.gpui.rs/). It is one client
of the server, like the browser UI in `apps/desktop/src` (which stays for browsers and remote
access). It ports that Svelte UI view by view; when in doubt, match the Svelte markup and
`app.css`.

It is its own crate, not a module of `crates/app`, so a change to the server doesn't recompile
the UI (and the reverse), and `cargo test -p linker-edit-ui` runs the views' tests without
building the server. `crates/app` calls [`run`](src/lib.rs).

## Layout

| File | What |
|---|---|
| `src/lib.rs` | `run(link)`: single instance, assets, the window, quit on close |
| `src/workspace.rs` | The root view: sidebar, chat column with its header, one side panel |
| `src/views/` | One file per region of `+page.svelte`: `sidebar`, `chat`, `panel` |
| `src/state.rs` | `AppState` and the `Link` to the server |
| `src/theme.rs` | `app.css`'s tokens, light and dark; text sizes and radii |
| `src/icons.rs` | The embedded Lucide icons and logos, and the asset source |
| `src/components.rs` | Small shared elements (logo, header button, placeholder) |
| `src/window.rs` | Window options, title-bar headers, the window buttons |
| `src/single_instance.rs` | A second launch brings the first window forward |

## GPUI

Through `gpui-kit` =0.10.0 (see the comment in `Cargo.toml`), which pins GPUI (`gpui-pre`
=0.3.12, a snapshot of zed@d2a4fb6) and brings gpui-component: inputs, virtual lists,
scrolling, dialogs, menus, tooltips, markdown, resizable panes. `lib.rs` names the kit `gpui`
(`extern crate gpui_kit as gpui`), so write `use gpui::{div, px, …}` for GPUI and
`use gpui_kit::component::…` for components. Docs: <https://gpui-kit.com>, docs.rs for
`gpui-component` 0.10.0 and `gpui-pre` 0.3.12. Upgrade on purpose, in its own PR.

## How state flows

```
server ──LinkEvent──▶ AppState (Entity) ──notify──▶ views re-render
   ▲                                                    │
   └──────────────ClientMessage── AppState::send ◀──────┘
```

- The server owns the state. `AppState` holds only what the server sent; a view never changes
  it to "predict" an answer. A rule enforced only in the window isn't enforced.
- `Link::pair()` makes the two ends. `run` takes the window's end; the server (or a test) holds
  `ServerEnd`. Over `/ws` to another computer, a socket task feeds the same channels.
- `ServerMessage` and `ClientMessage` in `state.rs` are stand-ins until `crates/protocol` has the
  `/ws` types: replace them with a re-export and fill in `AppState::apply`.
- State that is only the window's business (which panel is open, a draft, a scroll position, a
  search query) lives in the view that shows it, like the Svelte components' `$state`.

## Adding a view

1. Add `src/views/<name>.rs`. With state of its own, it is an entity (`impl Render`), made with
   `cx.new(|cx| View::new(state.clone(), cx))` and observing `AppState`
   (`cx.observe(&state, |_, _, cx| cx.notify()).detach()`). Without, it is a `RenderOnce`
   struct built fresh each render, taking callbacks (`components::OnClick`) for its events.
2. Export it from `views/mod.rs` and put it where its Svelte counterpart sits.
3. Start the doc comment with the Svelte file it ports.
4. Give the elements a test finds an `.id(…)` and end their chain with `.test_support()`
   (inert outside tests).

## Theme

`Theme::of(cx)` returns the current tokens; field names follow `app.css` (`muted_foreground`,
`sidebar_row_hover`, …). Never hard-code a color. Tailwind's `/70` is `.opacity(0.7)`.

- Text: `.text_token(TextSize::Sm)` sets the size and Tailwind's line height (gpui's
  `.text_sm()` sets only the size). The root is `text-sm` in the system font.
- Radii: `.rounded(theme.radius.md)`; gpui's `rounded_sm()` is 2px, app.css's is 4px.
  `rounded_full()` is fine.
- Spacing: gpui's `px_2`, `gap_1p5`, `h_8`… are Tailwind's (0.25rem steps), so copy them.
- Light or dark follows the OS; `LINKER_EDIT_THEME=light|dark` forces one. gpui-component's
  own theme is pointed at our tokens, so its controls match.

## Icons

`Icon::new(IconName::History).size(px(14.)).text_color(theme.muted_foreground)` (gpui-component's
`Icon`; it takes our `IconName` and `Logo`). Only the icons the Svelte UI imports are
embedded; to add one, see the top of `icons.rs`. Icons are one-color masks; multi-color marks
are stacked layers (`components::LinkerLogo`).

## The title bar

There is no system title bar, as in `desktop/main.ts`. Each top header is built with
`window::title_bar(id, padding_x, leftmost, rightmost, window)`, 52px high; it drags the
window (on Windows through the native caption, with Snap and double-click to maximize).
Wrap every control in a header in `window::no_drag()`, or pressing it drags the window.
The rightmost header leaves room for the minimize/maximize/close buttons, which the
workspace draws over the top right corner; the leftmost one clears macOS's traffic lights.

## Testing

`#[gpui::test]` gives a `TestAppContext`; `gpui_kit::test::TestWindowExt` finds elements by id
and clicks them. See `workspace.rs`'s tests: open the window as `run` does with a `Link` the
test drives, send `LinkEvent`s, `cx.run_until_parked()`, `window.render_frame(cx)`, then
`window.find("id")` / `window.click("id", cx)`.

```
cargo test -p linker-edit-ui
cargo clippy -p linker-edit-ui --all-targets -- -D warnings
```

To look at it: `cargo run -p linker-edit` with `LINKER_EDIT_CONFIG` pointing at a config of your
own (it also names the single-instance lock, so your copy doesn't hand over to someone
else's), and `LINKER_EDIT_THEME` to pick light or dark. A visual change gets before/after
screenshots.
