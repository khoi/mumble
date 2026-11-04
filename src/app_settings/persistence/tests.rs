use super::*;
use crate::app_settings::RecordingAudioBehavior;
use serde_json::json;
use std::sync::atomic::Ordering;

struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    fn new(data: &[u8]) -> Self {
        let sequence = crate::app_settings::SETTINGS_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "hex-settings-recovery-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(&path, data).unwrap();
        Self { path }
    }

    fn backups(&self) -> Vec<PathBuf> {
        fs::read_dir(self.path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".unreadable-")
            })
            .collect()
    }
}

impl Drop for SettingsFile {
    fn drop(&mut self) {
        fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
    }
}

#[test]
fn invalid_enum_resets_only_that_field_and_leaves_disk_alone() {
    let original = br#"{"recording_audio_behavior":"future-mode","sound_effect_volume":0.37,"labs":{"live_text":true}}"#;
    let file = SettingsFile::new(original);
    let settings = AppSettings::load_from(&file.path).unwrap();
    assert_eq!(
        settings.recording_audio_behavior,
        RecordingAudioBehavior::default()
    );
    assert_eq!(settings.sound_effect_volume, 0.37);
    assert!(settings.labs.live_text);
    assert_eq!(fs::read(&file.path).unwrap(), original);
    assert!(file.backups().is_empty());
}

#[test]
fn unbound_optional_shortcuts_stay_unbound_after_saving() {
    let settings = AppSettings {
        paste_last_hotkey: None,
        raw_dictation_hotkey: None,
        second_dictation_hotkey: None,
        ..Default::default()
    };
    let value = serde_json::to_value(&settings).unwrap();
    // Omitting Paste Last would restore a bound default.
    assert_eq!(value.get("paste_last_hotkey"), Some(&Value::Null));
    let loaded: AppSettings = serde_json::from_value(value).unwrap();
    assert!(loaded.paste_last_hotkey.is_none());
    assert!(loaded.raw_dictation_hotkey.is_none());
    assert!(loaded.second_dictation_hotkey.is_none());
}

#[test]
fn invalid_transcription_provider_keeps_unrelated_settings() {
    let file = SettingsFile::new(
        &serde_json::to_vec(&json!({
            "transcription": {"provider": "not-a-service", "language": "de"},
            "sound_effect_volume": 0.37
        }))
        .unwrap(),
    );
    let settings = AppSettings::load_from(&file.path).unwrap();
    // Only the unknown provider resets; the language and siblings survive.
    assert_eq!(
        settings.transcription.provider,
        AppSettings::default().transcription.provider
    );
    assert_eq!(settings.transcription.language, "de");
    assert_eq!(settings.sound_effect_volume, 0.37);
    settings.write_to(&file.path).unwrap();
    assert_eq!(file.backups().len(), 1);
}

#[test]
fn wrong_nested_type_and_multiple_bad_fields_keep_siblings() {
    let file = SettingsFile::new(br#"{
        "dictation_processing":{"default_mode":{"applications":["Xcode"],"post_processing":{"enabled":true,"deadline_seconds":"soon","prompt":"Tidy it"}}},
        "sound_effects":[], "recording_audio_behavior":"unknown", "microphone":"USB mic"
    }"#);
    let settings = AppSettings::load_from(&file.path).unwrap();
    let mode = &settings.dictation_processing.default_mode;
    assert_eq!(mode.applications, ["Xcode"]);
    assert!(mode.post_processing.enabled);
    assert_eq!(mode.post_processing.prompt, "Tidy it");
    assert_eq!(
        mode.post_processing.deadline_seconds,
        crate::app_settings::DictationPostProcessing::default().deadline_seconds
    );
    assert!(settings.sound_effects);
    assert_eq!(settings.microphone.as_deref(), Some("USB mic"));
}

#[test]
fn bad_entries_do_not_discard_good_entries() {
    let (settings, repaired) = decode(
        br#"{"dictation_processing":{"modes":[
        {"name":"Code","applications":["Xcode"]},
        {"name":"Mail","applications":"Mail"}
    ]}}"#,
    );
    assert!(repaired);
    let modes = &settings.dictation_processing.modes;
    assert_eq!(modes.len(), 2);
    assert_eq!(modes[0].applications, ["Xcode"]);
    assert_eq!(modes[1].name, "Mail");
    assert!(modes[1].applications.is_empty());
}

