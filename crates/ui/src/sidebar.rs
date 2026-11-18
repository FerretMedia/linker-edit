//! The sidebar's thread and project logic: a port of src/lib/sidebar.ts, which is ported from
//! T3 Code (MIT, see NOTICE): packages/client-runtime/src/state/threadSort.ts, threadSettled.ts,
//! threadInbox.ts and apps/web/src/components/Sidebar.logic.ts.
//!
//! Times are milliseconds since the epoch, as `Date.parse` gives them; the protocol's are ISO 8601.

use chrono::{DateTime, Datelike, Local, NaiveTime, SecondsFormat, TimeZone, Utc};
use linker_edit_protocol::{ProjectSummary, ThreadSummary};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Threads a project folder shows before "Show more".
pub const FOLDER_PREVIEW_COUNT: usize = 6;
/// Settled threads shown when the shelf opens, and how many more each "Show more" adds. As in T3 Code.
pub const SETTLED_INITIAL_COUNT: usize = 10;
pub const SETTLED_PAGE_COUNT: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSection {
    Pinned,
    Active,
    Snoozed,
    Settled,
}

/// A thread with its project, as the sidebar lists it.
#[derive(Clone, Copy, Debug)]
pub struct SidebarThread<'a> {
    pub thread: &'a ThreadSummary,
    pub project: &'a ProjectSummary,
}

impl<'a> SidebarThread<'a> {
    pub fn id(&self) -> &'a str {
        &self.thread.id
    }
}

