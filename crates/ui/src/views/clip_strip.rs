//! ClipStrip.svelte: one timeline's clips in a save, as tiles. A changed clip has a ring in its
//! change's color (labels.ts's CHANGE_STYLE), a removed one a cross, an added one a plus, and a
//! moved or trimmed one its seconds. Untouched runs fold into "+N". Thumbnails come from the
//! server's /media/<id>.

use gpui::{
    App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    Window, div, img, prelude::FluentBuilder, px, rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use linker_edit_protocol::{ClipChange, StripTile, TimelineStrip, TimelineStripState};

use crate::icons::IconName;
use crate::library::encode_uri_component;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::settings_parts::ring;

/// The changes in the order the legend shows them (CHANGE_STYLE's).
pub const CHANGES: [ClipChange; 5] = [
    ClipChange::Added,
    ClipChange::Moved,
    ClipChange::Trimmed,
    ClipChange::Changed,
    ClipChange::Removed,
];

pub fn change_name(change: ClipChange) -> &'static str {
    match change {
        ClipChange::Added => "added",
        ClipChange::Moved => "moved",
        ClipChange::Trimmed => "trimmed",
        ClipChange::Changed => "changed",
        ClipChange::Removed => "removed",
    }
}

/// CHANGE_STYLE: a change's ring and dot.
pub fn change_color(change: ClipChange, theme: &Theme) -> Hsla {
    match change {
        ClipChange::Added => theme.success,
        ClipChange::Moved => theme.primary,
        ClipChange::Trimmed => theme.warning,
        // violet-500
        ClipChange::Changed => rgb(0x8e51ff).into(),
        ClipChange::Removed => theme.destructive,
    }
}

/// "+2s" or "−1.5s", with a real minus, as the tiles write a delta.
pub fn signed(seconds: f64) -> String {
    format!("{}{}s", if seconds > 0. { "+" } else { "−" }, seconds.abs())
}

#[derive(IntoElement)]
pub struct ClipStrip {
    id: ElementId,
    strip: TimelineStrip,
    detail: Option<String>,
    /// The server's address for /media/<id>; None draws every tile as a film icon.
    media_url: Option<String>,
    on_preview: crate::components::OnClick,
}

impl ClipStrip {
    pub fn new(
        id: impl Into<ElementId>,
        strip: TimelineStrip,
        detail: Option<String>,
        media_url: Option<String>,
        on_preview: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        ClipStrip {
            id: id.into(),
            strip,
            detail,
            media_url,
            on_preview: Box::new(on_preview),
        }
    }
}

impl RenderOnce for ClipStrip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let (accent, foreground) = (theme.accent, theme.foreground);
        let name = self.strip.name.clone();
        let header = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .text_token(TextSize::Xs)
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.strip.name.clone()),
            )
            .when(self.strip.state == TimelineStripState::New, |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .rounded(theme.radius.sm)
                        .bg(theme.success.opacity(0.15))
                        .px_1()
                        .text_token(TextSize::X2s)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.success)
                        .child("new"),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!("play-{}", self.strip.id)))
                    .ml_auto()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .h_5()
                    .px_1()
                    .rounded(theme.radius.sm)
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent).text_color(foreground))
                    .tooltip(|window, cx| {
                        Tooltip::new("Cut preview of this version").build(window, cx)
                    })
                    .on_click(self.on_preview)
                    .child(Icon::new(IconName::PlayFilled).size(px(10.)))
                    .child("Play")
                    .test_support(),
            );

        let tiles = self.strip.tiles.iter().enumerate().map(|(index, tile)| {
            let id = SharedString::from(format!("{}-tile-{index}", name));
            match tile {
                StripTile::More { count } => {
                    let tip = format!(
                        "{count} unchanged {}",
                        if *count == 1 { "clip" } else { "clips" }
                    );
                    div()
                        .id(id)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_10()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_dashed()
                        .border_color(theme.border)
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .child(format!("+{count}"))
                        .into_any_element()
                }
                StripTile::Clip {
                    name,
                    media,
                    image,
                    change,
                    delta,
                } => {
                    let removed = *change == Some(ClipChange::Removed);
                    let tip = format!(
                        "{}{}{}",
                        if name.is_empty() { "Untitled" } else { name },
                        change
                            .map(|c| format!(" · {}", change_name(c)))
                            .unwrap_or_default(),
                        delta
                            .filter(|d| *d != 0.)
                            .map(|d| format!(" {}", signed(d)))
                            .unwrap_or_default()
                    );
                    let picture = match (image, media, &self.media_url) {
                        (true, Some(media), Some(base)) => {
                            img(format!("{base}/media/{}", encode_uri_component(media)))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .when(removed, |this| this.opacity(0.35).grayscale(true))
                                .into_any_element()
                        }
                        _ => div()
                            .flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .when(removed, |this| this.opacity(0.35))
                            .child(
                                Icon::new(IconName::Film)
                                    .size(px(16.))
                                    .text_color(theme.muted_foreground),
                            )
                            .into_any_element(),
                    };
                    div()
                        .id(id)
                        .relative()
                        .flex_shrink_0()
                        .size_10()
                        .overflow_hidden()
                        .rounded(theme.radius.md)
                        .bg(theme.card)
                        .map(|this| match change {
                            Some(change) => {
                                this.shadow(vec![ring(change_color(*change, &theme), 2.)])
                            }
                            None => this.border_1().border_color(theme.border),
                        })
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .child(picture)
                        .when(removed, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(theme.destructive)
                                    .child(Icon::new(IconName::XBold).size(px(20.))),
                            )
                        })
                        .when(*change == Some(ClipChange::Added), |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_0p5()
                                    .right_0p5()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size_3p5()
                                    .rounded_full()
                                    .bg(theme.success)
                                    .text_color(gpui::white())
                                    .child(Icon::new(IconName::PlusBold).size(px(10.))),
                            )
                        })
                        .when_some(delta.filter(|d| *d != 0.), |this, delta| {
                            this.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .bg(gpui::black().opacity(0.7))
                                    .text_center()
                                    .text_size(px(9.))
                                    .line_height(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(gpui::white())
                                    .child(signed(delta)),
                            )
                        })
                        .test_support()
                        .into_any_element()
                }
            }
        });

        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap_1p5()
            .child(header)
            .child(div().flex().flex_wrap().gap_1p5().p_0p5().children(tiles))
            .when_some(self.detail, |this, detail| {
                this.child(
                    div()
                        .text_token(TextSize::X2s)
                        .line_height(px(11. * 1.375))
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
            })
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::signed;

    #[test]
    fn writes_a_delta_with_its_sign() {
        assert_eq!(signed(2.), "+2s");
        assert_eq!(signed(-1.5), "−1.5s");
    }
}
