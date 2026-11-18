//! The sidebar's list: sidebar/SidebarThreadRow.svelte (T3 Code's SidebarThreadRow: "card" for
//! pinned threads, "folder" in a project folder, "slim" on a shelf), SidebarProject.svelte (the
//! folder header, with the green "open in Resolve" dot), SidebarShelfHeader.svelte,
//! SidebarSearchResult.svelte, and the list itself from Sidebar.svelte.
//!
//! What CSS shows on `group-hover` the rows show while [`Sidebar::hovered`] is theirs.

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, point,
    prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::base::input::Escape;
use gpui_kit::component::Icon;
use linker_edit_protocol::ProjectSummary;

use crate::icons::IconName;
use crate::labels::library_label;
use crate::sidebar::{
    FOLDER_PREVIEW_COUNT, SETTLED_PAGE_COUNT, SidebarThread, ThreadStatus, TopStatus,
    activity_timestamp, is_unread, relative_time, search_threads, settled_timestamp, should_recede,
    snooze_wake_label, thread_status, top_status,
};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::dialog::bare_input;
use crate::views::project_icon::ProjectIcon;
use crate::views::sidebar::{INSET, RowAction, Sidebar, Variant};

/// --sidebar-row-content-inset
const ROW_INSET: gpui::Pixels = px(10.);

fn status_look(status: TopStatus, theme: &Theme) -> (IconName, Hsla) {
    match status {
        TopStatus::Working => (IconName::CircleDashed, theme.info_foreground),
        TopStatus::Approval => (IconName::ShieldQuestion, theme.warning_foreground),
        TopStatus::Woke => (IconName::AlarmClock, theme.warning_foreground),
        TopStatus::Done => (IconName::CircleCheck, theme.success_foreground),
    }
}