/// An ISO 8601 time in milliseconds; None when missing or not a time.
pub fn time(iso: Option<&str>) -> Option<i64> {
    let iso = iso?;
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn ms(iso: &str) -> Option<i64> {
    time(Some(iso))
}

/// A time in milliseconds as `Date.toISOString` writes it.
pub fn iso(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The current time in milliseconds.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Snoozed and still asleep. A thread waiting for an approval wakes early, as one asking for input does in T3 Code.
pub fn is_snoozed(thread: &ThreadSummary, now: i64) -> bool {
    time(thread.sidebar.snoozed_until.as_deref()).is_some_and(|wake| wake > now) && !thread.waiting
}

/// When a snoozed thread woke: at its time, or early when it asked for an approval.
pub fn woke_at(thread: &ThreadSummary, now: i64) -> Option<&str> {
    let until = thread.sidebar.snoozed_until.as_deref();
    let wake = time(until)?;
    if thread.waiting {
        return thread.sidebar.snoozed_at.as_deref().or(until);
    }
    (wake <= now).then_some(until).flatten()
}

/// Woke since it was last looked at.
pub fn is_woke(thread: &ThreadSummary, now: i64) -> bool {
    let Some(woke) = time(woke_at(thread, now)) else {
        return false;
    };
    if thread.sidebar.settled_at.is_some() {
        return false;
    }
    time(thread.sidebar.last_read_at.as_deref()).is_none_or(|read| read < woke)
}

/// A turn ended after the user last looked. A thread never looked at isn't unread, as in T3 Code.
pub fn is_unread(thread: &ThreadSummary) -> bool {
    if thread.busy {
        return false;
    }
    let ended = time(thread.sidebar.turn_ended_at.as_deref());
    let read = time(thread.sidebar.last_read_at.as_deref());
    matches!((ended, read), (Some(ended), Some(read)) if ended > read)
}

pub fn thread_section(thread: &ThreadSummary, now: i64) -> SidebarSection {
    if is_snoozed(thread, now) {
        SidebarSection::Snoozed
    } else if thread.sidebar.settled_at.is_some() {
        SidebarSection::Settled
    } else if thread.sidebar.pinned_at.is_some() {
        SidebarSection::Pinned
    } else {
        SidebarSection::Active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadStatus {
    Approval,
    Working,
    Ready,
}

pub fn thread_status(thread: &ThreadSummary) -> ThreadStatus {
    if thread.waiting {
        ThreadStatus::Approval
    } else if thread.busy {
        ThreadStatus::Working
    } else {
        ThreadStatus::Ready
    }
}

/// What the row says on its right, before the time: T3 Code's "top status".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopStatus {
    Working,
    Approval,
    Woke,
    Done,
}

impl TopStatus {
    pub fn label(self) -> &'static str {
        match self {
            TopStatus::Working => "Working",
            TopStatus::Approval => "Approval",
            TopStatus::Woke => "Woke",
            TopStatus::Done => "Done",
        }
    }
}

pub fn top_status(thread: &ThreadSummary, now: i64) -> Option<TopStatus> {
    match thread_status(thread) {
        ThreadStatus::Working => Some(TopStatus::Working),
        ThreadStatus::Approval => Some(TopStatus::Approval),
        ThreadStatus::Ready if is_woke(thread, now) => Some(TopStatus::Woke),
        ThreadStatus::Ready if is_unread(thread) => Some(TopStatus::Done),
        ThreadStatus::Ready => None,
    }
}

/// Quiet rows: working, or done and seen. The open thread never recedes.
pub fn should_recede(thread: &ThreadSummary, now: i64, active: bool) -> bool {
    if active {
        return false;
    }
    if thread_status(thread) == ThreadStatus::Working {
        return true;
    }
    !is_unread(thread) && !is_woke(thread, now)
}

/// The time a thread's activity shows, and active threads sort by: its creation or the user's last message.
pub fn activity_time(thread: &ThreadSummary) -> i64 {
    let created = ms(&thread.sidebar.created_at).unwrap_or(0);
    let sent = time(Some(
        thread
            .sidebar
            .last_sent_at
            .as_deref()
            .unwrap_or(&thread.updated_at),
    ))
    .unwrap_or(0);
    created.max(sent)
}

/// activityTimestamp, in milliseconds.
pub fn activity_timestamp(thread: &ThreadSummary) -> i64 {
    match activity_time(thread) {
        0 => ms(&thread.updated_at).unwrap_or(0),
        time => time,
    }
}

fn by_id(left: &ThreadSummary, right: &ThreadSummary) -> std::cmp::Ordering {
    left.id.cmp(&right.id)
}

/// Newest send first.
pub fn sort_active(threads: &mut [SidebarThread]) {
    threads.sort_by(|left, right| {
        activity_time(right.thread)
            .cmp(&activity_time(left.thread))
            .then_with(|| by_id(left.thread, right.thread))
    });
}

/// Pinned threads have no manual order here (T3 Code's drag order isn't ported): newest first.
pub fn sort_pinned(threads: &mut [SidebarThread]) {
    let created = |thread: &ThreadSummary| ms(&thread.sidebar.created_at).unwrap_or(0);
    threads.sort_by(|left, right| {
        created(right.thread)
            .cmp(&created(left.thread))
            .then_with(|| by_id(left.thread, right.thread))
    });
}

/// Soonest to wake first.
pub fn sort_snoozed(threads: &mut [SidebarThread]) {
    let wake = |thread: &ThreadSummary| time(thread.sidebar.snoozed_until.as_deref()).unwrap_or(0);
    threads.sort_by(|left, right| {
        wake(left.thread)
            .cmp(&wake(right.thread))
            .then_with(|| by_id(left.thread, right.thread))
    });
}

/// settledTimestamp, in milliseconds.
pub fn settled_timestamp(thread: &ThreadSummary) -> i64 {
    time(thread.sidebar.settled_at.as_deref()).unwrap_or_else(|| activity_timestamp(thread))
}

/// Most recently settled first.
pub fn sort_settled(threads: &mut [SidebarThread]) {
    threads.sort_by(|left, right| {
        settled_timestamp(right.thread)
            .cmp(&settled_timestamp(left.thread))
            .then_with(|| by_id(left.thread, right.thread))
    });
}

/// A project's last activity: its newest thread's.
pub fn project_activity(project: &ProjectSummary) -> i64 {
    project
        .threads
        .iter()
        .filter(|thread| !thread.archived)
        .map(activity_time)
        .fold(0, i64::max)
}

/// `localeCompare` for names, near enough: ignoring case first.
fn compare_names(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

/// Pinned projects first, then by last activity; archived ones last.
pub fn sort_projects(projects: &[ProjectSummary]) -> Vec<&ProjectSummary> {
    let rank = |project: &ProjectSummary| {
        if project.sidebar.archived {
            2
        } else if project.sidebar.pinned {
            0
        } else {
            1
        }
    };
    let mut sorted: Vec<&ProjectSummary> = projects.iter().collect();
    sorted.sort_by(|left, right| {
        rank(left)
            .cmp(&rank(right))
            .then_with(|| project_activity(right).cmp(&project_activity(left)))
            .then_with(|| compare_names(&left.name, &right.name))
    });
    sorted
}

#[derive(Clone, Debug)]
pub struct SidebarFolder<'a> {
    pub project: &'a ProjectSummary,
    pub threads: Vec<SidebarThread<'a>>,
}

#[derive(Clone, Debug, Default)]
pub struct SidebarLists<'a> {
    pub pinned: Vec<SidebarThread<'a>>,
    pub folders: Vec<SidebarFolder<'a>>,
    pub snoozed: Vec<SidebarThread<'a>>,
    pub settled: Vec<SidebarThread<'a>>,
    /// Everything listed, in order, for moving to the next thread after a settle.
    pub order: Vec<SidebarThread<'a>>,
}

/// The sidebar's sections. A project gets a folder when it has active threads, or when it's pinned (not archived)
/// or open in Resolve; an archived project keeps one only for its active threads, below the rest.
pub fn partition_sidebar(projects: &[ProjectSummary], now: i64) -> SidebarLists<'_> {
    let mut lists = SidebarLists::default();
    for project in sort_projects(projects) {
        let mut active = Vec::new();
        for thread in project.threads.iter().filter(|thread| !thread.archived) {
            let listed = SidebarThread { thread, project };
            match thread_section(thread, now) {
                SidebarSection::Pinned => lists.pinned.push(listed),
                SidebarSection::Snoozed => lists.snoozed.push(listed),
                SidebarSection::Settled => lists.settled.push(listed),
                SidebarSection::Active => active.push(listed),
            }
        }
        if !active.is_empty()
            || (!project.sidebar.archived && (project.sidebar.pinned || project.open))
        {
            sort_active(&mut active);
            lists.folders.push(SidebarFolder {
                project,
                threads: active,
            });
        }
    }
    sort_pinned(&mut lists.pinned);
    sort_snoozed(&mut lists.snoozed);
    sort_settled(&mut lists.settled);
    lists.order = lists
        .pinned
        .iter()
        .chain(lists.folders.iter().flat_map(|folder| &folder.threads))
        .copied()
        .collect();
    lists
}

