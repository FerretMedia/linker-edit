//! HistoryPanel.svelte: the project's edit history, newest first, on one rail through the day
//! headings. Each save shows who made it (Claude, Codex or the user), its clip strips and the
//! lines no strip can show, its Resolve actions (folded), and Restore, which only writes the
//! request in the composer as +page.svelte's `restore` does: the agent restores, after the user
//! sends it. The panel asks for the history while open, after a reconnect and a switch of
//! project, as +page.svelte's `$effect` does; the server pushes every new save after.

use std::collections::HashSet;

use chrono::{DateTime, Local};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use linker_edit_protocol::{ClientMessage, HistoryEntry, Provider};

use crate::icons::{IconName, Logo};
use crate::labels::{day_label, split_change};
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::clip_strip::{CHANGES, ClipStrip, change_color, change_name};
use crate::views::settings_parts::ring;
use crate::window::{no_drag, title_bar};

pub enum HistoryEvent {
    Close,
    /// Restore: this text goes in the composer, which takes the focus.
    Restore(String),
    /// Play on a strip: the cut preview of that timeline as it was in that save.
    Preview {
        timeline_id: String,
        commit: String,
    },
}

/// What Restore writes in the composer for a save.
pub fn restore_draft(entry: &HistoryEntry) -> String {
    let names: Vec<String> = entry
        .timelines
        .iter()
        .map(|timeline| format!("\"{}\"", timeline.name))
        .collect();
    let which = if names.is_empty() {
        "the timelines as they were".to_string()
    } else {
        format!(
            "{} as {}",
            names.join(", "),
            if names.len() == 1 {
                "it was"
            } else {
                "they were"
            }
        )
    };
    format!(
        "Restore {which} in save {} (\"{}\") with restore_timeline.",
        entry.commit, entry.subject
    )
}

/// One row of the rail: a day's heading or a save.
#[derive(Debug, PartialEq)]
pub enum RailRow<'a> {
    Day(String),
    Save(&'a HistoryEntry),
}

/// The rail's rows: a heading before the first save of each day.
pub fn rail_rows(entries: &[HistoryEntry], today: chrono::NaiveDate) -> Vec<RailRow<'_>> {
    let mut rows = Vec::new();
    let mut day = String::new();
    for entry in entries {
        let label = local_date(&entry.date)
            .map(|date| day_label(date.date_naive(), today))
            .unwrap_or_default();
        if label != day {
            rows.push(RailRow::Day(label.clone()));
        }
        day = label;
        rows.push(RailRow::Save(entry));
    }
    rows
}

fn local_date(iso: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|date| date.with_timezone(&Local))
}

pub struct HistoryPanel {
    state: Entity<AppState>,
    expanded: HashSet<String>,
    /// The link and project it last asked for, to ask again after a reconnect or a switch.
    asked: Option<(bool, Option<String>)>,
}

impl EventEmitter<HistoryEvent> for HistoryPanel {}

