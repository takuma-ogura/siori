mod app;
pub mod config;
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
        println!();
        println!("Options:");
        println!("  --check    Run checks without starting TUI");
        println!("  --help     Show this help message");
        println!();
        println!("Keybindings (Files tab):");
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
        println!("  j/k/Up/Down Navigate commits");
        println!("  e          Edit commit message (amend HEAD)");
        println!("  t          Create/edit tag");
        println!("  T          Push all tags");
        println!("  d          Delete tag");
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
