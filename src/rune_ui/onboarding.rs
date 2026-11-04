//! The first run's state machine in the window: [`Tour`] feeds
//! `onboarding::Flow` the permission state, asks for permissions, relaunches
//! when macOS needs it, watches the practice dictation, and records
//! completion. The first run the person sees is the onboarding story over it
//! ([`onboarding_kit::live`]), the one onboarding on every OS. Transcription
//! is cloud-only, so there is no model step to download or install.
//!
//! [`onboarding_kit::live`]: super::onboarding_kit::live

use std::time::{Duration, Instant};

use super::Ui;
use super::platform::{PlatformRequest, PlatformState};
use crate::app_settings::AppSettings;
use crate::desktop::onboarding::{Effect, Flow, Input, PermissionState, SetupItem, Step};

/// How long the ready check shows before setup moves on.
const ADVANCE_BEAT: Duration = Duration::from_millis(700);

pub(super) struct Tour {
    pub(super) flow: Flow,
    advance_at: Option<Instant>,
    completed: bool,
}

impl Tour {
    fn new(flow: Flow, completed: bool) -> Self {
        Self {
            flow,
            advance_at: None,
            completed,
        }
    }

    /// The tour from Welcome, as on a new install: Run setup again.
    pub(super) fn replay(_settings: &AppSettings, platform: &PlatformState) -> Tour {
        Tour::new(
            Flow::new(
                Step::Welcome,
                platform.setup_status(),
                platform.caps.permissions,
                platform.caps.practice_dictation,
            ),
            false,
        )
    }

    /// The tour when this run still needs it.
    pub(super) fn start(_settings: &AppSettings, platform: &PlatformState) -> Option<Tour> {
        let status = platform.setup_status();
        let completed = crate::desktop::onboarding::completion_recorded();
        let resume = crate::desktop::onboarding::take_resume();
        let saved = (!completed)
            .then(crate::desktop::onboarding::saved_step)
            .flatten();
        (resume.is_some() || (!completed && (!status.ready() || saved.is_some()))).then(|| {
            Tour::new(
                Flow::new(
                    resume.or(saved).unwrap_or(Step::Welcome),
                    status,
                    platform.caps.permissions,
                    platform.caps.practice_dictation,
                ),
                completed,
            )
        })
    }
}

/// Feeds the flow fresh permission state; true when the tour changed.
pub(super) fn poll(ui: &mut Ui) -> bool {
    let status = ui.platform.setup_status();
    let accessibility_stale = ui
        .platform
        .stale(crate::desktop::onboarding::PermissionKind::Accessibility);
    let live = ui.live();
    let Some(tour) = &mut ui.tour else {
        return false;
    };
    let before = tour.flow.view();
    let step = before.step;
    let mut effect = tour.flow.on(Input::Status(status));
    tour.flow.set_accessibility_stale(accessibility_stale);
    if tour
        .advance_at
        .is_some_and(|at| at.elapsed() >= ADVANCE_BEAT)
    {
        tour.advance_at = None;
        if tour.flow.step() == Step::Setup {
            effect = tour.flow.on(Input::Continue);
        }
    }
    if status.ready() && !tour.completed {
        tour.completed = true;
        if live && let Err(error) = crate::desktop::onboarding::record_completion() {
            tracing::warn!(%error, "could not record onboarding completion");
        }
    }
    let practicing = tour.flow.step() == Step::TryIt;
    apply(ui, effect);
    let heard = ui.host.tour_inputs(practicing);
    let changed = !heard.is_empty();
    for heard in heard {
        input(ui, heard);
    }
    remember_step(ui, step);
    let after = ui.tour.as_ref().map(|tour| tour.flow.view());
    changed || after.as_ref() != Some(&before)
}

pub(super) fn input(ui: &mut Ui, input: Input) {
    let Some(tour) = &mut ui.tour else {
        return;
    };
    let step = tour.flow.step();
    let effect = tour.flow.on(input);
    apply(ui, effect);
    remember_step(ui, step);
}

/// Saves a new step so a run quit part-way resumes there. The story sounds
/// the success chime itself, on Hex is ready.
fn remember_step(ui: &Ui, before: Step) {
    if let Some(tour) = &ui.tour
        && tour.flow.step() != before
        && ui.live()
    {
        crate::desktop::onboarding::record_step(tour.flow.step());
    }
}

