//! TimelinePlayer.svelte: the picture of a timeline, the clip on each track stacked in track
//! order, each with its look. The playing is [`crate::media::player::Player`]'s; this draws what
//! it hands back. A clip that can't play shows what it is and why instead.

use std::sync::Arc;

use gpui::{
    App, Corners, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, RenderImage,
    RenderOnce, Styled, Window, canvas, div, px, rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::MediaKind;

use super::player_controls::white;
use crate::icons::IconName;
use crate::media::player::Picture;
use crate::theme::{TextSize, TextStyled};

#[derive(IntoElement)]
pub struct TimelinePlayer {
    pictures: Vec<Picture>,
    /// The stage's top corners: GPUI clips to rectangles, so the picture rounds its own.
    corner: Pixels,
}

impl TimelinePlayer {
    pub fn new(pictures: Vec<Picture>, corner: Pixels) -> Self {
        TimelinePlayer { pictures, corner }
    }
}

/// "MOV" for "a.mov", as the Svelte's `extension`.
fn extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) => {
            ext.to_uppercase()
        }
        _ => "This file".into(),
    }
}

/// A frame drawn over the whole stage (it is the stage's size, letterbox included).
fn frame(image: Arc<RenderImage>, corner: Pixels) -> impl IntoElement {
    let corners = Corners {
        top_left: corner,
        top_right: corner,
        ..Corners::default()
    };
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let _ = window.paint_image(bounds, bounds, corners, image, 0, false);
        },
    )
    .size_full()
}

impl RenderOnce for TimelinePlayer {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let corner = self.corner;
        let layers = self
            .pictures
            .into_iter()
            .enumerate()
            .map(|(index, picture)| {
                let clip = &picture.clip;
                let layer = div().absolute().top_0().left_0().size_full();
                if picture.playable && matches!(clip.kind, MediaKind::Image | MediaKind::Video) {
                    return layer
                        .id(("picture", index))
                        .opacity(picture.opacity)
                        .children(picture.image.map(|image| frame(image, corner)))
                        .test_support()
                        .into_any_element();
                }
                let why = if clip.kind == MediaKind::Missing || clip.media.is_none() {
                    "Media offline, or a title or generator.".to_string()
                } else {
                    format!(
                        "{} can't play here. Render to see it.",
                        extension(&clip.name)
                    )
                };
                let name = if clip.name.is_empty() {
                    "Untitled clip".to_string()
                } else {
                    clip.name.clone()
                };
                layer
                    .id(("unplayable", index))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .rounded_tl(corner)
                    .rounded_tr(corner)
                    .bg(rgb(0x171717))
                    .px_6()
                    .text_color(white(0.7))
                    .child(Icon::new(IconName::ImageOff).size(px(20.)))
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_token(TextSize::Xs)
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_token(TextSize::X2s)
                            .text_color(white(0.5))
                            .text_center()
                            .child(why),
                    )
                    .test_support()
                    .into_any_element()
            });
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded_tl(self.corner)
            .rounded_tr(self.corner)
            .bg(gpui::black())
            .children(layers)
    }
}

#[cfg(test)]
mod tests {
    use super::extension;

    #[test]
    fn names_the_files_kind() {
        assert_eq!(extension("Interview.braw"), "BRAW");
        assert_eq!(extension("Title"), "This file");
    }
}