impl HistoryPanel {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, _, cx| this.ask(cx)).detach();
        let mut panel = HistoryPanel {
            state,
            expanded: HashSet::new(),
            asked: None,
        };
        panel.ask(cx);
        panel
    }

    fn ask(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let now = (state.connected, state.project_id().map(String::from));
        if self.asked.as_ref() != Some(&now) {
            if now.0 && now.1.is_some() {
                state.send(ClientMessage::LoadHistory);
            }
            self.asked = Some(now);
        }
        cx.notify();
    }

    fn save(
        &self,
        entry: &HistoryEntry,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.expanded.contains(&entry.commit);
        let thread = self.state.read(cx).thread_id().map(String::from);
        let media_url = self.state.read(cx).server_url("");
        let (accent, foreground) = (theme.accent, theme.foreground);

        // Each strip with its line from the summary, and the lines no strip can show.
        let lines: Vec<(Option<&str>, &str)> = entry
            .changes
            .iter()
            .map(|change| split_change(change))
            .collect();
        let shown: HashSet<&str> = entry
            .timelines
            .iter()
            .map(|strip| strip.name.as_str())
            .collect();
        let strips = entry.timelines.iter().map(|strip| {
            let detail = lines
                .iter()
                .find(|(timeline, _)| *timeline == Some(strip.name.as_str()))
                .map(|(_, detail)| detail.to_string());
            let (timeline_id, commit) = (strip.id.clone(), entry.commit.clone());
            ClipStrip::new(
                SharedString::from(format!("strip-{}-{}", entry.commit, strip.id)),
                strip.clone(),
                detail,
                media_url.clone(),
                cx.listener(move |_, _, _, cx| {
                    cx.emit(HistoryEvent::Preview {
                        timeline_id: timeline_id.clone(),
                        commit: commit.clone(),
                    })
                }),
            )
            .into_any_element()
        });
        let rest = lines
            .iter()
            .filter(|(timeline, _)| timeline.is_none_or(|timeline| !shown.contains(timeline)))
            .map(|(timeline, detail)| {
                let medium = gpui::HighlightStyle {
                    font_weight: Some(FontWeight::MEDIUM),
                    ..Default::default()
                };
                let muted = gpui::HighlightStyle {
                    color: Some(theme.muted_foreground),
                    ..Default::default()
                };
                let (text, highlights) = match timeline {
                    Some(timeline) => {
                        let start = timeline.len() + 1;
                        (
                            format!("{timeline} {detail}"),
                            vec![
                                (0..timeline.len(), medium),
                                (start..start + detail.len(), muted),
                            ],
                        )
                    }
                    None => (detail.to_string(), vec![(0..detail.len(), muted)]),
                };
                div()
                    .text_token(TextSize::Xs)
                    .line_height(px(12. * 1.375))
                    .child(gpui::StyledText::new(text).with_highlights(highlights))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let has_changes = !entry.timelines.is_empty() || !rest.is_empty();

        let agent = match entry.agent {
            Some(Provider::Claude) => Icon::new(Logo::Claude)
                .size(px(14.))
                .text_color(rgb(0xd97757)),
            Some(Provider::Codex) => Icon::new(Logo::OpenAi)
                .size(px(14.))
                .text_color(theme.foreground),
            None => Icon::new(IconName::UserRound)
                .size(px(14.))
                .text_color(theme.muted_foreground),
        };
        let time = local_date(&entry.date)
            .map(|date| date.format("%I:%M %p").to_string())
            .unwrap_or_default();
        let small_button = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_1()
                .h_6()
                .px_1p5()
                .rounded(theme.radius.md)
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(move |style| style.bg(accent).text_color(foreground))
        };
        let commit = entry.commit.clone();
        let actions = entry.actions.len();
        let draft = restore_draft(entry);

        div()
            .id(SharedString::from(format!("save-{}", entry.commit)))
            .relative()
            .flex()
            .gap_3()
            .pb_5()
            .when(!last, |this| this.child(rail_line(theme)))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.card)
                    .shadow(vec![ring(theme.sidebar, 3.)])
                    .child(agent),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pt_0p5()
                    .child(
                        div()
                            .text_token(TextSize::Sm)
                            .line_height(px(14. * 1.375))
                            .font_weight(FontWeight::MEDIUM)
                            .line_clamp(2)
                            .child(entry.subject.clone()),
                    )
                    .child(
                        div()
                            .mt_0p5()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_token(TextSize::X2s)
                            .text_color(theme.muted_foreground)
                            .child(entry.author.clone())
                            .child("·")
                            .child(time)
                            .child("·")
                            .child(
                                div()
                                    .font_family(theme.mono_font_family)
                                    .child(entry.commit.clone()),
                            )
                            .when(entry.thread.is_some() && entry.thread == thread, |this| {
                                this.child(
                                    div()
                                        .rounded(theme.radius.sm)
                                        .border_1()
                                        .border_color(theme.border)
                                        .px_1()
                                        .line_height(px(16.))
                                        .child("This thread"),
                                )
                            }),
                    )
                    .when(has_changes, |this| {
                        this.child(
                            div()
                                .mt_2()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .rounded(theme.radius.lg)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.card)
                                .p_2p5()
                                .children(strips)
                                .children(rest),
                        )
                    })
                    .child(
                        div()
                            .mt_1p5()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(entry.prompt.is_some() || actions > 0, |this| {
                                this.child(
                                    small_button(SharedString::from(format!(
                                        "actions-{}",
                                        entry.commit
                                    )))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.expanded.remove(&commit) {
                                            this.expanded.insert(commit.clone());
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        Icon::new(if open {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(px(12.)),
                                    )
                                    .child(format!(
                                        "{actions} Resolve {}",
                                        if actions == 1 { "action" } else { "actions" }
                                    ))
                                    .test_support(),
                                )
                            })
                            .child(
                                small_button(SharedString::from(format!(
                                    "restore-{}",
                                    entry.commit
                                )))
                                .tooltip(|window, cx| {
                                    Tooltip::new(
                                        "Ask the agent to bring the timelines back to this save",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(HistoryEvent::Restore(draft.clone()))
                                }))
                                .child(Icon::new(IconName::RotateCcw).size(px(12.)))
                                .child("Restore")
                                .test_support(),
                            ),
                    )
                    .when(open, |this| {
                        this.child(
                            div()
                                .id(SharedString::from(format!(
                                    "actions-{}-detail",
                                    entry.commit
                                )))
                                .mt_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .pl_1p5()
                                .when_some(entry.prompt.clone(), |this, prompt| {
                                    this.child(
                                        div()
                                            .border_l_2()
                                            .border_color(theme.border)
                                            .pl_2p5()
                                            .text_token(TextSize::Xs)
                                            .line_height(px(12. * 1.625))
                                            .text_color(theme.muted_foreground)
                                            .child(prompt),
                                    )
                                })
                                .children(entry.actions.iter().map(|action| {
                                    div()
                                        .font_family(theme.mono_font_family)
                                        .text_token(TextSize::X2s)
                                        .line_height(px(11. * 1.625))
                                        .text_color(theme.muted_foreground)
                                        .child(action.clone())
                                }))
                                .test_support(),
                        )
                    }),
            )
            .test_support()
            .into_any_element()
    }
}

