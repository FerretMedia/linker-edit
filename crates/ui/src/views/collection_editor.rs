//! CollectionEditor.svelte: one workspace or series, as the settings page's "Workspaces & series"
//! shows the one picked: its name, icon, place and rules, setting it up with AI, working on its
//! own, the lessons and example notes it learned, and deleting it. It sends its changes itself;
//! the settings page closes when its setup thread opens ([`CollectionEditorEvent::Setup`]).
//!
//! It draws its own settings card (SettingsGroup.svelte and SettingsRow.svelte's look).

use chrono::DateTime;
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, PromptLevel, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, point,
    prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use linker_edit_protocol::{
    ClientMessage, CollectionIcon as Icon_, CollectionKind, CollectionMessage, CollectionSummary,
};

use crate::icons::IconName;
use crate::labels::collection_label;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::collection_icon::{CollectionIcon, CollectionIconPicker, IconChanged, IconSize};
use crate::views::context_menu::{ContextMenu, MenuEvent, MenuItem};
use crate::views::dialog::bare_input;
use crate::views::dialog::switch;

pub enum CollectionEditorEvent {
    /// Its setup thread opens: the settings page steps aside.
    Setup,
    Deleted,
}

/// What a change sends: update_collection's fields.
#[derive(Default)]
struct Changes {
    name: Option<String>,
    icon: Option<Icon_>,
    rules: Option<String>,
    parent_id: Option<Option<String>>,
    autonomous: Option<bool>,
}