fn apply(ui: &mut Ui, effect: Effect) {
    match effect {
        Effect::AdvanceSoon => {
            if let Some(tour) = &mut ui.tour {
                tour.advance_at = Some(Instant::now());
            }
        }
        Effect::RequestMicrophone => ui.ask(PlatformRequest::Permission(
            crate::desktop::onboarding::PermissionAction::RequestMicrophone,
        )),
        Effect::ShowGuide(kind) => ui.ask(PlatformRequest::Permission(
            crate::desktop::onboarding::PermissionAction::ShowGuide(kind),
        )),
        Effect::ResetPermission(kind) => ui.ask(PlatformRequest::Permission(
            crate::desktop::onboarding::PermissionAction::ResetAndPrompt(kind),
        )),
        Effect::Relaunch => ui.ask(PlatformRequest::Relaunch),
        Effect::Finish => {
            ui.tour = None;
            ui.first_run = None;
        }
        Effect::None => {}
    }
}

/// A first run at `step`.
pub(super) fn fixture(ui: &mut Ui, step: Step) {
    let status = ui.platform.setup_status();
    let mut flow = Flow::new(
        Step::Welcome,
        status,
        ui.platform.caps.permissions,
        ui.platform.caps.practice_dictation,
    );
    if step != Step::Welcome {
        flow.on(Input::Continue);
    }
    ui.tour = Some(Tour::new(flow, false));
    poll(ui);
}

/// The macOS tour's states, walked as a person would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MacStep {
    Setup,
    SetupStale,
    SetupDenied,
    /// Every permission on, Accessibility waiting for a restart.
    SetupRestart,
    TryIt,
    /// The shortcut listener failed after setup.
    TryItReopen,
    Done,
}

/// Walks a fresh macOS first run to `state`. `ui.platform` must carry
/// macOS capabilities.
pub(super) fn mac_fixture(ui: &mut Ui, state: MacStep) {
    use PermissionState::{NeedsRequest, NeedsRestart, NeedsSettings, Ready};
    let fresh = crate::desktop::onboarding::SetupStatus {
        microphone: NeedsRequest,
        accessibility: NeedsSettings,
    };
    let permissions = match state {
        MacStep::Setup => fresh,
        MacStep::SetupStale => crate::desktop::onboarding::SetupStatus {
            microphone: Ready,
            ..fresh
        },
        MacStep::SetupDenied => crate::desktop::onboarding::SetupStatus {
            microphone: NeedsSettings,
            ..fresh
        },
        MacStep::SetupRestart => crate::desktop::onboarding::SetupStatus {
            microphone: Ready,
            accessibility: NeedsRestart,
        },
        _ => crate::desktop::onboarding::SetupStatus {
            microphone: Ready,
            accessibility: Ready,
        },
    };
    ui.platform.permissions = fresh;
    ui.tour = Some(Tour::new(
        Flow::new(Step::Welcome, fresh, true, true),
        false,
    ));
    poll(ui);
    input(ui, Input::Continue);
    if state == MacStep::SetupStale {
        for item in [SetupItem::Microphone, SetupItem::Accessibility] {
            input(ui, Input::Grant(item));
        }
        ui.platform
            .note_stale(crate::desktop::onboarding::PermissionKind::Accessibility);
    }
    ui.platform.permissions = permissions;
    poll(ui);
    if permissions.ready() {
        input(ui, Input::Continue);
    }
    if let Some(tour) = &mut ui.tour {
        tour.advance_at = None;
    }
    if state == MacStep::TryItReopen {
        input(
            ui,
            Input::ListenerFailed {
                relaunch_allowed: false,
            },
        );
    }
    if state == MacStep::Done {
        input(ui, Input::Skip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::onboarding::ItemStatus;

    #[test]
    fn mac_stale_fixture_marks_accessibility_stale_and_dispatches_reset() {
        let mut ui = crate::rune_ui::mac_fixture();
        mac_fixture(&mut ui, MacStep::SetupStale);
        let rows = ui.tour.as_ref().unwrap().flow.view().rows;
        assert!(
            rows.iter()
                .any(|row| row.item == SetupItem::Accessibility && row.status == ItemStatus::Stale)
        );
        input(&mut ui, Input::Grant(SetupItem::Accessibility));
        assert!(
            !ui.platform
                .stale(crate::desktop::onboarding::PermissionKind::Accessibility)
        );
        match &ui.host {
            crate::rune_ui::platform::PlatformHost::Fixture(sent) => {
                assert_eq!(
                    sent.last(),
                    Some(&PlatformRequest::Permission(
                        crate::desktop::onboarding::PermissionAction::ResetAndPrompt(
                            crate::desktop::onboarding::PermissionKind::Accessibility
                        )
                    ))
                );
            }
            _ => panic!("expected fixture host"),
        }
    }
}
