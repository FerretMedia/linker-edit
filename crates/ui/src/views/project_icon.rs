//! sidebar/ProjectIcon.svelte (T3 Code's ProjectFavicon and ProjectMonogram; MIT, see NOTICE) and
//! sidebar/IconPicker.svelte (T3 Code's ProjectIconPickerDialog): a project's icon, and the dialog
//! that picks a Lucide icon, an emoji or a monogram, and a color.
//!
//! Any Lucide icon can be a project's, as with @lucide/svelte's full set: they come from
//! gpui-kit's embedded Lucide catalog (`gpui_kit::assets::AllAssets`, see icons.rs).

use std::sync::OnceLock;

use gpui::{
    AppContext, AssetSource, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::ProjectIcon as Icon_;
use linker_edit_protocol::ProjectIconColor;
use linker_edit_protocol::project_icon::{derive_project_identity, first_emoji, monogram_text};

use crate::icons::IconName;
use crate::project_icon_colors::{
    POPULAR_PROJECT_ICONS, PROJECT_EMOJIS, PROJECT_ICON_COLOR_OPTIONS, color, swatch,
};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::dialog::{Dialog, caption, field, outline_button, primary_button};
use crate::views::settings_parts::Segmented;

/// Older Lucide names the picker offers, under their names in gpui-kit's newer catalog.
const ALIASES: [(&str, &str); 2] = [("globe-2", "earth"), ("layers-3", "layers")];

/// The asset path of a Lucide icon by its name; None when there is no such icon.
pub fn lucide_path(name: &str) -> Option<SharedString> {
    let ours = format!("le/icons/{name}.svg");
    if crate::icons::icon_svg(&ours).is_some() {
        return Some(ours.into());
    }
    let name = ALIASES
        .iter()
        .find(|(old, _)| *old == name)
        .map_or(name, |(_, new)| new);
    let theirs = format!("icons/{name}.svg");
    matches!(gpui_kit::assets::AllAssets.load(&theirs), Ok(Some(_))).then(|| theirs.into())
}

/// Every Lucide icon's name, for searching them all.
fn lucide_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<String> = gpui_kit::assets::AllAssets
            .list("icons/")
            .unwrap_or_default()
            .iter()
            .filter_map(|path| {
                path.strip_prefix("icons/")
                    .and_then(|name| name.strip_suffix(".svg"))
                    .map(String::from)
            })
            .collect();
        names.sort();
        names
    })
}

/// A project's icon at `size`; the monogram and color from its name when it has none.
#[derive(IntoElement)]
pub struct ProjectIcon {
    icon: Option<Icon_>,
    name: SharedString,
    size: Pixels,
}

impl ProjectIcon {
    pub fn new(icon: Option<&Icon_>, name: impl Into<SharedString>, size: Pixels) -> Self {
        ProjectIcon {
            icon: icon.cloned(),
            name: name.into(),
            size,
        }
    }
}

fn monogram(text: String, color: ProjectIconColor, size: Pixels, theme: &Theme) -> gpui::Div {
    let tint = self::color(color, theme);
    // The SVG's 8.25/16 font, stretched to 6 or 12 of its 16 units: about 60% of the box.
    let glyphs = text.chars().count().max(1) as f32;
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(size * 0.25)
        .bg(tint.opacity(0.14))
        .text_color(tint)
        .font_family(theme.mono_font_family)
        .font_weight(FontWeight::BOLD)
        .text_size(size * if glyphs > 1. { 0.56 } else { 0.6 })
        .line_height(size)
        .child(text)
}

