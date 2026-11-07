//! The History pane: retained dictations (newest first) with search, one
//! menu for retention and two-step Clear all history, and a detail view with
//! Copy and Delete. It shares the host's store, which the listener records
//! into.

use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Menu as KitMenu, MenuItem, Placement, Select, SelectAppearance, TextInput};

use super::controls::{ButtonKind, Paint, button, modal_scrim_transition, modal_transition};
use super::list_detail::{self as ld, Item};
use super::lists::{error_panel, section_label};
use super::{Menu, Ui};
use crate::desktop::i18n::{event_age, tr, tr_fill, tr_named};
use crate::history::stats::{AppCount, HistoryStats};
use crate::history::{History, HistoryEntry, HistoryKind, HistoryRetention};

/// The stats dialog's width and padding.
const STATS_WIDTH: f32 = 460.0;
const PADDING: f32 = 18.0;

#[derive(Default)]
pub(super) struct HistoryPane {
    store: Option<History>,
    pub(super) entries: Vec<HistoryEntry>,
    search: TextEdit,
    /// The query the entries were loaded for.
    searched: String,
    /// The store's revision the entries were loaded at; `None` before the
    /// first load.
    revision: Option<u64>,
    pub(super) selected: Option<u64>,
    pub(super) clear_armed: bool,
    pub(super) copied: Option<u64>,
    error: Option<String>,
    /// Labs › Dictation stats: every retained entry summed up, while on.
    pub(super) stats: Option<HistoryStats>,
    /// The stats dialog is open.
    pub(super) stats_open: bool,
}

impl HistoryPane {
    /// The host's store; `None` when it could not open one.
    pub(super) fn shared(store: Option<History>) -> Self {
        Self {
            error: store
                .is_none()
                .then(|| tr("History could not be loaded.").to_string()),
            store,
            ..Self::default()
        }
    }

    /// Refreshes the bounded snapshot for the current query, and the stats.
    pub(super) fn reload(&mut self) {
        self.reload_entries();
        self.reload_stats();
    }

    fn reload_entries(&mut self) {
        self.searched = self.search.text.clone();
        let Some(store) = &self.store else {
            return;
        };
        let (revision, entries) = store.search_at(&self.searched);
        self.revision = Some(revision);
        self.entries = entries;
        if self
            .selected
            .is_some_and(|id| !self.entries.iter().any(|entry| entry.id == id))
        {
            self.selected = None;
        }
    }

    /// Sums up every retained entry, only while Labs › Dictation stats is
    /// on; off, nothing is read for it.
    fn reload_stats(&mut self) {
        self.stats = match &self.store {
            Some(store) if crate::labs::history_stats() => Some(store.stats()),
            _ => None,
        };
        if self
            .stats
            .as_ref()
            .is_none_or(|stats| stats.dictations == 0)
        {
            self.stats_open = false;
        }
    }

    /// While the pane shows, picks up new dictations and search edits. It
    /// searches again only when the query or the store's revision changed,
    /// and sums the stats again when the entries or the toggle changed, or
    /// once when the dialog closes because the stats emptied.
    pub(super) fn poll(&mut self, visible: bool) -> bool {
        let mut changed = false;
        // The dialog belongs to the pane and its switch: leaving History,
        // turning Dictation stats off, or emptying the stats closes it.
        if self.stats_open
            && (!visible
                || !crate::labs::history_stats()
                || self
                    .stats
                    .as_ref()
                    .is_none_or(|stats| stats.dictations == 0))
        {
            self.stats_open = false;
            changed = true;
        }
        let Some(store) = self.store.as_ref().filter(|_| visible) else {
            return changed;
        };
        let stale = self.search.text != self.searched || self.revision != Some(store.revision());
        if stale {
            let previous = std::mem::take(&mut self.entries);
            self.reload_entries();
            changed |= self.entries != previous;
        }
        if changed || self.stats.is_some() != crate::labs::history_stats() {
            self.reload_stats();
            return true;
        }
        changed
    }

