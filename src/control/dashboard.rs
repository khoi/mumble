use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use color_eyre::Result;
use crossterm::event::{self, Event as TerminalEvent, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::desktop::events::{
    CommandOutcome, DictationPhase, EventReader, TranscriptPhase, VoiceEvent, VoiceState,
};
use crate::dictation::context::ContextSnapshot;

enum RecentAction<'a> {
    Command(&'a str, &'a CommandOutcome, &'a str),
    Dictation(&'a DictationPhase, &'a str),
}

pub fn run(path: PathBuf) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, path);
    ratatui::restore();
    result.map_err(Into::into)
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, path: PathBuf) -> io::Result<()> {
    let mut events = EventReader::open(&path);
    loop {
        events.refresh()?;
        terminal.draw(|frame| {
            draw(
                frame,
                events.events(),
                events.current_context().cloned().flatten(),
                &path,
            )
        })?;
        if event::poll(Duration::from_millis(100))?
            && let TerminalEvent::Key(key) = event::read()?
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            return Ok(());
        }
    }
}

fn draw(
    frame: &mut ratatui::Frame,
    events: &VecDeque<VoiceEvent>,
    application: Option<String>,
    path: &Path,
) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let state = events.iter().rev().find_map(|event| match event {
        VoiceEvent::State { state, device, .. } => Some((*state, device.as_str())),
        _ => None,
    });
    let (state, device) = state.unwrap_or((VoiceState::Stopping, "waiting for listener"));
    let (state_label, state_color) = match state {
        VoiceState::Sleeping => ("SLEEPING", Color::Blue),
        VoiceState::Listening => ("LISTENING", Color::Green),
        VoiceState::Dictating => ("DICTATING", Color::Magenta),
        VoiceState::Transcribing => ("TRANSCRIBING", Color::Cyan),
        VoiceState::Stopping => ("OFFLINE", Color::DarkGray),
    };
    let latest = events.iter().rev().find_map(|event| match event {
        VoiceEvent::Transcript {
            phase,
            latency_ms,
            text,
            ..
        } => Some((*phase, *latency_ms, text.as_str())),
        _ => None,
    });
    let latest_action = events.iter().rev().find_map(|event| match event {
        VoiceEvent::Command {
            command,
            outcome,
            context,
            ..
        } => Some(RecentAction::Command(
            command.as_deref().unwrap_or("no match"),
            outcome,
            context,
        )),
        VoiceEvent::Dictation { phase, text, .. } => Some(RecentAction::Dictation(phase, text)),
        _ => None,
    });
    let context = ContextSnapshot {
        application,
        ..ContextSnapshot::default()
    };

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(state_color)),
                Span::styled(
                    state_label,
                    Style::default()
                        .fg(state_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Input  ", Style::default().fg(Color::DarkGray)),
                Span::raw(device),
            ]),
            Line::from(vec![
                Span::styled("Option ", Style::default().fg(Color::DarkGray)),
                Span::raw("hold to dictate, release to transcribe and paste"),
            ]),
            Line::from(vec![
                Span::styled("Context ", Style::default().fg(Color::DarkGray)),
                Span::raw(context.label()),
            ]),
        ])
        .block(Block::default().title(" Voice ").borders(Borders::ALL)),
        areas[0],
    );

    let current_transcript = latest.map_or_else(
        || {
            Line::from(Span::styled(
                "No speech yet",
                Style::default().fg(Color::DarkGray),
            ))
        },
        |(phase, latency, text)| {
            Line::from(vec![
                Span::styled(
                    format!("{}  ", phase_label(phase)),
                    Style::default().fg(phase_color(phase)),
                ),
                Span::raw(text),
                Span::styled(
                    format!("  {latency}ms"),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        },
    );
    let current_action = match latest_action {
        Some(RecentAction::Command(command, outcome, context)) => Line::from(vec![
            Span::styled("command    ", Style::default().fg(Color::DarkGray)),
            Span::styled(command, Style::default().fg(command_color(outcome))),
            Span::styled(
                format!("  {}", outcome_display(outcome).1),
                Style::default().fg(command_color(outcome)),
            ),
            Span::styled(format!("  {context}"), Style::default().fg(Color::DarkGray)),
        ]),
        Some(RecentAction::Dictation(phase, text)) => Line::from(vec![
            Span::styled("dictation  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                dictation_label(phase),
                Style::default().fg(dictation_color(phase)),
            ),
            Span::raw(if text.is_empty() {
                String::new()
            } else {
                format!("  {text}")
            }),
        ]),
        None => Line::from(""),
    };
    frame.render_widget(
        Paragraph::new(vec![current_transcript, current_action])
            .wrap(Wrap { trim: false })
            .block(Block::default().title(" Current ").borders(Borders::ALL)),
        areas[1],
    );

    draw_activity(frame, events, areas[2]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("q/esc", Style::default().fg(Color::Cyan)),
            Span::raw(" quit  "),
            Span::styled(
                path.display().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        areas[3],
    );
}

fn draw_activity(
    frame: &mut ratatui::Frame,
    events: &VecDeque<VoiceEvent>,
    area: ratatui::layout::Rect,
) {
    let activity: Vec<ListItem> = events
        .iter()
        .filter_map(|event| match event {
            VoiceEvent::Command {
                heard,
                command,
                outcome,
                context,
                ..
            } => {
                let (symbol, label) = outcome_display(outcome);
                let error = match outcome {
                    CommandOutcome::Failed(error) => format!("  {error}"),
                    _ => String::new(),
                };
                Some(Line::from(vec![
                    Span::styled(
                        format!("{symbol} {label:<12}"),
                        Style::default().fg(command_color(outcome)),
                    ),
                    Span::raw(heard),
                    Span::styled(
                        command
                            .as_deref()
                            .map_or_else(String::new, |id| format!("  {id}")),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("  [{context}]{error}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            }
            VoiceEvent::Dictation {
                phase,
                text,
                processing,
                ..
            } => {
                let detail = match phase {
                    DictationPhase::Failed(error) => error.as_str(),
                    _ => text.as_str(),
                };
                let processing = processing.as_ref().map_or_else(String::new, |processing| {
                    processing.fallback.as_ref().map_or_else(
                        || format!("  [{} · {} ms]", processing.profile, processing.latency_ms),
                        |error| {
                            format!(
                                "  [{} · {} ms · raw fallback: {}]",
                                processing.profile, processing.latency_ms, error
                            )
                        },
                    )
                });
                Some(Line::from(vec![
                    Span::styled(
                        format!("◆ {:<12}", dictation_label(phase)),
                        Style::default().fg(dictation_color(phase)),
                    ),
                    Span::styled("dictation", Style::default().fg(Color::DarkGray)),
                    Span::raw(if detail.is_empty() {
                        String::new()
                    } else {
                        format!("  {detail}")
                    }),
                    Span::styled(processing, Style::default().fg(Color::DarkGray)),
                ]))
            }
            VoiceEvent::SessionStarted { .. }
            | VoiceEvent::State { .. }
            | VoiceEvent::Transcript { .. }
            | VoiceEvent::Context { .. }
            | VoiceEvent::ApiServerStarted { .. }
            | VoiceEvent::ApiServerStopped { .. }
            | VoiceEvent::ApiAuthFailed { .. } => None,
        })
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(ListItem::new)
        .collect();
    frame.render_widget(
        List::new(activity).block(Block::default().title(" Log ").borders(Borders::ALL)),
        area,
    );
}

fn phase_label(phase: TranscriptPhase) -> &'static str {
    match phase {
        TranscriptPhase::Started => "started",
        TranscriptPhase::Updated => "updated",
        TranscriptPhase::Completed => "completed",
    }
}

fn phase_color(phase: TranscriptPhase) -> Color {
    match phase {
        TranscriptPhase::Started => Color::Cyan,
        TranscriptPhase::Updated => Color::Yellow,
        TranscriptPhase::Completed => Color::Green,
    }
}

fn outcome_display(outcome: &CommandOutcome) -> (&'static str, &'static str) {
    match outcome {
        CommandOutcome::Ignored => ("○", "ignored"),
        CommandOutcome::Woke => ("↑", "woke"),
        CommandOutcome::Slept => ("↓", "slept"),
        CommandOutcome::Submitted => ("…", "submitted"),
        CommandOutcome::Executed => ("✓", "executed"),
        CommandOutcome::Failed(_) => ("!", "failed"),
    }
}

fn command_color(outcome: &CommandOutcome) -> Color {
    match outcome {
        CommandOutcome::Executed | CommandOutcome::Woke | CommandOutcome::Slept => Color::Green,
        CommandOutcome::Submitted => Color::Cyan,
        CommandOutcome::Failed(_) => Color::Red,
        CommandOutcome::Ignored => Color::DarkGray,
    }
}

fn dictation_label(phase: &DictationPhase) -> &'static str {
    match phase {
        DictationPhase::Started => "recording",
        DictationPhase::Discarded => "discarded",
        DictationPhase::Cancelled => "cancelled",
        DictationPhase::Transcribing => "transcribing",
        DictationPhase::Pasted => "paste sent",
        DictationPhase::Printed => "printed",
        DictationPhase::Edited => "edited",
        DictationPhase::VoiceAction => "voice action",
        DictationPhase::Logged => "logged",
        DictationPhase::Repasted => "repasted",
        DictationPhase::MeetingPasted => "meeting pasted",
        DictationPhase::Failed(_) => "failed",
    }
}

fn dictation_color(phase: &DictationPhase) -> Color {
    match phase {
        DictationPhase::Started => Color::Magenta,
        DictationPhase::Transcribing => Color::Cyan,
        DictationPhase::Pasted
        | DictationPhase::Printed
        | DictationPhase::Edited
        | DictationPhase::VoiceAction
        | DictationPhase::Logged
        | DictationPhase::Repasted
        | DictationPhase::MeetingPasted => Color::Green,
        DictationPhase::Discarded => Color::DarkGray,
        DictationPhase::Cancelled => Color::Yellow,
        DictationPhase::Failed(_) => Color::Red,
    }
}
