/// Standalone updater binary.
///
/// This binary is spawned by the running API/worker process when an update is
/// triggered. It must be placed next to the target binary so the API can locate it.
///
/// Usage (invoked by the API — do not run manually unless testing):
/// ```
/// updater \
///   --zip-path    /tmp/api-update-1.2.0.zip \
///   --target-path /usr/local/bin/api \
///   --binary-name api \
///   --pid         12345 \
///   --restart-mode spawn|supervisor
/// ```
///
/// Steps:
///  1. Parse CLI arguments.
///  2. Wait (up to 30 s) for the process identified by `--pid` to exit.
///  3. Extract `--binary-name` from the zip archive into a temporary path.
///  4. Back up the existing binary (`target.bak`).
///  5. Move the new binary into place.
///  6. On Unix: make the new binary executable (`chmod +x`).
///  7. If `--restart-mode spawn`: exec the new binary.
///  8. Remove the backup and the source zip.
///  9. Exit 0.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs, io, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    let parsed = parse_args(&args).unwrap_or_else(|e| {
        eprintln!("updater: argument error: {}", e);
        process::exit(2);
    });

    eprintln!(
        "updater: starting (pid to wait={}, zip={}, target={}, binary={})",
        parsed.pid,
        parsed.zip_path.display(),
        parsed.target_path.display(),
        parsed.binary_name,
    );

    // 1. Wait for the calling process to exit.
    if parsed.pid != 0 {
        if !wait_for_pid_exit(parsed.pid, 30) {
            eprintln!(
                "updater: process {} did not exit within 30 seconds; proceeding anyway",
                parsed.pid
            );
        }
    }

    // 2. Extract the binary from the zip into a temporary location.
    let tmp_extracted = extract_binary(&parsed.zip_path, &parsed.binary_name).unwrap_or_else(|e| {
        eprintln!("updater: extraction failed: {}", e);
        process::exit(1);
    });

    // 3. Replace the target binary.
    replace_binary(&tmp_extracted, &parsed.target_path).unwrap_or_else(|e| {
        eprintln!("updater: replacement failed: {}", e);
        // Try to restore from backup before exiting.
        let backup = backup_path(&parsed.target_path);
        if backup.exists() {
            let _ = fs::rename(&backup, &parsed.target_path);
        }
        process::exit(1);
    });

    eprintln!("updater: binary replaced successfully");

    // 4. Optionally restart.
    if parsed.restart_mode == "spawn" {
        eprintln!(
            "updater: spawning new process: {}",
            parsed.target_path.display()
        );
        let err = exec_new_process(&parsed.target_path);
        eprintln!("updater: exec failed: {}", err);
        process::exit(1);
    } else {
        eprintln!("updater: supervisor mode — exiting; let the process manager restart the service");
    }

    // 5. Clean up zip archive.
    let _ = fs::remove_file(&parsed.zip_path);

    process::exit(0);
}

// Args

struct Args {
    zip_path: PathBuf,
    target_path: PathBuf,
    binary_name: String,
    pid: u32,
    restart_mode: String,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut zip_path: Option<PathBuf> = None;
    let mut target_path: Option<PathBuf> = None;
    let mut binary_name: Option<String> = None;
    let mut pid: u32 = 0;
    let mut restart_mode = "supervisor".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--zip-path" => {
                i += 1;
                zip_path = Some(PathBuf::from(
                    args.get(i).ok_or("missing value for --zip-path")?,
                ));
            }
            "--target-path" => {
                i += 1;
                target_path = Some(PathBuf::from(
                    args.get(i).ok_or("missing value for --target-path")?,
                ));
            }
            "--binary-name" => {
                i += 1;
                binary_name = Some(
                    args.get(i)
                        .ok_or("missing value for --binary-name")?
                        .clone(),
                );
            }
            "--pid" => {
                i += 1;
                let raw = args.get(i).ok_or("missing value for --pid")?;
                pid = raw.parse::<u32>().map_err(|_| "invalid --pid value")?;
            }
            "--restart-mode" => {
                i += 1;
                restart_mode = args
                    .get(i)
                    .ok_or("missing value for --restart-mode")?
                    .clone();
            }
            other => return Err(format!("unknown argument: {}", other)),
        }
        i += 1;
    }

    Ok(Args {
        zip_path: zip_path.ok_or("--zip-path is required")?,
        target_path: target_path.ok_or("--target-path is required")?,
        binary_name: binary_name.ok_or("--binary-name is required")?,
        pid,
        restart_mode,
    })
}

