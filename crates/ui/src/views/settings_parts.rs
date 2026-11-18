//! What the settings and library pages are made of: SettingsGroup.svelte, SettingsRow.svelte,
//! Switch.svelte and Segmented.svelte, the pages' frame (a nav with Back, a header, a scrolling
//! column), their buttons, and `confirm()`.

use futures::channel::oneshot;
use gpui::{
    AnyElement, App, BoxShadow, ClickEvent, Div, ElementId, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, PromptLevel, RenderOnce, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, hsla, point, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::components::OnClick;
use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::window::{no_drag, title_bar};

/// `ring-1 ring-border` and the like: a box shadow with spread, outside the box like CSS's.
pub fn ring(color: Hsla, width: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(width),
        inset: false,
    }
}

/// Tailwind's `shadow-sm shadow-black/10`.
pub fn shadow_sm() -> BoxShadow {
    BoxShadow {
        color: hsla(0., 0., 0., 0.1),
        offset: point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }
}

/// The browser's `confirm(message)`: the system's dialog with OK and Cancel. Answers true for OK.
pub fn confirm(message: &str, window: &mut Window, cx: &mut App) -> oneshot::Receiver<usize> {
    window.prompt(PromptLevel::Warning, message, None, &["OK", "Cancel"], cx)
}

/// `if (confirm(message)) then()`.
pub fn confirm_then(
    message: &str,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut App) + 'static,
) {
    let answer = confirm(message, window, cx);
    cx.spawn(async move |cx| {
        if answer.await == Ok(0) {
            cx.update(then);
        }
    })
    .detach();
}

/// `h-8 rounded-md border px-3 text-xs whitespace-nowrap hover:bg-accent`: the pages' button.
pub fn outline_button(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
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

/// `h-8 rounded-md bg-primary px-3 text-xs text-primary-foreground hover:opacity-90`.
pub fn primary_button(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .h_8()
        .px_3()
        .rounded(theme.radius.md)
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .text_token(TextSize::Xs)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|style| style.opacity(0.9))
}

/// `h-8 rounded-md px-3 text-xs hover:bg-accent`: Cancel.
pub fn ghost_button(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .h_8()
        .px_3()
        .rounded(theme.radius.md)
        .text_token(TextSize::Xs)
        .cursor_pointer()
        .hover(move |style| style.bg(accent))
}

/// `flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent
/// hover:text-foreground`, with a 14px icon.
pub fn icon_button(id: impl Into<ElementId>, icon: IconName, theme: &Theme) -> Stateful<Div> {
    let (accent, foreground) = (theme.accent, theme.foreground);
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
        .hover(move |style| style.bg(accent).text_color(foreground))
        .child(Icon::new(icon).size(px(14.)))
}

/// Takes the look and the clicks away from a button that can't be used now (`disabled`).
pub fn disable(button: Stateful<Div>, disabled: bool) -> Stateful<Div> {
    button.when(disabled, |this| this.opacity(0.5).cursor_default())
}

/// SettingsGroup.svelte: a card of rows under a heading.
pub fn group(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    rows: impl IntoIterator<Item = AnyElement>,
    theme: &Theme,
) -> Div {
    div()
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
                        .child(title.into()),
                )
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .mt_0p5()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(description),
                    )
                }),
        )
        .child(card(rows, theme))
}

/// `divide-y rounded-xl border bg-card`.
pub fn card(rows: impl IntoIterator<Item = AnyElement>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(theme.radius.xl)
        .border_1()
        .border_color(theme.border)
        .bg(theme.card)
        .children(divided(rows, theme))
}

/// Tailwind's `divide-y`: a line above every row but the first.
pub fn divided(
    rows: impl IntoIterator<Item = AnyElement>,
    theme: &Theme,
) -> impl Iterator<Item = Div> {
    let border = theme.border;
    rows.into_iter().enumerate().map(move |(index, row)| {
        div()
            .when(index > 0, |this| this.border_t_1().border_color(border))
            .child(row)
    })
}