impl RenderOnce for ProjectIcon {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let theme = Theme::of(cx);
        match self.icon {
            Some(Icon_::Monogram { text, color }) => monogram(text, color, self.size, theme),
            Some(Icon_::Emoji { emoji }) => div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(self.size)
                .text_size(self.size * 0.8)
                .line_height(self.size)
                .child(emoji),
            Some(Icon_::Lucide { name, color }) => {
                let path = lucide_path(&name).unwrap_or_else(|| IconName::FolderCode.path().into());
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size(self.size)
                    .child(
                        Icon::empty()
                            .path(path)
                            .size(self.size)
                            .text_color(self::color(color, theme)),
                    )
            }
            None => {
                let identity = derive_project_identity(&self.name);
                monogram(identity.monogram, identity.color, self.size, theme)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Lucide,
    Emoji,
    Monogram,
}

const MODES: [(Mode, &str); 3] = [
    (Mode::Lucide, "Icons"),
    (Mode::Emoji, "Emoji"),
    (Mode::Monogram, "Monogram"),
];

pub enum IconPickerEvent {
    Selected(Icon_),
    Close,
}

/// IconPicker.svelte.
pub struct IconPicker {
    focus: FocusHandle,
    project_name: String,
    mode: Mode,
    icon_name: String,
    color: ProjectIconColor,
    emoji: String,
    query: Entity<InputState>,
    letters: Entity<InputState>,
    custom_emoji: Entity<InputState>,
    automatic_monogram: String,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<IconPickerEvent> for IconPicker {}

impl IconPicker {
    pub fn new(
        current: Option<&Icon_>,
        project_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let automatic = derive_project_identity(project_name);
        let (mode, icon_name, color, letters, emoji) = match current {
            Some(Icon_::Lucide { name, color }) => (
                Mode::Lucide,
                name.clone(),
                *color,
                automatic.monogram.clone(),
                "🎬".into(),
            ),
            Some(Icon_::Monogram { text, color }) => (
                Mode::Monogram,
                "folder-code".into(),
                *color,
                text.clone(),
                "🎬".into(),
            ),
            Some(Icon_::Emoji { emoji }) => (
                Mode::Emoji,
                "folder-code".into(),
                automatic.color,
                automatic.monogram.clone(),
                emoji.clone(),
            ),
            None => (
                Mode::Lucide,
                "folder-code".into(),
                automatic.color,
                automatic.monogram.clone(),
                "🎬".into(),
            ),
        };
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search all Lucide icons"));
        let letters = cx.new(|cx| InputState::new(window, cx).default_value(letters));
        let custom_emoji = cx.new(|cx| InputState::new(window, cx).placeholder("Paste an emoji"));
        let subscriptions = vec![
            cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&letters, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&custom_emoji, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event
                    && let Some(next) = first_emoji(&input.read(cx).value())
                {
                    this.emoji = next;
                }
                cx.notify();
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        IconPicker {
            focus,
            project_name: project_name.into(),
            mode,
            icon_name,
            color,
            emoji,
            query,
            letters,
            custom_emoji,
            automatic_monogram: automatic.monogram,
            _subscriptions: subscriptions,
        }
    }

    fn monogram(&self, cx: &gpui::App) -> Option<String> {
        monogram_text(&self.letters.read(cx).value())
    }

    /// The icon as picked; None while the letters can't be a monogram.
    pub fn icon(&self, cx: &gpui::App) -> Option<Icon_> {
        Some(match self.mode {
            Mode::Monogram => Icon_::Monogram {
                text: self.monogram(cx)?,
                color: self.color,
            },
            Mode::Lucide => Icon_::Lucide {
                name: self.icon_name.clone(),
                color: self.color,
            },
            Mode::Emoji => Icon_::Emoji {
                emoji: self.emoji.clone(),
            },
        })
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(icon) = self.icon(cx) {
            cx.emit(IconPickerEvent::Selected(icon));
            cx.emit(IconPickerEvent::Close);
        }
    }

    fn icons(&self, cx: &gpui::App) -> Vec<String> {
        let query = self.query.read(cx).value().trim().to_lowercase();
        let normalized = query.split_whitespace().collect::<Vec<_>>().join("-");
        if normalized.is_empty() {
            return POPULAR_PROJECT_ICONS
                .iter()
                .filter(|name| lucide_path(name).is_some())
                .map(|name| name.to_string())
                .collect();
        }
        lucide_names()
            .iter()
            .filter(|name| name.contains(&normalized))
            .take(60)
            .cloned()
            .collect()
    }
}

/// "book-open" → "Book Open".
fn label(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tile(id: impl Into<gpui::ElementId>, pressed: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(40.))
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if pressed {
            theme.border
        } else {
            gpui::transparent_black()
        })
        .when(pressed, |this| this.bg(theme.accent))
        .cursor_pointer()
        .hover(move |style| style.bg(accent))
}

impl Render for IconPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let monogram = self.monogram(cx);
        let entity = cx.entity();
        let mut dialog = Dialog::new("icon-picker", "Choose project icon", &self.focus, {
            let entity = entity.clone();
            move |_, cx| entity.update(cx, |_, cx| cx.emit(IconPickerEvent::Close))
        })
        .child(
            caption("Choose an icon, emoji, or monogram.", &theme)
                .text_token(TextSize::Sm)
                .mt(px(-4.)),
        )
        .child(
            div().flex().child(Segmented::new(
                "icon-type",
                MODES.map(|(_, label)| label),
                MODES
                    .iter()
                    .position(|(mode, _)| *mode == self.mode)
                    .unwrap_or(0),
                {
                    let entity = entity.clone();
                    move |index, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.mode = MODES[index].0;
                            cx.notify()
                        })
                    }
                },
            )),
        );

        if self.mode != Mode::Emoji {
            let swatches = PROJECT_ICON_COLOR_OPTIONS.iter().map(|option| {
                let value = option.value;
                let pressed = value == self.color;
                div()
                    .id(SharedString::from(format!(
                        "color-{}",
                        option.label.to_lowercase()
                    )))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_6()
                    .rounded_full()
                    .border_1()
                    .border_color(if pressed {
                        theme.foreground.opacity(0.64)
                    } else {
                        gpui::transparent_black()
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.color = value;
                        cx.notify()
                    }))
                    .child(div().size_4().rounded_full().bg(swatch(value)))
                    .test_support()
            });
            dialog = dialog.child(
                div()
                    .child(
                        div()
                            .mb_2()
                            .text_token(TextSize::Xs)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground)
                            .child("Color"),
                    )
                    .child(div().flex().flex_wrap().gap_1p5().children(swatches)),
            );
        }

        dialog = match self.mode {
            Mode::Lucide => {
                let icons = self.icons(cx);
                let tint = color(self.color, &theme);
                let tiles = icons.iter().map(|name| {
                    let pressed = *name == self.icon_name;
                    let picked = name.clone();
                    let path = lucide_path(name).unwrap_or_default();
                    let tip = label(name);
                    tile(SharedString::from(format!("icon-{name}")), pressed, &theme)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.icon_name = picked.clone();
                            cx.notify()
                        }))
                        .child(Icon::empty().path(path).size(px(20.)).text_color(tint))
                        .test_support()
                });
                dialog
                    .child(field(&self.query, window, cx))
                    .child(
                        div()
                            .id("icon-grid")
                            .max_h(px(256.))
                            .overflow_y_scroll()
                            .child(div().grid().grid_cols(10).gap_1().p_0p5().children(tiles)),
                    )
                    .when(icons.is_empty(), |this| {
                        this.child(
                            caption("No icons found.", &theme)
                                .text_token(TextSize::Sm)
                                .py_8()
                                .text_center(),
                        )
                    })
            }
            Mode::Monogram => dialog.child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .py_2()
                    .child(ProjectIcon::new(
                        Some(&Icon_::Monogram {
                            text: monogram.clone().unwrap_or(self.automatic_monogram.clone()),
                            color: self.color,
                        }),
                        self.project_name.clone(),
                        px(48.),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_token(TextSize::Sm)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Letters"),
                            )
                            .child(
                                field(&self.letters, window, cx)
                                    .id("monogram-letters")
                                    .test_support(),
                            )
                            .child(caption("One or two letters or numbers.", &theme)),
                    ),
            ),
            Mode::Emoji => {
                let tiles = PROJECT_EMOJIS
                    .iter()
                    .enumerate()
                    .map(|(index, (emoji, _))| {
                        let picked = emoji.to_string();
                        tile(("emoji", index), self.emoji == *emoji, &theme)
                            .text_size(px(20.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.emoji = picked.clone();
                                cx.notify()
                            }))
                            .child(*emoji)
                            .test_support()
                    });
                dialog
                    .child(
                        div()
                            .id("emoji-grid")
                            .max_h(px(256.))
                            .overflow_y_scroll()
                            .child(div().grid().grid_cols(10).gap_1().p_0p5().children(tiles)),
                    )
                    .child(
                        div()
                            .child(
                                div()
                                    .mb_2()
                                    .text_token(TextSize::Xs)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_foreground)
                                    .child("Or paste any emoji"),
                            )
                            .child(field(&self.custom_emoji, window, cx)),
                    )
            }
        };

        let can_save = !(self.mode == Mode::Monogram && monogram.is_none());
        dialog.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_color(theme.border)
                .pt_4()
                .child(outline_button(
                    "icon-cancel",
                    "Cancel",
                    &theme,
                    cx.listener(|_, _, _, cx| cx.emit(IconPickerEvent::Close)),
                ))
                .child(
                    primary_button(
                        "icon-save",
                        "Save icon",
                        !can_save,
                        &theme,
                        cx.listener(|this, _, _, cx| this.save(cx)),
                    )
                    .test_support(),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lucide_icons_by_name_and_alias() {
        assert!(lucide_path("rocket").is_some());
        assert!(lucide_path("shopping-bag").is_some());
        assert!(lucide_path("globe-2").is_some());
        assert!(lucide_path("not-an-icon").is_none());
        assert!(lucide_names().len() > 1000);
        assert_eq!(label("book-open"), "Book Open");
    }
}
