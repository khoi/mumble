use serde_json::Value;

use super::{
    CONTROL_VERSION, ControlReply, ControlResponse, EXIT_NO_SPEECH, EXIT_NOT_RUNNING, EXIT_REFUSED,
    Output, fail,
};
use crate::control::{ModelInfo, StatusReport, settings::SettingValue};
use crate::dictation::vocabulary::VocabularyEntry;

/// Prints `reply` and returns its exit code.
pub(crate) fn print(reply: ControlReply, output: Output) -> u8 {
    let code = exit_code(&reply);
    if output.json {
        let response = ControlResponse {
            version: CONTROL_VERSION,
            reply,
        };
        match serde_json::to_string_pretty(&response) {
            Ok(json) => println!("{json}"),
            Err(error) => return fail(&error.to_string()),
        }
        return code;
    }
    match reply {
        ControlReply::Done { message, .. } => println!("{message}"),
        ControlReply::Status(report) => print!("{}", human_status(&report)),
        ControlReply::Settings { settings } => print!("{}", human_settings(&settings)),
        ControlReply::Models { models } => print!("{}", human_models(&models)),
        ControlReply::Vocabulary { entries } => print!("{}", human_vocabulary(&entries)),
        ControlReply::Transcript { text } => println!("{text}"),
        ControlReply::Error { message, .. } => eprintln!("hex: {message}"),
    }
    code
}

pub(super) fn exit_code(reply: &ControlReply) -> u8 {
    match reply {
        ControlReply::Error { code, .. } if code == "not-running" => EXIT_NOT_RUNNING,
        ControlReply::Error { code, .. } if code == "no-speech" => EXIT_NO_SPEECH,
        ControlReply::Error { .. } => EXIT_REFUSED,
        _ => 0,
    }
}

pub(super) fn human_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "none".into(),
        other => other.to_string(),
    }
}

pub(super) fn human_settings(settings: &[SettingValue]) -> String {
    if let [setting] = settings {
        return format!("{}\n", human_value(&setting.value));
    }
    let width = settings
        .iter()
        .map(|setting| setting.key.len())
        .max()
        .unwrap_or(0);
    settings
        .iter()
        .map(|setting| {
            let mut value = human_value(&setting.value);
            if value.len() > 60 {
                value = format!("{}…", value.chars().take(59).collect::<String>());
            }
            format!(
                "{:width$}  {value}{}\n",
                setting.key,
                if !setting.available {
                    "  (not on this OS)"
                } else if setting.writable {
                    ""
                } else {
                    "  (read-only)"
                }
            )
        })
        .collect()
}

pub(super) fn human_models(models: &[ModelInfo]) -> String {
    let width = models.iter().map(|model| model.id.len()).max().unwrap_or(0);
    let name_width = models
        .iter()
        .map(|model| model.name.len())
        .max()
        .unwrap_or(0);
    models
        .iter()
        .map(|model| {
            let languages = if model.languages.len() > 6 {
                format!("{} languages", model.languages.len())
            } else {
                model.languages.join(", ")
            };
            format!(
                "{} {:width$}  {:name_width$}  {:>8}  {:13}  {languages}{}\n",
                if model.selected { "*" } else { " " },
                model.id,
                model.name,
                model.size,
                if model.installed {
                    "installed"
                } else {
                    "not installed"
                },
                if model.source.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", model.source)
                },
            )
        })
        .collect()
}

pub(super) fn human_vocabulary(entries: &[VocabularyEntry]) -> String {
    if entries.is_empty() {
        return "The vocabulary is empty; `hex vocabulary add <term>` adds one\n".into();
    }
    entries
        .iter()
        .map(|entry| match entry.sounds_like.as_slice() {
            [] => format!("{}\n", entry.term),
            spellings => format!("{}  (sounds like {})\n", entry.term, spellings.join(", ")),
        })
        .collect()
}

pub(super) fn on(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

pub(super) fn human_status(report: &StatusReport) -> String {
    let memory = report
        .memory_footprint_bytes
        .map(|bytes| format!(", {} MB", bytes / 1_000_000))
        .unwrap_or_default();
    let capture = match report.capture.recording {
        Some(kind) => format!(
            "recording {}",
            serde_json::to_value(kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default()
        ),
        None => serde_json::to_value(report.capture.state)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
    };
    let pending = match report.capture.pending_jobs {
        0 => String::new(),
        jobs => format!(" ({jobs} pending)"),
    };
    let model = &report.model;
    let mut model_line = format!(
        "{} ({}){}",
        model.name,
        model.language,
        if model.installed {
            ""
        } else {
            ", not installed"
        }
    );
    if let Some(preparing) = &model.preparing {
        model_line.push_str(&format!(
            "; preparing {preparing} ({} MB)",
            model.downloaded_bytes / 1_000_000
        ));
    }
    if let Some(error) = &model.error {
        model_line.push_str(&format!("; error: {error}"));
    }
    let features = &report.features;
    let permissions = &report.permissions;
    let permission_line = if permissions.ready {
        "ready".to_string()
    } else {
        format!(
            "microphone {}, accessibility {}, model {}",
            permissions.microphone,
            permissions.accessibility,
            if permissions.transcription_model {
                "ready"
            } else {
                "missing"
            }
        )
    };
    let window = match (&report.window.pane, report.window.open) {
        (Some(pane), true) => format!("open ({pane})"),
        (None, true) => "open".into(),
        _ => "closed".into(),
    };
    format!(
        "Hex {} (pid {}{memory})\n\
         Listener     {}\n\
         Capture      {capture}{pending}\n\
         Model        {model_line}\n\
         Features     release mic while idle {}, history {}\n\
         Permissions  {permission_line}\n\
         Window       {window}\n\
         Update       {}\n\
         Labs         {}\n",
        report.app_version,
        report.pid,
        if report.listener.running {
            format!("running: {}", report.listener.status)
        } else {
            format!("waiting: {}", report.listener.status)
        },
        on(features.release_microphone_while_idle),
        features.history_retention,
        report.update,
        match report.labs.as_slice() {
            [] => "none".to_string(),
            names => names.join(", "),
        },
    )
}
