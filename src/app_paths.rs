use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use color_eyre::eyre::{Result, eyre};
use tracing_subscriber::fmt::writer::MakeWriterExt;

/// Whether this is a local build (`MUMBLE_LOCAL_BUILD=1`): made from a checkout
/// for its owner's own machine, so it never checks for or offers updates.
pub fn is_local_build() -> bool {
    option_env!("MUMBLE_LOCAL_BUILD") == Some("1")
}

/// Whether this is a macOS `Mumble Test.app` build (`MUMBLE_TEST_BUILD=1`), which
/// keeps its data, settings, and user config apart from the installed Mumble.
pub fn is_macos_test_build() -> bool {
    cfg!(target_os = "macos") && option_env!("MUMBLE_TEST_BUILD") == Some("1")
}

/// The name menus show, so a test build running beside Mumble reads as itself.
#[cfg(target_os = "macos")]
pub fn app_name() -> &'static str {
    if is_macos_test_build() {
        "Mumble Test"
    } else {
        "Mumble"
    }
}

pub fn support_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("MUMBLE_APPLICATION_SUPPORT_DIR") {
        return Ok(path.into());
    }
    let data =
        dirs::data_dir().ok_or_else(|| eyre!("application data directory is unavailable"))?;
    let folder = if is_macos_test_build() {
        "mumble-test"
    } else {
        "mumble"
    };
    Ok(data.join(folder))
}

pub fn logs_dir() -> Result<PathBuf> {
    static LOGS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    machine_local(&LOGS, "logs")
}

/// `name` in the support folder, except on Windows, where logs and models
/// belong to this computer: `%LOCALAPPDATA%\mumble`, not the Roaming
/// `%APPDATA%\mumble`, which managed networks copy to a server at every sign-in.
/// An older Mumble's Roaming copy moves over once. While it can't (a running Mumble
/// holds its files), the Roaming copy keeps being used, so nothing downloads
/// again.
fn machine_local(cell: &std::sync::OnceLock<PathBuf>, name: &str) -> Result<PathBuf> {
    if let Some(path) = cell.get() {
        return Ok(path.clone());
    }
    let roaming = support_dir()?.join(name);
    let path = match local_support_dir() {
        Some(local) => move_once(&roaming, &local.join(name)),
        None => roaming,
    };
    Ok(cell.get_or_init(|| path).clone())
}

/// `%LOCALAPPDATA%\mumble` on Windows, unless a data folder is chosen for this
/// run (`MUMBLE_APPLICATION_SUPPORT_DIR`); elsewhere none.
fn local_support_dir() -> Option<PathBuf> {
    if !cfg!(windows) || std::env::var_os("MUMBLE_APPLICATION_SUPPORT_DIR").is_some() {
        return None;
    }
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(|local| PathBuf::from(local).join("mumble"))
}

/// `local`, after moving `roaming` there if only `roaming` exists. A move that
/// fails leaves `roaming` in place and in use.
fn move_once(roaming: &Path, local: &Path) -> PathBuf {
    if local.exists() || !roaming.exists() {
        return local.to_path_buf();
    }
    let moved = local
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::rename(roaming, local));
    match moved {
        Ok(()) => local.to_path_buf(),
        Err(_) => roaming.to_path_buf(),
    }
}

/// Creates `directory` readable only by this user, and tightens it if an
/// older Mumble left it open. Tightening is best-effort: a folder Mumble can't
/// chmod (another owner, a mount that refuses it) is logged, never fatal.
/// Windows folders already inherit the per-user `%APPDATA%` permissions.
pub fn create_private_dir(directory: &Path) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    tighten(directory, 0o700);
    Ok(())
}

/// Opens `path` to append, readable only by this user, tightening a file an
/// older Mumble created open (best-effort, as `create_private_dir`).
pub fn open_private_append(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    tighten(path, 0o600);
    Ok(file)
}

#[cfg(unix)]
fn tighten(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
        tracing::warn!(path = %path.display(), %error, "could not make this private");
    }
}

/// Creates the logs directory, appends process diagnostics to `process.log`
/// alongside stderr, installs the global tracing subscriber, and routes Ctrl-C
/// to `shutdown`. Returns the logs directory.
pub fn init_process_logging(shutdown: &'static AtomicBool) -> Result<PathBuf> {
    let log_dir = logs_dir()?;
    create_private_dir(&log_dir)?;
    let process_log = open_private_append(&log_dir.join("process.log"))?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mumble=info"));
    tracing_subscriber::fmt()
        .with_env_filter(without_secrets(filter))
        .with_writer(std::io::stderr.and(Mutex::new(process_log)))
        .init();
    log_panics();
    ctrlc::set_handler(|| shutdown.store(true, Ordering::Relaxed))?;
    Ok(log_dir)
}