/// SettingsRow.svelte: what it is on the left, its controls on the right.
#[derive(IntoElement)]
pub struct SettingsRow {
    id: Option<ElementId>,
    label: SharedString,
    description: Option<SharedString>,
    detail: Vec<AnyElement>,
    controls: Vec<AnyElement>,
}

impl SettingsRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        SettingsRow {
            id: None,
            label: label.into(),
            description: None,
            detail: Vec::new(),
            controls: Vec::new(),
        }
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Under the description: a path, a progress bar, a problem.
    pub fn detail(mut self, element: impl IntoElement) -> Self {
        self.detail.push(element.into_any_element());
        self
    }

    pub fn control(mut self, element: impl IntoElement) -> Self {
        self.controls.push(element.into_any_element());
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_6()
            .p_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_token(TextSize::Sm).child(self.label))
                    .when_some(self.description, |this, text| {
                        this.child(
                            div()
                                .mt_0p5()
                                .text_token(TextSize::Xs)
                                .text_color(theme.muted_foreground)
                                .child(text),
                        )
                    })
                    .children(self.detail),
            )
            .when(!self.controls.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .children(self.controls),
                )
            });
        match self.id {
            Some(id) => row.id(id).test_support().into_any_element(),
            None => row.into_any_element(),
        }
    }
}

/// What a switch does with its new value.
type OnToggle = Box<dyn Fn(bool, &mut Window, &mut App) + 'static>;

/// What a choice does with the option picked.
type OnPick = Box<dyn Fn(usize, &mut Window, &mut App) + 'static>;

/// Switch.svelte: `h-5 w-9 rounded-full`, primary when on, with a white knob.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    on_change: OnToggle,
}

impl Switch {
    pub fn new(
        id: impl Into<ElementId>,
        checked: bool,
        on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Switch {
            id: id.into(),
            checked,
            on_change: Box::new(on_change),
        }
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let checked = self.checked;
        let on_change = self.on_change;
        div()
            .id(self.id)
            .relative()
            .flex_shrink_0()
            .h_5()
            .w_9()
            .rounded_full()
            .cursor_pointer()
            .bg(if checked { theme.primary } else { theme.input })
            .on_click(move |_, window, cx| on_change(!checked, window, cx))
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if checked { 18. } else { 2. }))
                    .size_4()
                    .rounded_full()
                    .bg(gpui::white())
                    .shadow(vec![shadow_sm()]),
            )
            .test_support()
    }
}

/// Segmented.svelte: a few options, all in view. Each option's id is `<id>-<index>`.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    options: Vec<SharedString>,
    selected: usize,
    on_change: OnPick,
}

impl Segmented {
    pub fn new(
        id: impl Into<SharedString>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        selected: usize,
        on_change: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Segmented {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            on_change: Box::new(on_change),
        }
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let on_change = std::rc::Rc::new(self.on_change);
        let options = self.options.into_iter().enumerate().map(|(index, label)| {
            let on_change = on_change.clone();
            let selected = index == self.selected;
            let foreground = theme.foreground;
            div()
                .id(SharedString::from(format!("{}-{index}", self.id)))
                .flex()
                .items_center()
                .h_7()
                .px_3()
                .rounded(theme.radius.md)
                .text_token(TextSize::Xs)
                .cursor_pointer()
                .map(|this| {
                    if selected {
                        this.bg(theme.card)
                            .text_color(theme.foreground)
                            .shadow(vec![shadow_sm(), ring(theme.border, 1.)])
                    } else {
                        this.text_color(theme.muted_foreground)
                            .hover(move |style| style.text_color(foreground))
                            .on_click(move |_, window, cx| on_change(index, window, cx))
                    }
                })
                .child(label)
                .test_support()
        });
        div()
            .id(ElementId::from(self.id.clone()))
            .flex()
            .flex_shrink_0()
            .rounded(theme.radius.lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted)
            .p_0p5()
            .children(options)
            .test_support()
    }
}