    fn entry(&self, id: u64) -> Option<&HistoryEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub(super) fn fixture() -> Self {
        let now = crate::history::now_ms();
        let entry =
            |id: u64, minutes: u64, kind, raw: &str, final_text: &str, app: Option<&str>| {
                HistoryEntry {
                    id,
                    timestamp_ms: now - minutes * 60_000 - 30_000,
                    kind,
                    raw_text: raw.into(),
                    final_text: final_text.into(),
                    application: app.map(str::to_owned),
                    processing: None,
                    audio_ms: 4200,
                    inference_ms: 180,
                    total_ms: 640,
                }
            };
        let mut rewritten = entry(
            2,
            42,
            HistoryKind::Dictation,
            "so um the deploy failed because the the cache key changed",
            "The deploy failed because the cache key changed.",
            Some("Slack"),
        );
        rewritten.processing = Some(crate::desktop::events::DictationProcessing {
            profile: "Chat".into(),
            latency_ms: 910,
            fallback: None,
        });
        Self {
            entries: vec![
                entry(
                    3,
                    2,
                    HistoryKind::Dictation,
                    "Let's move the standup to ten thirty tomorrow.",
                    "Let's move the standup to ten thirty tomorrow.",
                    Some("WezTerm"),
                ),
                rewritten,
                entry(
                    1,
                    180,
                    HistoryKind::Dictation,
                    "Thanks for sending this over! I'll review it today and follow up with comments.",
                    "Thanks for sending this over! I'll review it today and follow up with comments.",
                    Some("Firefox"),
                ),
            ],
            selected: Some(2),
            ..Self::default()
        }
    }

    pub(super) fn fixture_empty() -> Self {
        Self::default()
    }

    /// A representative summary for the stats dialog's snapshots and tests.
    pub(super) fn fixture_stats() -> HistoryStats {
        HistoryStats {
            dictations: 128,
            words: 1_482,
            audio_ms: 720_000,
            words_per_minute: Some(123),
            changed_by_output: 34,
            streak_days: 9,
            active_days: 21,
            top_apps: vec![
                AppCount {
                    application: "Slack".into(),
                    dictations: 41,
                },
                AppCount {
                    application: "WezTerm".into(),
                    dictations: 33,
                },
                AppCount {
                    application: "Firefox".into(),
                    dictations: 27,
                },
            ],
        }
    }
}

fn set_retention(ui: &mut Ui, retention: HistoryRetention) {
    ui.open = None;
    if ui.settings.history_retention == retention {
        return;
    }
    if !ui.update(|settings| settings.history_retention = retention) {
        return;
    }
    if let Some(store) = &ui.history.store
        && let Err(error) = store.set_retention(retention)
    {
        ui.history.error = Some(error.to_string());
    }
    ui.history.reload();
}

fn clear(ui: &mut Ui) {
    if !ui.history.clear_armed {
        ui.history.clear_armed = true;
        return;
    }
    ui.history.clear_armed = false;
    if let Some(store) = &ui.history.store {
        if let Err(error) = store.clear() {
            ui.history.error = Some(error.to_string());
        }
        ui.history.reload();
    } else {
        ui.history.entries.clear();
        ui.history.selected = None;
    }
}

fn delete(ui: &mut Ui, id: u64) {
    if let Some(store) = &ui.history.store {
        if let Err(error) = store.delete(id) {
            ui.history.error = Some(error.to_string());
        }
        ui.history.reload();
    } else {
        ui.history.entries.retain(|entry| entry.id != id);
        ui.history.selected = None;
    }
}

