//! `hex` control: the typed command/reply protocol an agent uses to drive the
//! one running Hex, the settings registry behind `hex settings`, and the CLI
//! client. Transports (the macOS local API's `POST /control`) only move
//! [`ControlRequest`]s and [`ControlResponse`]s; the app's host carries out
//! each command, and every rule shared between the window and the CLI lives
//! here or in the app's own setters.

pub mod cli;
#[cfg(all(debug_assertions, target_os = "macos"))]
pub(crate) mod dashboard;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod developer_control;
#[cfg(not(target_os = "macos"))]
pub(crate) mod host;
pub mod settings;
pub(crate) mod transcript_cli;

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::dictation::vocabulary::{self, VocabularyEntry};

/// The wire version of [`ControlRequest`] and [`ControlResponse`]. A change
/// that breaks either shape increments it.
pub const CONTROL_VERSION: u32 = 7;

/// What a capture command does. Commands are idempotent: starting while
/// recording or stopping while idle changes nothing and succeeds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureAction {
    /// Start recording, locked as after a double tap: the shortcut or
    /// `stop` finishes it.
    Start,
    /// Finish the recording and transcribe it.
    Stop,
    /// Start when idle, otherwise stop.
    Toggle,
    /// Discard the recording: nothing is transcribed, pasted, or saved.
    /// Dictations already transcribing finish.
    Cancel,
}

