//! `mumble.com`, the console entry point of Mumble on Windows. `mumble.exe` is a GUI
//! program so launching Mumble opens no console window (#850), but shells don't
//! wait for a GUI program or capture its output. A terminal resolves `mumble` to
//! `mumble.com` first (PATHEXT lists .COM before .EXE), and this runs `mumble.exe`
//! with the same arguments, standard handles, and console, waits for it, and
//! exits with its code. Opening the app (`mumble`, `mumble app [--hidden]`) doesn't
//! wait: the app stays resident, so the terminal returns once it starts, as
//! it would for a GUI program.
//!
//! It finds `mumble.exe` beside itself (`versions\<version>\mumble.com`), or, as
//! the copy in `%APPDATA%\mumble\bin` that PATH names, the version
//! `current-version` points at.

#[cfg(windows)]
fn main() {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };

    // Ctrl+C reaches every process on the console: mumble.exe handles it, and
    // this stays to report its exit code. Closing the console still ends it.
    unsafe extern "system" fn wait_for_mumble(event: u32) -> i32 {
        if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
            TRUE
        } else {
            FALSE
        }
    }
    // SAFETY: registers a handler that lives for the whole process.
    unsafe { SetConsoleCtrlHandler(Some(wait_for_mumble), TRUE) };

    let mumble = match std::env::current_exe().map(|path| locate(&path)) {
        Ok(Some(mumble)) => mumble,
        _ => {
            eprintln!("mumble: couldn't find mumble.exe; reinstall Mumble from anoma.ly/mumble");
            std::process::exit(1);
        }
    };
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mut command = std::process::Command::new(&mumble);
    command.args(&arguments);
    let result = if opens_app(&arguments) {
        command.spawn().map(|_| 0)
    } else {
        command.status().map(|status| status.code().unwrap_or(1))
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("mumble: couldn't start {}: {error}", mumble.display());
            std::process::exit(1);
        }
    }
}

/// `mumble`, `mumble app`, or `mumble app --hidden`: the resident app, not a command
/// that finishes. A headless render (`mumble app -- --snap …`) or `--help` does.
#[cfg_attr(not(windows), allow(dead_code))]
fn opens_app(arguments: &[std::ffi::OsString]) -> bool {
    match arguments.split_first() {
        None => true,
        Some((first, rest)) => first == "app" && rest.iter().all(|argument| argument == "--hidden"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("mumble-console is Mumble's Windows console entry point");
    std::process::exit(2);
}

/// `mumble.exe` beside `console`, or in the version `current-version` beside
/// its folder names.
#[cfg_attr(not(windows), allow(dead_code))]
fn locate(console: &std::path::Path) -> Option<std::path::PathBuf> {
    let folder = console.parent()?;
    let beside = folder.join("mumble.exe");
    if beside.is_file() {
        return Some(beside);
    }
    let support = folder.parent()?;
    let version = std::fs::read_to_string(support.join("current-version")).ok()?;
    let version = version.trim();
    let named = !version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+_-".contains(&byte));
    let current = support.join("versions").join(version).join("mumble.exe");
    (named && current.is_file()).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::{locate, opens_app};

    #[test]
    fn only_opening_the_app_returns_without_waiting() {
        let opens =
            |arguments: &[&str]| opens_app(&arguments.iter().map(Into::into).collect::<Vec<_>>());
        assert!(opens(&[]));
        assert!(opens(&["app"]));
        assert!(opens(&["app", "--hidden"]));
        assert!(!opens(&["app", "--", "--snap", "out.png"]));
        assert!(!opens(&["app", "--help"]));
        assert!(!opens(&["status", "--json"]));
        assert!(!opens(&["listen"]));
        assert!(!opens(&["--version"]));
    }

    #[test]
    fn finds_mumble_beside_it_or_through_current_version() {
        let root = std::env::temp_dir().join(format!("mumble-console-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let version = root.join("versions").join("3.4.0");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(version.join("mumble.exe"), b"").unwrap();

        assert_eq!(
            locate(&version.join("mumble.com")),
            Some(version.join("mumble.exe"))
        );
        let bin = root.join("bin").join("mumble.com");
        assert_eq!(locate(&bin), None, "no current-version yet");
        std::fs::write(root.join("current-version"), "3.4.0\n").unwrap();
        assert_eq!(locate(&bin), Some(version.join("mumble.exe")));
        std::fs::write(root.join("current-version"), "..\\..\\elsewhere").unwrap();
        assert_eq!(
            locate(&bin),
            None,
            "a pointer that leaves versions is ignored"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