/// The header's retention menu and the list + detail body.
pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Vec<El>, El) {
    let pane = &ui.history;
    let actions = header_actions(ui, p, layer);
    let mut notices = Vec::new();
    if let Some(error) = &pane.error {
        notices.push(error_panel(
            p,
            "history-error",
            tr("History could not be loaded."),
            error,
        ));
    }
    let searching = !pane.searched.trim().is_empty();
    let body = match empty(ui, p).filter(|_| !searching) {
        Some(empty) => empty,
        None => {
            let mut list = col().key("history-list").label(tr("History"));
            let count = pane.entries.len();
            for (index, entry) in pane.entries.iter().enumerate() {
                let id = entry.id;
                let mut meta = tr(entry.kind.label()).to_string();
                if let Some(application) = &entry.application {
                    meta.push_str(" · ");
                    meta.push_str(application);
                }
                list = list.child(
                    ld::item(
                        p,
                        Item {
                            key: format!("history-entry{id}"),
                            title: entry.final_text.replace('\n', " "),
                            meta,
                            meta_ink: None,
                            time: event_age(entry.timestamp_ms),
                            hover: vec![copy_action(ui, p, id, true)],
                            selected: pane.selected == Some(id),
                        },
                        move |ui: &mut Ui| {
                            ui.history.selected = Some(id);
                            ui.history.clear_armed = false;
                        },
                    )
                    .position_in_set(index + 1, count),
                );
            }
            let search = TextInput::new(&pane.search)
                .placeholder(tr("Search history"))
                .width(None)
                .leading_icon(p.k.glyph.search)
                .on_edit(|ui: &mut Ui| &mut ui.history.search)
                .build(p.k)
                .key("history-search");
            let column = ld::list_column(p, search, "history-scroll", list);
            ld::body(p, "history-body", column, detail(ui, p, layer))
        }
    };
    if let Some(modal) = stats_modal(ui, p) {
        layer.extend(modal);
    }
    (
        actions,
        col()
            .key("history-pane")
            .grow(1.0)
            .shrink()
            .children((!notices.is_empty()).then(|| ld::notices(notices)))
            .child(body),
    )
}

/// Labs › Dictation stats: every retained dictation summed up, in the
/// dialog the header's Stats button opens.
fn stats_lines(p: Paint, stats: &HistoryStats) -> Vec<El> {
    let wpm = stats
        .words_per_minute
        .map_or_else(|| "–".to_owned(), |wpm| wpm.to_string());
    let mut lines = vec![
        tr_named(
            "Dictations: {count} · Words: {words} · Words per minute: {wpm}",
            &[
                ("count", &stats.dictations.to_string()),
                ("words", &stats.words.to_string()),
                ("wpm", &wpm),
            ],
        ),
        tr_named(
            "Changed by Output: {percent}% · Daily streak: {days}",
            &[
                (
                    "percent",
                    &stats.changed_percent().unwrap_or_default().to_string(),
                ),
                ("days", &stats.streak_days.to_string()),
            ],
        ),
    ];
    if !stats.top_apps.is_empty() {
        let apps = stats
            .top_apps
            .iter()
            .map(|app| format!("{} ({})", app.application, app.dictations))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(tr_fill("Top apps: {}", &apps));
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            text(line)
                .key(format!("line{index}"))
                .font(super::controls::DESCRIPTION)
                .color(p.muted())
                .wrap()
        })
        .collect()
}