pub struct CollectionEditor {
    state: Entity<AppState>,
    pub collection_id: String,
    name: Entity<InputState>,
    rules: Entity<TextareaState>,
    saved_name: String,
    saved_rules: String,
    picker: Option<(Entity<CollectionIconPicker>, Subscription)>,
    parent_menu: Option<(Entity<ContextMenu<Option<String>>>, Subscription)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CollectionEditorEvent> for CollectionEditor {}

impl CollectionEditor {
    pub fn new(
        state: Entity<AppState>,
        collection_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let collection = Self::find(&state, &collection_id, cx);
        let (name, rules) = collection.map_or((String::new(), String::new()), |c| {
            (c.name.clone(), c.rules.clone())
        });
        let name_input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        let rules_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(8, 30)
                .placeholder("Lower thirds in Inter Bold. No swearing. End on the logo.")
        });
        rules_input.update(cx, |input, cx| input.set_value(rules.clone(), window, cx));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe(&name_input, |this, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.save_name(cx),
                _ => cx.notify(),
            }),
            cx.subscribe(&rules_input, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        CollectionEditor {
            state,
            collection_id,
            name: name_input,
            rules: rules_input,
            saved_name: name,
            saved_rules: rules,
            picker: None,
            parent_menu: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn find<'a>(
        state: &Entity<AppState>,
        id: &str,
        cx: &'a gpui::App,
    ) -> Option<&'a CollectionSummary> {
        state
            .read(cx)
            .app
            .as_ref()?
            .meta
            .collections
            .iter()
            .find(|collection| collection.id == id)
    }

    fn collection<'a>(&self, cx: &'a gpui::App) -> Option<&'a CollectionSummary> {
        Self::find(&self.state, &self.collection_id, cx)
    }

    /// A change saved elsewhere, as in another window, shows here unless it's being edited here.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self.collection(cx).cloned() else {
            return;
        };
        if self.name.read(cx).value() == self.saved_name {
            let name = collection.name.clone();
            self.name
                .update(cx, |input, cx| input.set_value(name, window, cx));
        }
        if self.rules.read(cx).value() == self.saved_rules {
            let rules = collection.rules.clone();
            self.rules
                .update(cx, |input, cx| input.set_value(rules, window, cx));
        }
        self.saved_name = collection.name.clone();
        self.saved_rules = collection.rules.clone();
        if let Some((picker, _)) = &self.picker {
            picker.update(cx, |picker, cx| {
                if *picker.icon() != collection.icon {
                    picker.set_icon(collection.icon.clone(), cx)
                }
            });
        }
        cx.notify();
    }

    fn update(&self, changes: Changes, cx: &gpui::App) {
        self.state.read(cx).send(ClientMessage::Collection(
            CollectionMessage::UpdateCollection {
                id: self.collection_id.clone(),
                name: changes.name,
                icon: changes.icon,
                rules: changes.rules,
                parent_id: changes.parent_id,
                autonomous: changes.autonomous,
            },
        ));
    }

    fn save_name(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.collection(cx).map(|c| c.name.clone()) else {
            return;
        };
        let typed = self.name.read(cx).value().to_string();
        let next = typed.split_whitespace().collect::<Vec<_>>().join(" ");
        if !next.is_empty() && next != current {
            self.update(
                Changes {
                    name: Some(next),
                    ..Default::default()
                },
                cx,
            );
        }
    }

    fn save_rules(&mut self, cx: &mut Context<Self>) {
        let rules = self.rules.read(cx).value().to_string();
        self.update(
            Changes {
                rules: Some(rules),
                ..Default::default()
            },
            cx,
        );
    }

    fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.take().is_none()
            && let Some(collection) = self.collection(cx).cloned()
        {
            let picker = cx.new(|cx| {
                CollectionIconPicker::new(collection.icon.clone(), &collection.name, window, cx)
            });
            let subscription = cx.subscribe(&picker, |this, _, IconChanged(icon), cx| {
                this.update(
                    Changes {
                        icon: Some(icon.clone()),
                        ..Default::default()
                    },
                    cx,
                )
            });
            self.picker = Some((picker, subscription));
        }
        cx.notify();
    }

    fn open_parent_menu(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.state.read(cx).app.as_ref() else {
            return;
        };
        let items = std::iter::once(MenuItem::new("parent-none", "None", None))
            .chain(
                app.meta
                    .collections
                    .iter()
                    .filter(|c| c.kind == CollectionKind::Workspace)
                    .map(|w| {
                        MenuItem::new(
                            format!("parent-{}", w.id),
                            w.name.clone(),
                            Some(w.id.clone()),
                        )
                    }),
            )
            .collect();
        let menu = cx.new(|cx| ContextMenu::new(items, at, false, window, cx));
        let subscription = cx.subscribe(&menu, |this, _, event: &MenuEvent<Option<String>>, cx| {
            if let MenuEvent::Select(parent) = event {
                this.update(
                    Changes {
                        parent_id: Some(parent.clone()),
                        ..Default::default()
                    },
                    cx,
                );
            }
            this.parent_menu = None;
            cx.notify();
        });
        self.parent_menu = Some((menu, subscription));
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self.collection(cx) else {
            return;
        };
        let label = collection_label(collection.kind);
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Delete {} \"{}\"? Its rules, lessons and notes are deleted too.",
                label.lower, collection.name
            ),
            None,
            &["OK", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.state.read(cx).send(ClientMessage::Collection(
                        CollectionMessage::DeleteCollection {
                            id: this.collection_id.clone(),
                        },
                    ));
                    cx.emit(CollectionEditorEvent::Deleted);
                });
            }
        })
        .detach();
    }
}

/// SettingsRow.svelte: what it is on the left, its control on the right.
fn row(
    label: impl Into<SharedString>,
    description: Option<String>,
    detail: Option<AnyElement>,
    control: Option<AnyElement>,
    theme: &Theme,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_6()
        .p_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_token(TextSize::Sm).child(label.into()))
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .mt_0p5()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(description),
                    )
                })
                .children(detail),
        )
        .when_some(control, |this, control| {
            this.child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(control),
            )
        })
}

/// The editor's buttons: `h-8 rounded-md border px-3 text-xs hover:bg-accent`.
fn button(id: impl Into<gpui::ElementId>, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .h_8()
        .px_3()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.border)
        .text_token(TextSize::Xs)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |style| style.bg(accent))
}

