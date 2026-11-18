//! The settings page's "Workspaces & series" section: CollectionsSettings.svelte (the list,
//! making new ones, the one picked), CollectionEditor.svelte (its name, icon, place, rules,
//! lessons and examples) and CollectionIconPicker.svelte.

use std::collections::HashSet;

use chrono::DateTime;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::{Sizable, Size};
use linker_edit_protocol::{
    ClientMessage, CollectionIcon, CollectionKind, CollectionMessage, CollectionSummary,
};

use crate::collection_icons::{
    CollectionIconView, EMOJI_SUGGESTIONS, ICON_COLORS, IconSize, LUCIDE_ICONS, color_name,
    icon_color, monogram, swatch,
};
use crate::icons::IconName;
use crate::labels::collection_label;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::choice_select::{Choice, ChoiceState, choice_state, chosen, sync_choices};
use crate::views::settings_parts::{
    Segmented, SettingsRow, Switch, confirm_then, disable, ghost_button, group, outline_button,
    primary_button, ring,
};

/// Something the settings page acts on.
pub enum CollectionsEvent {
    /// Its setup thread opens behind the settings page, which closes.
    SetupOpened,
}

struct Creating {
    kind: CollectionKind,
    name: Entity<InputState>,
    parent: Option<Entity<ChoiceState>>,
}

pub struct CollectionsSettings {
    state: Entity<AppState>,
    selected_id: Option<String>,
    creating: Option<Creating>,
    /// The ids before a create, to pick the new one once the server has it.
    known: Option<HashSet<String>>,
    editor: Option<Entity<CollectionEditor>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CollectionsEvent> for CollectionsSettings {}

fn collections(state: &AppState) -> &[CollectionSummary] {
    state
        .app
        .as_ref()
        .map_or(&[][..], |app| &app.meta.collections[..])
}

fn workspace_choices(state: &AppState, none: &str) -> Vec<Choice> {
    std::iter::once(Choice::new("", none.to_string()))
        .chain(
            collections(state)
                .iter()
                .filter(|collection| collection.kind == CollectionKind::Workspace)
                .map(|workspace| Choice::new(workspace.id.clone(), workspace.name.clone())),
        )
        .collect()
}

impl CollectionsSettings {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe_in(&state, window, |this, _, window, cx| {
            this.follow(window, cx)
        })];
        CollectionsSettings {
            state,
            selected_id: None,
            creating: None,
            known: None,
            editor: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }

    /// Picks a new collection once the server has it; drops a picked one that is gone.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if let Some(known) = &self.known
            && let Some(added) = collections(state)
                .iter()
                .find(|collection| !known.contains(&collection.id))
        {
            let id = added.id.clone();
            self.known = None;
            self.select(Some(id), window, cx);
        }
        let state = self.state.read(cx);
        if let Some(id) = &self.selected_id
            && !collections(state)
                .iter()
                .any(|collection| &collection.id == id)
        {
            self.select(None, window, cx);
        }
        cx.notify();
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = id.clone();
        // Keyed by id, so drafts start fresh.
        self.editor = id.map(|id| {
            let editor = cx.new(|cx| CollectionEditor::new(self.state.clone(), id, window, cx));
            self._subscriptions
                .push(
                    cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| match event {
                        EditorEvent::Deleted => {
                            this.selected_id = None;
                            this.editor = None;
                            cx.notify();
                        }
                        EditorEvent::SetupOpened => cx.emit(CollectionsEvent::SetupOpened),
                    }),
                );
            editor
        });
        cx.notify();
    }

    fn start_creating(
        &mut self,
        kind: CollectionKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.read(cx);
        let selected = self
            .selected_id
            .as_ref()
            .and_then(|id| collections(state).iter().find(|c| &c.id == id));
        let parent_id = match (kind, selected) {
            (CollectionKind::Series, Some(selected))
                if selected.kind == CollectionKind::Workspace =>
            {
                Some(selected.id.clone())
            }
            (CollectionKind::Series, Some(selected)) => selected.parent_id.clone(),
            _ => None,
        };
        let label = collection_label(kind);
        let choices = workspace_choices(
            state,
            &format!("No {}", collection_label(CollectionKind::Workspace).lower),
        );
        let placeholder = format!("New {} name", label.lower);
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        name.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(
            &name,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.create(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        let parent = (kind == CollectionKind::Series)
            .then(|| choice_state(choices, parent_id.as_deref().unwrap_or(""), window, cx));
        self.creating = Some(Creating { kind, name, parent });
        cx.notify();
    }

    fn create(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(creating) = &self.creating else {
            return;
        };
        let name = creating.name.read(cx).value().to_string();
        if name.trim().is_empty() {
            return;
        }
        let parent_id = creating
            .parent
            .as_ref()
            .map(|parent| chosen(parent, cx))
            .filter(|id| !id.is_empty());
        let state = self.state.read(cx);
        self.known = Some(collections(state).iter().map(|c| c.id.clone()).collect());
        state.send(ClientMessage::Collection(
            CollectionMessage::CreateCollection {
                kind: creating.kind,
                name,
                parent_id: Some(parent_id),
                icon: None,
            },
        ));
        self.creating = None;
        cx.notify();
    }

    fn counts(state: &AppState, collection: &CollectionSummary) -> String {
        let series = if collection.kind == CollectionKind::Workspace {
            collections(state)
                .iter()
                .filter(|c| {
                    c.kind == CollectionKind::Series && c.parent_id.as_ref() == Some(&collection.id)
                })
                .count()
        } else {
            0
        };
        let projects = collection.project_ids.len();
        let label = collection_label(CollectionKind::Series);
        let mut parts = Vec::new();
        if series > 0 {
            parts.push(format!(
                "{series} {}",
                if series == 1 {
                    label.lower
                } else {
                    label.lower_many
                }
            ));
        }
        parts.push(format!(
            "{projects} project{}",
            if projects == 1 { "" } else { "s" }
        ));
        parts.join(" · ")
    }

    fn row(
        &self,
        collection: &CollectionSummary,
        nested: bool,
        counts: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_id.as_deref() == Some(collection.id.as_str());
        let id = collection.id.clone();
        let accent = theme.accent;
        div()
            .id(SharedString::from(format!("collection-{}", collection.id)))
            .flex()
            .w_full()
            .items_center()
            .gap_3()
            .px_4()
            .py_2p5()
            .when(nested, |this| this.pl(px(44.)))
            .cursor_pointer()
            .hover(move |style| style.bg(accent))
            .when(selected, |this| this.bg(theme.accent))
            .on_click(cx.listener(move |this, _, window, cx| {
                let next = (this.selected_id.as_deref() != Some(id.as_str())).then(|| id.clone());
                this.select(next, window, cx);
            }))
            .child(CollectionIconView::new(&collection.icon, &collection.name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_token(TextSize::Sm)
                    .child(collection.name.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_token(TextSize::Xs)
                    .text_color(theme.muted_foreground)
                    .child(counts.to_string()),
            )
            .test_support()
            .into_any_element()
    }
}

impl Render for CollectionsSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let workspace = collection_label(CollectionKind::Workspace);
        let series = collection_label(CollectionKind::Series);
        let state = self.state.read(cx);
        let all = collections(state).to_vec();
        // The list as the Svelte one nests it: each workspace with its series, then loose series.
        let series_of = |parent: Option<&str>| -> Vec<&CollectionSummary> {
            all.iter()
                .filter(|c| c.kind == CollectionKind::Series && c.parent_id.as_deref() == parent)
                .collect()
        };
        let mut order: Vec<(CollectionSummary, bool, String)> = Vec::new();
        for item in all.iter().filter(|c| c.kind == CollectionKind::Workspace) {
            order.push((item.clone(), false, Self::counts(state, item)));
            for nested in series_of(Some(&item.id)) {
                order.push((nested.clone(), true, Self::counts(state, nested)));
            }
        }
        for loose in series_of(None) {
            order.push((loose.clone(), false, Self::counts(state, loose)));
        }
        let rows: Vec<AnyElement> = order
            .iter()
            .map(|(collection, nested, counts)| self.row(collection, *nested, counts, &theme, cx))
            .collect();

        let new_button = |id: &'static str, kind: CollectionKind, cx: &mut Context<Self>| {
            outline_button(id, &theme)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.start_creating(kind, window, cx)),
                )
                .child(Icon::new(IconName::Plus).size(px(14.)))
                .child(format!("New {}", collection_label(kind).lower))
                .test_support()
        };

        let form = self.creating.as_ref().map(|creating| {
            let empty = creating.name.read(cx).value().trim().is_empty();
            div()
                .id("create-collection")
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .p_3()
                .when(!all.is_empty(), |this| {
                    this.border_t_1().border_color(theme.border)
                })
                .child(div().flex_1().min_w_0().child(Input::new(&creating.name)))
                .when_some(creating.parent.clone(), |this, parent| {
                    this.child(div().w(px(176.)).child(Select::new(&parent)))
                })
                .child(
                    ghost_button("cancel-create", &theme)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.creating = None;
                            cx.notify();
                        }))
                        .child("Cancel")
                        .test_support(),
                )
                .child(
                    disable(primary_button("create", &theme), empty)
                        .when(!empty, |this| {
                            this.on_click(
                                cx.listener(|this, _, window, cx| this.create(window, cx)),
                            )
                        })
                        .child("Create")
                        .test_support(),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.creating = None;
                        cx.notify();
                    }
                }))
                .test_support()
        });

        let list = div()
            .overflow_hidden()
            .rounded(theme.radius.xl)
            .border_1()
            .border_color(theme.border)
            .bg(theme.card)
            .map(|this| {
                if !rows.is_empty() {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .children(crate::views::settings_parts::divided(rows, &theme)),
                    )
                } else if self.creating.is_none() {
                    this.child(
                        div()
                            .p_4()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "None yet. Make a {} for each client, brand or channel you edit for.",
                                workspace.lower
                            )),
                    )
                } else {
                    this
                }
            })
            .children(form);

        let muted = |text: String| {
            div()
                .px_1()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child(text)
        };

        div()
            .id("collections-settings")
            .flex()
            .flex_col()
            .gap_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_end()
                            .gap_2()
                            .px_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_token(TextSize::Xs)
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "A {} is a client, brand or channel. A {} holds its episodes. Each has its own rules for the agent.",
                                        workspace.lower, series.lower
                                    )),
                            )
                            .child(new_button("new-workspace", CollectionKind::Workspace, cx))
                            .child(new_button("new-series", CollectionKind::Series, cx)),
                    )
                    .child(list),
            )
            .map(|this| match &self.editor {
                Some(editor) => this.child(editor.clone()),
                None if !all.is_empty() => this.child(muted(
                    "Pick one to rename it, change its icon, write its rules or set it up with AI.".into(),
                ).mt(px(-16.))),
                None => this,
            })
            .child(muted(format!(
                "Each project has a file of its own too: PROJECT.md in the project's folder, with the video's brief, footage notes, decisions and feedback. The agent keeps it, and you can edit it. It comes after the {}'s and {}' rules.",
                workspace.lower, series.lower_many
            )))
            .test_support()
    }
}