// PID wait

/// Poll until the given PID no longer appears in the process table.
/// Returns `true` if the process exited before the timeout, `false` otherwise.
fn wait_for_pid_exit(pid: u32, timeout_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        if !pid_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // `kill(pid, 0)` succeeds (returns 0) if the process exists and we have
    // permission to signal it, even when no signal is actually sent.
    unsafe { unix_sys::kill(pid as i32, 0) == 0 }
}

#[cfg(unix)]
mod unix_sys {
    unsafe extern "C" {
        pub fn kill(pid: i32, sig: i32) -> i32;
        pub fn execv(path: *const i8, argv: *const *const i8) -> i32;
    }
}

#[cfg(not(unix))]
fn pid_alive(pid: u32) -> bool {
    // On Windows, check if the process directory in the snapshot contains the PID.
    // We use the `tasklist` approach via sysinfo (which is already a crate dep).
    use std::process::Command;
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains(&pid.to_string())
        }
        Err(_) => false, // assume gone
    }
}

// Extraction

/// Extract `binary_name` from the zip archive at `zip_path` and write it to a
/// temporary file next to `zip_path`. Returns the path to the extracted file.
fn extract_binary(zip_path: &Path, binary_name: &str) -> io::Result<PathBuf> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // Try exact match first, then basename match (handles subdirectory paths in zip).
    let idx = (0..archive.len())
        .find(|&i| {
            archive
                .by_index_raw(i)
                .map(|f| {
                    let name = f.name().to_string();
                    name == binary_name
                        || name.ends_with(&format!("/{}", binary_name))
                        || name.ends_with(&format!("\\{}", binary_name))
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("'{}' not found inside zip archive", binary_name),
            )
        })?;

    let mut entry = archive
        .by_index(idx)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let out_path = zip_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{}.new", binary_name));

    let mut out_file = fs::File::create(&out_path)?;
    io::copy(&mut entry, &mut out_file)?;

    Ok(out_path)
}

// Replacement

fn backup_path(target: &Path) -> PathBuf {
    let mut p = target.to_path_buf();
    let ext = match p.extension() {
        Some(e) => format!("{}.bak", e.to_string_lossy()),
        None => "bak".to_string(),
    };
    p.set_extension(ext);
    p
}

fn replace_binary(new_bin: &Path, target: &Path) -> io::Result<()> {
    let backup = backup_path(target);

    // Remove stale backup if present.
    if backup.exists() {
        fs::remove_file(&backup)?;
    }

    // Back up existing binary.
    if target.exists() {
        fs::rename(target, &backup)?;
    }

    // Move new binary into place.
    // `fs::rename` may fail across mount points; fall back to copy+remove.
    if let Err(_) = fs::rename(new_bin, target) {
        fs::copy(new_bin, target)?;
        fs::remove_file(new_bin)?;
    }

    // On Unix, set executable bit.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(target)?.permissions();
        let mode = perms.mode() | 0o111; // set +x for owner, group, other
        perms.set_mode(mode);
        fs::set_permissions(target, perms)?;
    }

    // Remove backup only after successful replacement.
    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }

    Ok(())
}

// Exec / spawn

/// Replace the current process with the new binary (Unix exec) or spawn it (Windows).
fn exec_new_process(target: &Path) -> io::Error {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(target.as_os_str().as_bytes())
            .expect("target path contains null byte");
        let argv: Vec<CString> = vec![path.clone()];
        let argv_ptrs: Vec<*const i8> = argv
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        unsafe { unix_sys::execv(path.as_ptr(), argv_ptrs.as_ptr()) };
        io::Error::last_os_error()
    }

    #[cfg(not(unix))]
    {
        match process::Command::new(target).spawn() {
            Ok(_) => io::Error::new(io::ErrorKind::Other, "spawned (non-exec platform)"),
            Err(e) => e,
        }
    }
}
