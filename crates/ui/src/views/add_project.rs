//! AddProjectDialog.svelte: add the project open in Resolve, one from Resolve's database, a
//! Blackmagic Cloud project by name, or a Linker-Edit project folder (with the folder picker).

use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::{ClientMessage, OrError, ProjectSource, ResolveLinkKind, ResolveStatus};

use crate::browse_path::trim_separators;
use crate::icons::IconName;
use crate::labels::library_label;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::dialog::{Dialog, caption, field, primary_button};
use crate::views::folder_picker::{FolderPicker, FolderPickerEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Open,
    Library,
    Cloud,
    Folder,
}

const TABS: [(Tab, &str, IconName); 4] = [
    (Tab::Open, "Open in Resolve", IconName::MonitorUp),
    (Tab::Library, "Database", IconName::HardDrive),
    (Tab::Cloud, "Cloud", IconName::Cloud),
    (Tab::Folder, "Folder", IconName::FolderInput),
];

pub struct DialogClosed;

pub struct AddProjectDialog {
    state: Entity<AppState>,
    focus: FocusHandle,
    tab: Tab,
    filter: Entity<InputState>,
    cloud_name: Entity<InputState>,
    folder: Option<Entity<FolderPicker>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DialogClosed> for AddProjectDialog {}

impl AddProjectDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let open = matches!(
            state.read(cx).app.as_ref().map(|app| &app.meta.resolve),
            Some(ResolveStatus::Connected {
                project: Some(_),
                ..
            })
        );
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter projects"));
        let cloud_name = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&cloud_name, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut dialog = AddProjectDialog {
            state,
            focus,
            tab: Tab::Open,
            filter,
            cloud_name,
            folder: None,
            _subscriptions: subscriptions,
        };
        // Starts where the likely project is; the user picks the tab after that.
        dialog.select(if open { Tab::Open } else { Tab::Library }, window, cx);
        dialog
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn select(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        match tab {
            // Asks Resolve for its projects each time the tab opens, so the list is current.
            Tab::Library => self.state.update(cx, |state, cx| state.load_library(cx)),
            Tab::Folder if self.folder.is_none() => {
                let state = self.state.clone();
                let picker =
                    cx.new(|cx| FolderPicker::new(state, "Project folder", false, window, cx));
                picker.update(cx, |picker, cx| picker.focus(window, cx));
                self._subscriptions.push(cx.subscribe(
                    &picker,
                    |this, _, event: &FolderPickerEvent, cx| match event {
                        FolderPickerEvent::Changed => cx.notify(),
                        FolderPickerEvent::Submit(_) => this.add_folder(cx),
                    },
                ));
                self.folder = Some(picker);
            }
            _ => {}
        }
        cx.notify();
    }

    fn add(&mut self, source: ProjectSource, cx: &mut Context<Self>) {
        self.state
            .read(cx)
            .send(ClientMessage::AddProject { source });
        cx.emit(DialogClosed);
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &self.folder else { return };
        let picker = picker.read(cx);
        if picker.status(cx).is_some_and(|status| status.is_project) {
            let dir = trim_separators(&picker.value(cx));
            self.add(ProjectSource::Folder { dir }, cx);
        }
    }

