//! Diff viewer for commit details

use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Run diff viewer for a commit
pub fn run_commit(repo_path: &Path, commit_ref: &str) -> Result<()> {
    let show_output = Command::new("git")
        .current_dir(repo_path)
        .args(["show", "--color=always", commit_ref])
        .output()?;

    // Use less as pager for commit view
    let mut child = Command::new("less")
        .arg("-R")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(&show_output.stdout)?;
    }
    child.wait()?;

    Ok(())
}
