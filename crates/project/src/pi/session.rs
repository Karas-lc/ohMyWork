//! Session selection model for Pi launch (plan 16).
//! User explicitly chooses New (ephemeral or named) or Resume from explicit session file path.
//! No auto-discovery of history, no binding to Workspace.

use std::path::PathBuf;

/// How to start the Pi session at launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewOrResume {
    /// Start a fresh session. Corresponds to --no-session (ephemeral) or --name if provided later.
    /// For V1 launch, simple new uses --no-session.
    New,
    /// Resume specific persisted session by its .jsonl file path (from user file picker).
    /// Passed as --session <path>
    Resume(PathBuf),
}

impl NewOrResume {
    /// Build the CLI args fragment for this mode. Caller prepends --mode rpc etc.
    pub fn to_args(&self) -> Vec<std::ffi::OsString> {
        use std::ffi::OsString;
        match self {
            NewOrResume::New => vec![OsString::from("--no-session")],
            NewOrResume::Resume(p) => vec![OsString::from("--session"), p.clone().into_os_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_to_args() {
        use std::ffi::OsString;
        assert_eq!(NewOrResume::New.to_args(), vec![OsString::from("--no-session")]);
    }

    #[test]
    fn resume_to_args() {
        use std::ffi::OsString;
        let p = PathBuf::from(r"C:\Users\test\session.jsonl");
        assert_eq!(
            NewOrResume::Resume(p.clone()).to_args(),
            vec![OsString::from("--session"), p.clone().into_os_string()]
        );
    }
}
