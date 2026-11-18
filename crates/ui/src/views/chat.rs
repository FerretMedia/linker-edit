//! The chat column's body (+page.svelte's `<main>` under its header): the messages, then the
//! approvals, why a message can't go, and the composer, all 46rem wide at most and centered.
//! The header is drawn by the workspace, which owns the side panel its toggles open.
//!
//! The messages are a virtual list (GPUI's `list`): only the rows in view are laid out and
//! drawn, so a thread of thousands of items scrolls like a short one. A row is a message, a
//! notice, a work group of consecutive tool calls, or "Working for…". The list follows the
//! newest output unless the user scrolled up to read, like the Svelte `stickToBottom`.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FollowMode, FontWeight,
    InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, list,
    prelude::FluentBuilder, px, relative,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::scroll::Scrollbar;
use linker_edit_protocol::{ChatItem, ClientMessage, Tone};

use crate::icons::IconName;
use crate::markdown::assistant_markdown;
use crate::state::{AppState, item_id};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::approval_card::ApprovalCard;
use crate::views::composer::{Composer, ComposerEvent};
use crate::views::thread_starter::{Starter, ThreadStarter};
use crate::views::tool_row::ToolRow;

/// The column's `max-w-[46rem]`.
pub const CHAT_MAX_WIDTH: gpui::Pixels = px(736.);

const SUGGESTIONS: [&str; 3] = [
    "Describe the open project and its timelines",
    "What's in the media pool?",
    "Duplicate the current timeline and remove the silences",
];

// ToolSearch is the agent loading tool schemas: plumbing, not work the user cares about.
const HIDDEN_TOOLS: [&str; 1] = ["ToolSearch"];

/// One row of the message list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A message or notice, by its index in the thread's items.
    Item(usize),
    /// Consecutive tool calls, shown as one work group like T3 Code's work log.
    Tools(Vec<usize>),
    /// "Working for 12s", while a turn runs and nothing waits for the user.
    Working,
}

/// The rows for a thread's items (without "Working").
pub fn rows(items: &[ChatItem]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            ChatItem::Tool { name, .. } => {
                if HIDDEN_TOOLS.contains(&name.as_str()) {
                    continue;
                }
                match rows.last_mut() {
                    Some(Row::Tools(tools)) => tools.push(index),
                    _ => rows.push(Row::Tools(vec![index])),
                }
            }
            _ => rows.push(Row::Item(index)),
        }
    }
    rows
}

/// What a row shows, cheaply: a row whose key changed is measured again.
fn row_key(row: &Row, items: &[ChatItem]) -> (String, u64) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let id = match row {
        Row::Item(index) => {
            let item = &items[*index];
            if let ChatItem::User { text, .. }
            | ChatItem::Assistant { text, .. }
            | ChatItem::Notice { text, .. } = item
            {
                text.hash(&mut hasher);
            }
            item_id(item).to_string()
        }
        Row::Tools(indices) => {
            for index in indices {
                if let ChatItem::Tool {
                    id,
                    status,
                    output,
                    input,
                    ..
                } = &items[*index]
                {
                    id.hash(&mut hasher);
                    (*status as u8).hash(&mut hasher);
                    output.as_ref().map(String::len).hash(&mut hasher);
                    input.to_string().len().hash(&mut hasher);
                }
            }
            item_id(&items[indices[0]]).to_string()
        }
        Row::Working => "working".into(),
    };
    (id, hasher.finish())
}

/// What the chat asks of the rest of the window.
pub enum ChatEvent {
    /// "Add project" in an empty app: the add-project dialog opens.
    AddProject,
}