/// The next thread to open after settling or snoozing the open one: the next listed one, wrapping around.
pub fn next_thread_after<'a>(
    order: &[SidebarThread<'a>],
    thread_id: &str,
) -> Option<SidebarThread<'a>> {
    let index = order.iter().position(|thread| thread.id() == thread_id)?;
    order[index + 1..]
        .iter()
        .chain(&order[..index])
        .next()
        .copied()
}

/// Lower case, without accents: "Café" → "cafe".
fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// `word` starts a word of `text`: at its start, or after a character that isn't a letter or digit.
fn starts_a_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(index, _)| {
        text[..index]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

/// Threads whose title (or project name) matches every word of the query: title starts first, then word starts,
/// then anywhere; ties by activity. Settled and archived threads are found too.
pub fn search_threads<'a>(projects: &'a [ProjectSummary], query: &str) -> Vec<SidebarThread<'a>> {
    let folded = fold(query);
    let words: Vec<&str> = folded.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let mut scored: Vec<(SidebarThread, u8)> = Vec::new();
    for project in projects {
        let name = fold(&project.name);
        for thread in &project.threads {
            let title = fold(&thread.title);
            let subject = format!("{title} {name}");
            if !words.iter().all(|word| subject.contains(word)) {
                continue;
            }
            let score = if title.starts_with(first) {
                0
            } else if starts_a_word(&title, first) {
                1
            } else if title.contains(first) {
                2
            } else {
                3
            };
            scored.push((SidebarThread { thread, project }, score));
        }
    }
    scored.sort_by(|(left, left_score), (right, right_score)| {
        left_score
            .cmp(right_score)
            .then_with(|| activity_time(right.thread).cmp(&activity_time(left.thread)))
            .then_with(|| by_id(left.thread, right.thread))
    });
    scored.into_iter().map(|(thread, _)| thread).collect()
}

