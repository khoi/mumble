pub(crate) mod app;
pub(crate) mod audio_control;
pub(crate) mod context;
pub(crate) mod dictation;
pub(crate) mod gpu_fallback;
pub(crate) mod input;
pub(crate) mod login_item;
pub(crate) mod paste;
pub(crate) mod rune;

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use color_eyre::Result;
use color_eyre::eyre::{WrapErr, bail};
use windows_sys::Win32::System::Console::{
    FreeConsole, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};

use crate::audio::{AudioInput, AudioInputEvent};
use crate::desktop::events::{
    DictationPhase, EventLog, TranscriptPhase, VoiceEvent, VoiceState, now_ms,
};
use crate::transcription::engine::TranscriptionEngine;

#[derive(Parser)]
#[command(
    name = "hex",
    version = crate::VERSION,
    about = "Local Hex dictation for Windows",
    long_about = "Local Hex dictation for Windows.\n\n\
        Control commands (status, dictation, settings, model, window, …) drive the one running \
        Hex over its owner-only pipe and never launch it, except `start` and `restart`. Every \
        one takes --json for a versioned reply (docs/cli.md). Exit codes: 0 done or nothing to \
        change, 1 refused or failed, 2 usage, 3 Hex isn't running.",
    after_help = "Run `hex <command> --help` for details, or `hex settings list` for every setting."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(flatten)]
    Control(crate::control::cli::ControlCli),
    /// Open the native Windows application.
    App {
        /// Start the resident listener without showing the Settings window.
        #[arg(long)]
        hidden: bool,
        /// Arguments for a headless render, such as `--snap out.png --scene modes`.
        #[arg(last = true)]
        rune_args: Vec<String>,
    },
    /// Listen globally: hold the configured shortcut to dictate and release to paste.
    Listen {
        /// Select an input device by a case-insensitive name fragment.
        #[arg(long)]
        device: Option<String>,
    },
    /// Record one microphone capture and transcribe it with the cloud service.
    Record {
        /// Select an input device by a case-insensitive name fragment.
        #[arg(long)]
        device: Option<String>,
        /// Copy a successful transcript to the clipboard.
        #[arg(long)]
        copy: bool,
    },
    /// List microphone devices visible through WASAPI.
    Devices,
}