// --- CollectionEditor.svelte ---

pub enum EditorEvent {
    Deleted,
    SetupOpened,
}

pub struct CollectionEditor {
    state: Entity<AppState>,
    id: String,
    name: Entity<InputState>,
    rules: Entity<TextareaState>,
    /// What the server had when the drafts last followed it: a change saved elsewhere shows
    /// here unless it is being edited here.
    saved_name: String,
    saved_rules: String,
    picking: bool,
    parent: Option<Entity<ChoiceState>>,
    letters: Entity<InputState>,
    emoji: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for CollectionEditor {}

impl CollectionEditor {
    pub fn new(
        state: Entity<AppState>,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let collection = collections(state.read(cx))
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .expect("the picked collection");
        let name = cx.new(|cx| InputState::new(window, cx).default_value(collection.name.clone()));
        let rules = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(8, 30)
                .placeholder("Lower thirds in Inter Bold. No swearing. End on the logo.")
                .default_value(collection.rules.clone())
        });
        let (letters_text, emoji_text) = match &collection.icon {
            CollectionIcon::Monogram { text, .. } => (text.clone(), String::new()),
            CollectionIcon::Emoji { emoji, .. } => (String::new(), emoji.clone()),
            CollectionIcon::Lucide { .. } => (String::new(), String::new()),
        };
        let letters = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(monogram(&collection.name))
                .default_value(letters_text)
        });
        let emoji = cx.new(|cx| InputState::new(window, cx).default_value(emoji_text));
        let parent = (collection.kind == CollectionKind::Series).then(|| {
            choice_state(
                workspace_choices(state.read(cx), "None"),
                collection.parent_id.as_deref().unwrap_or(""),
                window,
                cx,
            )
        });
        let mut subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| {
                this.follow(window, cx)
            }),
            cx.subscribe_in(
                &name,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Blur => this.save_name(window, cx),
                    InputEvent::PressEnter { .. } => this.save_name(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe(&rules, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&letters, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text: String = input.read(cx).value().chars().take(3).collect();
                    if let Some(CollectionIcon::Monogram { color, .. }) = this.icon(cx) {
                        this.update(Change::Icon(CollectionIcon::Monogram { text, color }), cx);
                    }
                }
            }),
            cx.subscribe(&emoji, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let emoji: String = input.read(cx).value().trim().chars().take(16).collect();
                    if let (false, Some(CollectionIcon::Emoji { color, .. })) =
                        (emoji.is_empty(), this.icon(cx))
                    {
                        this.update(Change::Icon(CollectionIcon::Emoji { emoji, color }), cx);
                    }
                }
            }),
        ];
        if let Some(parent) = &parent {
            subscriptions.push(cx.subscribe(
                parent,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let parent_id = value.clone().filter(|id| !id.is_empty());
                    this.update(Change::Parent(parent_id), cx);
                },
            ));
        }
        CollectionEditor {
            state,
            id,
            name,
            rules,
            saved_name: collection.name,
            saved_rules: collection.rules,
            picking: false,
            parent,
            letters,
            emoji,
            _subscriptions: subscriptions,
        }
    }

    fn collection<'a>(&self, cx: &'a App) -> Option<&'a CollectionSummary> {
        collections(self.state.read(cx))
            .iter()
            .find(|c| c.id == self.id)
    }

    fn icon(&self, cx: &App) -> Option<CollectionIcon> {
        self.collection(cx).map(|c| c.icon.clone())
    }

    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self.collection(cx).cloned() else {
            return;
        };
        if self.name.read(cx).value() == self.saved_name {
            self.name.update(cx, |input, cx| {
                input.set_value(collection.name.clone(), window, cx)
            });
        }
        if self.rules.read(cx).value() == self.saved_rules {
            self.rules.update(cx, |input, cx| {
                input.set_value(collection.rules.clone(), window, cx)
            });
        }
        self.saved_name = collection.name.clone();
        self.saved_rules = collection.rules.clone();
        if let Some(parent) = &self.parent {
            sync_choices(
                parent,
                workspace_choices(self.state.read(cx), "None"),
                collection.parent_id.as_deref().unwrap_or(""),
                window,
                cx,
            );
        }
        cx.notify();
    }

    fn update(&self, change: Change, cx: &App) {
        let mut message = CollectionMessage::UpdateCollection {
            id: self.id.clone(),
            name: None,
            icon: None,
            rules: None,
            parent_id: None,
            autonomous: None,
        };
        if let CollectionMessage::UpdateCollection {
            name,
            icon,
            rules,
            parent_id,
            autonomous,
            ..
        } = &mut message
        {
            match change {
                Change::Name(value) => *name = Some(value),
                Change::Icon(value) => *icon = Some(value),
                Change::Rules(value) => *rules = Some(value),
                Change::Parent(value) => *parent_id = Some(value),
                Change::Autonomous(value) => *autonomous = Some(value),
            }
        }
        self.state.read(cx).send(ClientMessage::Collection(message));
    }

    fn save_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self.collection(cx).cloned() else {
            return;
        };
        let typed = self.name.read(cx).value().to_string();
        let next = typed.split_whitespace().collect::<Vec<_>>().join(" ");
        if next.is_empty() {
            self.name.update(cx, |input, cx| {
                input.set_value(collection.name.clone(), window, cx)
            });
        } else if next != collection.name {
            self.update(Change::Name(next.chars().take(80).collect()), cx);
        }
    }

    fn rules_changed(&self, cx: &App) -> bool {
        self.collection(cx)
            .is_some_and(|collection| self.rules.read(cx).value() != collection.rules)
    }

    fn save_rules(&mut self, cx: &mut Context<Self>) {
        if self.rules_changed(cx) {
            let rules = self.rules.read(cx).value().to_string();
            self.update(Change::Rules(rules), cx);
        }
    }

    fn icon_picker(
        &self,
        collection: &CollectionSummary,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = collection.icon.clone();
        let color = icon_color(&icon);
        let kind_index = match icon {
            CollectionIcon::Monogram { .. } => 0,
            CollectionIcon::Lucide { .. } => 1,
            CollectionIcon::Emoji { .. } => 2,
        };
        let this = cx.entity().downgrade();
        let pick_kind = move |index: usize, window: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |this, cx| {
                let next = match index {
                    0 => CollectionIcon::Monogram {
                        text: String::new(),
                        color,
                    },
                    1 => CollectionIcon::Lucide {
                        name: "folder".into(),
                        color,
                    },
                    _ => CollectionIcon::Emoji {
                        emoji: EMOJI_SUGGESTIONS[0].into(),
                        color,
                    },
                };
                match &next {
                    CollectionIcon::Monogram { .. } => this
                        .letters
                        .update(cx, |input, cx| input.set_value("", window, cx)),
                    CollectionIcon::Emoji { emoji, .. } => {
                        let emoji = emoji.clone();
                        this.emoji
                            .update(cx, |input, cx| input.set_value(emoji, window, cx))
                    }
                    CollectionIcon::Lucide { .. } => {}
                }
                this.update(Change::Icon(next), cx);
            });
        };
        let cell = |id: SharedString, selected: bool| {
            let accent = theme.accent;
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size_8()
                .rounded(theme.radius.md)
                .cursor_pointer()
                .hover(move |style| style.bg(accent))
                .when(selected, |this| {
                    this.bg(theme.accent).shadow(vec![ring(theme.border, 1.)])
                })
        };
        let kind_row = match &icon {
            CollectionIcon::Monogram { .. } => div()
                .flex()
                .items_center()
                .gap_2()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child("Letters")
                .child(div().w_24().child(Input::new(&self.letters)))
                .into_any_element(),
            CollectionIcon::Lucide { name, .. } => div()
                .flex()
                .flex_wrap()
                .gap_0p5()
                .children(LUCIDE_ICONS.iter().map(|(lucide, glyph)| {
                    let selected = name == lucide;
                    let next = CollectionIcon::Lucide {
                        name: (*lucide).into(),
                        color,
                    };
                    cell(SharedString::from(format!("lucide-{lucide}")), selected)
                        .text_color(if selected {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update(Change::Icon(next.clone()), cx)
                        }))
                        .child(Icon::new(*glyph).size(px(16.)))
                        .test_support()
                }))
                .into_any_element(),
            CollectionIcon::Emoji { emoji, .. } => div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_0p5()
                .children(
                    EMOJI_SUGGESTIONS
                        .iter()
                        .enumerate()
                        .map(|(index, suggestion)| {
                            let next = CollectionIcon::Emoji {
                                emoji: (*suggestion).into(),
                                color,
                            };
                            cell(
                                SharedString::from(format!("emoji-{index}")),
                                emoji == suggestion,
                            )
                            .text_token(TextSize::Base)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let CollectionIcon::Emoji { emoji, .. } = &next {
                                    let emoji = emoji.clone();
                                    this.emoji
                                        .update(cx, |input, cx| input.set_value(emoji, window, cx));
                                }
                                this.update(Change::Icon(next.clone()), cx)
                            }))
                            .child(*suggestion)
                            .test_support()
                        }),
                )
                .child(div().ml_1().w_16().child(Input::new(&self.emoji)))
                .into_any_element(),
        };
        let colors = div()
            .flex()
            .items_center()
            .gap_1p5()
            .children(ICON_COLORS.iter().map(|&swatch_color| {
                let mut next = icon.clone();
                match &mut next {
                    CollectionIcon::Lucide { color, .. }
                    | CollectionIcon::Emoji { color, .. }
                    | CollectionIcon::Monogram { color, .. } => *color = swatch_color,
                }
                let selected = swatch_color == color;
                div()
                    .id(SharedString::from(format!(
                        "color-{}",
                        color_name(swatch_color)
                    )))
                    .size_5()
                    .rounded_full()
                    .bg(swatch(swatch_color))
                    .cursor_pointer()
                    // `ring-2 ring-foreground ring-offset-2`: a ring 2px out, with a gap.
                    .relative()
                    .when(selected, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top(px(-4.))
                                .left(px(-4.))
                                .size(px(28.))
                                .rounded_full()
                                .border_2()
                                .border_color(theme.foreground),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(Change::Icon(next.clone()), cx)
                    }))
                    .test_support()
            }));
        div()
            .id("icon-picker")
            .mt_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().child(Segmented::new(
                "icon-kind",
                ["Letters", "Icon", "Emoji"],
                kind_index,
                pick_kind,
            )))
            .child(kind_row)
            .child(colors)
            .test_support()
            .into_any_element()
    }
}

