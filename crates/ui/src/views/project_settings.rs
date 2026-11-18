//! ProjectSettingsDialog.svelte: a project's name and icon (with the icon picker), the workspace
//! or series it is in, its Resolve project (and linking it to the one open in Resolve), its
//! folder, and removing it from Linker-Edit. The form edits a copy; Save sends what changed.

use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, PromptLevel, Render, StatefulInteractiveElement, Styled, Subscription, Window,
    div, point, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::{
    ClientMessage, CollectionKind, CollectionMessage, ProjectIcon as Icon_, ProjectSidebarChanges,
    ProjectSummary, ResolveLinkKind, ResolveStatus,
};

use crate::icons::IconName;
use crate::labels::{COLLECTION_PICKER_LABEL, NO_COLLECTION, collection_label, library_label};
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::add_project::DialogClosed;
use crate::views::context_menu::{ContextMenu, MenuEvent, MenuItem};
use crate::views::dialog::{Dialog, caption, field, primary_button};
use crate::views::project_icon::{IconPicker, IconPickerEvent, ProjectIcon};

pub struct ProjectSettingsDialog {
    state: Entity<AppState>,
    focus: FocusHandle,
    project_id: String,
    /// Empty while the project shows its Resolve project's name, so a rename in Resolve still shows.
    initial_name: String,
    initial_icon: Option<Icon_>,
    initial_collection: String,
    name: Entity<InputState>,
    icon: Option<Icon_>,
    collection: String,
    picker: Option<(Entity<IconPicker>, Subscription)>,
    select: Option<(Entity<ContextMenu<String>>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DialogClosed> for ProjectSettingsDialog {}

impl ProjectSettingsDialog {
    pub fn new(
        state: Entity<AppState>,
        project_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app = state.read(cx).app.as_ref();
        let project = app.and_then(|app| app.meta.projects.iter().find(|p| p.id == project_id));
        let initial_name = project
            .map(|p| {
                if p.link.as_ref().is_some_and(|link| link.name == p.name) {
                    String::new()
                } else {
                    p.name.clone()
                }
            })
            .unwrap_or_default();
        let placeholder = project
            .map(|p| {
                p.link
                    .as_ref()
                    .map_or(p.name.clone(), |link| link.name.clone())
            })
            .unwrap_or_default();
        let initial_icon = project.and_then(|p| p.sidebar.icon.clone());
        let initial_collection = app
            .and_then(|app| {
                app.meta
                    .collections
                    .iter()
                    .find(|c| c.project_ids.contains(&project_id))
                    .map(|c| c.id.clone())
            })
            .unwrap_or_default();
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial_name.clone())
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        ProjectSettingsDialog {
            state,
            focus,
            project_id,
            initial_name,
            icon: initial_icon.clone(),
            initial_icon,
            collection: initial_collection.clone(),
            initial_collection,
            name,
            picker: None,
            select: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn picker(&self) -> Option<&Entity<IconPicker>> {
        self.picker.as_ref().map(|(picker, _)| picker)
    }

    fn project<'a>(&self, cx: &'a gpui::App) -> Option<&'a ProjectSummary> {
        self.state
            .read(cx)
            .app
            .as_ref()?
            .meta
            .projects
            .iter()
            .find(|project| project.id == self.project_id)
    }

    fn display_name(&self, cx: &gpui::App) -> String {
        let typed = self.name.read(cx).value().trim().to_string();
        if typed.is_empty() {
            self.project(cx).map(|p| p.name.clone()).unwrap_or_default()
        } else {
            typed
        }
    }

    /// Workspaces, each followed by its series, then series on their own.
    fn collection_options(&self, cx: &gpui::App) -> Vec<(String, String)> {
        let Some(app) = self.state.read(cx).app.as_ref() else {
            return Vec::new();
        };
        let collections = &app.meta.collections;
        let series = |parent: Option<&str>| -> Vec<&linker_edit_protocol::CollectionSummary> {
            collections
                .iter()
                .filter(|c| c.kind == CollectionKind::Series && c.parent_id.as_deref() == parent)
                .collect()
        };
        let series_word = collection_label(CollectionKind::Series).lower;
        let mut options = Vec::new();
        for workspace in collections
            .iter()
            .filter(|c| c.kind == CollectionKind::Workspace)
        {
            options.push((workspace.id.clone(), workspace.name.clone()));
            for entry in series(Some(&workspace.id)) {
                options.push((
                    entry.id.clone(),
                    format!("{} / {}", workspace.name, entry.name),
                ));
            }
        }
        for entry in series(None) {
            options.push((entry.id.clone(), format!("{} ({series_word})", entry.name)));
        }
        options
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.display_name(cx);
        let icon = self.icon.clone();
        let picker = cx.new(|cx| IconPicker::new(icon.as_ref(), &name, window, cx));
        let subscription = cx.subscribe_in(
            &picker,
            window,
            |this, _, event: &IconPickerEvent, window, cx| match event {
                IconPickerEvent::Selected(icon) => {
                    this.icon = Some(icon.clone());
                    cx.notify();
                }
                IconPickerEvent::Close => {
                    this.picker = None;
                    this.focus.focus(window, cx);
                    cx.notify();
                }
            },
        );
        self.picker = Some((picker, subscription));
        cx.notify();
    }

    fn open_select(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = std::iter::once((String::new(), NO_COLLECTION.to_string()))
            .chain(self.collection_options(cx))
            .map(|(id, label)| {
                let item = MenuItem::new(format!("collection-{id}"), label, id.clone());
                if id == self.collection {
                    item.icon(IconName::Check)
                } else {
                    item
                }
            })
            .collect();
        let menu = cx.new(|cx| ContextMenu::new(items, at, false, window, cx));
        let subscription = cx.subscribe_in(
            &menu,
            window,
            |this, _, event: &MenuEvent<String>, window, cx| {
                if let MenuEvent::Select(id) = event {
                    this.collection = id.clone();
                }
                this.select = None;
                this.focus.focus(window, cx);
                cx.notify();
            },
        );
        self.select = Some((menu, subscription));
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let typed = self.name.read(cx).value().to_string();
        let name = (typed.trim() != self.initial_name).then_some(typed);
        let icon = (self.icon != self.initial_icon).then(|| self.icon.clone());
        let state = self.state.read(cx);
        state.send(ClientMessage::UpdateProject {
            project_id: self.project_id.clone(),
            name,
            changes: ProjectSidebarChanges {
                icon,
                pinned: None,
                archived: None,
            },
        });
        if self.collection != self.initial_collection {
            state.send(ClientMessage::Collection(CollectionMessage::MoveProject {
                project_id: self.project_id.clone(),
                collection_id: (!self.collection.is_empty()).then(|| self.collection.clone()),
            }));
        }
        cx.emit(DialogClosed);
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.project(cx).map(|p| p.name.clone()) else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Remove \"{name}\" from Linker-Edit? Its folder, edit history and Resolve project stay; you can add it again."
            ),
            None,
            &["OK", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.state.read(cx).send(ClientMessage::RemoveProject {
                        project_id: this.project_id.clone(),
                    });
                    cx.emit(DialogClosed);
                });
            }
        })
        .detach();
    }
}

