//! The `mumble` control client: parses a command, sends it to the running Mumble,
//! and prints the answer for a person or, with `--json`, the versioned reply.
//!
//! Exit codes: 0 when the command ran (or there was nothing to change), 1
//! when Mumble refused or failed it, 2 for usage errors (clap), 3 when Mumble
//! isn't running, and 4 when `dictation stop --print` heard no speech. Nothing here launches Mumble except `start` and `restart`.

use std::io::Write;
mod human;

use clap::{Args, Subcommand};

use super::{
    CONTROL_VERSION, CaptureAction, ControlCommand, ControlReply, ControlRequest, ControlResponse,
    Pane,
};

pub const EXIT_REFUSED: u8 = 1;
pub const EXIT_NOT_RUNNING: u8 = 3;
pub const EXIT_NO_SPEECH: u8 = 4;

#[derive(Args, Clone, Copy, Debug, Default)]
pub struct Output {
    /// Print the versioned JSON reply (see docs/cli.md).
    #[arg(long)]
    pub json: bool,
}

/// The control commands at the top of `mumble`.
#[derive(Subcommand, Debug)]
pub enum ControlCli {
    /// Show what the running Mumble is doing.
    ///
    /// Recording, model, features, permissions, window, version, pid, and
    /// memory footprint.
    Status {
        #[command(flatten)]
        output: Output,
    },
    /// Open Mumble if it isn't running, and wait until it answers.
    Start {
        #[command(flatten)]
        output: Output,
    },
    /// Quit Mumble.
    ///
    /// Refused while a dictation is in progress unless --force.
    Quit {
        /// Quit even while recording; the recording is lost.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Quit and reopen Mumble (or open it if it isn't running).
    Restart {
        /// Restart even while recording; the recording is lost.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Start, stop, toggle, or cancel a dictation, as the shortcut would.
    ///
    /// `start` locks the recording: `stop` or the shortcut finishes it,
    /// `cancel` discards it. Repeating a command changes nothing.
    Dictation {
        #[arg(value_enum)]
        action: CaptureAction,
        /// With `stop`: wait for the transcript and print it instead of
        /// pasting it. Exits 4, printing nothing, when no speech was heard.
        #[arg(long)]
        print: bool,
        /// With `start` or `toggle`: paste the transcript as heard,
        /// skipping the Output pane's steps, like Dictate without
        /// processing.
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Paste the last dictation at the current focus (Option-Shift-V).
    PasteLast {
        #[command(flatten)]
        output: Output,
    },
    /// Read and change any persisted setting, shortcuts included.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Open the Mumble window at a pane, or close it.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },
    /// Replay the first-run onboarding.
    Onboarding {
        #[command(subcommand)]
        command: OnboardingCommand,
    },
    /// Check for a Mumble update now; Mumble shows what it finds.
    CheckForUpdates {
        #[command(flatten)]
        output: Output,
    },
    /// Show Launch at Login, or turn it on or off.
    LaunchAtLogin {
        #[arg(value_enum)]
        state: Option<Toggle>,
        #[command(flatten)]
        output: Output,
    },
    /// List, add, and remove vocabulary terms.
    Vocabulary {
        #[command(subcommand)]
        command: VocabularyCommand,
    },
    /// Read retained dictation history without starting the app, or clear
    /// it in the running Mumble (`mumble history clear --yes`).
    #[command(args_conflicts_with_subcommands = true)]
    History(crate::control::transcript_cli::HistoryArgs),
    /// Manage the ~/.config/mumble personal transformations workspace.
    Commands {
        #[command(subcommand)]
        command: CommandsCommand,
    },
    /// List the Labs features, or turn one on or off.
    Labs {
        #[command(subcommand)]
        command: Option<crate::labs::LabsCommand>,
    },
    /// Replaced by `mumble labs`.
    #[command(hide = true)]
    Experiments {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Print Mumble's recent log lines; works without Mumble running.
    Logs {
        /// Read process.log (Rust and native diagnostics) instead of the
        /// live.ndjson event log.
        #[arg(long)]
        process: bool,
        /// How many lines from the end.
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Keep printing new lines until interrupted.
        #[arg(short, long)]
        follow: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CommandsCommand {
    /// Create ~/.config/mumble with a starter config, or refresh its typings.
    Init,
    /// Load ~/.config/mumble's config as Mumble would and report what it registers
    /// or why it fails.
    Check,
}

#[derive(Subcommand, Debug)]
pub enum SettingsCommand {
    /// Every setting with its value, kind, and whether it can be set.
    List {
        #[command(flatten)]
        output: Output,
    },
    /// One setting's value, or every setting under a prefix (`labs`).
    Get {
        key: String,
        #[command(flatten)]
        output: Output,
    },
    /// Change a setting as the window would.
    ///
    /// Booleans take on/off, shortcuts take text such as `option+shift+v`,
    /// `right-command`, `fn`, or `f5`, and optional settings take `none`.
    Set {
        key: String,
        value: String,
        #[command(flatten)]
        output: Output,
    },
    /// Set several settings at once from a JSON object of `key: value`,
    /// all or none; other settings keep their values.
    ///
    /// Values are JSON (`true`, `0.4`, `"option+space"`, `null` for none).
    /// With Mumble stopped, writes settings.json directly.
    Apply {
        /// A JSON file, or `-` for standard input.
        file: std::path::PathBuf,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum OnboardingCommand {
    /// Open the first-run onboarding in the window now, as on a new install.
    ///
    /// Forgets that onboarding finished. Settings, history, models, and permissions stay,
    /// so their steps pass at once.
    #[command(visible_alias = "restart")]
    Reset {
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum WindowCommand {
    /// Open the window, at a pane if given.
    Show {
        #[arg(value_enum)]
        pane: Option<Pane>,
        #[command(flatten)]
        output: Output,
    },
    /// Close the window. Mumble keeps running in the menu bar.
    Hide {
        #[command(flatten)]
        output: Output,
    },
    /// Labs: open the window on a playground story by its path, name, or
    /// slug (`Prototypes/HUD/Processing`, `prototypes-hud-processing`).
    Story {
        story: String,
        /// The variant, by number from 1 or by name.
        #[arg(long)]
        variant: Option<String>,
        /// Run the story's `r` trigger once it's open, as pressing R does.
        #[arg(long)]
        play: bool,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum VocabularyCommand {
    /// Every term with its "sounds like" spellings.
    List {
        #[command(flatten)]
        output: Output,
    },
    /// Add a term, or more spellings to a term already in the list.
    Add {
        term: String,
        /// Spellings a transcript may show instead, comma-separated
        /// (`--sounds-like "capuchin,cap a chino"`).
        #[arg(long, value_name = "SPELLINGS")]
        sounds_like: Option<String>,
        #[command(flatten)]
        output: Output,
    },
    /// Remove a term and its spellings.
    Remove {
        term: String,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Toggle {
    On,
    Off,
}

/// Exits with `code` once stdout is flushed.
pub fn finish(code: u8) -> ! {
    let _ = std::io::stdout().flush();
    std::process::exit(code.into())
}

pub fn run(command: ControlCli) -> u8 {
    match command {
        ControlCli::Status { output } => send(ControlCommand::Status, output),
        ControlCli::Start { output } => start(output),
        ControlCli::Quit { force, output } => send(ControlCommand::Quit { force }, output),
        ControlCli::Restart { force, output } => {
            if !running() {
                #[cfg(target_os = "linux")]
                if crate::linux::service::instance_answers() {
                    return print(restart_older_mumble(), output);
                }
                return start(output);
            }
            #[cfg(target_os = "linux")]
            if let Err(error) = crate::linux::service::prepare_restart() {
                return print(ControlReply::error("failed", format!("{error:#}")), output);
            }
            #[cfg(target_os = "macos")]
            return send(ControlCommand::Restart { force }, output);
            #[cfg(not(target_os = "macos"))]
            match call(ControlRequest::new(ControlCommand::Restart { force })) {
                Ok(ControlResponse {
                    reply: ControlReply::Done { .. },
                    ..
                }) => print(reopen_after_restart(), output),
                Ok(response) => print(response.reply, output),
                Err(reply) => print(reply, output),
            }
        }
        ControlCli::Dictation {
            action,
            print,
            raw,
            output,
        } => {
            if let Err(error) = super::check_flags(action, print, raw) {
                eprintln!("mumble: {}", error.message);
                return 2;
            }
            send(ControlCommand::Dictation { action, print, raw }, output)
        }
        ControlCli::PasteLast { output } => send(ControlCommand::PasteLast, output),
        ControlCli::Settings { command } => settings(command),
        ControlCli::Window { command } => match command {
            WindowCommand::Show { pane, output } => {
                send(ControlCommand::WindowShow { pane }, output)
            }
            WindowCommand::Hide { output } => send(ControlCommand::WindowHide, output),
            WindowCommand::Story {
                story,
                variant,
                play,
                output,
            } => send(
                ControlCommand::WindowStory {
                    story,
                    variant,
                    play,
                },
                output,
            ),
        },
        ControlCli::Onboarding {
            command: OnboardingCommand::Reset { output },
        } => send(ControlCommand::OnboardingReset, output),
        ControlCli::CheckForUpdates { output } => send(ControlCommand::CheckForUpdates, output),
        ControlCli::LaunchAtLogin { state, output } => send(
            ControlCommand::LaunchAtLogin {
                enabled: state.map(|state| matches!(state, Toggle::On)),
            },
            output,
        ),
        ControlCli::Vocabulary { command } => match command {
            VocabularyCommand::List { output } => send(ControlCommand::VocabularyList, output),
            VocabularyCommand::Add {
                term,
                sounds_like,
                output,
            } => send(
                ControlCommand::VocabularyAdd {
                    term,
                    sounds_like: sounds_like.into_iter().collect(),
                },
                output,
            ),
            VocabularyCommand::Remove { term, output } => {
                send(ControlCommand::VocabularyRemove { term }, output)
            }
        },
        ControlCli::Logs {
            process,
            lines,
            follow,
        } => logs(process, lines, follow),
        ControlCli::History(crate::control::transcript_cli::HistoryArgs {
            action: Some(crate::control::transcript_cli::HistoryAction::Clear { yes, output }),
            ..
        }) => clear_history(yes, output),
        ControlCli::History(args) => report(crate::control::transcript_cli::history(&args)),
        ControlCli::Commands { command } => report(match command {
            CommandsCommand::Init => crate::personal_commands::initialize_workspace()
                .map(|workspace| println!("{}", workspace.display())),
            CommandsCommand::Check => {
                crate::personal_commands::check_workspace().map(|report| println!("{report}"))
            }
        }),
        ControlCli::Labs { command } => report(crate::labs::run(command.as_ref())),
        ControlCli::Experiments { .. } => report(crate::labs::run_retired_experiments()),
    }
}

/// The exit code of a command that runs here rather than in Mumble.
fn report(result: color_eyre::Result<()>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(error) => fail(&format!("{error:#}")),
    }
}

pub fn settings(command: SettingsCommand) -> u8 {
    match command {
        SettingsCommand::List { output } => send(ControlCommand::SettingsGet { key: None }, output),
        SettingsCommand::Get { key, output } => {
            send(ControlCommand::SettingsGet { key: Some(key) }, output)
        }
        SettingsCommand::Set { key, value, output } => {
            send(ControlCommand::SettingsSet { key, value }, output)
        }
        SettingsCommand::Apply { file, output } => apply_settings(&file, output),
    }
}

/// `mumble settings apply`: the running Mumble saves and adopts the settings, so
/// it never overwrites them from memory; with no Mumble running they are
/// written to settings.json here.
fn apply_settings(file: &std::path::Path, output: Output) -> u8 {
    use std::io::Read;

    let mut text = String::new();
    let read = if file == std::path::Path::new("-") {
        std::io::stdin().read_to_string(&mut text).map(drop)
    } else {
        std::fs::read_to_string(file).map(|read| text = read)
    };
    if let Err(error) = read {
        return fail(&format!("couldn't read {}: {error}", file.display()));
    }
    let values: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&text) {
        Ok(values) => values,
        Err(error) => return fail(&format!("expected a JSON object of settings: {error}")),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    loop {
        if running() {
            return send(ControlCommand::SettingsApply { values }, output);
        }
        // A Mumble that holds the service lock may have read settings.json
        // already; wait for it to answer instead of writing behind it.
        #[cfg(target_os = "linux")]
        let lock = crate::desktop::instance::acquire("service");
        #[cfg(not(target_os = "linux"))]
        let lock: color_eyre::Result<()> = Ok(());
        match lock {
            Ok(_lock) if !running() => return print(apply_offline(&values), output),
            Ok(_) => {}
            #[cfg(target_os = "linux")]
            Err(_) if crate::linux::service::instance_answers() => {
                return print(older_mumble(), output);
            }
            Err(_) if std::time::Instant::now() >= deadline => {
                return print(
                    ControlReply::error(
                        "busy",
                        "Mumble is starting but didn't answer; run `mumble settings apply` again",
                    ),
                    output,
                );
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

fn apply_offline(values: &serde_json::Map<String, serde_json::Value>) -> ControlReply {
    let result = crate::app_settings::AppSettings::load()
        .map_err(|error| {
            super::ControlError::new("internal", format!("couldn't read settings: {error:#}"))
        })
        .and_then(|before| {
            let change =
                super::settings::apply_all(&before, values, &crate::rune_ui::shortcut_rules())?;
            if !change.changed(&before) {
                return Ok(ControlReply::unchanged(
                    "Every setting already has its value",
                ));
            }
            change.settings.save().map_err(|error| {
                super::ControlError::new("internal", format!("couldn't save: {error:#}"))
            })?;
            Ok(ControlReply::done(change.message))
        });
    result.unwrap_or_else(ControlReply::from)
}

/// `mumble history clear`; the confirmation is asked here, before Mumble is.
pub fn clear_history(confirm: bool, output: Output) -> u8 {
    if !confirm {
        return print(
            ControlReply::error(
                "confirmation-required",
                "this deletes every retained dictation; pass --yes to clear history",
            ),
            output,
        );
    }
    send(ControlCommand::HistoryClear, output)
}

pub fn send(command: ControlCommand, output: Output) -> u8 {
    match call(ControlRequest::new(command)) {
        Ok(response) => print(response.reply, output),
        Err(reply) => print(reply, output),
    }
}

/// Every OS reaches the running Mumble through its local API's owner-only
/// discovery file and bearer-authenticated `POST /control`.
fn running() -> bool {
    crate::local_api::running_pid().is_some()
}

/// `mumble settings set key value` in the running Mumble, which saves and applies
/// it; `None` when no Mumble runs.
pub(crate) fn set_in_running_mumble(key: &str, value: &str) -> Option<ControlReply> {
    running().then(|| {
        call(ControlRequest::new(ControlCommand::SettingsSet {
            key: key.into(),
            value: value.into(),
        }))
        .map_or_else(|reply| reply, |response| response.reply)
    })
}

fn call(request: ControlRequest) -> Result<ControlResponse, ControlReply> {
    if !running() {
        return Err(absent());
    }
    crate::local_api::call_control(&request)
        .map_err(|error| ControlReply::error("transport", format!("{error:#}")))
}

pub(crate) fn not_running() -> ControlReply {
    ControlReply::error("not-running", "Mumble is not running; run `mumble start`")
}

/// Why no Mumble answered `/control`: on Linux an older Mumble an update left
/// running answers only on its socket (#915).
fn absent() -> ControlReply {
    #[cfg(target_os = "linux")]
    if crate::linux::service::instance_answers() {
        return older_mumble();
    }
    not_running()
}

/// An older Mumble that predates `mumble` commands over the local API.
#[cfg(target_os = "linux")]
fn older_mumble() -> ControlReply {
    ControlReply::error(
        "outdated",
        "an older Mumble is still running (Mumble was updated) and doesn't take `mumble` commands; run `mumble restart`",
    )
}

/// `mumble restart` with an older Mumble running: restarts its systemd unit into
/// the installed build, then waits until the new Mumble answers.
#[cfg(target_os = "linux")]
fn restart_older_mumble() -> ControlReply {
    use std::time::{Duration, Instant};

    if !crate::linux::service::supervised() {
        return ControlReply::error(
            "failed",
            "an older Mumble is running outside its systemd unit; quit it from its window or tray, then run `mumble start`",
        );
    }
    if let Err(error) = crate::linux::service::restart() {
        return ControlReply::error("failed", format!("{error:#}"));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if running() {
            return ControlReply::done("Restarted Mumble");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    ControlReply::done("Restarted Mumble; it isn't answering yet (it may still be starting up)")
}

#[cfg(target_os = "macos")]
fn start(output: Output) -> u8 {
    use std::time::{Duration, Instant};

    if let Some(pid) = crate::local_api::running_pid() {
        return print(
            ControlReply::unchanged(format!("Mumble is already running (pid {pid})")),
            output,
        );
    }
    let arguments = open_arguments(
        installed_bundle().as_deref(),
        crate::app_paths::is_macos_test_build(),
        std::env::vars_os(),
    );
    match std::process::Command::new("/usr/bin/open")
        .args(arguments)
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(_) | Err(_) => {
            return print(
                ControlReply::error("failed", "couldn't open Mumble.app; is it installed?"),
                output,
            );
        }
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if let Some(pid) = crate::local_api::running_pid() {
            return print(
                ControlReply::done(format!("Started Mumble (pid {pid})")),
                output,
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    print(
        ControlReply::done("Opened Mumble; it isn't answering yet (setup may be waiting)"),
        output,
    )
}

/// `mumble start` on Linux and Windows: a detached `mumble app --hidden`, which
/// forwards to the running Mumble, starts its systemd unit, or becomes Mumble,
/// then waits until it answers.
#[cfg(not(target_os = "macos"))]
fn start(output: Output) -> u8 {
    print(start_reply(), output)
}

#[cfg(not(target_os = "macos"))]
fn start_reply() -> ControlReply {
    use std::time::{Duration, Instant};

    if running() {
        return ControlReply::unchanged("Mumble is already running");
    }
    #[cfg(target_os = "linux")]
    if crate::linux::service::instance_answers() {
        return older_mumble();
    }
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            return ControlReply::error("failed", format!("couldn't find mumble: {error}"));
        }
    };
    let mut command = std::process::Command::new(executable);
    command
        .args(["app", "--hidden"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "linux")]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: no console, and
        // Ctrl-C in this terminal doesn't reach it.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    if let Err(error) = command.spawn() {
        return ControlReply::error("failed", format!("couldn't start Mumble: {error}"));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if running() {
            return ControlReply::done("Started Mumble");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    ControlReply::done("Started Mumble; it isn't answering yet (it may still be starting up)")
}

/// After a restart's quit on Linux and Windows: waits for the old Mumble to
/// go, then starts the new one. A Linux unit restarts by itself.
#[cfg(not(target_os = "macos"))]
fn reopen_after_restart() -> ControlReply {
    use std::time::{Duration, Instant};

    #[cfg(target_os = "linux")]
    if crate::linux::service::supervised() {
        return ControlReply::done("Restarting Mumble");
    }
    let started = Instant::now();
    while running() {
        if started.elapsed() > Duration::from_secs(30) {
            return ControlReply::error(
                "failed",
                "Mumble is still finishing its work; run `mumble start` once it quits",
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    start_reply()
}

/// `open` arguments for `mumble start`: the caller's `MUMBLE_*` environment, then
/// the Mumble.app this `mumble` belongs to, or the installed app by bundle id.
#[cfg(target_os = "macos")]
fn open_arguments(
    bundle: Option<&std::path::Path>,
    test_build: bool,
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<std::ffi::OsString> {
    let mut arguments = crate::desktop::onboarding::open_env_arguments(vars);
    match bundle {
        Some(bundle) => arguments.push(bundle.into()),
        None => arguments.extend([
            "-b".into(),
            if test_build {
                "anomaly.mumble.test"
            } else {
                "anomaly.mumble"
            }
            .into(),
        ]),
    }
    arguments
}

/// The Mumble.app this `mumble` belongs to, when it runs from (or links into) one.
#[cfg(target_os = "macos")]
fn installed_bundle() -> Option<std::path::PathBuf> {
    let executable = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(ToOwned::to_owned)
}

fn logs(process: bool, lines: usize, follow: bool) -> u8 {
    use std::io::{BufRead, Seek, SeekFrom};

    let path = match crate::app_paths::logs_dir() {
        Ok(directory) => directory.join(if process {
            "process.log"
        } else {
            "live.ndjson"
        }),
        Err(error) => return fail(&format!("{error:#}")),
    };
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) => return fail(&format!("couldn't read {}: {error}", path.display())),
    };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    // Read at most the last 4 MiB for the tail.
    let start = length.saturating_sub(4 * 1024 * 1024);
    let mut reader = std::io::BufReader::new(&mut file);
    if reader.seek(SeekFrom::Start(start)).is_err() {
        return fail("couldn't read the log");
    }
    let mut tail = std::collections::VecDeque::with_capacity(lines.min(10_000));
    let mut line = String::new();
    let mut first = start > 0;
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if !std::mem::take(&mut first) {
            if tail.len() == lines {
                tail.pop_front();
            }
            if lines > 0 {
                tail.push_back(line.clone());
            }
        }
        line.clear();
    }
    let mut stdout = std::io::stdout().lock();
    for line in tail {
        let _ = stdout.write_all(line.as_bytes());
    }
    let _ = stdout.flush();
    if !follow {
        return 0;
    }
    let mut position = reader.stream_position().unwrap_or(length);
    drop(reader);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let length = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(_) => continue,
        };
        if length < position {
            // Rotated or truncated: start over from the new file.
            match std::fs::File::open(&path) {
                Ok(reopened) => file = reopened,
                Err(_) => continue,
            }
            position = 0;
        }
        if length == position {
            continue;
        }
        if file.seek(SeekFrom::Start(position)).is_err() {
            continue;
        }
        let mut reader = std::io::BufReader::new(&mut file);
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if !line.ends_with('\n') {
                break;
            }
            position += line.len() as u64;
            if stdout.write_all(line.as_bytes()).is_err() {
                return 0;
            }
            line.clear();
        }
        line.clear();
        let _ = stdout.flush();
    }
}

fn fail(message: &str) -> u8 {
    eprintln!("mumble: {message}");
    EXIT_REFUSED
}

pub(crate) use human::print;
#[cfg(test)]
use human::*;

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::control::{StatusReport, settings::SettingValue};
    use crate::dictation::vocabulary::VocabularyEntry;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&ControlReply::done("ok")), 0);
        assert_eq!(exit_code(&ControlReply::unchanged("ok")), 0);
        assert_eq!(exit_code(&ControlReply::error("busy", "x")), EXIT_REFUSED);
        assert_eq!(exit_code(&not_running()), EXIT_NOT_RUNNING);
        assert_eq!(
            exit_code(&ControlReply::error("no-speech", "x")),
            EXIT_NO_SPEECH
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_older_mumble_is_not_reported_as_not_running() {
        let reply = older_mumble();
        assert_eq!(exit_code(&reply), EXIT_REFUSED);
        assert!(matches!(
            &reply,
            ControlReply::Error { code, message } if code == "outdated" && message.contains("mumble restart")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn start_forwards_the_callers_mumble_environment() {
        let vars = || {
            [
                ("HOME", "/Users/kit"),
                ("MUMBLE_RETAIN_DICTATION_AUDIO", "1"),
                ("MUMBLE_ENABLE_LLM", "1"),
            ]
            .map(|(key, value)| (key.into(), value.into()))
        };
        let strings = |arguments: Vec<std::ffi::OsString>| {
            arguments
                .into_iter()
                .map(|argument| argument.into_string().unwrap())
                .collect::<Vec<_>>()
        };
        let env = [
            "--env",
            "MUMBLE_ENABLE_LLM=1",
            "--env",
            "MUMBLE_RETAIN_DICTATION_AUDIO=1",
        ];
        let bundle = std::path::Path::new("/Applications/Mumble.app");
        assert_eq!(
            strings(open_arguments(Some(bundle), false, vars())),
            [&env[..], &["/Applications/Mumble.app"]].concat()
        );
        assert_eq!(
            strings(open_arguments(None, false, vars())),
            [&env[..], &["-b", "anomaly.mumble"]].concat()
        );
        assert_eq!(
            strings(open_arguments(None, true, [])),
            ["-b", "anomaly.mumble.test"]
        );
    }

    #[test]
    fn one_setting_prints_its_bare_value() {
        let setting = |key: &str, value: Value| SettingValue {
            key: key.into(),
            value,
            kind: "text".into(),
            choices: vec![],
            writable: true,
            about: String::new(),
            available: true,
        };
        assert_eq!(
            human_settings(&[setting("dictation_hotkey", "option".into())]),
            "option\n"
        );
        assert_eq!(human_settings(&[setting("a", Value::Null)]), "none\n");
        let listed = human_settings(&[
            setting("sound_effects", true.into()),
            setting("a", 0.5.into()),
        ]);
        assert_eq!(listed, "sound_effects  true\na              0.5\n");
    }

    #[test]
    fn vocabulary_lists_terms_with_their_spellings() {
        assert!(human_vocabulary(&[]).starts_with("The vocabulary is empty"));
        let entries = [
            VocabularyEntry {
                term: "cappuccino".into(),
                sounds_like: vec!["capuchin".into(), "cap a chino".into()],
            },
            VocabularyEntry {
                term: "Rune".into(),
                sounds_like: vec![],
            },
        ];
        assert_eq!(
            human_vocabulary(&entries),
            "cappuccino  (sounds like capuchin, cap a chino)\nRune\n"
        );
    }

    #[test]
    fn status_reads_for_a_person() {
        let mut report = StatusReport {
            app_version: "2.3.1".into(),
            pid: 42,
            memory_footprint_bytes: Some(412_000_000),
            ..Default::default()
        };
        report.features.release_microphone_while_idle = true;
        report.features.history_retention = "week".into();
        let text = human_status(&report);
        assert!(text.starts_with("Mumble 2.3.1 (pid 42, 412 MB)\n"));
        assert!(text.contains("Features     release mic while idle on, history week\n"));
        assert!(text.ends_with("Labs         none\n"));
        report.labs = vec!["live-text".into(), "history-stats".into()];
        assert!(human_status(&report).ends_with("Labs         live-text, history-stats\n"));
    }

    #[test]
    fn status_reports_labs_and_reads_older_replies_without_it() {
        let report = StatusReport {
            labs: vec!["live-text".into()],
            ..Default::default()
        };
        let mut json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["labs"], serde_json::json!(["live-text"]));
        json.as_object_mut().unwrap().remove("labs");
        let older: StatusReport = serde_json::from_value(json).unwrap();
        assert!(older.labs.is_empty());
    }
}