impl Sidebar {
    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return div().flex_1().into_any_element();
        };
        let projects = app.meta.projects.clone();
        let open = state.thread_id().map(String::from);
        let scroll = div()
            .id("sidebar-threads")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(INSET)
            .pb(INSET);

        if self.searching(cx) {
            let query = self.query(cx);
            let results = search_threads(&projects, &query);
            if results.is_empty() {
                return scroll
                    .child(
                        div()
                            .id("no-threads-found")
                            .px_2()
                            .py_6()
                            .text_center()
                            .text_token(TextSize::Xs)
                            .text_color(theme.sidebar_muted_foreground)
                            .child("No threads found")
                            .test_support(),
                    )
                    .into_any_element();
            }
            let rows: Vec<AnyElement> = results
                .iter()
                .enumerate()
                .map(|(index, thread)| {
                    self.search_result(thread, index, open.as_deref(), &theme, cx)
                })
                .collect();
            return scroll
                .child(div().flex().flex_col().gap_px().children(rows))
                .into_any_element();
        }

        if projects.is_empty() {
            let (hover, foreground) = (theme.sidebar_row_hover, theme.sidebar_foreground);
            return scroll
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_6()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground.opacity(0.6))
                        .child("No projects yet")
                        .child(
                            div()
                                .id("sidebar-add-project")
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .px_2p5()
                                .py_1()
                                .rounded(theme.radius.md)
                                .border_1()
                                .border_color(theme.sidebar_border)
                                .text_token(TextSize::X2s)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.sidebar_muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover).text_color(foreground))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(super::sidebar::SidebarEvent::AddProject)
                                }))
                                .child(Icon::new(IconName::Plus).size(px(12.)))
                                .child("Add project")
                                .test_support(),
                        ),
                )
                .into_any_element();
        }

        let lists = self.lists(&projects);
        let order = lists.order.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        for thread in &lists.pinned {
            rows.push(self.thread_row(
                thread,
                Variant::Card,
                RowAction::Settle,
                open.as_deref(),
                &order,
                &theme,
                cx,
            ));
        }
        if !lists.pinned.is_empty() && !lists.folders.is_empty() {
            rows.push(div().h_1().into_any_element());
        }
        for (index, folder) in lists.folders.iter().enumerate() {
            let collapsed = self.collapsed.contains(&folder.project.id);
            rows.push(self.folder_header(
                folder.project,
                collapsed,
                if collapsed { &folder.threads } else { &[] },
                // `first:mt-0`: the list's first item, not the first folder after pinned threads.
                index == 0 && lists.pinned.is_empty(),
                &theme,
                cx,
            ));
            if collapsed {
                continue;
            }
            let (shown, hidden) =
                self.folder_rows(&folder.project.id, &folder.threads, open.as_deref());
            for thread in &shown {
                rows.push(self.thread_row(
                    thread,
                    Variant::Folder,
                    RowAction::Settle,
                    open.as_deref(),
                    &order,
                    &theme,
                    cx,
                ));
            }
            let expanded = self.expanded_folders.contains(&folder.project.id);
            if hidden > 0 || (expanded && folder.threads.len() > FOLDER_PREVIEW_COUNT) {
                let project = folder.project.id.clone();
                rows.push(
                    show_more(
                        SharedString::from(format!("sidebar-folder-more-{project}")),
                        hidden > 0,
                        if hidden > 0 {
                            format!("Show {hidden} more")
                        } else {
                            "Show less".into()
                        },
                        true,
                        &theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_show_all(&project, cx)))
                    .test_support()
                    .into_any_element(),
                );
            }
        }

        if !lists.snoozed.is_empty() {
            rows.push(self.shelf_header(
                "Snoozed",
                lists.snoozed.len(),
                self.snoozed_expanded,
                true,
                &theme,
                cx,
            ));
            if self.snoozed_expanded {
                for thread in &lists.snoozed {
                    rows.push(self.thread_row(
                        thread,
                        Variant::Slim,
                        RowAction::Unsnooze,
                        open.as_deref(),
                        &order,
                        &theme,
                        cx,
                    ));
                }
            }
        }
        if !lists.settled.is_empty() {
            rows.push(self.shelf_header(
                "Settled",
                lists.settled.len(),
                self.settled_expanded,
                lists.snoozed.is_empty(),
                &theme,
                cx,
            ));
            if self.settled_expanded {
                for thread in lists.settled.iter().take(self.settled_shown) {
                    rows.push(self.thread_row(
                        thread,
                        Variant::Slim,
                        RowAction::Unsettle,
                        open.as_deref(),
                        &order,
                        &theme,
                        cx,
                    ));
                }
                if lists.settled.len() > self.settled_shown {
                    let more = (lists.settled.len() - self.settled_shown).min(SETTLED_PAGE_COUNT);
                    rows.push(
                        show_more(
                            "settled-more",
                            true,
                            format!("Show {more} more"),
                            false,
                            &theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settled_shown += SETTLED_PAGE_COUNT;
                            cx.notify()
                        }))
                        .test_support()
                        .into_any_element(),
                    );
                }
            }
        }

        scroll
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_full()
                    .gap_px()
                    .children(rows),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn thread_row(
        &self,
        listed: &SidebarThread,
        variant: Variant,
        action: RowAction,
        open: Option<&str>,
        order: &[SidebarThread],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = listed.thread;
        let project = listed.project;
        let id = thread.id.clone();
        let active = open == Some(id.as_str());
        let hovered = self.hovered.as_deref() == Some(id.as_str());
        let status = top_status(thread, self.now);
        let recede = should_recede(thread, self.now, active);
        let pinned = thread.sidebar.pinned_at.is_some();
        let time = relative_time(
            if action == RowAction::Unsettle {
                settled_timestamp(thread)
            } else {
                activity_timestamp(thread)
            },
            self.now,
        );
        let jump = self.jump_label(order, &id);
        let renaming = self
            .renaming
            .as_ref()
            .filter(|(renamed, _)| *renamed == id)
            .map(|(_, input)| input.clone());
        let secondary = theme.muted_foreground;

        // The title, or the field renaming it.
        let title_color = match variant {
            Variant::Card | Variant::Folder if recede && !hovered => theme.foreground.opacity(0.75),
            Variant::Card | Variant::Folder => theme.foreground,
            Variant::Slim if hovered => theme.foreground,
            Variant::Slim if recede => secondary,
            Variant::Slim if active || status == Some(TopStatus::Woke) => theme.foreground,
            Variant::Slim if status == Some(TopStatus::Done) => theme.muted_foreground,
            Variant::Slim => secondary.opacity(0.7),
        };
        let title = match renaming {
            Some(input) => {
                let editing = id.clone();
                div()
                    .id("rename-thread")
                    .flex_1()
                    .min_w_0()
                    .mx(px(-4.))
                    .px_1()
                    .rounded(theme.radius.sm)
                    .bg(theme.card)
                    .border_1()
                    .border_color(theme.foreground)
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .capture_action(cx.listener(move |this, _: &Escape, _, cx| {
                        this.finish_rename(&editing, false, cx);
                        cx.stop_propagation();
                    }))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(bare_input(&input, TextSize::Sm))
                    .test_support()
                    .into_any_element()
            }
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_token(TextSize::Sm)
                .font_weight(if recede {
                    FontWeight::NORMAL
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(title_color)
                .child(thread.title.clone())
                .into_any_element(),
        };

        let pin_indicator = pinned.then(|| {
            let unpin = id.clone();
            let foreground = theme.foreground;
            div()
                .id(SharedString::from(format!("unpin-{id}")))
                .flex()
                .items_center()
                .rounded(theme.radius.sm)
                .text_color(theme.muted_foreground.opacity(0.65))
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Unpin thread").build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.pin(unpin.clone(), false, cx)
                }))
                .child(Icon::new(IconName::Pin).size(px(12.)))
                .test_support()
        });

        let jump_hint = jump.map(|label| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .h_5()
                .min_w_5()
                .px_1()
                .rounded(theme.radius.sm)
                .bg(theme.foreground.opacity(0.07))
                .text_token(TextSize::Xs)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(label)
        });

        let woke_button = |size: f32, cx: &mut Context<Self>| {
            let woke = id.clone();
            div()
                .id(SharedString::from(format!("dismiss-woke-{id}")))
                .flex()
                .items_center()
                .gap_1()
                .rounded(theme.radius.sm)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.warning_foreground)
                .cursor_pointer()
                .hover(|style| style.underline())
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Dismiss Woke notification")
                        .build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.acknowledge_woke(woke.clone(), cx)
                }))
                .child(Icon::new(IconName::AlarmClock).size(px(size)))
                .child("Woke")
                .test_support()
        };

        let surface = div()
            .id(SharedString::from(format!("thread-{id}")))
            .relative()
            .w_full()
            .overflow_hidden()
            .rounded(theme.radius.md)
            .cursor_pointer()
            .map(|this| {
                if active {
                    // `light:ring-1 ring-inset`: drawn over the row, so it doesn't move what's in it.
                    this.bg(theme.sidebar_row_active)
                        .text_color(theme.sidebar_foreground)
                        .when(!theme.dark, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(theme.radius.md)
                                    .border_1()
                                    .border_color(theme.sidebar_border.opacity(0.7)),
                            )
                        })
                } else if hovered {
                    this.bg(theme.sidebar_row_hover)
                        .text_color(theme.sidebar_foreground)
                } else if recede {
                    this.text_color(theme.sidebar_muted_foreground.opacity(0.75))
                } else {
                    this.text_color(theme.sidebar_foreground)
                }
            })
            .when(
                recede && status == Some(TopStatus::Working) && !hovered,
                |this| this.opacity(0.7),
            )
            .on_hover(cx.listener({
                let id = id.clone();
                move |this, hovered: &bool, _, cx| this.set_hovered(&id, *hovered, cx)
            }))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, event: &gpui::ClickEvent, window, cx| {
                    if event.click_count() == 2 {
                        let modifiers = event.modifiers();
                        if !modifiers.modified() && this.renaming.is_none() {
                            this.start_rename(id.clone(), window, cx);
                        }
                        return;
                    }
                    this.select(id.clone(), cx)
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let id = id.clone();
                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                        let Some(app) = this.state.read(cx).app.as_ref() else {
                            return;
                        };
                        let items = app
                            .meta
                            .projects
                            .iter()
                            .find_map(|project| {
                                project.threads.iter().find(|t| t.id == id).map(|thread| {
                                    this.thread_menu(&SidebarThread { thread, project })
                                })
                            })
                            .unwrap_or_default();
                        this.open_menu(items, event.position, false, window, cx);
                        cx.stop_propagation();
                    }
                }),
            );

        if variant == Variant::Card {
            // The status slot: the status or time; on hover, Snooze and Settle.
            let status_slot = if let Some(label) = jump_hint {
                label.into_any_element()
            } else if hovered && status != Some(TopStatus::Woke) {
                let snooze = id.clone();
                let settle = id.clone();
                let foreground = theme.foreground;
                div()
                    .flex()
                    .items_stretch()
                    .h_5()
                    .when(!thread.waiting, |this| {
                        this.child(
                            div()
                                .id(SharedString::from(format!("snooze-{id}")))
                                .flex()
                                .items_center()
                                .px_1p5()
                                .rounded(theme.radius.md)
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(foreground))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new("Snooze thread")
                                        .build(window, cx)
                                })
                                .on_click(cx.listener(
                                    move |this, event: &gpui::ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        let at = event.position();
                                        let items = this.snooze_items(&snooze);
                                        this.open_menu(
                                            items,
                                            point(at.x + px(8.), at.y + px(12.)),
                                            true,
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                                .child(Icon::new(IconName::Clock).size(px(12.)))
                                .test_support(),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("settle-{id}")))
                            .flex()
                            .items_center()
                            .gap_1()
                            .mr(px(-4.))
                            .px_1p5()
                            .rounded(theme.radius.md)
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(foreground))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Settle thread")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.settle(settle.clone(), true, cx)
                            }))
                            .child(Icon::new(IconName::Check).size(px(14.)))
                            .child("Settle")
                            .test_support(),
                    )
                    .into_any_element()
            } else {
                match status {
                    Some(TopStatus::Woke) => woke_button(16., cx).into_any_element(),
                    Some(top) => {
                        let (icon, color) = status_look(top, theme);
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color)
                            .child(Icon::new(icon).size(px(16.)))
                            .child(top.label())
                            .into_any_element()
                    }
                    None => div()
                        .text_color(secondary)
                        .child(time.clone())
                        .into_any_element(),
                }
            };
            return div()
                .py_0p5()
                .child(
                    surface
                        .child(
                            div()
                                .px(ROW_INSET)
                                .py(INSET)
                                .child(
                                    div()
                                        .flex()
                                        .h_5()
                                        .min_w_0()
                                        .items_center()
                                        .gap_1p5()
                                        .child(ProjectIcon::new(
                                            project.sidebar.icon.as_ref(),
                                            project.name.clone(),
                                            px(16.),
                                        ))
                                        .children(pin_indicator)
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_token(TextSize::Xs)
                                                .text_color(secondary)
                                                .child(project.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .ml_auto()
                                                .flex()
                                                .flex_shrink_0()
                                                .items_center()
                                                .justify_end()
                                                .h_5()
                                                .min_w_8()
                                                .text_token(TextSize::Xs)
                                                .child(status_slot),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .flex()
                                        .min_w_0()
                                        .items_center()
                                        .gap_1p5()
                                        .child(title),
                                ),
                        )
                        .test_support(),
                )
                .into_any_element();
        }

        let in_folder = variant == Variant::Folder;
        let woke = status == Some(TopStatus::Woke);
        let status_text = if action == RowAction::Unsnooze
            && let Some(until) = &thread.sidebar.snoozed_until
        {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_token(TextSize::Xs)
                .text_color(theme.info_foreground)
                .child(Icon::new(IconName::AlarmClock).size(px(12.)))
                .child(snooze_wake_label(until, self.now))
                .into_any_element()
        } else if woke {
            woke_button(12., cx)
                .text_token(TextSize::Xs)
                .into_any_element()
        } else if let (true, Some(top)) = (in_folder, status) {
            let (icon, color) = status_look(top, theme);
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_token(TextSize::Xs)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(Icon::new(icon).size(px(12.)))
                .child(top.label())
                .into_any_element()
        } else {
            div()
                .text_token(TextSize::Xs)
                .text_color(secondary)
                .child(time)
                .into_any_element()
        };
        let foreground = theme.foreground;
        let hover_action = {
            let target = id.clone();
            let (button_id, icon, size, label) = match action {
                RowAction::Unsnooze => ("wake", IconName::AlarmClockOff, 12., "Wake thread now"),
                RowAction::Unsettle => ("unsettle", IconName::Undo2, 14., "Un-settle thread"),
                RowAction::Settle => ("settle", IconName::Check, 12., "Settle thread"),
            };
            div()
                .id(SharedString::from(format!("{button_id}-{id}")))
                .flex()
                .items_center()
                .h_6()
                .px(if action == RowAction::Settle {
                    px(8.)
                } else {
                    px(6.)
                })
                .mr(if action == RowAction::Settle {
                    px(-8.)
                } else {
                    px(-4.)
                })
                .rounded(theme.radius.md)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    match action {
                        RowAction::Unsnooze => this.snooze(target.clone(), None, cx),
                        RowAction::Unsettle => this.settle(target.clone(), false, cx),
                        RowAction::Settle => this.settle(target.clone(), true, cx),
                    }
                }))
                .child(Icon::new(icon).size(px(size)))
                .test_support()
        };
        // On hover the action takes the status's place, except "Woke", which stays beside it.
        let slot = if let Some(label) = jump_hint {
            label.into_any_element()
        } else if hovered && woke && action == RowAction::Settle {
            div()
                .flex()
                .items_center()
                .child(status_text)
                .child(hover_action)
                .into_any_element()
        } else if hovered {
            hover_action.into_any_element()
        } else {
            status_text
        };
        let icon_dim = !hovered && (!active || action == RowAction::Unsettle);
        surface
            .flex()
            .items_center()
            .gap_2p5()
            .h_9()
            .px_2p5()
            .when(in_folder, |this| this.pl_9())
            .when(!in_folder, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .when(icon_dim, |this| this.opacity(0.4))
                        .child(ProjectIcon::new(
                            project.sidebar.icon.as_ref(),
                            project.name.clone(),
                            px(16.),
                        )),
                )
            })
            .child(title)
            .children(pin_indicator)
            .child(
                div()
                    .relative()
                    .ml_auto()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_end()
                    .h_6()
                    .min_w_8()
                    .child(slot),
            )
            .test_support()
            .into_any_element()
    }

    fn folder_header(
        &self,
        project: &ProjectSummary,
        collapsed: bool,
        hidden: &[SidebarThread],
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("project:{}", project.id);
        let hovered = self.hovered.as_deref() == Some(key.as_str());
        // The most urgent state among the hidden threads, so a closed folder still says when one needs you.
        let dot = if !collapsed {
            None
        } else if hidden
            .iter()
            .any(|t| thread_status(t.thread) == ThreadStatus::Approval)
        {
            Some(theme.warning)
        } else if hidden
            .iter()
            .any(|t| thread_status(t.thread) == ThreadStatus::Ready && is_unread(t.thread))
        {
            Some(theme.success)
        } else if hidden
            .iter()
            .any(|t| thread_status(t.thread) == ThreadStatus::Working)
        {
            Some(theme.info)
        } else {
            None
        };
        let tooltip = match &project.link {
            Some(link) => format!(
                "{} · {}\n{}",
                link.name,
                library_label(link.kind),
                project.dir
            ),
            None => format!("Not linked to a Resolve project yet\n{}", project.dir),
        };
        let id = project.id.clone();
        let hover_key = key.clone();
        let (accent, foreground) = (theme.accent, theme.foreground);
        div()
            .id(SharedString::from(format!("sidebar-project-{id}")))
            .relative()
            .when(!first, |this| this.mt_2())
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_hovered(&hover_key, *hovered, cx)
            }))
            .child(
                div()
                    .id(SharedString::from(format!("project-folder-{id}")))
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .h_8()
                    .w_full()
                    .px(ROW_INSET)
                    .rounded(theme.radius.control)
                    .cursor_pointer()
                    .when(hovered, |this| this.bg(theme.sidebar_row_hover))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, _, cx| this.toggle_folder(&id, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener({
                            let id = id.clone();
                            move |this, event: &gpui::MouseDownEvent, window, cx| {
                                let Some(project) =
                                    this.state.read(cx).app.as_ref().and_then(|app| {
                                        app.meta.projects.iter().find(|p| p.id == id)
                                    })
                                else {
                                    return;
                                };
                                let items = this.project_menu(project);
                                this.open_menu(items, event.position, false, window, cx);
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .child(ProjectIcon::new(
                        project.sidebar.icon.as_ref(),
                        project.name.clone(),
                        px(16.),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_token(TextSize::Sm)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.sidebar_foreground.opacity(0.9))
                            .child(project.name.clone()),
                    )
                    .when(project.open, |this| {
                        this.child(
                            div()
                                .id(SharedString::from(format!("open-in-resolve-{id}")))
                                .size_1p5()
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(theme.success)
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new("Open in Resolve")
                                        .build(window, cx)
                                })
                                .test_support(),
                        )
                    })
                    .when_some(dot, |this, color| {
                        this.child(div().size_1p5().flex_shrink_0().rounded_full().bg(color))
                    })
                    .when(collapsed && !hidden.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_token(TextSize::Xs)
                                .text_color(theme.sidebar_muted_foreground)
                                .when(hovered, |this| this.invisible())
                                .child(hidden.len().to_string()),
                        )
                    })
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.))
                        .text_color(theme.sidebar_muted_foreground),
                    )
                    .test_support(),
            )
            .when(hovered, |this| {
                let name = project.name.clone();
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_6()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .id(SharedString::from(format!("new-thread-in-{id}")))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_6()
                                .rounded(theme.radius.md)
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent).text_color(foreground))
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(format!(
                                        "New thread in {name}"
                                    ))
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(super::sidebar::SidebarEvent::NewThread(id.clone()))
                                }))
                                .child(Icon::new(IconName::SquarePen).size(px(14.)))
                                .test_support(),
                        ),
                )
            })
            .into_any_element()
    }

    fn shelf_header(
        &self,
        label: &'static str,
        count: usize,
        expanded: bool,
        push_down: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .mx_0p5()
            .h_8()
            .flex_shrink_0()
            .when(push_down, |this| this.mt_auto())
            .child(
                div()
                    .id(SharedString::from(format!(
                        "sidebar-{}-shelf-toggle",
                        label.to_lowercase()
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_8()
                    .w_full()
                    .px_2()
                    .rounded(theme.radius.md)
                    .text_token(TextSize::Xs)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.sidebar_muted_foreground)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if label == "Snoozed" {
                            this.snoozed_expanded = !this.snoozed_expanded;
                        } else {
                            this.settled_expanded = !this.settled_expanded;
                        }
                        cx.notify()
                    }))
                    .child(div().min_w_0().truncate().child(label))
                    .child(div().flex_1().min_w_2())
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .child(count.to_string()),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.)),
                    )
                    .test_support(),
            )
            .into_any_element()
    }

    fn search_result(
        &self,
        listed: &SidebarThread,
        index: usize,
        open: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = listed.thread;
        let project = listed.project;
        let highlighted = index == self.active_result;
        let active = open == Some(thread.id.as_str());
        let key = format!("result:{}", thread.id);
        let hovered = self.hovered.as_deref() == Some(key.as_str());
        let parked = thread.sidebar.settled_at.is_some() || thread.archived;
        let lit = highlighted || active;
        let id = thread.id.clone();
        div()
            .id(("sidebar-search-result", index))
            .flex()
            .items_center()
            .gap_2p5()
            .min_h_9()
            .w_full()
            .px_2p5()
            .py_1()
            .rounded(theme.radius.md)
            .text_token(TextSize::Sm)
            .cursor_pointer()
            .map(|this| {
                if lit {
                    this.bg(theme.sidebar_row_active)
                        .text_color(theme.sidebar_foreground)
                } else if hovered {
                    this.bg(theme.sidebar_row_hover)
                        .text_color(theme.sidebar_foreground)
                } else {
                    this.text_color(theme.sidebar_muted_foreground.opacity(0.75))
                }
            })
            .on_hover(
                cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_hovered(&key, *hovered, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.active_result != index {
                    this.active_result = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _, window, cx| this.open_result(id.clone(), window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    let Some(app) = this.state.read(cx).app.as_ref() else {
                        return;
                    };
                    let items =
                        app.meta
                            .projects
                            .iter()
                            .find_map(|project| {
                                project.threads.iter().find(|t| t.id == id).map(|thread| {
                                    this.thread_menu(&SidebarThread { thread, project })
                                })
                            })
                            .unwrap_or_default();
                    this.open_menu(items, event.position, false, window, cx);
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .when(parked && !lit && !hovered, |this| this.opacity(0.4))
                    .child(ProjectIcon::new(
                        project.sidebar.icon.as_ref(),
                        project.name.clone(),
                        px(16.),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2p5()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(if parked {
                                FontWeight::NORMAL
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(if lit || hovered || !parked {
                                theme.foreground
                            } else {
                                theme.muted_foreground.opacity(0.7)
                            })
                            .child(thread.title.clone()),
                    )
                    .when(thread.archived, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground.opacity(0.7))
                                .child("Archived"),
                        )
                    })
                    .child(
                        div()
                            .w_8()
                            .flex_shrink_0()
                            .text_right()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground.opacity(0.55))
                            .child(relative_time(activity_timestamp(thread), self.now)),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

/// "Show N more" / "Show less" under a folder or the Settled shelf.
fn show_more(
    id: impl Into<gpui::ElementId>,
    more: bool,
    label: String,
    in_folder: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (hover, foreground) = (theme.sidebar_row_hover, theme.sidebar_foreground);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2p5()
        .h_9()
        .w_full()
        .px_2p5()
        .when(in_folder, |this| this.pl_9())
        .rounded(theme.radius.md)
        .text_token(TextSize::Sm)
        .text_color(theme.sidebar_muted_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover).text_color(foreground))
        .child(
            Icon::new(if more {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            })
            .size(px(16.)),
        )
        .child(label)
}