impl Render for ProjectSettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let Some(project) = self.project(cx).cloned() else {
            return div().into_any_element();
        };
        let app = self.state.read(cx).app.as_ref();
        // The project open in Resolve when no Linker-Edit project has it, to link this one to.
        let relink_to = app.and_then(|app| match &app.meta.resolve {
            ResolveStatus::Connected {
                project: Some(open),
                ..
            } if !app.meta.projects.iter().any(|p| p.open) => Some(open.clone()),
            _ => None,
        });
        let has_collections = app.is_some_and(|app| !app.meta.collections.is_empty());
        let options = self.collection_options(cx);
        let collection_label_text = options
            .iter()
            .find(|(id, _)| *id == self.collection)
            .map_or(NO_COLLECTION.to_string(), |(_, label)| label.clone());
        let display_name = self.display_name(cx);
        let entity = cx.entity();
        let accent = theme.accent;
        let label = |text: &'static str| {
            div()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let resolve_project = match &project.link {
            Some(link) => {
                let icon = match link.kind {
                    ResolveLinkKind::Local => IconName::HardDrive,
                    ResolveLinkKind::Network => IconName::Server,
                    ResolveLinkKind::Cloud => IconName::Cloud,
                };
                let database = project
                    .local
                    .database
                    .as_ref()
                    .map_or(String::new(), |db| format!(" \"{}\"", db.name));
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_token(TextSize::Sm)
                    .child(
                        Icon::new(icon)
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(link.name.clone())
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(format!("· {}{database}", library_label(link.kind))),
                    )
                    .into_any_element()
            }
            None => caption(
                "Not linked yet. The first message you send links it to the project open in Resolve.",
                &theme,
            )
            .text_token(TextSize::Sm)
            .into_any_element(),
        };

        let dialog = Dialog::new(
            "project-settings-dialog",
            "Project settings",
            &self.focus,
            {
                let entity = entity.clone();
                move |_, cx| entity.update(cx, |_, cx| cx.emit(DialogClosed))
            },
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap_3()
                .child(
                    div().flex().flex_col().gap_1().child(label("Icon")).child(
                        div()
                            .id("choose-project-icon")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size_8()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Choose project icon")
                                    .build(window, cx)
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_picker(window, cx)),
                            )
                            .child(ProjectIcon::new(
                                self.icon.as_ref(),
                                display_name.clone(),
                                px(20.),
                            ))
                            .test_support(),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .flex_col()
                        .gap_1()
                        .child(label("Name"))
                        .child(
                            field(&self.name, window, cx)
                                .id("project-name")
                                .test_support(),
                        ),
                ),
        )
        .when(self.icon.is_some(), |this| {
            let foreground = theme.foreground;
            this.child(
                div()
                    .id("use-letters")
                    .mt(px(-8.))
                    .self_start()
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(foreground).underline())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.icon = None;
                        cx.notify()
                    }))
                    .child("Use the letters of its name")
                    .test_support(),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label(COLLECTION_PICKER_LABEL))
                .child(
                    div()
                        .id("project-collection")
                        .flex()
                        .items_center()
                        .h_8()
                        .w_full()
                        .px_2p5()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .text_token(TextSize::Sm)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                            let at = event.position();
                            this.open_select(point(at.x, at.y + px(12.)), window, cx)
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(collection_label_text),
                        )
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(px(14.))
                                .text_color(theme.muted_foreground),
                        )
                        .test_support(),
                )
                .when(!has_collections, |this| {
                    let workspace = collection_label(CollectionKind::Workspace);
                    let series = collection_label(CollectionKind::Series);
                    this.child(caption(
                        format!(
                            "Make {} and {} in Settings.",
                            workspace.lower_many, series.lower_many
                        ),
                        &theme,
                    ))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_token(TextSize::Xs)
                .child(label("Resolve project"))
                .child(resolve_project)
                .when_some(relink_to, |this, open| {
                    this.child(
                        div()
                            .id("relink-project")
                            .mt_1()
                            .self_start()
                            .flex()
                            .items_center()
                            .h_7()
                            .px_2p5()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .text_token(TextSize::Xs)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.read(cx).send(ClientMessage::RelinkProject {
                                    project_id: this.project_id.clone(),
                                });
                                cx.emit(DialogClosed);
                            }))
                            .child(format!("Link to \"{open}\", open in Resolve now"))
                            .test_support(),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("Folder"))
                .child(
                    div()
                        .font_family(theme.mono_font_family)
                        .text_token(TextSize::X2s)
                        .child(project.dir.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .border_t_1()
                .border_color(theme.border)
                .pt_4()
                .child(
                    div()
                        .id("remove-project")
                        .flex()
                        .items_center()
                        .h_8()
                        .px_2p5()
                        .rounded(theme.radius.md)
                        .text_token(TextSize::Sm)
                        .text_color(theme.destructive)
                        .cursor_pointer()
                        .hover({
                            let tint = theme.destructive.opacity(0.1);
                            move |style| style.bg(tint)
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx)))
                        .child("Remove from Linker-Edit")
                        .test_support(),
                )
                .child(div().flex_1())
                .child(
                    primary_button(
                        "save-project",
                        "Save",
                        false,
                        &theme,
                        cx.listener(|this, _, _, cx| this.save(cx)),
                    )
                    .test_support(),
                ),
        );
        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .child(dialog)
            .children(self.picker.as_ref().map(|(picker, _)| picker.clone()))
            .children(self.select.as_ref().map(|(select, _)| select.clone()))
            .into_any_element()
    }
}