    fn render_tab(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let state = self.state.read(cx);
        let resolve = state.app.as_ref().map(|app| app.meta.resolve.clone());
        match self.tab {
            Tab::Open => match resolve {
                Some(ResolveStatus::Connected {
                    project: Some(project),
                    database,
                    ..
                }) => vec![
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded(theme.radius.lg)
                        .border_1()
                        .border_color(theme.border)
                        .p_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_token(TextSize::Sm)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(project),
                                )
                                .child(caption(
                                    database.map_or("Open in Resolve now".into(), |db| {
                                        format!("{} · {}", db.name, db.r#type)
                                    }),
                                    theme,
                                )),
                        )
                        .child(
                            primary_button(
                                "add-open",
                                "Add",
                                false,
                                theme,
                                cx.listener(|this, _, _, cx| this.add(ProjectSource::Open, cx)),
                            )
                            .test_support(),
                        )
                        .into_any_element(),
                ],
                _ => vec![
                    caption(
                        "Open a project in Resolve to add it here, or pick one from Resolve's database.",
                        theme,
                    )
                    .text_token(TextSize::Sm)
                    .into_any_element(),
                ],
            },
            Tab::Library => match &state.library {
                None => vec![
                    caption("Asking Resolve for its projects…", theme)
                        .py_6()
                        .text_center()
                        .into_any_element(),
                ],
                Some(OrError::Error { error }) => vec![
                    div()
                        .text_token(TextSize::Sm)
                        .text_color(theme.destructive)
                        .child(error.clone())
                        .into_any_element(),
                ],
                Some(OrError::Ok(library)) => {
                    let query = self.filter.read(cx).value().trim().to_lowercase();
                    let library = library.clone();
                    let accent = theme.accent;
                    let projects: Vec<AnyElement> = library
                        .projects
                        .iter()
                        .filter(|project| {
                            query.is_empty()
                                || format!("{}/{}", project.folder.join("/"), project.name)
                                    .to_lowercase()
                                    .contains(&query)
                        })
                        .enumerate()
                        .map(|(index, project)| {
                            let source = ProjectSource::Library {
                                name: project.name.clone(),
                                folder: project.folder.clone(),
                                database: library.database.clone(),
                            };
                            div()
                                .id(("library-project", index))
                                .flex()
                                .flex_col()
                                .w_full()
                                .rounded(theme.radius.md)
                                .px_2()
                                .py_1p5()
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.add(source.clone(), cx)
                                }))
                                .child(div().truncate().text_token(TextSize::Sm).child(project.name.clone()))
                                .when(!project.folder.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .truncate()
                                            .text_token(TextSize::X2s)
                                            .text_color(theme.muted_foreground)
                                            .child(project.folder.join(" / ")),
                                    )
                                })
                                .test_support()
                                .into_any_element()
                        })
                        .collect();
                    let others: Vec<String> = library
                        .databases
                        .iter()
                        .filter(|db| {
                            library.database.as_ref().is_none_or(|current| {
                                db.name != current.name
                                    || db.host.clone().unwrap_or_default()
                                        != current.host.clone().unwrap_or_default()
                            })
                        })
                        .map(|db| format!("\"{}\"", db.name))
                        .collect();
                    let from = match &library.database {
                        Some(db) => format!("From \"{}\".", db.name),
                        None => "From the database Resolve has open.".into(),
                    };
                    let note = if others.is_empty() {
                        from
                    } else {
                        format!(
                            "{from} Resolve also has {}: switch to it in Resolve's Project Manager to see its projects here.",
                            others.join(", ")
                        )
                    };
                    let empty = projects.is_empty();
                    vec![
                        field(&self.filter, window, cx).into_any_element(),
                        div()
                            .id("library-projects")
                            .mx(px(-4.))
                            .max_h(px(288.))
                            .overflow_y_scroll()
                            .children(projects)
                            .when(empty, |this| {
                                this.child(caption("No projects match.", theme).py_4().text_center())
                            })
                            .into_any_element(),
                        caption(note, theme).into_any_element(),
                    ]
                }
            },
            Tab::Cloud => {
                let name = self.cloud_name.read(cx).value().trim().to_string();
                let cloud = library_label(ResolveLinkKind::Cloud);
                vec![
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child("Project name, exactly as in Resolve")
                        .child(field(&self.cloud_name, window, cx).id("cloud-name").test_support())
                        .into_any_element(),
                    caption(
                        format!("You open {cloud} projects in Resolve's Project Manager yourself: {cloud}'s terms don't allow scripts to use it, so Linker-Edit never opens, creates or uploads cloud projects. If the project is open in Resolve now, add it from the first tab instead."),
                        theme,
                    )
                    .into_any_element(),
                    div()
                        .flex()
                        .justify_end()
                        .child(
                            primary_button(
                                "add-cloud",
                                "Add",
                                name.is_empty(),
                                theme,
                                cx.listener(move |this, _, _, cx| {
                                    this.add(ProjectSource::Cloud { name: name.clone() }, cx)
                                }),
                            )
                            .test_support(),
                        )
                        .into_any_element(),
                ]
            }
            Tab::Folder => {
                let Some(picker) = self.folder.clone() else {
                    return Vec::new();
                };
                let value = picker.read(cx).value(cx);
                let status = picker.read(cx).status(cx);
                let hint = if value.trim().is_empty() {
                    "Pick the folder of a Linker-Edit project."
                } else {
                    match status {
                        None => "Checking the folder…",
                        Some(status) if !status.exists => "There's no such folder.",
                        Some(status) if !status.is_project => {
                            "Go into a folder marked as a Linker-Edit project to add it."
                        }
                        Some(_) => "A Linker-Edit project folder, such as a teammate's edit history you cloned. It stays where it is.",
                    }
                };
                let ready = status.is_some_and(|status| status.is_project);
                vec![
                    picker.into_any_element(),
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(caption(hint, theme).flex_1().min_w_0())
                        .child(
                            primary_button(
                                "add-folder",
                                "Add",
                                !ready,
                                theme,
                                cx.listener(|this, _, _, cx| this.add_folder(cx)),
                            )
                            .test_support(),
                        )
                        .into_any_element(),
                ]
            }
        }
    }
}

impl Render for AddProjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let entity = cx.entity();
        let tabs = TABS
            .iter()
            .map(|(tab, label, icon)| {
                let tab = *tab;
                let selected = tab == self.tab;
                let foreground = theme.foreground;
                div()
                    .id(SharedString::from(format!("add-tab-{label}")))
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .h_7()
                    .rounded(theme.radius.md)
                    .text_token(TextSize::Xs)
                    .cursor_pointer()
                    .map(|this| {
                        if selected {
                            this.bg(theme.card).text_color(theme.foreground).shadow_sm()
                        } else {
                            this.text_color(theme.muted_foreground)
                                .hover(move |style| style.text_color(foreground))
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.select(tab, window, cx)))
                    .child(Icon::new(*icon).size(px(14.)))
                    .child(*label)
                    .test_support()
            })
            .collect::<Vec<_>>();
        let body = self.render_tab(&theme, window, cx);
        Dialog::new(
            "add-project-dialog",
            "Add project",
            &self.focus,
            move |_, cx| entity.update(cx, |_, cx| cx.emit(DialogClosed)),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .rounded(theme.radius.lg)
                .bg(theme.accent)
                .p_1()
                .children(tabs),
        )
        .children(body)
    }
}
