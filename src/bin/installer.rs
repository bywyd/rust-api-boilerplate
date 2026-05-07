/// Standalone installer binary.
///
/// Supports three subcommands:
///
///   installer install   [OPTIONS]   — copy binaries, write config, register service
///   installer uninstall [OPTIONS]   — stop service, remove registration, remove files
///   installer status    [OPTIONS]   — print live service status
///
/// On Linux the service is managed via systemd; on Windows via the Service Control Manager.
///
/// Flags (all optional; interactive prompts fill missing values when running in a TTY):
///   --install-dir PATH        Install root directory
///   --service-name NAME       Service / unit name          [default: api]
///   --display-name NAME       Human-readable service name  [default: API Service]
///   --description TEXT        Service description
///   --no-start                Skip starting the service after install
///   --non-interactive         Never prompt; use defaults for every missing value

use dialoguer::{Confirm, Input, Select};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{env, fs, io, process};

// Embedded default config

static DEFAULT_CONFIG: &str = include_str!("../../config/default.yaml");

// Binaries to bundle

#[cfg(windows)]
const BINARY_NAMES: &[&str] = &["api.exe", "worker.exe", "updater.exe", "installer.exe", "uninstaller.exe"];
#[cfg(not(windows))]
const BINARY_NAMES: &[&str] = &["api", "worker", "updater", "installer", "uninstaller"];

// Platform defaults

#[cfg(windows)]
fn default_install_dir(service_name: &str) -> PathBuf {
    PathBuf::from(format!(r"C:\Program Files\{}", service_name))
}
#[cfg(not(windows))]
fn default_install_dir(service_name: &str) -> PathBuf {
    PathBuf::from(format!("/opt/{}", service_name))
}

// Privilege check

#[cfg(unix)]
fn check_privileges() -> Result<(), String> {
    // SAFETY: getuid() has no preconditions.
    let uid = unsafe { libc_sys::getuid() };
    if uid != 0 {
        Err("installer must be run as root (sudo installer ...)".into())
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
    // `net session` exits 0 when the caller is an administrator.
    let status = process::Command::new("net")
        .args(["session"])
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err("installer must be run as Administrator".into()),
    }
}

// Arg parsing

#[derive(Default)]
struct Flags {
    install_dir: Option<PathBuf>,
    service_name: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    no_start: bool,
    non_interactive: bool,
}

fn parse_args(args: &[String]) -> (String, Flags) {
    let mut subcommand = String::from("help");
    let mut flags = Flags::default();
    let mut i = 1usize;

    if args.len() > 1 {
        subcommand = args[1].clone();
        i = 2;
    }

    while i < args.len() {
        match args[i].as_str() {
            "--install-dir" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.install_dir = Some(PathBuf::from(v));
                }
            }
            "--service-name" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.service_name = Some(v.clone());
                }
            }
            "--display-name" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.display_name = Some(v.clone());
                }
            }
            "--description" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    flags.description = Some(v.clone());
                }
            }
            "--no-start" => flags.no_start = true,
            "--non-interactive" => flags.non_interactive = true,
            other => {
                eprintln!("installer: unknown flag: {}", other);
                process::exit(2);
            }
        }
        i += 1;
    }

    (subcommand, flags)
}

// Entry point