/// The stats dialog's scrim and panel, while its button and stats show.
fn stats_modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.history.stats_open || !ui.settings.labs.history_stats {
        return None;
    }
    let stats = ui
        .history
        .stats
        .as_ref()
        .filter(|stats| stats.dictations > 0)?;
    let c = &p.k.c;
    let mut panel = col()
        .key("history-stats-dialog")
        .role("dialog")
        .label(tr("Dictation stats"))
        .w(STATS_WIDTH)
        .pt(PADDING)
        .px(PADDING)
        .pb(14.0)
        .gap(12.0)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        // Presses inside the panel stay inside.
        .on_press(|_: &mut Ui| {})
        .child(
            text(tr("Dictation stats"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .children(stats_lines(p, stats))
        .child(
            row().key("footer").justify_end().child(
                button(
                    p,
                    "history-stats-close",
                    tr("Close"),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| close_stats(ui),
                )
                .default_button(),
            ),
        );
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    let layer = |key: &str| {
        el().key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
    };
    Some([
        layer("history-stats-scrim")
            .fill(c.scrim)
            .on_press(close_stats)
            .transition(modal_scrim_transition(p)),
        layer("history-stats-layer")
            .center()
            .transition(modal_transition(p))
            // Keys stay inside, Escape or a press outside closes, and focus
            // returns to the Stats button that opened it.
            .child(panel.modal(true, close_stats)),
    ])
}

/// Closes the stats dialog, from its button, Escape, or the scrim.
pub(super) fn close_stats(ui: &mut Ui) {
    ui.history.stats_open = false;
}

/// What the retention menu's trigger says: how long History keeps
/// dictations, or that it is off.
fn kept_label(retention: HistoryRetention) -> String {
    match retention {
        HistoryRetention::Off => tr("History off").to_string(),
        HistoryRetention::Forever => tr("Kept forever").to_string(),
        other => tr_fill("Kept {}", tr(other.label())),
    }
}

/// One menu for everything about keeping: how long, then Clear all
/// history… last. Once Clear all is
/// chosen, the menu gives way to its confirmation until it is answered.
fn header_actions(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<El> {
    if ui.history.clear_armed {
        return vec![
            button(
                p,
                "history-confirm-clear",
                tr("Clear all history"),
                ButtonKind::Danger,
                |ui: &mut Ui, _| clear(ui),
            ),
            button(
                p,
                "history-cancel-clear",
                tr("Cancel"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| ui.history.clear_armed = false,
            ),
        ];
    }
    let saved = ui.settings.history_retention;
    let open = ui.open == Some(Menu::HistoryRetention);
    let current = kept_label(saved);
    let trigger = Select::new::<&str>("history-retention-field", &[])
        .appearance(SelectAppearance::Inline)
        .current(Some(&current))
        .open(open)
        .on_toggle(|ui: &mut Ui| {
            ui.open = (ui.open != Some(Menu::HistoryRetention)).then_some(Menu::HistoryRetention);
        })
        .trigger(p.k)
        .label(tr("History retention"))
        .a11y_value(current.clone());
    let mut menu = KitMenu::new("history-retention")
        .trigger(trigger)
        .placement(Placement::BottomEnd)
        .gutter(4.0)
        .min_width(320.0)
        .open(open)
        .on_close(|ui: &mut Ui| ui.open = None);
    for retention in HistoryRetention::ALL {
        let mut item = MenuItem::new(tr(retention.label())).checked(saved == retention);
        if retention.is_off() {
            item = item.description(tr("Keeps no new dictations"));
        }
        menu = menu.item(item.on_select(move |ui: &mut Ui| set_retention(ui, retention)));
    }
    let nothing = ui.history.entries.is_empty() && ui.history.searched.trim().is_empty();
    menu = menu.separator().item(
        MenuItem::new(tr("Clear all history…"))
            .danger(true)
            .disabled(nothing)
            .on_select(|ui: &mut Ui| {
                ui.open = None;
                clear(ui);
            }),
    );
    let menu = menu.build(p.k, layer);
    if ui.settings.labs.history_stats
        && ui
            .history
            .stats
            .as_ref()
            .is_some_and(|stats| stats.dictations > 0)
    {
        return vec![
            row()
                .key("history-actions")
                .gap(8.0)
                .child(menu)
                .child(button(
                    p,
                    "history-stats-open",
                    tr("Stats"),
                    ButtonKind::Secondary,
                    |ui: &mut Ui, _| {
                        ui.open = None;
                        ui.history.stats_open = true;
                    },
                )),
        ];
    }
    vec![menu]
}

/// Copy as an icon: a row's hover action, or the card's.
fn copy_action(ui: &Ui, p: Paint, id: u64, in_row: bool) -> El {
    let glyph = if ui.history.copied == Some(id) {
        p.k.glyph.check_small
    } else {
        ld::COPY_ICON
    };
    let press = move |ui: &mut Ui, tx: &mut Tx| {
        if let Some(entry) = ui.history.entry(id) {
            tx.copy(entry.final_text.clone());
            ui.history.copied = Some(id);
            ui.history.error = None;
        }
    };
    if in_row {
        ld::row_action(p, "row-copy", glyph, tr("Copy"), press)
    } else {
        ld::ghost_icon(p, "history-copy", glyph, tr("Copy"), press)
    }
}

/// Nothing retained, or nothing matching: what History keeps and for how
/// long.
fn empty(ui: &Ui, p: Paint) -> Option<El> {
    let pane = &ui.history;
    if !pane.entries.is_empty() {
        return None;
    }
    let retention = ui.settings.history_retention;
    Some(if !pane.searched.trim().is_empty() {
        ld::empty_state(
            p,
            "history-empty",
            ld::SEARCH_ICON,
            tr("No dictations match."),
            None,
        )
    } else if retention.is_off() {
        ld::empty_state(
            p,
            "history-empty",
            ld::HISTORY_ICON,
            tr("History is off."),
            Some(tr("Choose how long to keep dictations to see them here.")),
        )
    } else {
        let hint = match retention {
            HistoryRetention::Forever => tr("Everything you dictate is kept here.").to_string(),
            other => tr_fill(
                "Everything you dictate is kept here for {}.",
                tr(other.label()),
            ),
        };
        ld::empty_state(
            p,
            "history-empty",
            ld::HISTORY_ICON,
            tr("No dictations retained yet."),
            Some(&hint),
        )
    })
}

fn detail(ui: &Ui, p: Paint, _layer: &mut Vec<El>) -> El {
    let pane = &ui.history;
    let Some(entry) = pane.selected.and_then(|id| pane.entry(id)) else {
        return empty(ui, p).unwrap_or_else(|| {
            ld::empty_state(
                p,
                "history-detail",
                ld::HISTORY_ICON,
                tr("Select a history entry."),
                None,
            )
        });
    };
    let id = entry.id;
    let mut latency = tr_named(
        "{audio} ms audio · {inference} ms inference",
        &[
            ("audio", &entry.audio_ms.to_string()),
            ("inference", &entry.inference_ms.to_string()),
        ],
    );
    if entry.total_ms > 0 {
        latency.push_str(&format!(
            " · {}",
            tr_fill("{} ms total", &entry.total_ms.to_string())
        ));
    }
    let mut facts = Vec::new();
    if let Some(application) = &entry.application {
        facts.push((tr("Application"), application.clone()));
    }
    if let Some(processing) = &entry.processing {
        let mut summary = format!("{} · {} ms", processing.profile, processing.latency_ms);
        if let Some(fallback) = &processing.fallback {
            summary.push_str(&format!(" · {}", tr_fill("fell back: {}", fallback)));
        }
        facts.push((tr("Processing"), summary));
    }
    facts.push((tr("Latency"), latency));
    let mut body = col().key("detail-body").gap(20.0).child(
        text(entry.final_text.clone())
            .key("final")
            .font(p.k.ty.body)
            .color(p.text())
            .wrap(),
    );
    if entry.raw_text.trim() != entry.final_text.trim() {
        body = body.child(
            col()
                .key("raw")
                .gap(6.0)
                .child(section_label(p, "label", tr("Raw transcript")))
                .child(
                    text(entry.raw_text.clone())
                        .key("value")
                        .font(p.k.ty.body)
                        .color(p.muted())
                        .wrap(),
                ),
        );
    }
    body = body.child(ld::facts(p, facts));
    col()
        .key("history-detail")
        .grow(1.0)
        .shrink()
        .child(ld::card_header(
            ld::card_title(p, tr(entry.kind.label())),
            vec![ld::meta_text(p, &event_age(entry.timestamp_ms))],
            vec![
                copy_action(ui, p, id, false),
                ld::ghost_icon(
                    p,
                    "history-delete",
                    ld::TRASH_ICON,
                    tr("Delete"),
                    move |ui: &mut Ui, _| delete(ui, id),
                ),
            ],
        ))
        .child(ld::card_scroll(p, "detail-scroll", body))
}