/// Loads DLLs only from System32 and Hex's own folder, never the current
/// folder or PATH. Another app's incompatible `dxcompiler.dll` on PATH
/// crashed Hex at its first shader compile (#587), and a DLL planted on PATH
/// must not load into Hex. Hex's own imports (vulkan-1.dll included) loaded
/// before this; Windows, the GPU drivers, and COM load theirs by full path.
pub(crate) fn restrict_dll_search() {
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
    };
    // SAFETY: changes this process's DLL search order; no pointers.
    if unsafe {
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)
    } == 0
    {
        eprintln!(
            "hex: SetDefaultDllDirectories failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

pub fn run(shutdown: &'static AtomicBool) -> Result<()> {
    color_eyre::install()?;
    // Parse first so `--version` and `--help` exit without creating app data.
    let command = Cli::parse().command.unwrap_or(Command::App {
        hidden: false,
        rune_args: Vec::new(),
    });
    // Control and read commands talk to the running Hex or read files: no
    // log, lock, or Hex of their own.
    if let Command::Control(command) = command {
        crate::control::cli::finish(crate::control::cli::run(command));
    }
    // A headless render (`--snap`) draws fixtures without starting the
    // runtime or touching the data folder, and prints to the terminal that
    // ran it.
    if let Command::App { rune_args, .. } = &command
        && !rune_args.is_empty()
    {
        crate::rune_ui::render_app(rune_args.clone()).run();
        return Ok(());
    }
    let log_dir = crate::app_paths::init_process_logging(shutdown)?;
    // 2.3.1's updater installed this build into its own folder.
    if matches!(command, Command::App { .. }) && crate::updater::legacy::take_over() {
        return Ok(());
    }

    let event_path = log_dir.join("live.ndjson");
    match command {
        Command::App { hidden, .. } => {
            detach_from_console();
            crate::windows::rune::open(event_path, shutdown, hidden)
        }
        Command::Listen { device } => {
            shutdown.store(false, Ordering::Relaxed);
            let _instance = crate::desktop::instance::acquire("windows-listener")?;
            crate::labs::log_launch();
            let settings = crate::app_settings::AppSettings::load()?;
            let history = crate::history::History::open_default(settings.history_retention)?;
            let config = crate::windows::dictation::WindowsDictationConfig {
                device,
                history: Some(history),
                fallback_to_default_device: false,
                ..crate::windows::dictation::WindowsDictationConfig::from_settings(&settings)
            };
            let _personal_commands =
                crate::app_settings::transformations_used(&settings.dictation_processing)
                    .then(|| {
                        crate::personal_commands::PersonalCommands::start(
                            config.transformations.clone(),
                        )
                    })
                    .flatten();
            // Settings a foreground listener loaded stay as they are, and
            // only its shortcut drives it.
            crate::windows::dictation::run(
                &event_path,
                config,
                std::sync::mpsc::channel().1,
                shutdown,
            )
        }
        Command::Record { device, copy } => capture(&event_path, device.as_deref(), copy, shutdown),
        Command::Devices => {
            for device in crate::audio::input_device_names()? {
                println!("{device}");
            }
            Ok(())
        }
        Command::Control(_) => unreachable!("handled before logging"),
    }
}

fn detach_from_console() {
    // The binary also exposes terminal-oriented subcommands, so it cannot use a
    // crate-wide Windows GUI subsystem. Detach only for the native app: an
    // Explorer-launched console disappears, while a parent terminal remains.
    // FreeConsole leaves the standard handles holding closed console handle
    // values that Windows soon reuses for unrelated objects, so a later print
    // panics (a read-only file) or writes into whatever file took the value.
    // Null handles make std discard the output; redirected handles are kept.
    unsafe {
        let console_handles: Vec<_> = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .filter(|&id| {
                let mut mode = 0;
                GetConsoleMode(GetStdHandle(id), &mut mode) != 0
            })
            .collect();
        FreeConsole();
        for id in console_handles {
            SetStdHandle(id, std::ptr::null_mut());
        }
    }
}

fn capture(
    event_path: &Path,
    device: Option<&str>,
    copy: bool,
    shutdown: &AtomicBool,
) -> Result<()> {
    shutdown.store(false, Ordering::Relaxed);
    let _instance = crate::desktop::instance::acquire("windows-capture")?;
    // Applies the saved transcription provider and language to the runtime,
    // which the engine reads when it sends the clip to the cloud service.
    let _ = crate::app_settings::AppSettings::load();
    let engine = TranscriptionEngine::start()?;
    let input = match device {
        Some(name) => AudioInput::open_matching(name)?,
        None => AudioInput::open_system_default()?,
    };
    let sample_rate = input.sample_rate;
    let device_name = input.device_name.clone();
    let events = EventLog::create(event_path)?;

    wait_for_enter(&format!(
        "Ready on {device_name}. Press Enter to start recording."
    ))?;
    if shutdown.load(Ordering::Relaxed) {
        return Ok(());
    }

    events.emit(&VoiceEvent::SessionStarted {
        timestamp_ms: now_ms(),
    })?;
    emit_state(&events, VoiceState::Dictating, &device_name)?;
    events.dictation(DictationPhase::Started, "")?;

    let stopped = Arc::new(AtomicBool::new(false));
    let reader_stopped = stopped.clone();
    std::thread::Builder::new()
        .name("windows-capture-console".into())
        .spawn(move || {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            reader_stopped.store(true, Ordering::Release);
        })?;
    println!("Recording. Press Enter to stop; Ctrl-C cancels.");

    let mut samples = Vec::new();
    while !stopped.load(Ordering::Acquire) && !shutdown.load(Ordering::Relaxed) {
        match input.recv_timeout(Duration::from_millis(50)) {
            AudioInputEvent::Chunk { samples: chunk, .. } => samples.extend(chunk),
            AudioInputEvent::Timeout => {}
            AudioInputEvent::StreamFailed(error) => {
                let error = format!("microphone stream stopped: {error}");
                fail_capture(&events, &device_name, &error)?;
                bail!(error);
            }
        }
    }
    drop(input);

    if shutdown.load(Ordering::Relaxed) {
        events.dictation(DictationPhase::Cancelled, "")?;
        emit_state(&events, VoiceState::Stopping, &device_name)?;
        println!("Capture canceled.");
        return Ok(());
    }

    let audio_ms = samples.len() as u64 * 1_000 / u64::from(sample_rate);
    if audio_ms < crate::dictation::MINIMUM_HOLD_DURATION.as_millis() as u64 {
        events.dictation(DictationPhase::Discarded, "")?;
        emit_state(&events, VoiceState::Stopping, &device_name)?;
        println!("Capture shorter than 300 ms; discarded.");
        return Ok(());
    }

    emit_state(&events, VoiceState::Transcribing, &device_name)?;
    events.dictation(DictationPhase::Transcribing, "")?;
    let started = Instant::now();
    let samples = crate::dictation::resample_for_parakeet(&samples, sample_rate);
    let text = match engine.finish(None, samples).and_then(|pending| {
        pending
            .wait()
            .map_err(|error| color_eyre::eyre::eyre!(error))
    }) {
        Ok(transcribed) => transcribed.text.trim().to_string(),
        Err(error) => {
            fail_capture(&events, &device_name, &format!("{error:#}"))?;
            return Err(error);
        }
    };
    if text.is_empty() {
        fail_capture(&events, &device_name, "transcription was empty")?;
        bail!("transcription was empty");
    }
    let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    events.emit(&VoiceEvent::Transcript {
        timestamp_ms: now_ms(),
        phase: TranscriptPhase::Completed,
        latency_ms,
        text: text.clone(),
    })?;

    if copy {
        let copy_result = arboard::Clipboard::new()
            .wrap_err("could not open the Windows clipboard")
            .and_then(|mut clipboard| {
                clipboard
                    .set_text(text.clone())
                    .wrap_err("could not copy the transcript")
            });
        if let Err(error) = copy_result {
            fail_capture(&events, &device_name, &format!("{error:#}"))?;
            return Err(error);
        }
    }
    emit_state(&events, VoiceState::Stopping, &device_name)?;
    events.flush()?;
    tracing::info!(
        audio_ms,
        latency_ms,
        copied = copy,
        "completed Windows capture"
    );
    println!("{text}");
    if copy {
        eprintln!("Copied to clipboard.");
    }
    Ok(())
}

fn wait_for_enter(message: &str) -> Result<()> {
    print!("{message}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(())
}

fn emit_state(events: &EventLog, state: VoiceState, device: &str) -> Result<()> {
    events.emit(&VoiceEvent::State {
        timestamp_ms: now_ms(),
        state,
        device: device.into(),
    })?;
    Ok(())
}

fn fail_capture(events: &EventLog, device: &str, error: &str) -> Result<()> {
    events.dictation(DictationPhase::Failed(error.into()), "")?;
    emit_state(events, VoiceState::Stopping, device)
}
