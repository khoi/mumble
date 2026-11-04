#[cfg(windows)]
use std::path::PathBuf;
use std::process::Command;

fn main() {
    check_windows_split();
    build_version();
    test_build_commit();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        #[cfg(windows)]
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            embed_windows_resources();
        }
        return;
    }
    compile_permission_guide();
}

/// The version Hex reports (`HEX_BUILD_VERSION`): the crate version, which
/// the Release workflow stamps (scripts/set-crate-version.sh), else, for any
/// other build of the unstamped `0.0.0`, `0.0.0-dev-local-<commit>` with
/// `-dirty` for uncommitted changes (`0.0.0-dev-local` without git, as in Nix).
/// It counts as a dev build, so it follows the dev channel's current release.
/// Fails every OS's build when Cargo.lock puts gpu-allocator and wgpu-hal on
/// different `windows` crates. wgpu-hal passes D3D12 types into
/// gpu-allocator, so the split only breaks the Windows build, and
/// `cargo update -p rune` has re-resolved gpu-allocator onto an older
/// `windows` twice (#997, #1085). Fix it by setting gpu-allocator's `windows`
/// (and iana-time-zone's `windows-core`) back to wgpu-hal's version.
fn check_windows_split() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let Ok(lock) = std::fs::read_to_string("Cargo.lock") else {
        return;
    };
    let windows = |name: &str| {
        lock.split("[[package]]")
            .find(|block| block.contains(&format!("\nname = \"{name}\"\n")))
            .and_then(|block| {
                block
                    .lines()
                    .map(|line| line.trim().trim_end_matches(',').trim_matches('"'))
                    .find(|dependency| {
                        *dependency == "windows" || dependency.starts_with("windows ")
                    })
                    .map(str::to_owned)
            })
    };
    let (allocator, hal) = (windows("gpu-allocator"), windows("wgpu-hal"));
    assert_eq!(
        allocator, hal,
        "Cargo.lock: gpu-allocator must use wgpu-hal's `windows` crate, or the Windows build breaks"
    );
}

fn build_version() {
    println!("cargo:rerun-if-env-changed=HEX_VERSION");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    // CI builds name their version in HEX_VERSION and check `hex --version`
    // against it, test builds included (they name the crate version).
    if let Some(named) = std::env::var("HEX_VERSION")
        .ok()
        .filter(|named| !named.is_empty())
    {
        println!("cargo:rustc-env=HEX_BUILD_VERSION={named}");
        return;
    }
    if version != "0.0.0" {
        println!("cargo:rustc-env=HEX_BUILD_VERSION={version}");
        return;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let mut version = "0.0.0-dev-local".to_owned();
    if let Some(commit) =
        git(&["rev-parse", "--short=7", "HEAD"]).filter(|commit| !commit.is_empty())
    {
        version = format!("{version}-{commit}");
        if git(&["status", "--porcelain", "--untracked-files=no"])
            .is_some_and(|status| !status.is_empty())
        {
            version.push_str("-dirty");
        }
        let reference = git(&["symbolic-ref", "-q", "HEAD"]);
        for name in ["HEAD", "packed-refs", "index"]
            .into_iter()
            .chain(reference.as_deref())
        {
            if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name])
                .filter(|path| std::path::Path::new(path).exists())
            {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    println!("cargo:rustc-env=HEX_BUILD_VERSION={version}");
}

/// A test or local build carries its commit (`HEX_COMMIT`), which the sidebar shows
/// beside the version.
fn test_build_commit() {
    println!("cargo:rerun-if-env-changed=HEX_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=HEX_TEST_BUILD");
    println!("cargo:rerun-if-env-changed=HEX_LOCAL_BUILD");
    if std::env::var("HEX_TEST_BUILD").as_deref() != Ok("1")
        && std::env::var("HEX_LOCAL_BUILD").as_deref() != Ok("1")
    {
        return;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let Some(commit) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return;
    };
    println!("cargo:rustc-env=HEX_COMMIT={commit}");
    let reference = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "packed-refs"]
        .into_iter()
        .chain(reference.as_deref())
    {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name])
            .filter(|path| std::path::Path::new(path).exists())
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

/// Gives the executable its icon and the "Hex" name Explorer and Task
/// Manager show, whatever the file is called.
#[cfg(windows)]
fn embed_windows_resources() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let icon = manifest_dir.join("resources/windows/hex.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = version
        .split(['.', '-'])
        .take(3)
        .map(|part| part.parse::<u16>().unwrap_or(0).to_string())
        .chain(std::iter::repeat("0".to_string()))
        .take(4)
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        r#"#include <winver.h>
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Hex"
      VALUE "ProductName", "Hex"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "hex.exe"
      VALUE "InternalName", "hex"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
    );
    let resource = output_dir.join("hex.rc");
    std::fs::write(&resource, script).expect("write the Windows resource script");
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("compile the Windows resources");
}

fn compile_permission_guide() {
    println!("cargo:rerun-if-changed=native/permission_guide.m");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=CoreImage");
    println!("cargo:rustc-link-lib=framework=IOKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");

    cc::Build::new()
        .file("native/permission_guide.m")
        .flag("-fobjc-arc")
        .compile("hex_permission_guide");
}