pub struct Chat {
    state: Entity<AppState>,
    composer: Entity<Composer>,
    starter: Option<(String, Entity<ThreadStarter>)>,
    list: ListState,
    rows: Vec<Row>,
    keys: Vec<(String, u64)>,
    thread: Option<String>,
    /// The tool rows the user opened, by tool id.
    expanded: HashSet<String>,
    /// While "Working for…" shows: since when, and the tick that redraws it each second.
    working: Option<(Instant, Task<()>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for Chat {}

impl Chat {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(state.clone(), window, cx));
        let list = ListState::new(0, ListAlignment::Top, px(800.));
        list.set_follow_mode(FollowMode::Tail);
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe(&composer, |this, _, event: &ComposerEvent, _| match event {
                ComposerEvent::Sent => this.follow(),
            }),
        ];
        let mut chat = Chat {
            state,
            composer,
            starter: None,
            list,
            rows: Vec::new(),
            keys: Vec::new(),
            thread: None,
            expanded: HashSet::new(),
            working: None,
            _subscriptions: subscriptions,
        };
        chat.sync(window, cx);
        chat
    }

    pub fn composer(&self) -> &Entity<Composer> {
        &self.composer
    }

    pub fn list_state(&self) -> &ListState {
        &self.list
    }

    /// Keeps the newest output in view again.
    fn follow(&self) {
        self.list.set_follow_mode(FollowMode::Tail);
    }

    /// Brings the rows in step with the thread: a new thread starts over at its end; otherwise
    /// only the rows from the first that changed are measured again.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let thread = state.thread_id().map(str::to_string);
        let (rows, keys, starter) = match &state.app {
            Some(app) => {
                let mut rows = rows(&app.items);
                if app.meta.busy && app.meta.approvals.is_empty() {
                    rows.push(Row::Working);
                }
                let keys: Vec<_> = rows.iter().map(|row| row_key(row, &app.items)).collect();
                let starter = if app.items.is_empty() {
                    let project = state.project();
                    let collection = state.collection();
                    match (project, collection) {
                        (Some(project), Some(collection)) if project.setup => {
                            Some(Starter::Setup {
                                collection: collection.name.clone(),
                            })
                        }
                        (_, Some(collection)) if collection.autonomous => {
                            Some(Starter::Autonomous {
                                collection: collection.name.clone(),
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                (rows, keys, starter)
            }
            None => (Vec::new(), Vec::new(), None),
        };

        if thread != self.thread {
            self.thread = thread.clone();
            self.list.reset(rows.len());
            self.follow();
        } else {
            let same = self
                .keys
                .iter()
                .zip(&keys)
                .take_while(|(old, new)| old == new)
                .count();
            if same < self.keys.len() || same < keys.len() {
                self.list.splice(same..self.keys.len(), keys.len() - same);
            }
        }

        let working = rows.last() == Some(&Row::Working);
        if working && self.working.is_none() {
            let tick = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            });
            self.working = Some((Instant::now(), tick));
        } else if !working {
            self.working = None;
        }

        // One starter per empty thread; a new thread gets a fresh one.
        let key = thread.unwrap_or_default();
        match starter {
            Some(starter) => {
                let current = self.starter.as_ref().is_some_and(|(thread, view)| {
                    *thread == key && *view.read(cx).starter() == starter
                });
                if !current {
                    let state = self.state.clone();
                    let view = cx.new(|cx| ThreadStarter::new(state, starter, window, cx));
                    self.starter = Some((key, view));
                }
            }
            None => self.starter = None,
        }

        self.rows = rows;
        self.keys = keys;
        cx.notify();
    }

    fn toggle_tool(&mut self, row: usize, tool: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&tool) {
            self.expanded.insert(tool);
        }
        self.list.remeasure_items(row..row + 1);
        cx.notify();
    }

    fn use_suggestion(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |composer, cx| {
            composer.set_draft(text, window, cx);
            composer.focus(window, cx);
        });
    }

    fn render_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let count = self.rows.len();
        let content = self.row_content(index, &theme, cx);
        // The column's px-5, py-6 and gap-4, given to each row.
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(CHAT_MAX_WIDTH)
                    .px_5()
                    .pt(if index == 0 { px(24.) } else { px(16.) })
                    .when(index + 1 == count, |this| this.pb(px(24.)))
                    .child(content),
            )
            .into_any_element()
    }

    fn row_content(&mut self, index: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let (Some(app), Some(row)) = (state.app.as_ref(), self.rows.get(index)) else {
            return div().into_any_element();
        };
        match row {
            Row::Tools(indices) => {
                let tools = indices
                    .iter()
                    .filter_map(|item| match app.items.get(*item) {
                        Some(ChatItem::Tool {
                            id,
                            name,
                            input,
                            status,
                            output,
                        }) => {
                            let tool = id.clone();
                            Some(ToolRow::new(
                                id,
                                name,
                                input,
                                *status,
                                output.as_deref(),
                                self.expanded.contains(id),
                                cx.listener(move |this, _, _, cx| {
                                    this.toggle_tool(index, tool.clone(), cx)
                                }),
                            ))
                        }
                        _ => None,
                    });
                div()
                    .id(("work-group", index))
                    .flex()
                    .flex_col()
                    .children(tools)
                    .test_support()
                    .into_any_element()
            }
            Row::Working => {
                let seconds = self
                    .working
                    .as_ref()
                    .map_or(0, |(since, _)| since.elapsed().as_secs());
                let elapsed = if seconds < 60 {
                    format!("{seconds}s")
                } else {
                    format!("{}m {}s", seconds / 60, seconds % 60)
                };
                div()
                    .id("working")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .text_token(TextSize::Sm)
                    .text_color(theme.muted_foreground)
                    .child(div().size_1p5().rounded_full().bg(theme.primary))
                    .child(format!("Working for {elapsed}"))
                    .test_support()
                    .into_any_element()
            }
            Row::Item(item) => match &app.items[*item] {
                ChatItem::User { id, text } => div()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .id(SharedString::from(format!("user-{id}")))
                            .max_w(relative(0.8))
                            .rounded(theme.radius.xl2)
                            .bg(theme.message)
                            .p_3()
                            .text_size(TextSize::Sm.size())
                            .line_height(relative(1.625))
                            .child(text.clone())
                            .test_support(),
                    )
                    .into_any_element(),
                ChatItem::Assistant { id, text } => div()
                    .id(SharedString::from(format!("assistant-{id}")))
                    .px_1()
                    .child(assistant_markdown(
                        SharedString::from(format!("assistant-text-{id}")),
                        text,
                        theme,
                        cx,
                    ))
                    .test_support()
                    .into_any_element(),
                ChatItem::Notice { id, text, tone } => match tone {
                    Tone::Error => div()
                        .id(SharedString::from(format!("notice-{id}")))
                        .flex()
                        .items_start()
                        .gap_2()
                        .rounded(theme.radius.lg)
                        .border_1()
                        .border_color(theme.destructive.opacity(0.25))
                        .bg(theme.destructive.opacity(0.08))
                        .px_3()
                        .py_2()
                        .text_token(TextSize::Sm)
                        .text_color(theme.destructive)
                        .child(
                            div()
                                .mt_0p5()
                                .flex_shrink_0()
                                .child(Icon::new(IconName::CircleAlert).size(px(16.))),
                        )
                        .child(div().min_w_0().child(text.clone()))
                        .test_support()
                        .into_any_element(),
                    Tone::Info => div()
                        .id(SharedString::from(format!("notice-{id}")))
                        .px_1()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child(text.clone())
                        .test_support()
                        .into_any_element(),
                },
                ChatItem::Tool { .. } => div().into_any_element(),
            },
        }
    }

    /// An empty thread: the starter, or "What are we editing?" with suggestions; below it
    /// "Working for…" while the first turn runs.
    fn render_empty(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let working = (self.rows.last() == Some(&Row::Working)).then(|| {
            let index = self.rows.len() - 1;
            div()
                .w_full()
                .max_w(CHAT_MAX_WIDTH)
                .mx_auto()
                .px_5()
                .pb_6()
                .child(self.row_content(index, theme, cx))
        });
        if let Some((_, starter)) = &self.starter {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .px_5()
                .child(starter.clone())
                .children(working)
                .into_any_element();
        }
        let folder = self
            .state
            .read(cx)
            .app
            .as_ref()
            .and_then(|app| app.meta.workspace_dir.clone())
            .unwrap_or_default();
        let suggestions = SUGGESTIONS.iter().enumerate().map(|(index, text)| {
            div()
                .id(("suggestion", index))
                .px_3()
                .py_1p5()
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .text_token(TextSize::Sm)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.use_suggestion(text, window, cx)),
                )
                .child(*text)
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .size_full()
            .px_5()
            .py_10()
            .child(
                div()
                    .text_token(TextSize::X2l)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("What are we editing?"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .max_w(px(576.))
                    .children(suggestions),
            )
            .child(
                div()
                    .text_token(TextSize::Xs)
                    .text_color(theme.muted_foreground.opacity(0.7))
                    .child(format!("Working folder: {folder}")),
            )
            .children(working)
            .into_any_element()
    }

    /// No thread at all: the app has no projects yet.
    fn render_no_project(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .px_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .max_w(px(384.))
                    .child(
                        Icon::new(IconName::FolderPlus)
                            .size(px(28.))
                            .text_color(theme.muted_foreground.opacity(0.7)),
                    )
                    .child(
                        div()
                            .text_token(TextSize::Xl)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Add a project"),
                    )
                    .child(
                        div()
                            .text_token(TextSize::Sm)
                            .text_color(theme.muted_foreground)
                            .text_center()
                            .child("A project is one Resolve project with its own threads and edit history: on this computer, on a network database or in Blackmagic Cloud."),
                    )
                    .child(
                        div()
                            .id("add-project-empty")
                            .flex()
                            .items_center()
                            .h_8()
                            .px_3()
                            .rounded(theme.radius.md)
                            .bg(theme.primary)
                            .text_color(theme.primary_foreground)
                            .text_token(TextSize::Sm)
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ChatEvent::AddProject)))
                            .child("Add project")
                            .test_support(),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return div().id("chat").flex_1().into_any_element();
        };
        if app.meta.thread_id.is_none() {
            return div()
                .id("chat")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(self.render_no_project(&theme, cx))
                .test_support()
                .into_any_element();
        }
        let empty = app.items.is_empty();
        let approvals = app.meta.approvals.clone();
        let blocked = app.meta.blocked.clone();
        let project_id = app.meta.project_id.clone();

        let messages = if empty {
            div()
                .id("chat-messages")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.render_empty(&theme, cx))
                .into_any_element()
        } else {
            div()
                .id("chat-messages")
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, index, _, cx| this.render_row(index, cx)),
                    )
                    .size_full(),
                )
                .child(Scrollbar::vertical(&self.list))
                .test_support()
                .into_any_element()
        };

        let approval_cards = approvals.into_iter().map(|request| {
            let id = request.id.clone();
            let state = self.state.clone();
            ApprovalCard::new(request, move |allow, _, cx| {
                state.read(cx).send(ClientMessage::Approve {
                    id: id.clone(),
                    allow,
                });
            })
        });

        let blocked_bar = blocked.map(|blocked| {
            let state = self.state.clone();
            div()
                .id("blocked")
                .flex()
                .items_center()
                .gap_3()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.warning.opacity(0.3))
                .bg(theme.warning.opacity(0.08))
                .px_3()
                .py_2()
                .text_token(TextSize::Sm)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.warning_foreground)
                        .child(blocked.message),
                )
                .when_some(
                    project_id.filter(|_| blocked.can_open),
                    |this, project_id| {
                        this.child(
                            div()
                                .id("open-in-resolve")
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap_1p5()
                                .h_7()
                                .px_2p5()
                                .rounded(theme.radius.md)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.card)
                                .text_token(TextSize::Xs)
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.accent))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Resolve saves the project it has open first",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(move |_, _, cx: &mut App| {
                                    state.read(cx).send(ClientMessage::OpenInResolve {
                                        project_id: project_id.clone(),
                                    })
                                })
                                .child(Icon::new(IconName::MonitorUp).size(px(14.)))
                                .child("Open in Resolve")
                                .test_support(),
                        )
                    },
                )
                .test_support()
        });

        div()
            .id("chat")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(messages)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .max_w(CHAT_MAX_WIDTH)
                    .mx_auto()
                    .px_5()
                    .pb_4()
                    .flex_shrink_0()
                    .children(approval_cards)
                    .children(blocked_bar)
                    .child(self.composer.clone()),
            )
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linker_edit_protocol::ToolStatus;
    use serde_json::json;

    fn tool(id: &str, name: &str) -> ChatItem {
        ChatItem::Tool {
            id: id.into(),
            name: name.into(),
            input: json!({}),
            status: ToolStatus::Done,
            output: None,
        }
    }

    fn text(id: &str) -> ChatItem {
        ChatItem::Assistant {
            id: id.into(),
            text: "ok".into(),
        }
    }

    #[test]
    fn groups_consecutive_tool_calls_and_hides_tool_search() {
        let items = [
            text("a"),
            tool("t1", "Bash"),
            tool("s", "ToolSearch"),
            tool("t2", "Read"),
            text("b"),
            tool("s2", "ToolSearch"),
            tool("t3", "Bash"),
        ];
        assert_eq!(
            rows(&items),
            [
                Row::Item(0),
                Row::Tools(vec![1, 3]),
                Row::Item(4),
                Row::Tools(vec![6]),
            ]
        );
        // A tool group's key changes when one of its tools does.
        let before = row_key(&Row::Tools(vec![1, 3]), &items);
        let mut later = items.clone();
        later[3] = ChatItem::Tool {
            id: "t2".into(),
            name: "Read".into(),
            input: json!({}),
            status: ToolStatus::Done,
            output: Some("file".into()),
        };
        assert_ne!(before, row_key(&Row::Tools(vec![1, 3]), &later));
        assert_eq!(before.0, "t1");
    }
}