/// `mt-2 h-1 rounded-full bg-border` with the done part in primary; None while it can't say
/// (`w-1/3 animate-pulse`, drawn as a third, still).
pub fn progress_bar(percent: Option<f64>, theme: &Theme) -> Div {
    let fill = div().h_full().rounded_full().bg(theme.primary);
    div()
        .h_1()
        .w_full()
        .overflow_hidden()
        .rounded_full()
        .bg(theme.border)
        .child(match percent {
            Some(percent) => fill.w(gpui::relative((percent / 100.).clamp(0., 1.) as f32)),
            None => fill.w(gpui::relative(1. / 3.)).opacity(0.7),
        })
}

/// SettingsPage.svelte's `path` snippet: a path in mono, with a button that copies it.
#[derive(IntoElement)]
pub struct PathLine {
    id: ElementId,
    path: SharedString,
    copied: bool,
    on_copy: OnClick,
}

impl PathLine {
    pub fn new(
        id: impl Into<ElementId>,
        path: impl Into<SharedString>,
        copied: bool,
        on_copy: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        PathLine {
            id: id.into(),
            path: path.into(),
            copied,
            on_copy: Box::new(on_copy),
        }
    }
}

impl RenderOnce for PathLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (accent, foreground) = (theme.accent, theme.foreground);
        div()
            .mt_1()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family)
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .child(self.path),
            )
            .child(
                div()
                    .id(self.id)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size_5()
                    .rounded(theme.radius.sm)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent).text_color(foreground))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Copy path").build(window, cx)
                    })
                    .on_click(self.on_copy)
                    .child(
                        Icon::new(if self.copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        })
                        .size(px(12.)),
                    )
                    .test_support(),
            )
    }
}

/// A tab in the pages' nav: `flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-sm`,
/// `bg-sidebar-row-active font-medium shadow-[0_0_0_1px_var(--border)]` when it is the one open.
pub fn nav_tab(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let (accent, foreground) = (theme.accent, theme.foreground);
    div()
        .id(id)
        .flex()
        .w_full()
        .items_center()
        .gap_2p5()
        .h_8()
        .px_2p5()
        .rounded(theme.radius.lg)
        .text_token(TextSize::Sm)
        .whitespace_nowrap()
        .cursor_pointer()
        .map(|this| {
            if active {
                this.bg(theme.sidebar_row_active)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .shadow(vec![ring(theme.border, 1.)])
            } else {
                this.text_color(theme.muted_foreground)
                    .hover(move |style| style.bg(accent).text_color(foreground))
            }
        })
        .child(Icon::new(icon).size(px(16.)).flex_shrink_0())
        .child(label.into())
}

/// The settings and library pages' layout: a 240px nav with Back on the left, a header and a
/// scrolling column of `max_width` on the right. Both headers are the title bar. The caller
/// ends it with `.test_support()`, which changes its type in tests.
pub struct PageFrame {
    pub id: &'static str,
    pub on_back: OnClick,
    pub tabs: Vec<AnyElement>,
    pub footer: AnyElement,
    pub header: Vec<AnyElement>,
    pub content: Vec<AnyElement>,
    pub max_width: gpui::Pixels,
}

impl PageFrame {
    pub fn render(self, window: &mut Window, cx: &App) -> Stateful<Div> {
        let theme = Theme::of(cx);
        let (accent, foreground) = (theme.accent, theme.foreground);
        let back = no_drag().child(
            div()
                .id("back")
                .flex()
                .items_center()
                .gap_2()
                .h_8()
                .px_2p5()
                .rounded(theme.radius.lg)
                .text_token(TextSize::Sm)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(move |style| style.bg(accent).text_color(foreground))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Back (Esc)").build(window, cx)
                })
                .on_click(self.on_back)
                .child(Icon::new(IconName::ArrowLeft).size(px(16.)))
                .child("Back")
                .test_support(),
        );
        let nav = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(240.))
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                title_bar(
                    format!("{}-nav-header", self.id),
                    px(8.),
                    true,
                    false,
                    window,
                )
                .child(back),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_2()
                    .pb_2()
                    .children(self.tabs),
            )
            .child(
                div()
                    .mt_auto()
                    .px_4()
                    .py_3()
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .child(self.footer),
            );
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .child(
                title_bar(format!("{}-header", self.id), px(32.), false, true, window)
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .children(self.header),
            )
            .child(
                div()
                    .id(format!("{}-scroll", self.id))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap_8()
                            .w_full()
                            .max_w(self.max_width)
                            .px_8()
                            .py_8()
                            .children(self.content),
                    )
                    .test_support(),
            );
        div()
            .id(self.id)
            .flex()
            .flex_row()
            .size_full()
            .bg(theme.background)
            .child(nav)
            .child(main)
    }
}

