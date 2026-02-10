mod app;
pub mod config;
mod diff_viewer;
pub mod ui;
pub mod version;

use anyhow::{Context, Result};
use crossterm::{
    ExecutableCommand,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use git2::{Repository, Status, StatusOptions};
use std::io::stdout;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn run() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    let mut terminal = ratatui::Terminal::new(ratatui::prelude::CrosstermBackend::new(stdout()))?;

    let mut app = app::App::new()?;
    let mut last_activity = Instant::now();
    let mut last_refresh = Instant::now();

    let mut last_spinner_tick = Instant::now();

    while app.running {
        terminal.draw(|f| ui::ui(f, &mut app))?;

        // Tick spinner animation (every 80ms)
        if app.processing.is_active() {
            if last_spinner_tick.elapsed() >= Duration::from_millis(80) {
                app.tick_spinner();
                last_spinner_tick = Instant::now();
            }
            // Check if background operation completed
            app.check_processing()?;
        }

        // 16ms polling for ~60fps responsiveness
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Block input during processing
                    if !app.processing.is_active() {
                        app.handle_key(key.code, key.modifiers)?;
                        last_activity = Instant::now();
                    }
                }
                Event::Mouse(mouse) => {
                    if !app.processing.is_active() {
                        app.handle_mouse(mouse)?;
                        last_activity = Instant::now();
                    }
                }
                _ => {}
            }
        }

        // Auto-refresh ONLY when idle (no input for 2+ seconds) and not processing
        let idle_time = last_activity.elapsed();
        if !app.processing.is_active()
            && idle_time >= Duration::from_secs(2)
            && last_refresh.elapsed() >= Duration::from_secs(3)
        {
            let _ = app.refresh_status_only();
            last_refresh = Instant::now();
        }
    }

    disable_raw_mode()?;
    stdout().execute(DisableMouseCapture)?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle "siori diff" subcommand
    if args.len() >= 2 && args[1] == "diff" {
        if let Err(e) = diff_mode(&args[2..]) {
            eprintln!("Error: {:#}", e);
            std::process::exit(1);
        }
        return;
    }

    if args.iter().any(|a| a == "--check") {
        match check_mode() {
            Ok(_) => {
                println!("siori: All checks passed!");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("siori: Check failed: {:#}", e);
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("siori - minimal git TUI");
        println!();
        println!("Usage: siori [OPTIONS]");
        println!("       siori diff [-C <path>] <commit>              Show diff for commit");
        println!("       siori diff [-C <path>] --file <path>         Show file diff (unstaged)");
        println!("       siori diff [-C <path>] --file <path> --staged Show file diff (staged)");
        println!();
        println!("Options:");
        println!("  --check    Run checks without starting TUI");
        println!("  --help     Show this help message");
        println!();
        println!("Keybindings (Files tab):");
        println!("  Enter      Copy diff command to clipboard");
        println!("  Space      Stage/unstage file");
        println!("  c          Enter commit message");
        println!("  P          Push to remote");
        println!("  r          Switch repository (for nested repos)");
        println!("  R          Refresh (full reload)");
        println!("  j/k/Up/Down Navigate files");
        println!("  Tab        Switch to Log tab");
        println!("  q          Quit");
        println!();
        println!("Keybindings (Log tab):");
        println!("  Enter      Copy diff command to clipboard");
        println!("  j/k/Up/Down Navigate commits");
        println!("  e          Edit commit message (amend HEAD)");
        println!("  t          Create/edit tag");
        println!("  T          Push all tags");
        println!("  x          Delete tag");
        println!("  P          Push to remote");
        println!("  p          Pull from remote");
        println!("  r          Switch repository (for nested repos)");
        println!("  R          Refresh (full reload)");
        println!("  Tab        Switch to Files tab");
        println!("  q          Quit");
        println!();
        println!("Mouse:");
        println!("  Click      Select item / Switch tab");
        println!("  Scroll     Navigate up/down");
        std::process::exit(0);
    }

    if let Err(e) = run() {
        let err_str = format!("{:#}", e);
        if err_str.contains("Device not configured") || err_str.contains("not a terminal") {
            eprintln!("siori: Cannot start TUI - no terminal detected.");
            eprintln!("       Run 'siori --check' to verify repository status.");
        } else {
            eprintln!("Error: {}", err_str);
        }
        std::process::exit(1);
    }
}

fn check_mode() -> Result<()> {
    let repo = Repository::discover(".").context("Not a git repository")?;
    let branch = match repo.head() {
        Ok(head) => head.shorthand().unwrap_or("HEAD").to_string(),
        Err(_) => "(no commits yet)".to_string(),
    };
    println!("Branch: {}", branch);

    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let staged = statuses
        .iter()
        .filter(|e| {
            e.status()
                .intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED)
        })
        .count();
    let unstaged = statuses
        .iter()
        .filter(|e| {
            e.status()
                .intersects(Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED)
        })
        .count();

    println!("Staged: {} files", staged);
    println!("Changes: {} files", unstaged);

    let commit_count = if let Ok(mut revwalk) = repo.revwalk() {
        if revwalk.push_head().is_ok() {
            revwalk.take(10).count()
        } else {
            0
        }
    } else {
        0
    };
    println!("Recent commits: {}", commit_count);
    Ok(())
}