fn main() {
    let args: Vec<String> = env::args().collect();
    let (subcommand, mut flags) = parse_args(&args);

    // When launched with no arguments in an interactive terminal (e.g. double-click
    // on Windows), show a menu instead of printing help and immediately closing.
    let subcommand = if subcommand == "help" && args.len() == 1 && is_tty() {
        let choices = &["Install", "Uninstall", "Status", "Help / Quit"];
        let idx = Select::new()
            .with_prompt("What would you like to do?")
            .items(choices)
            .default(0)
            .interact()
            .unwrap_or(3);
        match idx {
            0 => {
                flags.non_interactive = false;
                "install".to_string()
            }
            1 => {
                flags.non_interactive = false;
                "uninstall".to_string()
            }
            2 => {
                flags.non_interactive = false;
                "status".to_string()
            }
            _ => "help".to_string(),
        }
    } else {
        subcommand
    };

    match subcommand.as_str() {
        "install" => cmd_install(flags),
        "uninstall" => cmd_uninstall(flags),
        "status" => cmd_status(flags),
        "help" | "--help" | "-h" => {
            print_help();
            pause_if_tty();
        }
        other => {
            eprintln!("installer: unknown subcommand '{}'. Run 'installer help'.", other);
            pause_if_tty();
            process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        r#"Usage: installer <SUBCOMMAND> [OPTIONS]

Subcommands:
  install     Copy binaries, write config and register as a system service
  uninstall   Stop the service, remove registration and delete installed files
  status      Show the current service status

Options:
  --install-dir PATH      Installation root directory
  --service-name NAME     Service name (default: api)
  --display-name NAME     Human-readable service name (default: API Service)
  --description TEXT      Service description
  --no-start              Do not start the service after installation
  --non-interactive       Never prompt; use built-in defaults for any missing value
"#
    );
}

// install

fn cmd_install(flags: Flags) {
    if let Err(e) = check_privileges() {
        fatal(&e);
    }

    let interactive = !flags.non_interactive && is_tty();

    // Gather values — flags first, then interactive prompts, then hardcoded defaults.
    let service_name = resolve(
        flags.service_name,
        interactive,
        "Service name",
        "api",
    );

    let install_dir = flags
        .install_dir
        .unwrap_or_else(|| {
            let default = default_install_dir(&service_name);
            if interactive {
                let s: String = Input::new()
                    .with_prompt("Install directory")
                    .default(default.display().to_string())
                    .interact_text()
                    .unwrap_or_else(|_| default.display().to_string());
                PathBuf::from(s)
            } else {
                default
            }
        });

    let display_name = resolve(
        flags.display_name,
        interactive,
        "Display name",
        "API Service",
    );

    let description = resolve(
        flags.description,
        interactive,
        "Description",
        "Rust API boilerplate service",
    );

    let should_start = if flags.no_start {
        false
    } else if interactive {
        Confirm::new()
            .with_prompt("Start the service after installation?")
            .default(true)
            .interact()
            .unwrap_or(true)
    } else {
        true
    };

    // Confirm summary.
    if interactive {
        println!();
        println!("Installation summary:");
        println!("  Service name  : {}", service_name);
        println!("  Install dir   : {}", install_dir.display());
        println!("  Display name  : {}", display_name);
        println!("  Description   : {}", description);
        println!("  Start service : {}", should_start);
        println!();

        let ok = Confirm::new()
            .with_prompt("Proceed?")
            .default(true)
            .interact()
            .unwrap_or(true);

        if !ok {
            println!("Aborted.");
            return;
        }
    }

    // 1. Create directories.
    let config_dir = install_dir.join("config");
    println!("Creating {}", install_dir.display());
    create_dir_all(&install_dir);
    create_dir_all(&config_dir);

    // 2. Copy binaries from the same directory as this installer.
    let src_dir = installer_dir();
    println!("Copying binaries from {}", src_dir.display());
    for name in BINARY_NAMES {
        let src = src_dir.join(name);
        let dst = install_dir.join(name);
        if src.exists() {
            copy_file(&src, &dst);
            #[cfg(unix)]
            set_executable(&dst);
            println!("  Copied {}", name);
        } else {
            println!("  Warning: {} not found in {}, skipping", name, src_dir.display());
        }
    }

    // 3. Write default config (only if not already present).
    let config_path = config_dir.join("default.yaml");
    if config_path.exists() {
        println!("Config already exists at {} — not overwriting", config_path.display());
    } else {
        fs::write(&config_path, DEFAULT_CONFIG).unwrap_or_else(|e| {
            fatal(&format!("Failed to write config: {}", e));
        });
        println!("Wrote default config to {}", config_path.display());
    }

    // 4. Register service.
    println!("Registering service '{}'…", service_name);
    register_service(&service_name, &display_name, &description, &install_dir);

    // 5. Optionally start.
    if should_start {
        println!("Starting service '{}'…", service_name);
        start_service(&service_name);
    }

    println!();
    println!("Installation complete.");
    println!();
    #[cfg(unix)]
    println!(
        "  Edit config : {}\n  Service     : systemctl [start|stop|status] {}",
        config_path.display(),
        service_name
    );
    #[cfg(windows)]
    println!(
        "  Edit config : {}\n  Service     : sc [start|stop|query] {}",
        config_path.display(),
        service_name
    );
    pause_if_tty();
}

// uninstall

fn cmd_uninstall(flags: Flags) {
    if let Err(e) = check_privileges() {
        fatal(&e);
    }

    let interactive = !flags.non_interactive && is_tty();

    let service_name = resolve(flags.service_name, interactive, "Service name", "api");
    let install_dir = flags
        .install_dir
        .unwrap_or_else(|| default_install_dir(&service_name));

    if interactive {
        println!("Uninstall summary:");
        println!("  Service name : {}", service_name);
        println!("  Install dir  : {}", install_dir.display());
        println!();
        let ok = Confirm::new()
            .with_prompt("This will permanently delete the install directory. Proceed?")
            .default(false)
            .interact()
            .unwrap_or(false);
        if !ok {
            println!("Aborted.");
            return;
        }
    }

    println!("Stopping service '{}'…", service_name);
    stop_service(&service_name);

    println!("Removing service registration…");
    deregister_service(&service_name);

    if install_dir.exists() {
        println!("Removing {}", install_dir.display());
        fs::remove_dir_all(&install_dir).unwrap_or_else(|e| {
            eprintln!("Warning: could not remove install directory: {}", e);
        });
    }

    println!("Uninstall complete.");
    pause_if_tty();
}

// status

fn cmd_status(flags: Flags) {
    let interactive = !flags.non_interactive && is_tty();
    let service_name = resolve(flags.service_name, interactive, "Service name", "api");
    show_status(&service_name);
    pause_if_tty();
}

// Platform: Linux

#[cfg(unix)]
fn register_service(name: &str, display: &str, desc: &str, install_dir: &Path) {
    let api_bin = install_dir.join("api");
    let config_dir = install_dir.join("config");

    let unit = format!(
        "[Unit]\n\
         Description={desc}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin}\n\
         WorkingDirectory={work}\n\
         Environment=APP_ENV=production\n\
         Environment=APP_CONFIG_DIR={cfg}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier={name}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        desc = desc,
        bin = api_bin.display(),
        work = install_dir.display(),
        cfg = config_dir.display(),
        name = name,
    );

    // Display name is embedded as a comment — systemd doesn't have a separate field.
    let unit = format!("# DisplayName: {}\n{}", display, unit);

    let unit_path = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
    fs::write(&unit_path, &unit).unwrap_or_else(|e| {
        fatal(&format!("Failed to write unit file {}: {}", unit_path.display(), e));
    });
    println!("Wrote {}", unit_path.display());

    run_cmd("systemctl", &["daemon-reload"]);
    run_cmd("systemctl", &["enable", name]);
}

#[cfg(unix)]
fn deregister_service(name: &str) {
    run_cmd("systemctl", &["disable", "--now", name]);
    let unit_path = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
    if unit_path.exists() {
        let _ = fs::remove_file(&unit_path);
        println!("Removed {}", unit_path.display());
    }
    run_cmd("systemctl", &["daemon-reload"]);
}

#[cfg(unix)]
fn start_service(name: &str) {
    run_cmd("systemctl", &["start", name]);
}

#[cfg(unix)]
fn stop_service(name: &str) {
    run_cmd("systemctl", &["stop", name]);
}

#[cfg(unix)]
fn show_status(name: &str) {
    // `systemctl status` exits non-zero when inactive — don't treat that as fatal.
    let _ = process::Command::new("systemctl")
        .args(["status", name])
        .status();
}

// Platform: Windows

#[cfg(windows)]
fn register_service(name: &str, display: &str, desc: &str, install_dir: &Path) {
    let api_bin = install_dir.join("api.exe");
    let config_dir = install_dir.join("config");

    // Create service.
    // sc.exe does its own command-line parsing (not standard argv) so we must
    // NOT double-quote the path here — Command already handles quoting with spaces.
    run_cmd(
        "sc",
        &[
            "create", name,
            "binPath=", api_bin.to_str().unwrap_or(""),
            "start=", "auto",
            "obj=", "LocalSystem",
            "DisplayName=", display,
        ],
    );

    // Set description.
    run_cmd("sc", &["description", name, desc]);

    // Write environment variables for the service into the registry.
    // These are read by the SCM and injected into the service process environment.
    let reg_key = format!(
        r"HKLM\SYSTEM\CurrentControlSet\Services\{}\Environment",
        name
    );

    run_cmd(
        "reg",
        &[
            "add", &reg_key,
            "/v", "APP_ENV",
            "/t", "REG_SZ",
            "/d", "production",
            "/f",
        ],
    );

    run_cmd(
        "reg",
        &[
            "add", &reg_key,
            "/v", "APP_CONFIG_DIR",
            "/t", "REG_SZ",
            "/d", &config_dir.display().to_string(),
            "/f",
        ],
    );
}

#[cfg(windows)]
fn deregister_service(name: &str) {
    run_cmd("sc", &["stop", name]);
    run_cmd("sc", &["delete", name]);
}

#[cfg(windows)]
fn start_service(name: &str) {
    run_cmd("sc", &["start", name]);
}

#[cfg(windows)]
fn stop_service(name: &str) {
    run_cmd("sc", &["stop", name]);
}

#[cfg(windows)]
fn show_status(name: &str) {
    let _ = process::Command::new("sc")
        .args(["query", name])
        .status();
}

// Helpers

/// Return the directory of the currently running installer binary.
fn installer_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve a string value: use `provided` if `Some`, otherwise prompt (if interactive)
/// or fall back to `default_val`.
fn resolve(
    provided: Option<String>,
    interactive: bool,
    prompt: &str,
    default_val: &str,
) -> String {
    if let Some(v) = provided {
        return v;
    }
    if interactive {
        Input::new()
            .with_prompt(prompt)
            .default(default_val.to_string())
            .interact_text()
            .unwrap_or_else(|_| default_val.to_string())
    } else {
        default_val.to_string()
    }
}

fn create_dir_all(path: &Path) {
    if let Err(e) = fs::create_dir_all(path) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            fatal(&format!("Cannot create directory {}: {}", path.display(), e));
        }
    }
}

