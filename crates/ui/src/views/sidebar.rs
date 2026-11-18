//! The sidebar (Sidebar.svelte): logo and version, the computer picker, search with
//! "Add project" and "New thread", the projects and their threads, and the footer with
//! Settings and Resolve's status. 256px wide, like its `w-64`; not resizable there either.
//! Placeholder content until the protocol's projects arrive.

use gpui::{
    Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::components::LinkerLogo;
use crate::icons::IconName;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::window::title_bar;

pub const SIDEBAR_WIDTH: gpui::Pixels = px(256.);

pub struct Sidebar {
    state: Entity<AppState>,
}

pub enum SidebarEvent {
    /// Opens the settings page at a section (Settings, or Remote access from the computer menu).
    Settings(crate::settings::SettingsSection),
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Sidebar { state }
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let _state = self.state.read(cx);
        let inset = px(8.); // --sidebar-content-inset

        let header = title_bar("sidebar-header", px(16.), true, false, window)
            .gap_2()
            .child(LinkerLogo::new(px(20.)))
            .child(
                div()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child("Linker-Edit"),
            )
            .child(
                div()
                    .ml_auto()
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .child("dev"),
            );

        // ComputerPicker.svelte's closed state.
        let computer = div().mx_2().mb_1().child(
            div()
                .id("computer-picker")
                .flex()
                .items_center()
                .gap_2()
                .h_8()
                .px_2p5()
                .rounded(theme.radius.lg)
                .text_token(TextSize::Xs)
                .hover(|style| style.bg(theme.accent))
                .child(
                    Icon::new(IconName::Monitor)
                        .size(px(14.))
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child("This computer"),
                )
                .child(
                    Icon::new(IconName::ChevronsUpDown)
                        .size(px(14.))
                        .text_color(theme.muted_foreground),
                ),
        );

        let icon_button = |id: &'static str, icon: IconName| {
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size_7()
                .rounded(theme.radius.control)
                .hover(|style| style.bg(theme.sidebar_row_hover))
                .child(Icon::new(icon).size(px(16.)).text_color(theme.sidebar_icon))
        };

        // SidebarSearch.svelte.
        let search = div()
            .flex()
            .items_center()
            .gap_1()
            .p(inset)
            .child(
                div()
                    .id("sidebar-search")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .h_8()
                    .px_2()
                    .rounded(theme.radius.md)
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.sidebar_muted_foreground)
                    .hover(|style| style.bg(theme.sidebar_row_hover))
                    .child(
                        Icon::new(IconName::Search)
                            .size(px(16.))
                            .text_color(theme.sidebar_icon),
                    )
                    .child("Search"),
            )
            .child(icon_button("add-project", IconName::FolderPlus))
            .child(icon_button("new-thread", IconName::SquarePen));

        // The empty list, as Sidebar.svelte shows it with no projects yet.
        let threads = div()
            .id("sidebar-threads")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(inset)
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
                            .hover(|style| {
                                style
                                    .bg(theme.sidebar_row_hover)
                                    .text_color(theme.sidebar_foreground)
                            })
                            .child(Icon::new(IconName::Plus).size(px(12.)))
                            .child("Add project"),
                    ),
            );

        // SidebarFooter.svelte: Settings and Resolve's status.
        let footer = div()
            .flex()
            .items_start()
            .gap_1()
            .px(inset)
            .pt_1()
            .pb(inset)
            .child(
                icon_button("settings", IconName::Settings)
                    .size_8()
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(SidebarEvent::Settings(
                            crate::settings::SettingsSection::General,
                        ))
                    }))
                    .test_support(),
            )
            .child(
                div()
                    .id("resolve-status")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .gap_2()
                    .min_h_8()
                    .px_2()
                    .py_1p5()
                    .rounded(theme.radius.control)
                    .hover(|style| style.bg(theme.sidebar_row_hover))
                    .child(
                        div()
                            .mt_1p5()
                            .size_2()
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(theme.muted_foreground.opacity(0.4)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_token(TextSize::Xs)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Resolve"),
                            )
                            .child(
                                div()
                                    .mt_0p5()
                                    .text_token(TextSize::X2s)
                                    .text_color(theme.muted_foreground)
                                    .child("Waiting for the server"),
                            ),
                    ),
            );

        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(header)
            .child(computer)
            .child(search)
            .child(threads)
            .child(footer)
            .test_support()
    }
}