/// "now", "5m", "3h", "2d": T3 Code's compact sidebar time.
pub fn relative_time(ms: i64, now: i64) -> String {
    let seconds = (now - ms).max(0) / 1000;
    if seconds < 60 {
        return "now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

/// How long until a snoozed thread wakes: "45m", "3h", "2d".
pub fn snooze_wake_label(snoozed_until: &str, now: i64) -> String {
    let Some(wake) = ms(snoozed_until).filter(|wake| *wake > now) else {
        return "now".into();
    };
    let minutes = (wake.div_euclid(60_000) - now.div_euclid(60_000)).max(1);
    if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 24 * 60 {
        format!("{}h", (minutes + 59) / 60)
    } else {
        format!("{}d", (minutes + 24 * 60 - 1) / (24 * 60))
    }
}

#[derive(Clone, Copy)]
enum SnoozeSetting {
    After(i64),
    At(Day, u32, u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Day {
    Today,
    Tomorrow,
    Monday,
}

/// T3 Code's default snooze choices.
const SNOOZE_PRESETS: [SnoozeSetting; 5] = [
    SnoozeSetting::After(60),
    SnoozeSetting::After(180),
    SnoozeSetting::At(Day::Today, 18, 0),
    SnoozeSetting::At(Day::Tomorrow, 9, 0),
    SnoozeSetting::At(Day::Monday, 9, 0),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnoozePreset {
    pub id: String,
    pub label: String,
    /// The wake time, shown after the label.
    pub when_label: String,
    pub snoozed_until: String,
}

fn pluralize(count: i64, unit: &str) -> String {
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// `toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })`, as en-US writes it.
fn time_of_day(time: &DateTime<Local>) -> String {
    time.format("%-I:%M %p").to_string()
}

fn resolve_preset(setting: SnoozeSetting, now: DateTime<Local>) -> Option<SnoozePreset> {
    let (wake, label, id, names_day) = match setting {
        SnoozeSetting::After(minutes) => {
            let wake = now + chrono::Duration::minutes(minutes);
            let label = if minutes % 60 == 0 {
                format!("In {}", pluralize(minutes / 60, "hour"))
            } else {
                format!("In {}", pluralize(minutes, "minute"))
            };
            (wake, label, format!("after:{minutes}"), false)
        }
        SnoozeSetting::At(day, hour, minute) => {
            let weekday = now.weekday().num_days_from_sunday() as i64;
            let days_ahead = match day {
                Day::Today => 0,
                Day::Tomorrow => 1,
                Day::Monday => match (1 - weekday + 7) % 7 {
                    0 => 7,
                    days => days,
                },
            };
            let date = now.date_naive() + chrono::Duration::days(days_ahead);
            let wake = Local
                .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(hour, minute, 0)?))
                .earliest()?;
            if wake <= now {
                return None;
            }
            let label = match days_ahead {
                0 if hour < 12 => "This morning",
                0 if hour < 17 => "This afternoon",
                0 => "This evening",
                1 => "Tomorrow",
                _ => "Next week",
            };
            let day_name = match day {
                Day::Today => "today",
                Day::Tomorrow => "tomorrow",
                Day::Monday => "monday",
            };
            (
                wake,
                label.to_string(),
                format!("at:{day_name}:{hour:02}:{minute:02}"),
                label != "Next week",
            )
        }
    };
    let when_label = if names_day || wake.date_naive() == now.date_naive() {
        time_of_day(&wake)
    } else {
        format!("{} {}", wake.format("%a"), time_of_day(&wake))
    };
    Some(SnoozePreset {
        id,
        label,
        when_label,
        snoozed_until: iso(wake.timestamp_millis()),
    })
}

/// The snooze menu: soonest first, one row per wake minute.
pub fn snooze_presets(now: DateTime<Local>) -> Vec<SnoozePreset> {
    let minute = |preset: &SnoozePreset| ms(&preset.snoozed_until).unwrap_or(0).div_euclid(60_000);
    let mut presets: Vec<SnoozePreset> = SNOOZE_PRESETS
        .iter()
        .filter_map(|setting| resolve_preset(*setting, now))
        .collect();
    presets.sort_by_key(minute);
    presets.dedup_by_key(|preset| minute(preset));
    presets
}

/// The next time any snoozed thread wakes, so the sidebar re-sorts then without a ticking clock.
pub fn next_wake(projects: &[ProjectSummary], now: i64) -> Option<i64> {
    projects
        .iter()
        .flat_map(|project| &project.threads)
        .filter_map(|thread| time(thread.sidebar.snoozed_until.as_deref()))
        .filter(|wake| *wake > now)
        .min()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use linker_edit_protocol::{LocalLink, ProjectSidebarState, ThreadSidebarState};

    // From src/lib/sidebar.test.ts.
    pub const NOW: i64 = 1_794_744_000_000; // 2026-11-15T12:00:00.000Z

    pub fn at(minutes_ago: f64) -> Option<String> {
        Some(iso(NOW - (minutes_ago * 60_000.0) as i64))
    }

    pub fn thread(id: &str, edit: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut thread = ThreadSummary {
            id: id.into(),
            title: id.into(),
            updated_at: at(500.0).unwrap(),
            archived: false,
            busy: false,
            waiting: false,
            sidebar: ThreadSidebarState {
                created_at: at(1000.0).unwrap(),
                ..Default::default()
            },
        };
        edit(&mut thread);
        thread
    }

    pub fn project(
        id: &str,
        threads: Vec<ThreadSummary>,
        edit: impl FnOnce(&mut ProjectSummary),
    ) -> ProjectSummary {
        let mut project = ProjectSummary {
            id: id.into(),
            name: id.into(),
            dir: format!("/projects/{id}"),
            link: None,
            local: LocalLink::default(),
            open: false,
            threads,
            setup: false,
            sidebar: ProjectSidebarState::default(),
        };
        edit(&mut project);
        project
    }

    fn ids(threads: &[SidebarThread]) -> Vec<String> {
        threads
            .iter()
            .map(|thread| thread.id().to_string())
            .collect()
    }

    #[test]
    fn the_test_clock_is_the_typescript_one() {
        assert_eq!(iso(NOW), "2026-11-15T12:00:00.000Z");
    }

    #[test]
    fn puts_pinned_threads_on_top_active_ones_in_their_project_and_the_rest_on_shelves() {
        let projects = [project(
            "a",
            vec![
                thread("pinned", |t| t.sidebar.pinned_at = at(5.0)),
                thread("active", |_| {}),
                thread("settled", |t| t.sidebar.settled_at = at(3.0)),
                thread("snoozed", |t| t.sidebar.snoozed_until = at(-60.0)),
                thread("woke", |t| t.sidebar.snoozed_until = at(1.0)),
                thread("archived", |t| t.archived = true),
            ],
            |_| {},
        )];
        let lists = partition_sidebar(&projects, NOW);
        assert_eq!(ids(&lists.pinned), ["pinned"]);
        assert_eq!(
            lists
                .folders
                .iter()
                .map(|folder| ids(&folder.threads))
                .collect::<Vec<_>>(),
            [["active", "woke"]]
        );
        assert_eq!(ids(&lists.snoozed), ["snoozed"]);
        assert_eq!(ids(&lists.settled), ["settled"]);
        assert_eq!(ids(&lists.order), ["pinned", "active", "woke"]);
    }

    #[test]
    fn wakes_a_snoozed_thread_early_when_it_asks_for_an_approval() {
        let asking = thread("asking", |t| {
            t.sidebar.snoozed_until = at(-60.0);
            t.sidebar.snoozed_at = at(10.0);
            t.waiting = true;
        });
        let projects = [project("a", vec![asking.clone()], |_| {})];
        assert_eq!(
            ids(&partition_sidebar(&projects, NOW).folders[0].threads),
            ["asking"]
        );
        assert_eq!(top_status(&asking, NOW), Some(TopStatus::Approval));
    }

    #[test]
    fn sorts_threads_by_the_last_message_and_projects_by_their_newest_thread() {
        let projects = [
            project(
                "quiet",
                vec![thread("old", |t| t.sidebar.last_sent_at = at(300.0))],
                |_| {},
            ),
            project(
                "busy",
                vec![
                    thread("older", |t| t.sidebar.last_sent_at = at(200.0)),
                    thread("newest", |t| t.sidebar.last_sent_at = at(1.0)),
                    thread("new", |t| t.sidebar.created_at = at(10.0).unwrap()),
                ],
                |_| {},
            ),
        ];
        let lists = partition_sidebar(&projects, NOW);
        let folders: Vec<_> = lists
            .folders
            .iter()
            .map(|f| f.project.id.as_str())
            .collect();
        assert_eq!(folders, ["busy", "quiet"]);
        assert_eq!(ids(&lists.folders[0].threads), ["newest", "new", "older"]);
    }

    #[test]
    fn lists_pinned_projects_first_and_archived_ones_last_and_drops_empty_ones() {
        let projects = [
            project(
                "archived",
                vec![thread("x", |t| t.sidebar.last_sent_at = at(1.0))],
                |p| p.sidebar.archived = true,
            ),
            project(
                "recent",
                vec![thread("y", |t| t.sidebar.last_sent_at = at(2.0))],
                |_| {},
            ),
            project(
                "pinned",
                vec![thread("z", |t| {
                    t.sidebar.settled_at = at(1.0);
                    t.sidebar.last_sent_at = at(500.0);
                })],
                |p| p.sidebar.pinned = true,
            ),
            project(
                "settled",
                vec![thread("w", |t| t.sidebar.settled_at = at(1.0))],
                |_| {},
            ),
            project("open in Resolve", vec![], |p| p.open = true),
        ];
        let sorted: Vec<_> = sort_projects(&projects)
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(
            sorted,
            ["pinned", "recent", "settled", "open in Resolve", "archived"]
        );
        let folders: Vec<_> = partition_sidebar(&projects, NOW)
            .folders
            .iter()
            .map(|f| f.project.id.as_str())
            .collect();
        assert_eq!(folders, ["pinned", "recent", "open in Resolve", "archived"]);
    }

    #[test]
    fn sorts_the_settled_shelf_by_when_they_were_settled_newest_first() {
        let projects = [project(
            "a",
            vec![
                thread("first", |t| t.sidebar.settled_at = at(60.0)),
                thread("last", |t| t.sidebar.settled_at = at(5.0)),
            ],
            |_| {},
        )];
        assert_eq!(
            ids(&partition_sidebar(&projects, NOW).settled),
            ["last", "first"]
        );
    }

    #[test]
    fn opens_the_next_listed_thread_after_the_open_one_is_settled_wrapping_around() {
        let projects = [project(
            "a",
            vec![
                thread("one", |t| t.sidebar.last_sent_at = at(1.0)),
                thread("two", |t| t.sidebar.last_sent_at = at(2.0)),
            ],
            |_| {},
        )];
        let order = partition_sidebar(&projects, NOW).order;
        assert_eq!(
            next_thread_after(&order, "one").map(|t| t.id()),
            Some("two")
        );
        assert_eq!(
            next_thread_after(&order, "two").map(|t| t.id()),
            Some("one")
        );
        assert!(next_thread_after(&order, "gone").is_none());
    }

    #[test]
    fn is_unread_when_a_turn_ended_after_the_user_last_looked() {
        let read = |ended: f64, read: Option<f64>, busy: bool| {
            thread("a", |t| {
                t.sidebar.turn_ended_at = at(ended);
                t.sidebar.last_read_at = read.and_then(at);
                t.busy = busy;
            })
        };
        assert!(is_unread(&read(1.0, Some(5.0), false)));
        assert!(!is_unread(&read(5.0, Some(1.0), false)));
        assert!(!is_unread(&read(1.0, None, false)));
        assert!(!is_unread(&read(1.0, Some(5.0), true)));
        assert_eq!(
            top_status(&read(1.0, Some(5.0), false), NOW),
            Some(TopStatus::Done)
        );
    }

    #[test]
    fn says_woke_until_the_woken_thread_is_looked_at() {
        let snoozed = |until: f64, read: f64| {
            thread("a", |t| {
                t.sidebar.snoozed_until = at(until);
                t.sidebar.last_read_at = at(read);
            })
        };
        assert!(is_woke(&snoozed(1.0, 30.0), NOW));
        assert!(!is_woke(&snoozed(10.0, 1.0), NOW));
        assert!(!is_woke(&snoozed(-10.0, 30.0), NOW));
    }

    #[test]
    fn finds_threads_by_every_word_title_starts_first_settled_and_archived_too() {
        let projects = [
            project(
                "Podcast",
                vec![
                    thread("1", |t| {
                        t.title = "Ep 12 rough cut".into();
                        t.sidebar.last_sent_at = at(5.0);
                    }),
                    thread("2", |t| {
                        t.title = "Color the rough cut".into();
                        t.sidebar.last_sent_at = at(1.0);
                        t.sidebar.settled_at = at(1.0);
                    }),
                    thread("3", |t| {
                        t.title = "Captions".into();
                        t.archived = true;
                    }),
                ],
                |_| {},
            ),
            project(
                "Launch",
                vec![thread("4", |t| t.title = "Rough mix".into())],
                |_| {},
            ),
        ];
        assert_eq!(ids(&search_threads(&projects, "rough")), ["4", "2", "1"]);
        assert_eq!(ids(&search_threads(&projects, "rough cut")), ["2", "1"]);
        assert_eq!(ids(&search_threads(&projects, "podcast captions")), ["3"]);
        assert!(search_threads(&projects, "  ").is_empty());
        // Accents don't matter.
        assert_eq!(ids(&search_threads(&projects, "CAPTÏONS")), ["3"]);
    }

    #[test]
    fn writes_ages_the_way_t3_codes_sidebar_does() {
        let age = |minutes: f64| relative_time(NOW - (minutes * 60_000.0) as i64, NOW);
        assert_eq!(age(0.5), "now");
        assert_eq!(age(5.0), "5m");
        assert_eq!(age(180.0), "3h");
        assert_eq!(age(60.0 * 24.0 * 9.0), "9d");
        assert_eq!(snooze_wake_label(&at(-45.0).unwrap(), NOW), "45m");
        assert_eq!(snooze_wake_label(&at(-150.0).unwrap(), NOW), "3h");
        assert_eq!(snooze_wake_label(&at(1.0).unwrap(), NOW), "now");
    }

    #[test]
    fn offers_t3_codes_snooze_choices_soonest_first_without_the_ones_already_past() {
        let local = |d, h| Local.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap();
        let labels = |now| {
            snooze_presets(now)
                .into_iter()
                .map(|preset| preset.label)
                .collect::<Vec<_>>()
        };
        // A Monday, after 18:00.
        assert_eq!(
            labels(local(16, 19)),
            ["In 1 hour", "In 3 hours", "Tomorrow", "Next week"]
        );
        // A Sunday.
        assert_eq!(
            labels(local(15, 8)),
            ["In 1 hour", "In 3 hours", "This evening", "Tomorrow"]
        );
        let next_week = snooze_presets(local(16, 19)).pop().unwrap();
        assert_eq!(next_week.when_label, "Mon 9:00 AM");
        assert_eq!(next_week.id, "at:monday:09:00");
    }

    #[test]
    fn knows_when_the_next_snoozed_thread_wakes() {
        let projects = [project(
            "a",
            vec![
                thread("past", |t| t.sidebar.snoozed_until = at(5.0)),
                thread("later", |t| t.sidebar.snoozed_until = at(-60.0)),
                thread("soon", |t| t.sidebar.snoozed_until = at(-5.0)),
            ],
            |_| {},
        )];
        assert_eq!(next_wake(&projects, NOW), time(at(-5.0).as_deref()));
    }
}
