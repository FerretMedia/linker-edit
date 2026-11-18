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
| `src/lib.rs` | `run(link)`: single instance, assets, the HTTP client, the window, quit on close |
| `src/workspace.rs` | The root view and its routing: sidebar, chat column with its header, one side panel; or the settings or library page in their place; a dialog over it; the window's keys |
| `src/state.rs` | `AppState`, the `Link` to the server and the `ServerAddress` it answers HTTP at |
| `src/transport.rs` | The `/ws` client: a `Link` to a server in another process |
| `src/http.rs` | The one HTTP client (reqwest): pictures for GPUI's `img`, and blocking requests off the UI thread |
| `src/media/` | Video and sound: `timeline` (port of `player.ts`), `ffmpeg` (finding and running it), `video` (frames), `audio` (cpal, the mixer and clock), `player` (the engine both preview views use), `sound` (one file on its own: the library's ▶) |
| `src/labels.rs`, `context.rs`, `markdown.rs`, `sidebar.rs`, `browse_path.rs`, `project_icon_colors.rs`, `collection_icons.rs`, `settings.rs`, `library.rs` | Ports of src/lib's `labels.ts`, `context.ts`, `markdown.ts`, `sidebar.ts`, `browsePath.ts`, `projectIconColors.ts`, `collectionIcons.ts` (with CollectionIcon.svelte), `settings.ts` (with `resolve_status_line`), `library.ts` |
| `src/theme.rs` | `app.css`'s tokens, light and dark; text sizes and radii |
| `src/icons.rs` | The embedded Lucide icons and logos, and the asset source |
| `src/components.rs` | Small shared elements (logo, header button) |
| `src/window.rs` | Window options, title-bar headers, the window buttons |
| `src/single_instance.rs` | A second launch brings the first window forward |

`src/views/`, one file per Svelte component (or a few that belong together):

| Region | Files |
|---|---|
| Sidebar | `sidebar` (with `sidebar_rows`, `sidebar_footer`: transcription, `app_update_card`, the Linker account, Settings, Resolve's status), `computer_picker`, `context_menu` |
| Chat | `chat`, `composer` (with the provider picker and context meter), `tool_row`, `approval_card`, `thread_starter` |
| Side panels | `panel` (which panels, their frame and header), `preview_panel` (with `timeline_player`, `player_controls`), `history_panel` (with `clip_strip`), `project_file_panel` |
| Pages | `settings_page` (with `remote_access`, `linker_connect`, `collections_settings`: the collection editor and icon picker), `library_page`; `settings_parts`: SettingsGroup/Row, Switch, Segmented, the pages' frame and buttons, `confirm()` |
| Dialogs | `dialog` (the frame, its buttons and text field), `add_project`, `folder_picker`, `project_settings`, `project_icon` (with the icon picker), `library_folder_dialog`, `choice_select` |

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
  `ServerEnd` and sends `LinkEvent::Connected`, then `Message(ServerMessage::Snapshot)`.
- `transport::connect(url, Auth)` is the other way to get a `Link`: a WebSocket to a server's
  `/ws` on its own thread (tokio), reconnecting every second like the browser. `Auth::Local` for
  a server on this computer (no token: it lets in a loopback Host with no Origin);
  `Auth::Bearer(session)` for a Linker Connect session, ended with `LinkEvent::SignedOut` on a 401.
- `Link::server` is where that server answers HTTP: a `ServerAddress` (`base`, such as
  `http://127.0.0.1:4317`, and its `Auth`). `transport::connect` fills it in; a server in this
  process gives it with `link.with_server(ServerAddress::new(url, Auth::Local)?)`. Pictures,
  media, the preview's timeline, sign-in links and the computer menu go through it; without
  one (tests) nothing is fetched. `AppState::home` is the server the window opened on,
  `AppState::server` the one it is on now; the computer menu moves between them with
  `AppState::relink(link)`, and `away()` says it's on another computer. `on_this_computer()`
  (a loopback server) decides where the system's folder dialog makes sense. `http.rs` sends the
  session only to the current server's own address.
- `AppState` holds `app: Option<protocol::AppState>` (the snapshot with its patches applied),
  `connected`, the history, project file, library and library view panels ask for, and toasts.
  It derives what +page.svelte does: `project()`, `thread()`, `collection()`, `chat_title()`,
  `rendering()`, `can_send(draft)`, `start_blocked()`, `resolve_busy()`, `history_entries()`,
  `project_file_in_view()`. `browse_folders(path)` answers the folder picker. A message the
  server refused comes as `AppEvent::Refused` (subscribe to it).
- State that is only the window's business (which panel is open, a draft, a scroll position, a
  search query) lives in the view that shows it, like the Svelte components' `$state`.
- The workspace routes: `open_settings(section)` (the sidebar's gear, Ctrl+, and the computer
  menu's "Remote access…" all come here through `SidebarEvent::Settings`), `open_library(tab)`,
  `close_page`, `toggle_panel`, `open_preview(target)` (History's Play), `add_project`. Each page,
  side panel and dialog is a view made when it opens and dropped, with its subscription, when it
  closes, as the Svelte components unmount; the preview stops its video first.
  When a page opens or a dialog closes, the keyboard goes back to the workspace (or the page):
  focus left on something no longer drawn reaches nothing, and Escape and Ctrl+, stop working.

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
- Spacing: gpui's `px_2`, `gap_1p5`, `h_8`… are Tailwind's (0.25rem steps), so copy them. The rem is
  16px as in the browser (`theme.rs` sets gpui-component's font size, which its Root makes the rem).
- Light or dark follows the OS; `LINKER_EDIT_THEME=light|dark` forces one. gpui-component's
  own theme is pointed at our tokens, so its controls match.

## Icons

`Icon::new(IconName::History).size(px(14.)).text_color(theme.muted_foreground)` (gpui-component's
`Icon`; it takes our `IconName` and `Logo`). Only the icons the Svelte UI imports are
embedded as `IconName`s; to add one, see the top of `icons.rs`. Project icons can be any Lucide
icon, as in T3 Code: those come from gpui-kit's full Lucide catalog (`views/project_icon.rs`).
Icons are one-color masks; multi-color marks are stacked layers (`components::LinkerLogo`).

## The title bar

There is no system title bar, as in `desktop/main.ts`. Each top header is built with
`window::title_bar(id, padding_x, leftmost, rightmost, window)`, 52px high; it drags the
window (on Windows through the native caption, with Snap and double-click to maximize).
Wrap every control in a header in `window::no_drag()`, or pressing it drags the window.
The rightmost header leaves room for the minimize/maximize/close buttons, which the
workspace draws over the top right corner; the leftmost one clears macOS's traffic lights.

## Video

GPUI has no `<video>`, so `src/media` plays it: one ffmpeg process per clip on screen (or
coming up within 3 s) decodes frames at the stage's size, with the clip's look applied, as raw
BGRA on a pipe; each becomes a GPUI image. Another ffmpeg per audio clip decodes PCM for the
mixer, which the sound card (cpal) pulls; what it has played is the clock the pictures follow.
Media comes from the server over HTTP (`/media/<id>`, `/preview/<file>`, `/library/…`), with
the `ServerAddress`'s session, so it works for another computer's server too. The library
page's ▶ plays one file through `media::sound` on the same mixer and sound card. ffmpeg is found next to the binary, on PATH, or in a checkout's
`node_modules/ffmpeg-static`; packaging puts it next to the binary. Linux builds need ALSA's
headers (`libasound2-dev`) for cpal.

## Testing

`#[gpui::test]` gives a `TestAppContext`; `gpui_kit::test::TestWindowExt` finds elements by id
and clicks them. `workspace::tests` has the helpers every flow test uses: `open_workspace` (the
window as `run` opens it, on a link the test drives), `Server` (the link's other end: `send`,
`message`, `received`), `render`, and `steps(&[Do::Click(…), Do::Press(…), Do::Input(…)])`, one
frame per step. Flows live in `workspace.rs` (the chat), `views/sidebar_tests.rs` (the sidebar
and its dialogs) and `pages_tests.rs` (pages, panels, the collection editor).

```
cargo test -p linker-edit-ui
cargo clippy -p linker-edit-ui --all-targets -- -D warnings
```

To look at it: `cargo run -p linker-edit` with `LINKER_EDIT_CONFIG` pointing at a config of your
own (it also names the single-instance lock, so your copy doesn't hand over to someone
else's), and `LINKER_EDIT_THEME` to pick light or dark. A visual change gets before/after
screenshots.

Until crates/server runs in the binary, `LINKER_EDIT_SERVER_URL=http://127.0.0.1:<port>` (dev only)
opens the window on a server already running, such as the TypeScript one started with its own
`LINKER_EDIT_CONFIG` (a `port`, temp folders, `"resolveMcp": ""` to keep Resolve out, and a
`settings.json` beside it with `"resolveHere": false` so it doesn't download the speech engine)
and `LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl` to replay a recorded turn
instead of calling Claude. `LINKER_EDIT_SERVER_TOKEN` sends a Linker Connect session as Bearer.