/// Text with its https links made clickable (RemoteAccessSettings.svelte's `parts`), as
/// Tailscale's hints often carry one. The parts in order, each with whether it is a link.
pub fn link_parts(text: &str) -> Vec<(&str, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("https://") {
        let run = &rest[start..];
        let run_end = run.find(char::is_whitespace).unwrap_or(run.len());
        // `\S+[^\s.,)]`: the run without what trails it, and more than the scheme.
        let link = run[..run_end].trim_end_matches(['.', ',', ')']);
        if link.len() < "https://".len() + 2 {
            parts.push((&rest[..start + "https://".len()], false));
            rest = &rest[start + "https://".len()..];
            continue;
        }
        parts.push((&rest[..start], false));
        parts.push((link, true));
        rest = &rest[start + link.len()..];
    }
    parts.push((rest, false));
    parts.retain(|(part, _)| !part.is_empty());
    parts
}

/// `link_parts` drawn: links in primary, underlined, opening in the browser.
pub fn linked_text(id: impl Into<ElementId>, text: &str, theme: &Theme) -> gpui::InteractiveText {
    let mut ranges = Vec::new();
    let mut urls = Vec::new();
    let mut offset = 0;
    for (part, link) in link_parts(text) {
        if link {
            ranges.push(offset..offset + part.len());
            urls.push(part.to_string());
        }
        offset += part.len();
    }
    let style = gpui::HighlightStyle {
        color: Some(theme.primary),
        underline: Some(gpui::UnderlineStyle {
            thickness: px(1.),
            color: Some(theme.primary),
            wavy: false,
        }),
        ..Default::default()
    };
    let styled = gpui::StyledText::new(text.to_string())
        .with_highlights(ranges.iter().map(|range| (range.clone(), style)));
    gpui::InteractiveText::new(id, styled)
        .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index]))
}

/// An ISO date as `toLocaleString(…, { dateStyle: "medium", timeStyle: "short" })` writes it in
/// English: "Nov 18, 2026, 3:04 PM", in this computer's time zone. The text itself when it
/// isn't a date.
pub fn medium_date_time(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d, %Y, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| iso.to_string())
}

/// `before<span class="text-foreground">strong</span>after`, flowing as one paragraph.
pub fn emphasized(before: &str, strong: &str, after: &str, color: Hsla) -> gpui::StyledText {
    let start = before.len();
    let text = format!("{before}{strong}{after}");
    let style = gpui::HighlightStyle {
        color: Some(color),
        ..Default::default()
    };
    gpui::StyledText::new(text).with_highlights([(start..start + strong.len(), style)])
}

/// The header's title: `text-sm font-medium`.
pub fn page_title(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_token(TextSize::Sm)
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

#[cfg(test)]
mod tests {
    use super::link_parts;

    #[test]
    fn makes_links_of_https_addresses_without_what_trails_them() {
        assert_eq!(
            link_parts("Log in at https://login.tailscale.com/a/1b2c. Then refresh."),
            [
                ("Log in at ", false),
                ("https://login.tailscale.com/a/1b2c", true),
                (". Then refresh.", false)
            ]
        );
        assert_eq!(
            link_parts("(see https://tailscale.com)"),
            [
                ("(see ", false),
                ("https://tailscale.com", true),
                (")", false)
            ]
        );
        assert_eq!(link_parts("No link here"), [("No link here", false)]);
    }
}