fn diff_mode(args: &[String]) -> Result<()> {
    // Parse -C option for repository path
    let repo_path: PathBuf = if let Some(idx) = args.iter().position(|a| a == "-C") {
        args.get(idx + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing path after -C"))?
            .into()
    } else {
        let repo = Repository::discover(".").context("Not a git repository")?;
        repo.workdir()
            .ok_or_else(|| anyhow::anyhow!("Not a git working directory"))?
            .to_path_buf()
    };

    // Filter out -C and its argument for remaining parsing
    let filtered_args: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            *a != "-C" && args.get(i.wrapping_sub(1)).map(|s| s.as_str()) != Some("-C")
        })
        .map(|(_, a)| a)
        .collect();

    // Parse arguments
    let is_file_mode = filtered_args.iter().any(|a| *a == "--file");
    let is_staged = filtered_args.iter().any(|a| *a == "--staged");

    if is_file_mode {
        // Find file path (argument after --file)
        let file_idx = filtered_args.iter().position(|a| *a == "--file");
        let file_path = file_idx
            .and_then(|i| filtered_args.get(i + 1))
            .ok_or_else(|| anyhow::anyhow!("Missing file path after --file"))?;

        open_editor_diff(&repo_path, file_path, is_staged)
    } else {
        // Commit mode: show diff for a specific commit
        let commit_ref = filtered_args.first().map(|s| s.as_str()).unwrap_or("HEAD");
        diff_viewer::run_commit(&repo_path, commit_ref)
    }
}

/// Open editor with diff highlights: changed lines shown in green, jump to first change.
fn open_editor_diff(repo_path: &std::path::Path, file_path: &str, staged: bool) -> Result<()> {
    let editor_cmd = config::Config::load().editor.resolve();
    let full_path = repo_path.join(file_path);

    // Parse git diff to find changed line numbers
    let diff_args = if staged {
        vec!["diff", "--cached", "-U0", "--", file_path]
    } else {
        vec!["diff", "-U0", "--", file_path]
    };
    let diff_output = Command::new("git")
        .current_dir(repo_path)
        .args(&diff_args)
        .output();

    let mut added_lines: Vec<usize> = Vec::new();
    if let Ok(output) = diff_output {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.starts_with("@@") {
                // @@ -old,count +new,count @@
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let new_part = parts[2].trim_start_matches('+');
                    let mut split = new_part.split(',');
                    let start: usize = split.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                    let count: usize = split.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                    for n in start..start + count {
                        added_lines.push(n);
                    }
                }
            }
        }
    }

    let parts: Vec<&str> = editor_cmd.split_whitespace().collect();
    let (cmd, extra_args) = parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty editor command"))?;

    let mut command = Command::new(cmd);
    command.args(extra_args);

    // Jump to first changed line
    if let Some(&first) = added_lines.first() {
        command.arg(format!("+{}", first));
    }

    command.arg(&full_path);

    // Highlight changed lines with green background
    if !added_lines.is_empty() {
        let pattern = added_lines
            .iter()
            .map(|n| format!("\\%{}l", n))
            .collect::<Vec<_>>()
            .join("\\|");
        command.arg("-c").arg(format!(
            "hi SioriDiff guibg=#2d4a3e ctermbg=22 | match SioriDiff /{}/",
            pattern
        ));
    }

    let status = command.status()?;
    if !status.success() {
        anyhow::bail!("Editor exited with status {}", status);
    }
    Ok(())
}