enum Change {
    Name(String),
    Icon(CollectionIcon),
    Rules(String),
    Parent(Option<String>),
    Autonomous(bool),
}

impl Render for CollectionEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let Some(collection) = self.collection(cx).cloned() else {
            return div().into_any_element();
        };
        let label = collection_label(collection.kind);
        let workspace = collection_label(CollectionKind::Workspace);
        let series_label = collection_label(CollectionKind::Series);
        let state = self.state.read(cx);
        let series_count = if collection.kind == CollectionKind::Workspace {
            collections(state)
                .iter()
                .filter(|c| {
                    c.kind == CollectionKind::Series && c.parent_id.as_ref() == Some(&collection.id)
                })
                .count()
        } else {
            0
        };
        let project_names: Vec<String> = collection
            .project_ids
            .iter()
            .filter_map(|id| {
                state
                    .app
                    .as_ref()?
                    .meta
                    .projects
                    .iter()
                    .find(|p| &p.id == id)
                    .map(|p| p.name.clone())
            })
            .collect();
        let empty = series_count == 0 && project_names.is_empty();
        let rules_changed = self.rules_changed(cx);
        // Fewer rounds over time means the brief works.
        let rounds: Vec<String> = collection
            .edits
            .iter()
            .take(8)
            .map(|edit| edit.rounds.to_string())
            .collect();

        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(
            SettingsRow::new("Name")
                .control(
                    div()
                        .id("collection-name")
                        .w_56()
                        .child(Input::new(&self.name))
                        .test_support(),
                )
                .into_any_element(),
        );
        let mut icon_row = SettingsRow::new("Icon")
            .control(CollectionIconView::new(&collection.icon, &collection.name).size(IconSize::Lg))
            .control(
                outline_button("change-icon", &theme)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picking = !this.picking;
                        cx.notify();
                    }))
                    .child(if self.picking { "Done" } else { "Change" })
                    .test_support(),
            );
        if self.picking {
            icon_row = icon_row.detail(self.icon_picker(&collection, &theme, cx));
        }
        rows.push(icon_row.into_any_element());
        if let Some(parent) = &self.parent {
            rows.push(
                SettingsRow::new(format!("In {}", workspace.lower))
                    .control(div().w_56().child(Select::new(parent)))
                    .into_any_element(),
            );
        }
        rows.push(
            SettingsRow::new("Projects")
                .description(if project_names.is_empty() {
                    "None yet. Put a project in it from the project's settings.".to_string()
                } else {
                    project_names.join(", ")
                })
                .into_any_element(),
        );
        let setup_id = collection.id.clone();
        rows.push(
            SettingsRow::new("Set up with AI")
                .description("Point the agent at videos you've finished. It studies them, asks what it can't see, and proposes these rules for you to accept.")
                .control(
                    outline_button("setup-collection", &theme)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.read(cx).send(ClientMessage::Collection(CollectionMessage::SetupCollection {
                                id: setup_id.clone(),
                            }));
                            cx.emit(EditorEvent::SetupOpened);
                        }))
                        .child(Icon::new(IconName::Sparkles).size(px(14.)))
                        .child(if collection.setup_project_id.is_some() { "Open setup" } else { "Set up with AI" })
                        .test_support(),
                )
                .into_any_element(),
        );
        let this = cx.entity().downgrade();
        rows.push(
            SettingsRow::new("Works on its own")
                .description(format!("Give a project in this {} footage and the agent makes the whole first cut without asking, then says what it decided. You give feedback on the video.", label.lower))
                .control(Switch::new("autonomous", collection.autonomous, move |autonomous, _, cx| {
                    let _ = this.update(cx, |this, cx| this.update(Change::Autonomous(autonomous), cx));
                }))
                .into_any_element(),
        );
        let saved_rules = collection.rules.clone();
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .child(div().text_token(TextSize::Sm).child("Rules for the agent"))
                .child(
                    div()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "In plain words, for every project in this {}{}. More specific rules come after these and win where they disagree.",
                            label.lower,
                            if collection.kind == CollectionKind::Workspace {
                                format!(" and its {}", series_label.lower_many)
                            } else {
                                String::new()
                            }
                        )),
                )
                .child(
                    div()
                        .id("collection-rules")
                        .min_h(px(160.))
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .px_2p5()
                        .py_2()
                        .font_family(theme.mono_font_family)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            let keys = &event.keystroke;
                            if keys.key == "s" && (keys.modifiers.control || keys.modifiers.platform) {
                                cx.stop_propagation();
                                this.save_rules(cx);
                            }
                        }))
                        .child(
                            Textarea::new(&self.rules)
                                .appearance(false)
                                .with_size(Size::XSmall)
                                .text_size(TextSize::Xs.size())
                                .line_height(gpui::relative(1.625)),
                        )
                        .test_support(),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family)
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child(collection.rules_file.clone()),
                        )
                        .when(rules_changed, |this| {
                            this.child(
                                outline_button("undo-rules", &theme)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let saved = saved_rules.clone();
                                        this.rules.update(cx, |input, cx| input.set_value(saved, window, cx));
                                        cx.notify();
                                    }))
                                    .child("Undo")
                                    .test_support(),
                            )
                        })
                        .child(
                            disable(primary_button("save-rules", &theme), !rules_changed)
                                .when(rules_changed, |this| {
                                    this.on_click(cx.listener(|this, _, _, cx| this.save_rules(cx)))
                                })
                                .child(if rules_changed { "Save rules" } else { "Saved" })
                                .test_support(),
                        ),
                )
                .into_any_element(),
        );

        let lessons_text = {
            let mut text = if collection.lessons.is_empty() {
                "None yet. When your feedback on an edit holds for the next video too, the agent proposes a lesson and you accept or decline it.".to_string()
            } else {
                "The agent follows these after the rules above. You accepted each one; remove any that turned out wrong.".to_string()
            };
            if !rounds.is_empty() {
                text.push_str(&format!(
                    " Rounds of feedback before you approved an edit, latest first: {}.",
                    rounds.join(" · ")
                ));
            }
            text
        };
        let remove_button = |id: SharedString, tip: &'static str| {
            let (accent, destructive) = (theme.accent, theme.destructive);
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size_7()
                .rounded(theme.radius.md)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(move |style| style.bg(accent).text_color(destructive))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                })
                .child(Icon::new(IconName::X).size(px(14.)))
        };
        let lessons = (!collection.lessons.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.border)
                .children(crate::views::settings_parts::divided(
                    collection.lessons.iter().map(|lesson| {
                        let (collection_id, lesson_id) = (collection.id.clone(), lesson.id.clone());
                        let date = DateTime::parse_from_rfc3339(&lesson.added_at)
                            .map(|date| {
                                date.with_timezone(&chrono::Local)
                                    .format("%b %-d, %Y")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| lesson.added_at.clone());
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .px_2p5()
                            .py_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div().text_token(TextSize::Sm).child(lesson.rule.clone()),
                                    )
                                    .child(
                                        div()
                                            .mt_0p5()
                                            .text_token(TextSize::Xs)
                                            .text_color(theme.muted_foreground)
                                            .child(format!(
                                                "From “{}”{} · {date}",
                                                lesson.feedback,
                                                lesson
                                                    .project
                                                    .as_ref()
                                                    .map(|p| format!(" · {p}"))
                                                    .unwrap_or_default()
                                            )),
                                    ),
                            )
                            .child(
                                remove_button(
                                    SharedString::from(format!("remove-lesson-{}", lesson.id)),
                                    "Remove this lesson",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.read(cx).send(ClientMessage::Collection(
                                        CollectionMessage::RemoveLesson {
                                            id: collection_id.clone(),
                                            lesson_id: lesson_id.clone(),
                                        },
                                    ))
                                }))
                                .test_support(),
                            )
                            .into_any_element()
                    }),
                    &theme,
                ))
        });
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .child(
                    div()
                        .text_token(TextSize::Sm)
                        .child("Lessons from your feedback"),
                )
                .child(
                    div()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child(lessons_text),
                )
                .children(lessons)
                .into_any_element(),
        );
        if !collection.examples.is_empty() {
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .child(div().text_token(TextSize::Sm).child("Example videos it studied"))
                    .child(
                        div()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child("The agent's notes on each, kept as files it reads when it needs a reference."),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .children(crate::views::settings_parts::divided(
                                collection.examples.iter().map(|example| {
                                    let (collection_id, name) = (collection.id.clone(), example.name.clone());
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2p5()
                                        .py_1p5()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .font_family(theme.mono_font_family)
                                                .text_token(TextSize::X2s)
                                                .text_color(theme.muted_foreground)
                                                .child(example.file.clone()),
                                        )
                                        .child(
                                            remove_button(
                                                SharedString::from(format!("remove-example-{}", example.name)),
                                                "Remove these notes. The video stays.",
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.state.read(cx).send(ClientMessage::Collection(
                                                    CollectionMessage::RemoveExample {
                                                        id: collection_id.clone(),
                                                        name: name.clone(),
                                                    },
                                                ))
                                            }))
                                            .test_support(),
                                        )
                                        .into_any_element()
                                }),
                                &theme,
                            )),
                    )
                    .into_any_element(),
            );
        }
        let (delete_id, delete_name, delete_label) =
            (collection.id.clone(), collection.name.clone(), label.lower);
        let editor = cx.entity().downgrade();
        rows.push(
            SettingsRow::new(format!("Delete {}", label.lower))
                .description(if empty {
                    "Its rules, lessons and notes are deleted too. Projects are never deleted.".to_string()
                } else {
                    format!(
                        "Move its projects{} out first.",
                        if collection.kind == CollectionKind::Workspace {
                            format!(" and {}", series_label.lower_many)
                        } else {
                            String::new()
                        }
                    )
                })
                .control(
                    disable(outline_button("delete-collection", &theme), !empty)
                        .text_color(theme.destructive)
                        .when(empty, |this| {
                            this.on_click(move |_, window, cx| {
                                let (editor, id) = (editor.clone(), delete_id.clone());
                                confirm_then(
                                    &format!("Delete {delete_label} \"{delete_name}\"? Its rules, lessons and notes are deleted too."),
                                    window,
                                    cx,
                                    move |cx| {
                                        let _ = editor.update(cx, |this, cx| {
                                            this.state.read(cx).send(ClientMessage::Collection(
                                                CollectionMessage::DeleteCollection { id },
                                            ));
                                            cx.emit(EditorEvent::Deleted);
                                        });
                                    },
                                );
                            })
                        })
                        .child(Icon::new(IconName::Trash2).size(px(14.)))
                        .child("Delete")
                        .test_support(),
                )
                .into_any_element(),
        );

        div()
            .id("collection-editor")
            .child(group(
                collection.name.clone(),
                Some(label.one.into()),
                rows,
                &theme,
            ))
            .font_weight(FontWeight::NORMAL)
            .test_support()
            .into_any_element()
    }
}