fn copy_file(src: &Path, dst: &Path) {
    if let Err(e) = fs::copy(src, dst) {
        fatal(&format!(
            "Cannot copy {} → {}: {}",
            src.display(),
            dst.display(),
            e
        ));
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        let _ = fs::set_permissions(path, perms);
    }
}

fn run_cmd(program: &str, args: &[&str]) {
    let status = process::Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|e| {
            fatal(&format!("Failed to run `{} {}`: {}", program, args.join(" "), e));
        });
    if !status.success() {
        eprintln!(
            "Warning: `{} {}` exited with code {}",
            program,
            args.join(" "),
            status.code().unwrap_or(-1)
        );
    }
}

fn is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: isatty() on fd 0 (stdin) is always safe.
        unsafe { tty_sys::isatty(0) != 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let handle = io::stdout().as_raw_handle();
        // FILE_TYPE_CHAR = 2 means the handle is a character device (console).
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

/// When running in an interactive terminal (double-click or manual run), wait for
/// the user to press Enter before exiting so they can read the output.
fn pause_if_tty() {
    if is_tty() {
        println!();
        println!("Press Enter to exit...");
        let stdin = io::stdin();
        let _ = stdin.lock().lines().next();
    }
}

fn fatal(msg: &str) -> ! {
    eprintln!("installer: error: {}", msg);
    pause_if_tty();
    process::exit(1);
}