#[test]
fn unknown_keys_are_ignored_without_repair() {
    let (settings, repaired) =
        decode(br#"{"future":{"flag":true},"labs":{"future":42,"live_text":true}}"#);
    assert!(!repaired);
    assert!(settings.labs.live_text);
}

#[test]
fn first_save_preserves_exact_original_once_even_after_reload() {
    let original =
        br#"{ "sound_theme":"future", "future_key": [1,2,3], "sound_effect_volume":0.37 }"#;
    let file = SettingsFile::new(original);
    let settings = AppSettings::load_from(&file.path).unwrap();
    settings.write_to(&file.path).unwrap();
    let backups = file.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let loaded = AppSettings::load_from(&file.path).unwrap();
    assert_eq!(loaded.sound_effect_volume, 0.37);
    loaded.write_to(&file.path).unwrap();
    assert_eq!(file.backups(), backups);
}

#[test]
fn malformed_json_and_non_objects_launch_with_defaults_and_are_backed_up() {
    for original in [&b"{not json"[..], &b"null"[..], &b"[]"[..], &b"\xff"[..]] {
        let file = SettingsFile::new(original);
        let settings = AppSettings::load_from(&file.path).unwrap();
        assert_eq!(settings, AppSettings::default());
        assert_eq!(fs::read(&file.path).unwrap(), original);
        settings.write_to(&file.path).unwrap();
        assert_eq!(file.backups().len(), 1);
        assert_eq!(fs::read(&file.backups()[0]).unwrap(), original);
    }
}

#[test]
fn backup_failure_blocks_save_and_preserves_original() {
    let original = b"{bad json";
    let file = SettingsFile::new(original);
    fs::create_dir(backup_path(&file.path, original)).unwrap();
    assert!(AppSettings::default().write_to(&file.path).is_err());
    assert_eq!(fs::read(&file.path).unwrap(), original);
}

#[test]
fn interrupted_save_reuses_the_complete_backup_but_refuses_a_partial_one() {
    let original = b"{bad json";
    let file = SettingsFile::new(original);
    backup(&file.path, original).unwrap();
    AppSettings::default().write_to(&file.path).unwrap();
    assert_eq!(file.backups().len(), 1);
    fs::write(&file.path, original).unwrap();
    fs::write(&file.backups()[0], b"partial").unwrap();
    assert!(AppSettings::default().write_to(&file.path).is_err());
    assert_eq!(fs::read(&file.path).unwrap(), original);
}

#[test]
fn save_checks_current_disk_bytes_not_only_what_was_loaded() {
    let file = SettingsFile::new(b"{}");
    let settings = AppSettings::load_from(&file.path).unwrap();
    let damaged = br#"{"sound_effects":"bad"}"#;
    fs::write(&file.path, damaged).unwrap();
    settings.write_to(&file.path).unwrap();
    assert_eq!(fs::read(&file.backups()[0]).unwrap(), damaged);
}

#[test]
fn out_of_range_setting_cannot_prevent_launch() {
    let file = SettingsFile::new(
        &serde_json::to_vec(&json!({
            "recording_duck_volume": 2.0, "sound_effect_volume": 0.37
        }))
        .unwrap(),
    );
    let settings = AppSettings::load_from(&file.path).unwrap();
    assert_eq!(settings.recording_duck_volume, 0.2);
    assert_eq!(settings.sound_effect_volume, 0.37);
    settings.write_to(&file.path).unwrap();
    assert_eq!(file.backups().len(), 1);
}

#[test]
fn unreadable_file_does_not_block_load_or_get_overwritten() {
    let file = SettingsFile::new(b"{}");
    fs::remove_file(&file.path).unwrap();
    fs::create_dir(&file.path).unwrap();
    assert_eq!(
        AppSettings::load_from(&file.path).unwrap(),
        AppSettings::default()
    );
    assert!(AppSettings::default().write_to(&file.path).is_err());
    assert!(file.path.is_dir());
}

#[test]
fn a_managed_lock_reads_off_and_never_rewrites_the_saved_switch() {
    use crate::policy::{Lock, Policy};
    let file = SettingsFile::new(br#"{"labs":{"live_text":true}}"#);
    let policy = Policy::default().with(Lock::Labs);
    let (mut settings, _) = AppSettings::read_managed_from(&file.path, policy).unwrap();
    assert!(!settings.labs.live_text);

    // Saving another change, or the locked switch itself, keeps the file's.
    settings.sound_effects = false;
    settings.labs.playground = true;
    settings.write_managed_to(&file.path, policy).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&file.path).unwrap()).unwrap();
    assert_eq!(saved["labs"]["live_text"], json!(true));
    assert_eq!(saved["labs"]["playground"], json!(false));
    assert_eq!(saved["sound_effects"], json!(false));

    // Without the policy the user's own choices come back.
    let (unmanaged, _) = AppSettings::read_managed_from(&file.path, Policy::default()).unwrap();
    assert!(unmanaged.labs.live_text);
    assert!(!unmanaged.labs.playground);
}