/// `absolute top-0 bottom-0 left-[0.8125rem] w-px bg-border`: the rail.
fn rail_line(theme: &Theme) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(13.))
        .w(px(1.))
        .bg(theme.border)
}

impl Render for HistoryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let entries: Option<Vec<HistoryEntry>> =
            self.state.read(cx).history_entries().map(<[_]>::to_vec);
        let count = entries.as_ref().map_or(0, Vec::len);
        let (accent, foreground) = (theme.accent, theme.foreground);

        let header = title_bar("history-panel-header", px(16.), false, true, window)
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::new(IconName::History)
                    .size(px(16.))
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child("Edit history"),
            )
            .when(count > 0, |this| {
                this.child(
                    div()
                        .id("history-count")
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .px_1p5()
                        .py_0p5()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child(count.to_string())
                        .test_support(),
                )
            })
            .child(
                no_drag().ml_auto().child(
                    div()
                        .id("close-panel")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_7()
                        .rounded(theme.radius.md)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent).text_color(foreground))
                        .tooltip(|window, cx| Tooltip::new("Close history").build(window, cx))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(HistoryEvent::Close)))
                        .child(Icon::new(IconName::X).size(px(16.)))
                        .test_support(),
                ),
            );

        let legend = (count > 0).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_3()
                .gap_y_1()
                .border_b_1()
                .border_color(theme.border)
                .px_4()
                .py_2()
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .children(CHANGES.iter().map(|&change| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .size_2()
                                .rounded_full()
                                .bg(change_color(change, &theme)),
                        )
                        .child(change_name(change))
                }))
        });

        let body = match &entries {
            None => div()
                .px_1()
                .py_6()
                .text_center()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child("Loading history…")
                .into_any_element(),
            Some(entries) if entries.is_empty() => div()
                .id("history-empty")
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .px_4()
                .py_12()
                .text_center()
                .child(Icon::new(IconName::History).size(px(24.)).text_color(theme.muted_foreground.opacity(0.6)))
                .child(div().text_token(TextSize::Sm).font_weight(FontWeight::MEDIUM).child("No saves yet"))
                .child(
                    div()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child("Linker-Edit saves your Resolve project before each turn and after every turn that changes it."),
                )
                .test_support()
                .into_any_element(),
            Some(entries) => {
                let rows = rail_rows(entries, Local::now().date_naive());
                let total = rows.len();
                let rows: Vec<AnyElement> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| match row {
                        RailRow::Day(label) => div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap_3()
                            .h_7()
                            .when(index > 0, |this| this.child(rail_line(&theme)))
                            .child(div().relative().flex_shrink_0().w(px(26.)))
                            .child(
                                div()
                                    .text_token(TextSize::X2s)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_foreground)
                                    .child(label.to_uppercase()),
                            )
                            .into_any_element(),
                        RailRow::Save(entry) => self.save(entry, index == total - 1, &theme, cx),
                    })
                    .collect();
                div().flex().flex_col().children(rows).into_any_element()
            }
        };

        div()
            .id("history-panel")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(384.))
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(header)
            .children(legend)
            .child(
                div()
                    .id("history-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .py_3()
                    .child(body),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linker_edit_protocol::{TimelineStrip, TimelineStripState};

    fn entry(commit: &str, date: &str, timelines: &[&str]) -> HistoryEntry {
        HistoryEntry {
            commit: commit.into(),
            date: date.into(),
            author: "Claude".into(),
            agent: Some(Provider::Claude),
            subject: "Cut the intro".into(),
            prompt: None,
            actions: vec![],
            changes: vec![],
            thread: None,
            timelines: timelines
                .iter()
                .map(|name| TimelineStrip {
                    id: format!("id-{name}"),
                    name: (*name).into(),
                    state: TimelineStripState::Changed,
                    tiles: vec![],
                })
                .collect(),
        }
    }

    // +page.svelte's restore()
    #[test]
    fn writes_the_restore_request_for_the_composer() {
        assert_eq!(
            restore_draft(&entry("abc1234", "2026-11-18T10:00:00Z", &["AI - Cut"])),
            "Restore \"AI - Cut\" as it was in save abc1234 (\"Cut the intro\") with restore_timeline."
        );
        assert_eq!(
            restore_draft(&entry("abc1234", "2026-11-18T10:00:00Z", &["A", "B"])),
            "Restore \"A\", \"B\" as they were in save abc1234 (\"Cut the intro\") with restore_timeline."
        );
        assert_eq!(
            restore_draft(&entry("abc1234", "2026-11-18T10:00:00Z", &[])),
            "Restore the timelines as they were in save abc1234 (\"Cut the intro\") with restore_timeline."
        );
    }

    #[test]
    fn puts_a_heading_before_each_day() {
        use chrono::TimeZone;
        let noon = |day: u32| {
            Local
                .with_ymd_and_hms(2026, 11, day, 12, 0, 0)
                .unwrap()
                .to_rfc3339()
        };
        let entries = [
            entry("c", &noon(18), &[]),
            entry("b", &noon(18), &[]),
            entry("a", &noon(17), &[]),
        ];
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let rows = rail_rows(&entries, today);
        assert_eq!(
            rows,
            [
                RailRow::Day("Today".into()),
                RailRow::Save(&entries[0]),
                RailRow::Save(&entries[1]),
                RailRow::Day("Yesterday".into()),
                RailRow::Save(&entries[2]),
            ]
        );
    }
}
