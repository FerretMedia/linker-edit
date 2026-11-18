//! The agent's replies as markdown: src/lib/markdown.ts and app.css's `.prose-chat`.
//!
//! gpui-component's markdown view draws it, with text selection. Agent output is untrusted, so,
//! as marked is set up in the browser:
//! - raw HTML is shown as the text it is written as (the view's `raw_html(false)`);
//! - only http(s) and mailto links are links, opened outside the app; others are plain text;
//! - an image shows as its alt text and loads nothing;
//! - a single line break is a line break (marked's `breaks: true`).
//!
//! The last two are done to the source before the view parses it, by [`prepare`].

use std::sync::Arc;

use gpui::{App, ElementId, Overflow, SharedString, StyleRefinement, Styled, px, relative, rems};
use gpui_kit::component::text::{TextView, TextViewStyle};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::theme::Theme;

/// Whether a link may open: `/^(https?:|mailto:)/i`, as markdown.ts allows.
pub fn safe_link(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("mailto:")
}

/// The source with each soft line break made a hard one, and each image replaced by its alt text.
pub fn prepare(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    // (offset, length to replace, replacement), in source order.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut image: Option<(usize, usize, String)> = None;
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { .. }) if image.is_none() => {
                image = Some((range.start, range.end, String::new()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((start, end, alt)) = image.take() {
                    edits.push((start, end - start, escape(&alt)));
                }
            }
            Event::Text(alt) | Event::Code(alt) if image.is_some() => {
                image.as_mut().unwrap().2.push_str(&alt);
            }
            Event::SoftBreak if image.is_none() => {
                // A backslash before the newline makes it a hard break.
                let newline = text[range.clone()]
                    .find(['\r', '\n'])
                    .map_or(range.start, |index| range.start + index);
                edits.push((newline, 0, "\\".into()));
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(text.len() + edits.len());
    let mut at = 0;
    for (offset, length, replacement) in edits {
        if offset < at {
            continue;
        }
        out.push_str(&text[at..offset]);
        out.push_str(&replacement);
        at = offset + length;
    }
    out.push_str(&text[at..]);
    out
}

/// Escapes markdown's punctuation so alt text stays text.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for char in text.chars() {
        if char.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(char);
    }
    out
}

/// `.prose-chat`: 14px with a 1.65 line height; `* + *` 0.75em apart; h1 20px, h2 18px,
/// h3 and h4 16px, all semibold; code in the mono font on the muted background, blocks on the
/// card with a border.
pub fn assistant_markdown(
    id: impl Into<ElementId>,
    text: &str,
    theme: &Theme,
    _cx: &App,
) -> impl gpui::IntoElement {
    let mut code_block = StyleRefinement::default()
        .bg(theme.card)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius.lg)
        .p_3()
        .font_family(theme.mono_font_family)
        .text_size(px(12.));
    code_block.overflow.x = Some(Overflow::Scroll);
    let style = TextViewStyle {
        // 0.75em of 14px.
        paragraph_gap: rems(0.65625),
        heading_font_size: Some(Arc::new(|level, _| match level {
            1 => px(20.),
            2 => px(18.),
            _ => px(16.),
        })),
        code_block,
        ..TextViewStyle::default()
    };
    let inline_code = StyleRefinement::default()
        .font_family(theme.mono_font_family)
        .text_size(px(13.))
        .bg(theme.muted)
        .border_1()
        .border_color(theme.border)
        .rounded(px(4.))
        .px(px(4.8))
        .py(px(0.8));
    let blockquote = StyleRefinement::default()
        .border_l_2()
        .border_color(theme.border)
        .pl_3()
        .text_color(theme.muted_foreground);
    TextView::markdown(id, SharedString::from(prepare(text)))
        .markdown_extensions(
            gpui_kit::component::text::MarkdownExtensions::default().raw_html(false),
        )
        .style(style)
        .inline_code_style(inline_code)
        .blockquote_style(blockquote)
        .selectable(true)
        // A link the browser wouldn't open is drawn as plain text.
        .link(|link, _| safe_link(&link.url).then(|| link.clone()))
        .on_link_click(|url, _, _, cx| {
            if safe_link(url) {
                cx.open_url(url);
            }
        })
        .text_size(px(14.))
        .line_height(relative(1.65))
}

#[cfg(test)]
mod tests {
    use super::*;

    // markdown.test.ts: what the browser neutralizes. Raw HTML is the view's own `raw_html(false)`.
    #[test]
    fn neutralizes_links_that_arent_web_or_mail() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///C:/x",
            "#",
        ] {
            assert!(!safe_link(url), "{url}");
        }
        for url in [
            "https://example.com/a?b=1&c=2",
            "HTTP://a.example",
            "mailto:a@b.c",
        ] {
            assert!(safe_link(url), "{url}");
        }
    }

    #[test]
    fn shows_an_image_as_its_alt_text() {
        assert_eq!(
            prepare(r#"![x" onerror="alert(1)](https://a.example/i.png)"#),
            r#"x\" onerror\=\"alert\(1\)"#
        );
        assert_eq!(prepare("See ![the *logo*](l.png)."), "See the logo.");
    }

    #[test]
    fn keeps_single_line_breaks() {
        assert_eq!(prepare("one\ntwo\n\nthree"), "one\\\ntwo\n\nthree");
        assert_eq!(prepare("- a\n  b"), "- a\\\n  b");
        // Code keeps its own lines.
        assert_eq!(prepare("```\na\nb\n```"), "```\na\nb\n```");
        assert_eq!(prepare("one\r\ntwo"), "one\\\r\ntwo");
    }
}
