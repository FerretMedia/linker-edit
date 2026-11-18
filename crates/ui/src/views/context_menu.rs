//! sidebar/ContextMenu.svelte: T3 Code's thread and project menus. Opens at the pointer (or with
//! its right edge there, for a button's menu), kept on screen; an item with children opens a
//! submenu beside it on hover or click. ↑/↓ move through the rows (a submenu's included),
//! Enter picks, → opens a submenu, Escape or a click outside closes it.
//!
//! The menu holds data, not callbacks: picking a row emits its action for the owner to run.

use gpui::{
    AnyElement, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};

/// One row of a menu.
#[derive(Clone)]
pub struct MenuItem<A> {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<IconName>,
    /// Shown on the right, like a snooze preset's wake time.
    pub detail: Option<SharedString>,
    pub destructive: bool,
    pub disabled: bool,
    pub separator_before: bool,
    pub children: Vec<MenuItem<A>>,
    pub action: Option<A>,
}

impl<A> MenuItem<A> {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, action: A) -> Self {
        MenuItem {
            id: id.into(),
            label: label.into(),
            icon: None,
            detail: None,
            destructive: false,
            disabled: false,
            separator_before: false,
            children: Vec::new(),
            action: Some(action),
        }
    }

    pub fn submenu(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        children: Vec<MenuItem<A>>,
    ) -> Self {
        MenuItem {
            id: id.into(),
            label: label.into(),
            icon: None,
            detail: None,
            destructive: false,
            disabled: false,
            separator_before: false,
            children,
            action: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn separator(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

pub enum MenuEvent<A> {
    Select(A),
    Close,
}

/// Where a row is: its index, and its index in the open submenu.
type Path = (usize, Option<usize>);

pub struct ContextMenu<A: Clone + 'static> {
    items: Vec<MenuItem<A>>,
    position: Point<Pixels>,
    /// The menu's right edge at `position`, as for a button's menu.
    align_end: bool,
    focus: FocusHandle,
    open_child: Option<usize>,
    highlighted: Option<Path>,
    /// The pointer is over the submenu, which sits outside the menu's own bounds.
    in_submenu: bool,
}

impl<A: Clone + 'static> EventEmitter<MenuEvent<A>> for ContextMenu<A> {}

impl<A: Clone + 'static> ContextMenu<A> {
    pub fn new(
        items: Vec<MenuItem<A>>,
        position: Point<Pixels>,
        align_end: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut menu = ContextMenu {
            items,
            position,
            align_end,
            focus,
            open_child: None,
            highlighted: None,
            in_submenu: false,
        };
        menu.highlighted = menu.rows().first().copied();
        menu
    }

    pub fn items(&self) -> &[MenuItem<A>] {
        &self.items
    }

    /// The rows the arrow keys go through, in order: enabled ones, the open submenu's after its item.
    fn rows(&self) -> Vec<Path> {
        let mut rows = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            if !item.disabled {
                rows.push((index, None));
            }
            if self.open_child == Some(index) {
                rows.extend(
                    item.children
                        .iter()
                        .enumerate()
                        .filter(|(_, child)| !child.disabled)
                        .map(|(child, _)| (index, Some(child))),
                );
            }
        }
        rows
    }

    fn item(&self, path: Path) -> Option<&MenuItem<A>> {
        let item = self.items.get(path.0)?;
        match path.1 {
            Some(child) => item.children.get(child),
            None => Some(item),
        }
    }

    fn choose(&mut self, path: Path, cx: &mut Context<Self>) {
        let Some(item) = self.item(path) else { return };
        if item.disabled {
            return;
        }
        if !item.children.is_empty() {
            self.open_child = Some(path.0);
            cx.notify();
            return;
        }
        // Select first: the owner may drop the menu, and its subscription, on Close.
        if let Some(action) = item.action.clone() {
            cx.emit(MenuEvent::Select(action));
        }
        cx.emit(MenuEvent::Close);
    }

    fn hover(&mut self, path: Path, cx: &mut Context<Self>) {
        if path.1.is_none() {
            let item = &self.items[path.0];
            self.open_child = (!item.children.is_empty() && !item.disabled).then_some(path.0);
        }
        if self.item(path).is_some_and(|item| !item.disabled) {
            self.highlighted = Some(path);
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(MenuEvent::Close),
            "up" | "down" => {
                let rows = self.rows();
                if rows.is_empty() {
                    return;
                }
                let at = self
                    .highlighted
                    .and_then(|path| rows.iter().position(|row| *row == path));
                let next = match (at, event.keystroke.key.as_str()) {
                    (None, "down") => 0,
                    (None, _) => rows.len() - 1,
                    (Some(at), "down") => (at + 1) % rows.len(),
                    (Some(at), _) => (at + rows.len() - 1) % rows.len(),
                };
                self.highlighted = Some(rows[next]);
                cx.notify();
            }
            "enter" | "space" => {
                if let Some(path) = self.highlighted {
                    self.choose(path, cx);
                }
            }
            "right" => {
                if let Some(path @ (_, None)) = self.highlighted
                    && self
                        .item(path)
                        .is_some_and(|item| !item.children.is_empty())
                {
                    self.choose(path, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_items(
        &self,
        items: &[MenuItem<A>],
        parent: Option<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let path: Path = match parent {
                Some(parent) => (parent, Some(index)),
                None => (index, None),
            };
            if item.separator_before {
                rows.push(
                    div()
                        .mx_2()
                        .my_1()
                        .h_px()
                        .bg(theme.border)
                        .into_any_element(),
                );
            }
            let highlighted = self.highlighted == Some(path)
                || (parent.is_none() && self.open_child == Some(index));
            let icon_color = if item.destructive {
                theme.destructive
            } else {
                theme.muted_foreground.opacity(0.8)
            };
            let submenu = (parent.is_none() && self.open_child == Some(index)).then(|| {
                let children = self.render_items(&item.children, Some(index), theme, cx);
                div()
                    .absolute()
                    .top(px(-5.))
                    .left_full()
                    .ml(px(2.))
                    .child(deferred(
                        anchored().snap_to_window_with_margin(px(8.)).child(
                            menu_box(theme)
                                .id("context-submenu")
                                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                    this.in_submenu = *hovered
                                }))
                                .children(children),
                        ),
                    ))
            });
            rows.push(
                div()
                    .id(SharedString::from(format!("menu-{}", item.id)))
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_h_7()
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius.sm)
                    .text_token(TextSize::Sm)
                    .whitespace_nowrap()
                    .text_color(if item.destructive {
                        theme.destructive
                    } else {
                        theme.foreground
                    })
                    .when(highlighted, |this| this.bg(theme.accent))
                    .map(|this| {
                        if item.disabled {
                            this.opacity(0.64)
                        } else {
                            this.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| this.choose(path, cx)))
                        }
                    })
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hover(path, cx)
                        }
                    }))
                    .when_some(item.icon, |this, icon| {
                        this.child(
                            div()
                                .mx(px(-2.))
                                .flex_shrink_0()
                                .child(Icon::new(icon).size(px(16.)).text_color(icon_color)),
                        )
                    })
                    .child(div().flex_1().min_w_0().child(item.label.clone()))
                    .when_some(item.detail.clone(), |this, detail| {
                        this.child(
                            div()
                                .ml_auto()
                                .flex_shrink_0()
                                .pl_4()
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        )
                    })
                    .when(!item.children.is_empty(), |this| {
                        this.child(
                            div().ml_auto().mr(px(-2.)).flex_shrink_0().child(
                                Icon::new(IconName::ChevronRight)
                                    .size(px(16.))
                                    .text_color(theme.foreground.opacity(0.8)),
                            ),
                        )
                    })
                    .children(submenu)
                    .test_support()
                    .into_any_element(),
            );
        }
        rows
    }
}

/// The menu's box: `rounded-lg border border-foreground/10 bg-popover p-1` with T3 Code's shadow.
fn menu_box(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .min_w(px(160.))
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.foreground.opacity(0.1))
        .bg(theme.popover)
        .p_1()
        .shadow_lg()
        .occlude()
}

impl<A: Clone + 'static> Render for ContextMenu<A> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let items = self.items.clone();
        let rows = self.render_items(&items, None, &theme, cx);
        deferred(
            anchored()
                .position(self.position)
                .anchor(if self.align_end {
                    gpui::Anchor::TopRight
                } else {
                    gpui::Anchor::TopLeft
                })
                .snap_to_window_with_margin(px(8.))
                .child(
                    menu_box(&theme)
                        .id("context-menu")
                        .track_focus(&self.focus)
                        .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if !this.in_submenu {
                                cx.emit(MenuEvent::Close)
                            }
                        }))
                        .children(rows)
                        .test_support(),
                ),
        )
        .with_priority(1)
    }
}
