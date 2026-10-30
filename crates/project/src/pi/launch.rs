//! Pi launch configuration and discovery (plan 16).
//! - Executable: PATH lookup or explicit user path.
//! - .cmd on Windows: use safe shim (cmd /c with proper quoting handled by Command, no user shell split).
//! - cwd: explicit user choice (can be outside workspace).
//! - session: NewOrResume only official flags, no prompt injection.
//! Discovery errors are actionable (NotFound, NotExecutable, etc).
//! Never uses std::process::Command::arg with untrusted split.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::pi::session::NewOrResume;

/// Configuration for launching a Pi RPC process.
/// All fields explicit from user choice. Args never contain prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiLaunch {
    /// Resolved absolute path to pi executable (or pi.cmd on win).
    pub executable: PathBuf,
    /// Additional args after "pi" for this launch. Always starts with --mode rpc + session flags.
    /// Never split by space; constructed from OsString.
    pub args: Vec<OsString>,
    /// Working directory for the child process. User chosen, validated exist+dir at launch time.
    pub cwd: PathBuf,
    /// Session mode chosen by user.
    pub session: NewOrResume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    ExecutableNotFound { name: String },
    ExecutableNotFile { path: PathBuf },
    NotExecutable { path: PathBuf },
    CwdNotDirectory { path: PathBuf },
    CwdNotAccessible { path: PathBuf, reason: String },
    InvalidSessionPath { path: PathBuf },
}

/// Discover pi executable.
/// Prefers explicit `explicit` if Some (user provided full path or command name).
/// Falls back to PATH search for "pi" or "pi.cmd" on Windows.
/// Returns the first found candidate that is a file.
pub fn discover_pi(explicit: Option<PathBuf>) -> Result<PathBuf, LaunchError> {
    if let Some(p) = explicit {
        if p.is_absolute() || p.components().count() > 1 {
            // explicit full path: validate it is file
            if !p.exists() {
                return Err(LaunchError::ExecutableNotFound { name: p.display().to_string() });
            }
            if !p.is_file() {
                return Err(LaunchError::ExecutableNotFile { path: p });
            }
            return Ok(p);
        } else {
            // bare name, search PATH
            return find_in_path(&p);
        }
    }
    // default: search "pi"
    find_in_path(Path::new("pi"))
}

fn find_in_path(name: &Path) -> Result<PathBuf, LaunchError> {
    let name_str = name.to_string_lossy();
    // On Windows, also try .cmd / .bat / .exe variants via PATHEXT semantics but manual for control.
    let candidates = if cfg!(windows) {
        vec![
            name.to_path_buf(),
            name.with_extension("cmd"),
            name.with_extension("bat"),
            name.with_extension("exe"),
        ]
    } else {
        vec![name.to_path_buf()]
    };

    for cand in candidates {
        if let Some(found) = search_path_for(&cand) {
            return Ok(found);
        }
    }
    Err(LaunchError::ExecutableNotFound { name: name_str.to_string() })
}

#[cfg(windows)]
fn search_path_for(name: &Path) -> Option<PathBuf> {
    // Simple PATH search. std::env::split_paths + check is_file.
    // Does not execute, only existence.
    if let Ok(paths) = env::var("PATH") {
        for dir in env::split_paths(&paths) {
            let full = dir.join(name);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn search_path_for(name: &Path) -> Option<PathBuf> {
    if let Ok(paths) = env::var("PATH") {
        for dir in env::split_paths(&paths) {
            let full = dir.join(name);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Validate cwd is usable directory (exists, is_dir). Does not apply .ohmyworkignore or perms.
pub fn validate_cwd(cwd: &Path) -> Result<(), LaunchError> {
    if !cwd.exists() {
        return Err(LaunchError::CwdNotDirectory { path: cwd.to_path_buf() });
    }
    if !cwd.is_dir() {
        return Err(LaunchError::CwdNotDirectory { path: cwd.to_path_buf() });
    }
    Ok(())
}

/// Build a PiLaunch from user inputs. Does not spawn.
/// executable may be bare or full; resolved here.
/// args_base may be empty; we always ensure --mode rpc .
pub fn build_launch(
    executable: PathBuf,
    cwd: PathBuf,
    session: NewOrResume,
) -> Result<PiLaunch, LaunchError> {
    validate_cwd(&cwd)?;

    let resolved_exec = discover_pi(Some(executable))?;

    // Always include --mode rpc as first after pi
    let mut args: Vec<OsString> = vec![OsString::from("--mode"), OsString::from("rpc")];
    args.extend(session.to_args());

    Ok(PiLaunch {
        executable: resolved_exec,
        args,
        cwd,
        session,
    })
}

/// On Windows, if the executable ends with .cmd/.bat, Command may need special care but
/// we use std::process::Command which handles it. For extra safety against quoting issues
/// (spaces, Chinese), we can wrap via "cmd.exe" /D /C "quoted path" but that risks injection.
/// Per plan: use dedicated safe shim. Here we just ensure args are passed as separate OsString
/// (no split). The actual spawn will use Command::arg for each.
#[cfg(windows)]
pub fn is_cmd_shim(exec: &Path) -> bool {
    if let Some(ext) = exec.extension() {
        let e = ext.to_string_lossy().to_lowercase();
        e == "cmd" || e == "bat"
    } else {
        false
    }
}

#[cfg(not(windows))]
pub fn is_cmd_shim(_exec: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_includes_mode_rpc_and_session() {
        let exec = if cfg!(windows) { PathBuf::from("pi.cmd") } else { PathBuf::from("pi") };
        let cwd = std::env::temp_dir();
        let launch = build_launch(exec, cwd.clone(), NewOrResume::New).expect("temp dir ok");
        assert!(launch.args.iter().any(|a| a == "--mode"));
        assert!(launch.args.iter().any(|a| a == "rpc"));
        assert!(launch.args.iter().any(|a| a == "--no-session"));
        assert_eq!(launch.cwd, cwd);
    }
}