fn remove_button(
    id: impl Into<gpui::ElementId>,
    tip: String,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
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
            gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .child(Icon::new(IconName::X).size(px(14.)))
}

impl Render for CollectionEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let Some(collection) = self.collection(cx).cloned() else {
            return div().into_any_element();
        };
        let app = self.state.read(cx).app.as_ref();
        let project_names: Vec<String> = app.map_or(Vec::new(), |app| {
            app.meta
                .projects
                .iter()
                .filter(|p| collection.project_ids.contains(&p.id))
                .map(|p| p.name.clone())
                .collect()
        });
        let series_count = app.map_or(0, |app| {
            if collection.kind == CollectionKind::Workspace {
                app.meta
                    .collections
                    .iter()
                    .filter(|c| {
                        c.kind == CollectionKind::Series
                            && c.parent_id.as_deref() == Some(collection.id.as_str())
                    })
                    .count()
            } else {
                0
            }
        });
        let parent_name = collection.parent_id.as_ref().and_then(|parent| {
            app.and_then(|app| app.meta.collections.iter().find(|c| &c.id == parent))
                .map(|c| c.name.clone())
        });
        let label = collection_label(collection.kind);
        let workspace = collection_label(CollectionKind::Workspace);
        let series = collection_label(CollectionKind::Series);
        let rules_changed = self.rules.read(cx).value() != collection.rules;
        let empty = series_count == 0 && project_names.is_empty();
        // Fewer rounds over time means the brief works.
        let rounds: Vec<String> = collection
            .edits
            .iter()
            .take(8)
            .map(|edit| edit.rounds.to_string())
            .collect();
        let name_focused = self.name.read(cx).focus_handle(cx).is_focused(window);
        let rules_focused = self.rules.read(cx).focus_handle(cx).is_focused(window);
        let border = |focused: bool| if focused { theme.primary } else { theme.border };

        let mut rows: Vec<AnyElement> =
            vec![
                row(
                    "Name",
                    None,
                    None,
                    Some(
                        div()
                            .id("collection-name")
                            .flex()
                            .items_center()
                            .h_8()
                            .w(px(224.))
                            .px_2p5()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(border(name_focused))
                            .text_token(TextSize::Sm)
                            .child(bare_input(&self.name, TextSize::Sm))
                            .test_support()
                            .into_any_element(),
                    ),
                    &theme,
                )
                .into_any_element(),
                row(
                    "Icon",
                    None,
                    self.picker
                        .as_ref()
                        .map(|(picker, _)| div().mt_3().child(picker.clone()).into_any_element()),
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(CollectionIcon::new(
                                &collection.icon,
                                collection.name.clone(),
                                IconSize::Lg,
                            ))
                            .child(
                                button("change-collection-icon", &theme)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_picker(window, cx)
                                    }))
                                    .child(if self.picker.is_some() {
                                        "Done"
                                    } else {
                                        "Change"
                                    })
                                    .test_support(),
                            )
                            .into_any_element(),
                    ),
                    &theme,
                )
                .into_any_element(),
            ];
        if collection.kind == CollectionKind::Series {
            rows.push(
                row(
                    format!("In {}", workspace.lower),
                    None,
                    None,
                    Some(
                        div()
                            .id("collection-parent")
                            .flex()
                            .items_center()
                            .h_8()
                            .w(px(224.))
                            .px_2p5()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .text_token(TextSize::Sm)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                                let at = event.position();
                                this.open_parent_menu(point(at.x, at.y + px(12.)), window, cx)
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(parent_name.unwrap_or("None".into())),
                            )
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .size(px(14.))
                                    .text_color(theme.muted_foreground),
                            )
                            .test_support()
                            .into_any_element(),
                    ),
                    &theme,
                )
                .into_any_element(),
            );
        }
        rows.push(
            row(
                "Projects",
                Some(if project_names.is_empty() {
                    "None yet. Put a project in it from the project's settings.".into()
                } else {
                    project_names.join(", ")
                }),
                None,
                None,
                &theme,
            )
            .into_any_element(),
        );
        rows.push(
            row(
                "Set up with AI",
                Some("Point the agent at videos you've finished. It studies them, asks what it can't see, and proposes these rules for you to accept.".into()),
                None,
                Some(
                    button("setup-collection", &theme)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.read(cx).send(ClientMessage::Collection(
                                CollectionMessage::SetupCollection {
                                    id: this.collection_id.clone(),
                                },
                            ));
                            cx.emit(CollectionEditorEvent::Setup);
                        }))
                        .child(Icon::new(IconName::Sparkles).size(px(14.)))
                        .child(if collection.setup_project_id.is_some() {
                            "Open setup"
                        } else {
                            "Set up with AI"
                        })
                        .test_support()
                        .into_any_element(),
                ),
                &theme,
            )
            .into_any_element(),
        );
        rows.push(
            row(
                "Works on its own",
                Some(format!("Give a project in this {} footage and the agent makes the whole first cut without asking, then says what it decided. You give feedback on the video.", label.lower)),
                None,
                Some(
                    switch("collection-autonomous", collection.autonomous, &theme, {
                        let entity = cx.entity();
                        move |autonomous, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.update(
                                    Changes {
                                        autonomous: Some(autonomous),
                                        ..Default::default()
                                    },
                                    cx,
                                )
                            })
                        }
                    })
                    .test_support()
                    .into_any_element(),
                ),
                &theme,
            )
            .into_any_element(),
        );

        let primary = theme.primary;
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
                                format!(" and its {}", series.lower_many)
                            } else {
                                String::new()
                            }
                        )),
                )
                .child(
                    div()
                        .id("collection-rules")
                        .w_full()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(border(rules_focused))
                        .px_2p5()
                        .py_2()
                        .font_family(theme.mono_font_family)
                        .text_token(TextSize::Xs)
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            let keys = &event.keystroke;
                            if keys.key == "s" && keys.modifiers.secondary(cx) {
                                cx.stop_propagation();
                                if this.rules.read(cx).value() != this.saved_rules {
                                    this.save_rules(cx);
                                }
                            }
                        }))
                        .child(Textarea::new(&self.rules).appearance(false))
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
                            let rules = collection.rules.clone();
                            this.child(
                                button("undo-rules", &theme)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let rules = rules.clone();
                                        this.rules.update(cx, |input, cx| {
                                            input.set_value(rules, window, cx)
                                        });
                                        cx.notify();
                                    }))
                                    .child("Undo")
                                    .test_support(),
                            )
                        })
                        .child(
                            div()
                                .id("save-rules")
                                .flex()
                                .items_center()
                                .h_8()
                                .px_3()
                                .rounded(theme.radius.md)
                                .bg(primary)
                                .text_color(theme.primary_foreground)
                                .text_token(TextSize::Xs)
                                .whitespace_nowrap()
                                .map(|this| {
                                    if rules_changed {
                                        this.cursor_pointer()
                                            .hover(|style| style.opacity(0.9))
                                            .on_click(cx.listener(|this, _, _, cx| this.save_rules(cx)))
                                    } else {
                                        this.opacity(0.5)
                                    }
                                })
                                .child(if rules_changed { "Save rules" } else { "Saved" })
                                .test_support(),
                        ),
                )
                .into_any_element(),
        );

        let date = |iso: &str| {
            DateTime::parse_from_rfc3339(iso)
                .map(|date| date.format("%b %-d, %Y").to_string())
                .unwrap_or_default()
        };
        let lessons_text = format!(
            "{}{}",
            if collection.lessons.is_empty() {
                "None yet. When your feedback on an edit holds for the next video too, the agent proposes a lesson and you accept or decline it."
            } else {
                "The agent follows these after the rules above. You accepted each one; remove any that turned out wrong."
            },
            if rounds.is_empty() {
                String::new()
            } else {
                format!(
                    " Rounds of feedback before you approved an edit, latest first: {}.",
                    rounds.join(" · ")
                )
            }
        );
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
                .when(!collection.lessons.is_empty(), |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_col()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .children(collection.lessons.iter().enumerate().map(
                                |(index, lesson)| {
                                    let lesson_id = lesson.id.clone();
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap_2()
                                        .px_2p5()
                                        .py_2()
                                        .when(index > 0, |this| {
                                            this.border_t_1().border_color(theme.border)
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_token(TextSize::Sm)
                                                        .child(lesson.rule.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .mt_0p5()
                                                        .text_token(TextSize::Xs)
                                                        .text_color(theme.muted_foreground)
                                                        .child(format!(
                                                            "From “{}”{} · {}",
                                                            lesson.feedback,
                                                            lesson.project.as_ref().map_or(
                                                                String::new(),
                                                                |p| format!(" · {p}")
                                                            ),
                                                            date(&lesson.added_at)
                                                        )),
                                                ),
                                        )
                                        .child(
                                            remove_button(
                                                SharedString::from(format!(
                                                    "remove-lesson-{}",
                                                    lesson.id
                                                )),
                                                "Remove this lesson".into(),
                                                &theme,
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.state.read(cx).send(ClientMessage::Collection(
                                                    CollectionMessage::RemoveLesson {
                                                        id: this.collection_id.clone(),
                                                        lesson_id: lesson_id.clone(),
                                                    },
                                                ))
                                            }))
                                            .test_support(),
                                        )
                                },
                            )),
                    )
                })
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
                            .children(collection.examples.iter().enumerate().map(|(index, example)| {
                                let name = example.name.clone();
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2p5()
                                    .py_1p5()
                                    .when(index > 0, |this| {
                                        this.border_t_1().border_color(theme.border)
                                    })
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
                                            SharedString::from(format!("remove-example-{index}")),
                                            "Remove these notes. The video stays.".into(),
                                            &theme,
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.state.read(cx).send(ClientMessage::Collection(
                                                CollectionMessage::RemoveExample {
                                                    id: this.collection_id.clone(),
                                                    name: name.clone(),
                                                },
                                            ))
                                        }))
                                        .test_support(),
                                    )
                            })),
                    )
                    .into_any_element(),
            );
        }

        let destructive = theme.destructive;
        rows.push(
            row(
                format!("Delete {}", label.lower),
                Some(if empty {
                    "Its rules, lessons and notes are deleted too. Projects are never deleted."
                        .into()
                } else {
                    format!(
                        "Move its projects{} out first.",
                        if collection.kind == CollectionKind::Workspace {
                            format!(" and {}", series.lower_many)
                        } else {
                            String::new()
                        }
                    )
                }),
                None,
                Some(
                    button("delete-collection", &theme)
                        .text_color(destructive)
                        .when(!empty, |this| this.opacity(0.5))
                        .when(empty, |this| {
                            this.on_click(
                                cx.listener(|this, _, window, cx| this.delete(window, cx)),
                            )
                        })
                        .child(Icon::new(IconName::Trash2).size(px(14.)))
                        .child("Delete")
                        .test_support()
                        .into_any_element(),
                ),
                &theme,
            )
            .into_any_element(),
        );

        let count = rows.len();
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            div()
                .when(index > 0 && index < count, |this| {
                    this.border_t_1().border_color(theme.border)
                })
                .child(row)
        });
        div()
            .id(SharedString::from(format!(
                "collection-editor-{}",
                collection.id
            )))
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_1()
                    .child(
                        div()
                            .text_token(TextSize::Sm)
                            .font_weight(FontWeight::MEDIUM)
                            .child(collection.name.clone()),
                    )
                    .child(
                        div()
                            .mt_0p5()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(label.one),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(theme.radius.xl)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.card)
                    .children(rows),
            )
            .children(self.parent_menu.as_ref().map(|(menu, _)| menu.clone()))
            .test_support()
            .into_any_element()
    }
}