/// The window's panes, by the names `hex window show` takes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Pane {
    Settings,
    /// Settings with the transcription controls (General).
    Models,
    #[value(alias = "replacements")]
    Modes,
    History,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum ControlCommand {
    Status,
    Quit {
        #[serde(default)]
        force: bool,
    },
    Restart {
        #[serde(default)]
        force: bool,
    },
    Dictation {
        action: CaptureAction,
        /// `stop` only: answer with the transcript instead of pasting it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        print: bool,
        /// `start` and `toggle` only: the dictation this starts pastes the
        /// transcript as heard, skipping the Output pane's steps.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        raw: bool,
    },
    PasteLast,
    SettingsGet {
        #[serde(default)]
        key: Option<String>,
    },
    SettingsSet {
        key: String,
        value: String,
    },
    /// Several settings at once, all or none (`hex settings apply`).
    SettingsApply {
        values: serde_json::Map<String, serde_json::Value>,
    },
    WindowShow {
        #[serde(default)]
        pane: Option<Pane>,
    },
    WindowHide,
    /// Replay the first-run onboarding in the window (dev, test, and local
    /// builds); settings, history, models, and permissions stay.
    OnboardingReset,
    /// Open the window on a playground story (Labs).
    WindowStory {
        story: String,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default)]
        play: bool,
    },
    HistoryClear,
    CheckForUpdates,
    /// Report Launch at Login, or turn it on or off.
    LaunchAtLogin {
        #[serde(default)]
        enabled: Option<bool>,
    },
    /// The shared vocabulary.
    VocabularyList,
    VocabularyAdd {
        term: String,
        #[serde(default)]
        sounds_like: Vec<String>,
    },
    VocabularyRemove {
        term: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlRequest {
    pub version: u32,
    #[serde(flatten)]
    pub command: ControlCommand,
}

impl ControlRequest {
    pub fn new(command: ControlCommand) -> Self {
        Self {
            version: CONTROL_VERSION,
            command,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ControlReply {
    /// The command ran. `changed` is false when it was already so.
    Done {
        message: String,
        changed: bool,
    },
    Status(Box<StatusReport>),
    Settings {
        settings: Vec<settings::SettingValue>,
    },
    Vocabulary {
        entries: Vec<VocabularyEntry>,
    },
    /// `hex dictation stop --print`: the processed transcript.
    Transcript {
        text: String,
    },
    Error {
        code: String,
        message: String,
    },
}

impl ControlReply {
    pub fn done(message: impl Into<String>) -> Self {
        Self::Done {
            message: message.into(),
            changed: true,
        }
    }

    pub fn unchanged(message: impl Into<String>) -> Self {
        Self::Done {
            message: message.into(),
            changed: false,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<ControlError> for ControlReply {
    fn from(error: ControlError) -> Self {
        Self::error(error.code, error.message)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ControlResponse {
    pub version: u32,
    #[serde(flatten)]
    pub reply: ControlReply,
}

impl From<ControlReply> for ControlResponse {
    fn from(reply: ControlReply) -> Self {
        Self {
            version: CONTROL_VERSION,
            reply,
        }
    }
}

/// A refused or failed command, with a stable code (see `docs/cli.md`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlError {
    pub code: &'static str,
    pub message: String,
}

impl ControlError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// How long `hex dictation stop --print` waits for its transcript.
pub const PRINT_WAIT: Duration = Duration::from_secs(60);
/// How long the app may take to carry out any other control command.
const CONTROL_DEADLINE: Duration = Duration::from_secs(10);

impl ControlCommand {
    /// How long a transport waits for the app's answer to this command.
    pub fn deadline(&self) -> Duration {
        match self {
            Self::Dictation { print: true, .. } => PRINT_WAIT + Duration::from_secs(5),
            _ => CONTROL_DEADLINE,
        }
    }
}

/// What a printing stop gets back from the listener once its dictation
/// finishes: the transcript, processed as a paste would be, or why there is
/// none. A printed transcript is never pasted.
#[derive(Debug, Eq, PartialEq)]
pub enum PrintedTranscript {
    Text(String),
    NoSpeech,
    /// The Linux and Windows listeners stop by themselves without a recording.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NotRecording,
    Failed(String),
}

/// The one-shot channel a printing stop rides through the listener's queue.
#[derive(Clone, Debug)]
pub struct PrintReply(SyncSender<PrintedTranscript>);

impl PrintReply {
    pub fn channel() -> (Self, Receiver<PrintedTranscript>) {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        (Self(sender), receiver)
    }

    /// Answers the waiting command; one that gave up is not told.
    pub fn send(self, transcript: PrintedTranscript) {
        let _ = self.0.send(transcript);
    }
}

/// Shortcut events compare by kind; where a reply goes is not part of it.
impl PartialEq for PrintReply {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for PrintReply {}

/// Waits up to `wait` for a printing stop's transcript. A dropped reply
/// means the dictation was cancelled or failed before it had text.
pub fn await_printed(receiver: &Receiver<PrintedTranscript>, wait: Duration) -> ControlReply {
    match receiver.recv_timeout(wait) {
        Ok(PrintedTranscript::Text(text)) => ControlReply::Transcript { text },
        Ok(PrintedTranscript::NoSpeech) => {
            ControlReply::error("no-speech", "no speech was recorded; nothing to print")
        }
        Ok(PrintedTranscript::NotRecording) => ControlReply::error(
            "not-recording",
            "dictation isn't recording; nothing to print",
        ),
        Ok(PrintedTranscript::Failed(error)) => {
            ControlReply::error("failed", format!("couldn't transcribe it: {error}"))
        }
        Err(RecvTimeoutError::Timeout) => ControlReply::error(
            "failed",
            format!(
                "no transcript within {} s; nothing was printed or pasted",
                wait.as_secs()
            ),
        ),
        Err(RecvTimeoutError::Disconnected) => ControlReply::error(
            "failed",
            "the dictation was cancelled or failed; nothing to print",
        ),
    }
}

/// The action a request asks for; `print` applies only to `stop`.
pub fn check_flags(
    action: CaptureAction,
    print: bool,
    raw: bool,
) -> Result<CaptureAction, ControlError> {
    if print && action != CaptureAction::Stop {
        return Err(ControlError::new(
            "invalid-value",
            "--print works only with `hex dictation stop`",
        ));
    }
    if raw && !matches!(action, CaptureAction::Start | CaptureAction::Toggle) {
        return Err(ControlError::new(
            "invalid-value",
            "--raw works only with `hex dictation start` and `hex dictation toggle`",
        ));
    }
    Ok(action)
}

/// One request from a transport to the app's control host.
pub struct ControlCall {
    pub request: ControlRequest,
    pub reply: SyncSender<ControlResponse>,
}

/// Rejects a request from a `hex` built for another protocol version.
pub fn check_version(request: &ControlRequest) -> Result<(), ControlError> {
    if request.version == CONTROL_VERSION {
        return Ok(());
    }
    Err(version_mismatch(request.version.into()))
}

pub fn version_mismatch(version: u64) -> ControlError {
    ControlError::new(
        "version-mismatch",
        format!(
            "this hex speaks control version {version}, but the running Hex speaks \
             {CONTROL_VERSION}; use the hex inside the running app or restart Hex"
        ),
    )
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StatusReport {
    pub app_version: String,
    pub pid: u32,
    /// The process's physical memory footprint, as Activity Monitor shows it.
    pub memory_footprint_bytes: Option<u64>,
    pub listener: ListenerReport,
    pub capture: CaptureReport,
    pub transcription: TranscriptionReport,
    pub features: FeatureReport,
    pub permissions: PermissionReport,
    pub window: WindowReport,
    /// current, checking, available, or unavailable.
    pub update: String,
    /// The Labs features turned on (`crate::labs`), by `hex labs` name.
    #[serde(default)]
    pub labs: Vec<String>,
    /// What this OS's Hex can do, by capability name (`rune_ui::platform`).
    /// A command for one it can't do answers `unsupported`.
    #[serde(default)]
    pub capabilities: std::collections::BTreeMap<String, bool>,
}

/// `StatusReport::update` for an updater status.
pub fn update_label(status: &crate::desktop::host::DesktopUpdateStatus) -> &'static str {
    use crate::desktop::host::DesktopUpdateStatus;
    match status {
        DesktopUpdateStatus::Checking | DesktopUpdateStatus::Downloading => "checking",
        DesktopUpdateStatus::Available
        | DesktopUpdateStatus::ReadyToRestart
        | DesktopUpdateStatus::SwitchAvailable { .. }
        | DesktopUpdateStatus::PackageUpdate { .. } => "available",
        DesktopUpdateStatus::Unavailable
        | DesktopUpdateStatus::Managed
        | DesktopUpdateStatus::Failed => "unavailable",
        DesktopUpdateStatus::Current
        | DesktopUpdateStatus::SourceBuild
        | DesktopUpdateStatus::PackageManager => "current",
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ListenerReport {
    pub running: bool,
    pub status: String,
    /// What a Wayland shortcuts portal granted, in the desktop's own keys,
    /// including keys changed only in its settings (#723); empty elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<GrantedShortcutReport>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct GrantedShortcutReport {
    pub name: String,
    /// `None` when the desktop refused it or names no key.
    pub trigger: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureState {
    #[default]
    Idle,
    Recording,
    Transcribing,
    /// The listener did not answer in time.
    Unknown,
}

/// What is recording.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureKind {
    Dictation,
    /// A programmatic dictation through the local API (`POST /dictations`).
    Service,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CaptureReport {
    pub state: CaptureState,
    pub recording: Option<CaptureKind>,
    /// Accepted dictations still transcribing, processing, or pasting.
    pub pending_jobs: usize,
}

/// The listener's capture state, as it answers a status query.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaptureSnapshot {
    pub recording: Option<CaptureKind>,
    pub pending_jobs: usize,
    /// No microphone is open: none was found, or the stream is reopening.
    pub microphone_unavailable: bool,
}

impl From<CaptureSnapshot> for CaptureReport {
    fn from(snapshot: CaptureSnapshot) -> Self {
        Self {
            state: if snapshot.recording.is_some() {
                CaptureState::Recording
            } else if snapshot.pending_jobs > 0 {
                CaptureState::Transcribing
            } else {
                CaptureState::Idle
            },
            recording: snapshot.recording,
            pending_jobs: snapshot.pending_jobs,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TranscriptionReport {
    /// The cloud service transcription uses: `claude` or `codex`.
    pub provider: String,
    pub language: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FeatureReport {
    pub release_microphone_while_idle: bool,
    pub history_retention: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PermissionReport {
    /// Every required permission granted.
    pub ready: bool,
    /// ready, needs-request, needs-settings, or needs-restart.
    pub microphone: String,
    pub accessibility: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WindowReport {
    pub open: bool,
    pub pane: Option<String>,
}

/// Refuses to stop Hex while it records, unless forced. `capture` is the
/// listener's answer (idle when no listener runs or setup gates it); a
/// listener that couldn't answer might be recording, so it refuses too.
pub fn check_quit(
    capture: Result<CaptureSnapshot, ControlError>,
    force: bool,
) -> Result<(), ControlError> {
    if force {
        return Ok(());
    }
    match capture {
        Ok(capture) if capture.recording.is_some() || capture.pending_jobs > 0 => {
            Err(ControlError::new(
                "busy",
                "a dictation is recording or transcribing; wait for it or pass --force",
            ))
        }
        Ok(_) => Ok(()),
        Err(error) => Err(ControlError::new(
            "busy",
            format!(
                "couldn't confirm nothing is recording ({}); try again or pass --force",
                error.message
            ),
        )),
    }
}

/// Carries out `hex vocabulary add|remove` on the saved list. The reply's
/// `changed` says whether the list needs saving.
pub fn edit_vocabulary(
    list: &mut Vec<VocabularyEntry>,
    command: &ControlCommand,
) -> Result<ControlReply, ControlError> {
    let invalid = |message: String| ControlError::new("invalid-value", message);
    match command {
        ControlCommand::VocabularyAdd { term, sounds_like } => {
            let sounds_like: Vec<String> = sounds_like
                .iter()
                .flat_map(|spellings| vocabulary::parse_sounds_like(spellings))
                .collect();
            let existed = vocabulary::contains(list, term);
            Ok(
                match vocabulary::add(list, term, &sounds_like).map_err(invalid)? {
                    false => ControlReply::unchanged(format!(
                        "“{}” is already in the vocabulary",
                        term.trim()
                    )),
                    true if existed => {
                        ControlReply::done(format!("Added spellings to “{}”", term.trim()))
                    }
                    true => ControlReply::done(format!("Added “{}”", term.trim())),
                },
            )
        }
        ControlCommand::VocabularyRemove { term } => Ok(if vocabulary::remove(list, term) {
            ControlReply::done(format!("Removed “{}”", term.trim()))
        } else {
            ControlReply::unchanged(format!("“{}” isn't in the vocabulary", term.trim()))
        }),
        _ => Err(ControlError::new("internal", "not a vocabulary edit")),
    }
}

/// Process memory footprint on macOS; `None` where unmeasured.
pub fn memory_footprint() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: rusage_info_v4 is plain data that proc_pid_rusage fills.
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        // SAFETY: the pointer refers to a live, correctly sized buffer.
        let status = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V4,
                (&mut info as *mut libc::rusage_info_v4).cast(),
            )
        };
        (status == 0).then_some(info.ri_phys_footprint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_printing_stop_answers_through_its_reply_channel() {
        let wait = Duration::from_millis(50);
        let (reply, printed) = PrintReply::channel();
        reply.send(PrintedTranscript::Text("Ship it.".into()));
        assert_eq!(
            await_printed(&printed, wait),
            ControlReply::Transcript {
                text: "Ship it.".into()
            }
        );
        let (reply, printed) = PrintReply::channel();
        reply.send(PrintedTranscript::NoSpeech);
        assert!(matches!(
            await_printed(&printed, wait),
            ControlReply::Error { code, .. } if code == "no-speech"
        ));
        // A dictation cancelled or failed before it had text drops the reply.
        let (reply, printed) = PrintReply::channel();
        drop(reply);
        assert!(matches!(
            await_printed(&printed, wait),
            ControlReply::Error { code, .. } if code == "failed"
        ));
        let (_reply, printed) = PrintReply::channel();
        assert!(matches!(
            await_printed(&printed, wait),
            ControlReply::Error { code, .. } if code == "failed"
        ));
        // Only `stop` prints, and older requests without the field still parse.
        assert!(check_flags(CaptureAction::Start, true, false).is_err());
        assert!(check_flags(CaptureAction::Stop, true, false).is_ok());
        assert!(check_flags(CaptureAction::Toggle, false, true).is_ok());
        assert!(check_flags(CaptureAction::Stop, false, true).is_err());
        assert_eq!(
            serde_json::from_str::<ControlRequest>(
                r#"{"version":6,"command":"dictation","action":"stop"}"#
            )
            .unwrap()
            .command,
            ControlCommand::Dictation {
                action: CaptureAction::Stop,
                print: false,
                raw: false,
            }
        );
    }

    /// `cancel` discards the recording, and takes neither flag.
    #[test]
    fn cancel_parses_and_takes_no_flags() {
        let cancel = serde_json::from_str::<ControlRequest>(
            r#"{"version":6,"command":"dictation","action":"cancel"}"#,
        )
        .unwrap();
        let ControlCommand::Dictation { action, .. } = cancel.command else {
            panic!("a dictation command");
        };
        assert_eq!(action, CaptureAction::Cancel);
        assert_eq!(check_flags(action, false, false), Ok(CaptureAction::Cancel));
        assert!(check_flags(action, true, false).is_err());
        assert!(check_flags(action, false, true).is_err());
    }

    fn round_trip_request(command: ControlCommand) {
        let request = ControlRequest::new(command);
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<ControlRequest>(&json).unwrap(),
            request
        );
    }

    #[test]
    fn every_command_round_trips() {
        for command in [
            ControlCommand::Status,
            ControlCommand::Quit { force: true },
            ControlCommand::Restart { force: false },
            ControlCommand::Dictation {
                action: CaptureAction::Toggle,
                print: false,
                raw: false,
            },
            ControlCommand::Dictation {
                action: CaptureAction::Stop,
                print: true,
                raw: false,
            },
            ControlCommand::Dictation {
                action: CaptureAction::Start,
                print: false,
                raw: true,
            },
            ControlCommand::PasteLast,
            ControlCommand::SettingsGet { key: None },
            ControlCommand::SettingsSet {
                key: "release_microphone_while_idle".into(),
                value: "on".into(),
            },
            ControlCommand::SettingsApply {
                values: serde_json::Map::from_iter([("sound_effects".into(), false.into())]),
            },
            ControlCommand::WindowShow {
                pane: Some(Pane::History),
            },
            ControlCommand::WindowHide,
            ControlCommand::OnboardingReset,
            ControlCommand::WindowStory {
                story: "HUD/Processing".into(),
                variant: Some("2".into()),
                play: true,
            },
            ControlCommand::HistoryClear,
            ControlCommand::CheckForUpdates,
            ControlCommand::LaunchAtLogin {
                enabled: Some(true),
            },
            ControlCommand::VocabularyList,
            ControlCommand::VocabularyAdd {
                term: "cappuccino".into(),
                sounds_like: vec!["capuchin".into()],
            },
            ControlCommand::VocabularyRemove {
                term: "cappuccino".into(),
            },
        ] {
            round_trip_request(command);
        }
        let request: ControlRequest =
            serde_json::from_str(r#"{"version":6,"command":"vocabulary-add","term":"Rune"}"#)
                .unwrap();
        assert_eq!(
            request.command,
            ControlCommand::VocabularyAdd {
                term: "Rune".into(),
                sounds_like: vec![],
            }
        );
    }

    #[test]
    fn vocabulary_edits_report_whether_they_changed_the_list() {
        let add = |term: &str, sounds_like: &[&str]| ControlCommand::VocabularyAdd {
            term: term.into(),
            sounds_like: sounds_like.iter().map(|&text| text.into()).collect(),
        };
        let remove = |term: &str| ControlCommand::VocabularyRemove { term: term.into() };
        let mut list = Vec::new();
        assert_eq!(
            edit_vocabulary(&mut list, &add("cappuccino", &["capuchin, cap a chino"])).unwrap(),
            ControlReply::done("Added “cappuccino”")
        );
        assert_eq!(list[0].sounds_like, ["capuchin", "cap a chino"]);
        assert_eq!(
            edit_vocabulary(&mut list, &add("Cappuccino", &[])).unwrap(),
            ControlReply::unchanged("“Cappuccino” is already in the vocabulary")
        );
        assert_eq!(
            edit_vocabulary(&mut list, &add("cappuccino", &["cap you chino"])).unwrap(),
            ControlReply::done("Added spellings to “cappuccino”")
        );
        assert_eq!(
            edit_vocabulary(&mut list, &add(" ", &[])).unwrap_err().code,
            "invalid-value"
        );
        assert_eq!(
            edit_vocabulary(&mut list, &remove("CAPPUCCINO")).unwrap(),
            ControlReply::done("Removed “CAPPUCCINO”")
        );
        assert_eq!(
            edit_vocabulary(&mut list, &remove("cappuccino")).unwrap(),
            ControlReply::unchanged("“cappuccino” isn't in the vocabulary")
        );
        assert!(list.is_empty());
    }

    #[test]
    fn requests_are_flat_and_versioned() {
        let json =
            serde_json::to_value(ControlRequest::new(ControlCommand::Restart { force: true }))
                .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"version": 7, "command": "restart", "force": true})
        );
        // Optional fields may be left out.
        let request: ControlRequest =
            serde_json::from_str(r#"{"version":6,"command":"quit"}"#).unwrap();
        assert_eq!(request.command, ControlCommand::Quit { force: false });
        assert!(serde_json::from_str::<ControlRequest>(r#"{"version":6,"command":"x"}"#).is_err());
    }

    #[test]
    fn every_reply_round_trips() {
        for reply in [
            ControlReply::done("Started dictation"),
            ControlReply::unchanged("Already recording"),
            ControlReply::Status(Box::default()),
            ControlReply::Settings {
                settings: vec![settings::SettingValue {
                    key: "sound_effects".into(),
                    value: serde_json::json!(true),
                    kind: "bool".into(),
                    choices: vec![],
                    writable: true,
                    about: "Play tones".into(),
                    available: true,
                }],
            },
            ControlReply::Vocabulary {
                entries: vec![VocabularyEntry {
                    term: "cappuccino".into(),
                    sounds_like: vec!["capuchin".into()],
                }],
            },
            ControlReply::Transcript {
                text: "Ship it.".into(),
            },
            ControlReply::error("busy", "Hex is busy"),
        ] {
            let response = ControlResponse::from(reply);
            let json = serde_json::to_string(&response).unwrap();
            assert_eq!(
                serde_json::from_str::<ControlResponse>(&json).unwrap(),
                response
            );
        }
        let json = serde_json::to_value(ControlResponse::from(ControlReply::Transcript {
            text: "Ship it.".into(),
        }))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"version": 7, "status": "transcript", "text": "Ship it."})
        );
        let json =
            serde_json::to_value(ControlResponse::from(ControlReply::error("busy", "x"))).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"version": 7, "status": "error", "code": "busy", "message": "x"})
        );
    }

    #[test]
    fn a_request_from_another_version_is_refused() {
        let mut request = ControlRequest::new(ControlCommand::Status);
        assert!(check_version(&request).is_ok());
        request.version = 99;
        assert_eq!(
            check_version(&request).unwrap_err().code,
            "version-mismatch"
        );
    }

    #[test]
    fn quitting_waits_for_recordings_unless_forced() {
        assert!(check_quit(Ok(CaptureSnapshot::default()), false).is_ok());
        let busy = CaptureSnapshot {
            recording: None,
            pending_jobs: 1,
            microphone_unavailable: false,
        };
        assert_eq!(check_quit(Ok(busy), false).unwrap_err().code, "busy");
        assert!(check_quit(Ok(busy), true).is_ok());
        // A listener that can't answer may be recording: only --force quits.
        let unanswered = || {
            Err(ControlError::new(
                "unavailable",
                "the listener didn't answer",
            ))
        };
        assert_eq!(check_quit(unanswered(), false).unwrap_err().code, "busy");
        assert!(check_quit(unanswered(), true).is_ok());
    }

    #[test]
    fn capture_reports_derive_their_state() {
        let report = CaptureReport::from(CaptureSnapshot {
            recording: Some(CaptureKind::Dictation),
            pending_jobs: 2,
            microphone_unavailable: false,
        });
        assert_eq!(report.state, CaptureState::Recording);
        let report = CaptureReport::from(CaptureSnapshot {
            recording: None,
            pending_jobs: 2,
            microphone_unavailable: true,
        });
        assert_eq!(report.state, CaptureState::Transcribing);
    }
}