/// `filter`, with tungstenite held to warnings whatever `RUST_LOG` asks: its
/// handshake trace prints the request with its authorization header (Labs ›
/// Cloud transcription's sign-in).
fn without_secrets(filter: tracing_subscriber::EnvFilter) -> tracing_subscriber::EnvFilter {
    filter.add_directive(
        "tungstenite=warn"
            .parse()
            .expect("tungstenite=warn is a valid directive"),
    )
}

/// Records every panic in `process.log` before the installed report runs.
/// Desktop apps have no console, so the default stderr report is otherwise
/// lost and a crashed worker thread leaves no trace.
fn log_panics() {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            thread = std::thread::current().name().unwrap_or("unnamed"),
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic: {info}"
        );
        report(info);
    }));
}

pub fn opencode_workspace() -> Result<PathBuf> {
    Ok(support_dir()?.join("opencode"))
}

pub fn local_api_discovery_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("local-api.json"))
}

pub fn personal_commands_status_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("personal-commands.json"))
}

/// The config hash the person allowed to use `fetch` and run programs. It
/// lives here, not in the config folder, so a config cannot allow itself.
pub fn transform_trust_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("transform-trust.json"))
}

pub fn personal_commands_workspace() -> Result<PathBuf> {
    user_config_dir()
}

/// The per-user config folder holding `mumble.config.ts` on every OS: `~/.config/mumble`, or `~/.config/mumble-test` for `Mumble Test.app`.
fn user_config_dir() -> Result<PathBuf> {
    let home = home_dir().ok_or_else(|| eyre!("home directory is unavailable"))?;
    Ok(user_config_dir_in(&home, is_macos_test_build()))
}

/// The home folder. On Windows `USERPROFILE` names it, as `HOME` does
/// elsewhere, so a copy of Mumble launched with another profile folder (an
/// isolated benchmark) reads that folder's config; `dirs::home_dir` asks the
/// shell for the signed-in user's own folder instead. Every path Mumble derives
/// from the user's folders goes through these, as Annotate's engine does
/// (`scrawl_router::user_dirs`).
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|profile| !profile.is_empty()) {
        return Some(profile.into());
    }
    dirs::home_dir()
}

fn user_config_dir_in(home: &std::path::Path, test_build: bool) -> PathBuf {
    home.join(".config")
        .join(if test_build { "mumble-test" } else { "mumble" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_never_carries_the_websocket_handshake() {
        #[derive(Clone, Default)]
        struct Captured(std::sync::Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Captured {
            fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buffer);
                Ok(buffer.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for asked in ["trace", "tungstenite=trace,mumble=trace"] {
            let captured = Captured::default();
            let writer = captured.clone();
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(without_secrets(tracing_subscriber::EnvFilter::new(asked)))
                .with_writer(move || writer.clone())
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                tracing::trace!(target: "tungstenite::handshake::client", "Bearer sk-ant-secret");
                tracing::trace!(target: "mumble", "still logged");
            });
            let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
            assert!(log.contains("still logged"), "{asked}: {log}");
            assert!(!log.contains("sk-ant-secret"), "{asked}: {log}");
        }
    }

    #[test]
    fn an_older_roaming_folder_moves_to_the_local_one_once() {
        let root = std::env::temp_dir().join(format!("mumble-local-move-{}", std::process::id()));
        let roaming = root.join("Roaming").join("mumble").join("models");
        let local = root.join("Local").join("mumble").join("models");
        fs::create_dir_all(&roaming).unwrap();
        fs::write(roaming.join("model.gguf"), b"weights").unwrap();

        assert_eq!(move_once(&roaming, &local), local);
        assert_eq!(fs::read(local.join("model.gguf")).unwrap(), b"weights");
        assert!(!roaming.exists());

        // Later runs, and fresh installs, use the local folder as it is.
        assert_eq!(move_once(&roaming, &local), local);
        let fresh = root.join("Fresh").join("models");
        assert_eq!(move_once(&root.join("none"), &fresh), fresh);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_move_that_fails_keeps_the_roaming_folder_in_use() {
        let root = std::env::temp_dir().join(format!("mumble-local-stay-{}", std::process::id()));
        let roaming = root.join("Roaming").join("models");
        fs::create_dir_all(&roaming).unwrap();
        // A file where the local parent folder should be makes the move fail.
        fs::write(root.join("Local"), b"").unwrap();

        assert_eq!(
            move_once(&roaming, &root.join("Local").join("models")),
            roaming
        );
        assert!(roaming.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_logs_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("mumble-private-dir-{}", std::process::id()));
        let logs = root.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o755)).unwrap();

        create_private_dir(&logs).unwrap();
        create_private_dir(&root.join("fresh")).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&logs), 0o700);
        assert_eq!(mode(&root.join("fresh")), 0o700);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_build_has_its_own_config_folder() {
        let home = std::path::Path::new("/Users/kit");
        assert_eq!(
            user_config_dir_in(home, false),
            home.join(".config").join("mumble")
        );
        assert_eq!(
            user_config_dir_in(home, true),
            home.join(".config").join("mumble-test")
        );
    }
}
