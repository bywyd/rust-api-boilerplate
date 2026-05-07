/// Standalone uninstaller binary.
///
/// Double-click (or run with no arguments) to interactively remove the
/// application.  The install directory is **auto-detected** as the folder that
/// contains this binary — no configuration needed.
///
/// Flags (all optional):
///   --service-name NAME     Service / unit name  [default: api]
///   --install-dir  PATH     Override the auto-detected install directory
///   --non-interactive       Skip confirmation prompt; proceed immediately
///   --help / -h             Print this message

use dialoguer::Confirm;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{env, fs, io, process};

// Privilege check

#[cfg(unix)]
fn check_privileges() -> Result<(), String> {
    // SAFETY: getuid() has no preconditions.
    let uid = unsafe { libc_sys::getuid() };
    if uid != 0 {
        Err("uninstaller must be run as root (sudo uninstaller)".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
mod libc_sys {
    unsafe extern "C" {
        pub fn getuid() -> u32;
    }
}

#[cfg(windows)]
fn check_privileges() -> Result<(), String> {
    // `net session` exits 0 only when the caller is an administrator.
    let status = process::Command::new("net")
        .args(["session"])
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err("uninstaller must be run as Administrator".into()),
    }
}

// Arg parsing

#[derive(Default)]
struct Flags {
    service_name: Option<String>,
    install_dir: Option<PathBuf>,
    non_interactive: bool,
}

/// Returns `None` when `--help` is requested.
fn parse_args(args: &[String]) -> Option<Flags> {
    let mut flags = Flags::default();
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return None,
            "--service-name" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.service_name = Some(v.clone());
                }
            }
            "--install-dir" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.install_dir = Some(PathBuf::from(v));
                }
            }
            "--non-interactive" => flags.non_interactive = true,
            other => {
                eprintln!("uninstaller: unknown flag: {}", other);
                process::exit(2);
            }
        }
        i += 1;
    }
    Some(flags)
}

// Entry point

fn main() {
    let args: Vec<String> = env::args().collect();
    let flags = match parse_args(&args) {
        Some(f) => f,
        None => {
            print_help();
            pause_if_tty();
            return;
        }
    };

    if let Err(e) = check_privileges() {
        fatal(&e);
    }

    let interactive = !flags.non_interactive && is_tty();

    let service_name = flags.service_name.unwrap_or_else(|| "api".to_string());

    // Auto-detect install dir as the directory that contains this binary.
    let install_dir = flags.install_dir.unwrap_or_else(|| {
        env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    });

    println!("Uninstaller");
    println!("  Service name      : {}", service_name);
    println!("  Install directory : {}", install_dir.display());
    println!();

    if interactive {
        let ok = Confirm::new()
            .with_prompt(
                "This will permanently remove the service and all installed files. Proceed?",
            )
            .default(false)
            .interact()
            .unwrap_or(false);

        if !ok {
            println!("Aborted.");
            pause_if_tty();
            return;
        }
        println!();
    }

    // 1. Stop the service (best-effort; ignore failures so we can still clean up).
    println!("Stopping service '{}'...", service_name);
    stop_service(&service_name);

    // 2. Remove the service registration.
    println!("Removing service registration...");
    deregister_service(&service_name);

    // 3. Delete all installed files, including this binary.
    println!("Removing installed files from {}...", install_dir.display());
    remove_install_dir(&install_dir);

    println!();
    println!("Uninstall complete.");
    pause_if_tty();
}

fn print_help() {
    println!(
        r#"Usage: uninstaller [OPTIONS]

Permanently removes the installed service and all application files.
The install directory is auto-detected from the location of this binary.

Options:
  --service-name NAME     Service / unit name  [default: api]
  --install-dir  PATH     Override the auto-detected install directory
  --non-interactive       Skip confirmation prompt and proceed immediately
  --help, -h              Print this message
"#
    );
}

// Platform: Linux / macOS

#[cfg(unix)]
fn stop_service(name: &str) {
    run_cmd("systemctl", &["stop", name]);
}

#[cfg(unix)]
fn deregister_service(name: &str) {
    run_cmd("systemctl", &["disable", name]);
    let unit = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
    if unit.exists() {
        let _ = fs::remove_file(&unit);
        println!("  Removed {}", unit.display());
    }
    run_cmd("systemctl", &["daemon-reload"]);
}

#[cfg(unix)]
fn remove_install_dir(install_dir: &Path) {
    // On Linux the kernel keeps the inode alive until the last fd closes, so
    // we can remove our own directory even while running inside it.
    if install_dir.exists() {
        match fs::remove_dir_all(install_dir) {
            Ok(()) => println!("  Removed {}", install_dir.display()),
            Err(e) => eprintln!(
                "  Warning: could not fully remove {}: {}",
                install_dir.display(),
                e
            ),
        }
    } else {
        println!("  Directory does not exist — nothing to remove.");
    }
}

// Platform: Windows

#[cfg(windows)]
fn stop_service(name: &str) {
    run_cmd("sc", &["stop", name]);
}

#[cfg(windows)]
fn deregister_service(name: &str) {
    // Stop again to be safe, then delete the service entry.
    run_cmd("sc", &["stop", name]);
    run_cmd("sc", &["delete", name]);

    // Remove the environment variables written into the registry by the installer.
    let reg_key = format!(
        r"HKLM\SYSTEM\CurrentControlSet\Services\{}\Environment",
        name
    );
    run_cmd("reg", &["delete", &reg_key, "/f"]);
}

/// Windows cannot delete a running .exe, so we write a small CMD script to
/// %TEMP% that waits for this process to exit and then removes the directory.
/// The script loops until the directory is gone to handle any file locks, then
/// deletes itself.
#[cfg(windows)]
fn remove_install_dir(install_dir: &Path) {
    if !install_dir.exists() {
        println!("  Directory does not exist — nothing to remove.");
        return;
    }

    let dir_str = install_dir
        .to_str()
        .unwrap_or("")
        .trim_end_matches(['/', '\\'])
        .to_string();

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let script = env::temp_dir().join(format!("_uninstall_{}.cmd", ts));

    // Batch script: wait ~2 s for the parent process to exit, then loop until
    // the directory is fully removed, then self-delete.
    let content = format!(
        "@echo off\r\n\
         ping -n 3 127.0.0.1 >nul\r\n\
         :WAIT\r\n\
         if exist \"{dir}\\\" (\r\n\
             rmdir /s /q \"{dir}\"\r\n\
             ping -n 2 127.0.0.1 >nul\r\n\
             goto WAIT\r\n\
         )\r\n\
         del \"%~f0\"\r\n",
        dir = dir_str
    );

    match fs::write(&script, content.as_bytes()) {
        Ok(()) => {
            let spawned = process::Command::new("cmd")
                .args(["/c", "start", "", "/min", script.to_str().unwrap_or("")])
                .spawn();
            match spawned {
                Ok(_) => println!(
                    "  Scheduled removal of {} (runs after this process exits).",
                    install_dir.display()
                ),
                Err(e) => eprintln!("  Warning: could not schedule cleanup script: {}", e),
            }
        }
        Err(e) => {
            eprintln!("  Warning: could not write cleanup script: {}", e);
            // Best-effort direct attempt — removes everything except the locked .exe.
            let _ = fs::remove_dir_all(install_dir);
        }
    }
}

// Helpers

fn run_cmd(program: &str, args: &[&str]) {
    match process::Command::new(program).args(args).status() {
        Err(e) => eprintln!(
            "  Warning: could not run `{} {}`: {}",
            program,
            args.join(" "),
            e
        ),
        Ok(s) if !s.success() => eprintln!(
            "  Warning: `{} {}` exited with code {}",
            program,
            args.join(" "),
            s.code().unwrap_or(-1)
        ),
        Ok(_) => {}
    }
}

fn is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: isatty() on fd 1 (stdout) is always safe.
        unsafe { tty_sys::isatty(1) != 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let handle = io::stdout().as_raw_handle();
        // FILE_TYPE_CHAR (2) means the handle is a console/character device.
        unsafe { win_sys::GetFileType(handle as *mut std::ffi::c_void) == 2 }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(unix)]
mod tty_sys {
    unsafe extern "C" {
        pub fn isatty(fd: i32) -> i32;
    }
}

#[cfg(windows)]
mod win_sys {
    unsafe extern "system" {
        pub fn GetFileType(handle: *mut std::ffi::c_void) -> u32;
    }
}

fn pause_if_tty() {
    if is_tty() {
        println!();
        println!("Press Enter to exit...");
        let stdin = io::stdin();
        let _ = stdin.lock().lines().next();
    }
}

fn fatal(msg: &str) -> ! {
    eprintln!("uninstaller: error: {}", msg);
    pause_if_tty();
    process::exit(1);
}
