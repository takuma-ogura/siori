use anyhow::{Context, Result};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

// Debug logging helper
fn debug_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/siori_debug.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

use crossterm::{
    ExecutableCommand,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use git2::{DiffOptions, Repository, Status, StatusOptions};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
};
use std::io::stdout;

// ============================================================================
// Color Scheme (Tokyo Night)
// ============================================================================
mod colors {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(26, 27, 38);
    pub const FG: Color = Color::Rgb(169, 177, 214);
    pub const FG_BRIGHT: Color = Color::Rgb(192, 202, 245);
    pub const GREEN: Color = Color::Rgb(158, 206, 106);
    pub const YELLOW: Color = Color::Rgb(224, 175, 104);
    pub const RED: Color = Color::Rgb(247, 118, 142);
    pub const BLUE: Color = Color::Rgb(122, 162, 247);
    pub const PURPLE: Color = Color::Rgb(187, 154, 247);
    pub const DIM: Color = Color::Rgb(86, 95, 137);
    pub const SELECTED: Color = Color::Rgb(40, 52, 87);
}

// ============================================================================
// Types
// ============================================================================
#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum Tab {
    #[default]
    Files,
    Log,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum InputMode {
    #[default]
    Normal,
    Insert,
}

#[derive(Clone)]
struct FileEntry {
    path: String,
    status: FileStatus,
    staged: bool,
    diff_stats: Option<(usize, usize)>, // (additions, deletions)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum FileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
}

#[derive(Clone)]
struct CommitEntry {
    id: String,
    message: String,
    time: String,
    is_head: bool,
    remote_branches: Vec<String>,
}

/// 視覚リストのエントリ - files配列のどのエントリを指すか
#[derive(Clone, Copy)]
struct VisualEntry {
    file_index: usize,
}

struct App {
    tab: Tab,
    running: bool,
    input_mode: InputMode,
    commit_message: String,
    files: Vec<FileEntry>,
    visual_list: Vec<VisualEntry>, // 視覚的な表示順序
    commits: Vec<CommitEntry>,
    files_state: ListState,
    commits_state: ListState,
    branch_name: String,
    ahead_behind: Option<(usize, usize)>,
    message: Option<(String, bool)>, // (message, is_error)
    repo: Repository,
}

impl App {
    fn new() -> Result<Self> {
        let repo = Repository::discover(".").context("Not a git repository")?;
        let mut app = Self {
            tab: Tab::default(),
            running: true,
            input_mode: InputMode::default(),
            commit_message: String::new(),
            files: Vec::new(),
            visual_list: Vec::new(),
            commits: Vec::new(),
            files_state: ListState::default(),
            commits_state: ListState::default(),
            branch_name: String::new(),
            ahead_behind: None,
            message: None,
            repo,
        };
        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        self.refresh_status()?;
        self.refresh_branch_info()?;
        self.refresh_log()?;
        Ok(())
    }

    fn refresh_status(&mut self) -> Result<()> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);

        let statuses = self.repo.statuses(Some(&mut opts))?;
        self.files.clear();
        self.visual_list.clear();

        // Pass 1: Collect all staged files
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            if status.intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED)
            {
                let file_status = if status.contains(Status::INDEX_NEW) {
                    FileStatus::Added
                } else if status.contains(Status::INDEX_DELETED) {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                let diff_stats = self.get_diff_stats(&path, true);
                self.files.push(FileEntry {
                    path,
                    status: file_status,
                    staged: true,
                    diff_stats,
                });
            }
        }

        // Pass 2: Collect all unstaged/untracked files
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            if status.intersects(Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED) {
                let file_status = if status.contains(Status::WT_NEW) {
                    FileStatus::Untracked
                } else if status.contains(Status::WT_DELETED) {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                let diff_stats = self.get_diff_stats(&path, false);
                self.files.push(FileEntry {
                    path,
                    status: file_status,
                    staged: false,
                    diff_stats,
                });
            }
        }

        // Build visual_list: maps visual position to files index
        // STAGED section first, then CHANGES section
        for (i, file) in self.files.iter().enumerate() {
            if file.staged {
                self.visual_list.push(VisualEntry { file_index: i });
            }
        }
        for (i, file) in self.files.iter().enumerate() {
            if !file.staged {
                self.visual_list.push(VisualEntry { file_index: i });
            }
        }

        // DEBUG: Log state after building visual_list
        debug_log("\n=== refresh_status() completed ===");
        debug_log(&format!("files.len() = {}", self.files.len()));
        for (i, file) in self.files.iter().enumerate() {
            debug_log(&format!(
                "  files[{}] = {} | staged={} | status={:?}",
                i, file.path, file.staged, file.status
            ));
        }
        debug_log(&format!("visual_list.len() = {}", self.visual_list.len()));
        for (i, ve) in self.visual_list.iter().enumerate() {
            let file = &self.files[ve.file_index];
            debug_log(&format!(
                "  visual_list[{}] -> files[{}] = {} (staged={})",
                i, ve.file_index, file.path, file.staged
            ));
        }
        debug_log(&format!("files_state.selected() = {:?}", self.files_state.selected()));

        // Select first item if nothing selected
        if self.files_state.selected().is_none() && !self.visual_list.is_empty() {
            self.files_state.select(Some(0));
        }

        // Clamp selection to valid range
        if let Some(idx) = self.files_state.selected()
            && idx >= self.visual_list.len()
        {
            let new_idx = if self.visual_list.is_empty() {
                None
            } else {
                Some(self.visual_list.len() - 1)
            };
            self.files_state.select(new_idx);
        }

        Ok(())
    }

    fn get_diff_stats(&self, path: &str, staged: bool) -> Option<(usize, usize)> {
        let mut opts = DiffOptions::new();
        opts.pathspec(path);

        let diff = if staged {
            let head = self.repo.head().ok()?.peel_to_tree().ok()?;
            self.repo
                .diff_tree_to_index(Some(&head), None, Some(&mut opts))
                .ok()?
        } else {
            self.repo
                .diff_index_to_workdir(None, Some(&mut opts))
                .ok()?
        };

        let stats = diff.stats().ok()?;
        Some((stats.insertions(), stats.deletions()))
    }

    fn refresh_branch_info(&mut self) -> Result<()> {
        if let Ok(head) = self.repo.head() {
            self.branch_name = head.shorthand().unwrap_or("HEAD").to_string();

            // Get ahead/behind
            if let (Ok(local), Ok(remote)) = (
                head.peel_to_commit().map(|c| c.id()),
                self.repo
                    .find_branch(
                        &format!("origin/{}", self.branch_name),
                        git2::BranchType::Remote,
                    )
                    .and_then(|b| b.get().peel_to_commit().map(|c| c.id())),
            ) && let Ok((ahead, behind)) = self.repo.graph_ahead_behind(local, remote)
            {
                self.ahead_behind = Some((ahead, behind));
            }
        } else {
            self.branch_name = "(no commits)".to_string();
        }
        Ok(())
    }

    fn refresh_log(&mut self) -> Result<()> {
        self.commits.clear();

        // Handle unborn branch (no commits yet)
        let Ok(mut revwalk) = self.repo.revwalk() else {
            return Ok(());
        };
        if revwalk.push_head().is_err() {
            return Ok(()); // No commits yet
        }
        // Ignore sorting errors on edge cases
        let _ = revwalk.set_sorting(git2::Sort::TIME);

        let head_id = self.repo.head().ok().and_then(|h| h.target());

        // Get remote branch OIDs
        let mut remote_refs: std::collections::HashMap<git2::Oid, Vec<String>> =
            std::collections::HashMap::new();
        if let Ok(refs) = self.repo.references() {
            for reference in refs.flatten() {
                if let Some(name) = reference.name()
                    && name.starts_with("refs/remotes/")
                    && let Ok(commit) = reference.peel_to_commit()
                {
                    let short_name = name.strip_prefix("refs/remotes/").unwrap_or(name);
                    remote_refs
                        .entry(commit.id())
                        .or_default()
                        .push(short_name.to_string());
                }
            }
        }

        for (i, oid) in revwalk.enumerate() {
            if i >= 100 {
                break;
            }
            // Skip invalid commits instead of crashing
            let Ok(oid) = oid else { continue };
            let Ok(commit) = self.repo.find_commit(oid) else {
                continue;
            };

            let time = commit.time();
            let time_str = format_relative_time(time.seconds());

            self.commits.push(CommitEntry {
                id: format!("{:.7}", oid),
                message: commit.summary().unwrap_or("").to_string(),
                time: time_str,
                is_head: Some(oid) == head_id,
                remote_branches: remote_refs.get(&oid).cloned().unwrap_or_default(),
            });
        }

        if self.commits_state.selected().is_none() && !self.commits.is_empty() {
            self.commits_state.select(Some(0));
        }

        Ok(())
    }

    fn stage_selected(&mut self) -> Result<()> {
        debug_log("\n=== stage_selected() called ===");
        debug_log(&format!("files_state.selected() = {:?}", self.files_state.selected()));
        debug_log(&format!("visual_list.len() = {}", self.visual_list.len()));
        debug_log(&format!("files.len() = {}", self.files.len()));

        let Some(visual_idx) = self.files_state.selected() else {
            debug_log("ERROR: No file selected");
            self.message = Some(("No file selected".to_string(), true));
            return Ok(());
        };
        debug_log(&format!("visual_idx = {}", visual_idx));

        let Some(visual_entry) = self.visual_list.get(visual_idx) else {
            debug_log(&format!("ERROR: visual_list[{}] out of bounds", visual_idx));
            self.message = Some(("Invalid selection".to_string(), true));
            return Ok(());
        };
        debug_log(&format!("visual_entry.file_index = {}", visual_entry.file_index));

        let Some(file) = self.files.get(visual_entry.file_index) else {
            debug_log(&format!("ERROR: files[{}] out of bounds", visual_entry.file_index));
            self.message = Some(("File not found".to_string(), true));
            return Ok(());
        };
        debug_log(&format!(
            "file = {} | staged={} | status={:?}",
            file.path, file.staged, file.status
        ));

        let mut index = self.repo.index()?;
        let file_path = file.path.clone();
        let file_status = file.status;
        let is_staged = file.staged;
        debug_log(&format!("Action: {} (is_staged={})", if is_staged { "UNSTAGE" } else { "STAGE" }, is_staged));

        if is_staged {
            // Unstage
            if file_status == FileStatus::Added {
                // INDEX_NEW: remove from index (for new files or initial repo)
                index.remove_path(std::path::Path::new(&file_path))?;
                index.write()?;
                self.message = Some((format!("Unstaged (new): {}", file_path), false));
            } else if let Ok(head_commit) = self.repo.head().and_then(|h| h.peel_to_commit()) {
                // INDEX_MODIFIED/DELETED: reset to HEAD
                match self
                    .repo
                    .reset_default(Some(head_commit.as_object()), [&file_path])
                {
                    Ok(_) => {
                        self.message = Some((format!("Unstaged: {}", file_path), false));
                    }
                    Err(e) => {
                        self.message = Some((format!("Unstage failed: {}", e), true));
                    }
                }
            } else {
                self.message = Some(("Cannot unstage: no HEAD".to_string(), true));
            }
        } else {
            // Stage
            if file_status == FileStatus::Deleted {
                index.remove_path(std::path::Path::new(&file_path))?;
            } else {
                index.add_path(std::path::Path::new(&file_path))?;
            }
            index.write()?;
            self.message = Some((format!("Staged: {}", file_path), false));
        }

        // Remember file info before refresh
        let target_path = file_path.clone();
        let target_staged = !is_staged; // After operation, staged status is flipped

        self.refresh_status()?;

        // Find the file in its new position and update selection
        for (i, ve) in self.visual_list.iter().enumerate() {
            if let Some(f) = self.files.get(ve.file_index)
                && f.path == target_path
                && f.staged == target_staged
            {
                self.files_state.select(Some(i));
                debug_log(&format!(
                    "Cursor followed to visual_idx={} ({})",
                    i, target_path
                ));
                break;
            }
        }

        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        let message = self.commit_message.trim();
        if message.is_empty() {
            self.message = Some(("Empty commit message".to_string(), true));
            return Ok(());
        }

        // Perform commit in a block to release borrows before refresh
        {
            let mut index = self.repo.index()?;
            let tree_id = index.write_tree()?;
            let tree = self.repo.find_tree(tree_id)?;
            let signature = self.repo.signature()?;

            // Handle initial commit (no parent)
            let parent_commit = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());

            match parent_commit {
                Some(parent) => {
                    self.repo.commit(
                        Some("HEAD"),
                        &signature,
                        &signature,
                        message,
                        &tree,
                        &[&parent],
                    )?;
                }
                None => {
                    // Initial commit
                    self.repo
                        .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;
                }
            }
        }

        self.commit_message.clear();
        self.input_mode = InputMode::Normal;
        self.message = Some(("Committed successfully".to_string(), false));
        self.refresh()?;
        Ok(())
    }

    fn push(&mut self) -> Result<()> {
        // Use git command for push (handles authentication)
        let output = std::process::Command::new("git")
            .args(["push"])
            .output()
            .context("Failed to execute git push")?;

        if output.status.success() {
            self.message = Some(("Pushed successfully".to_string(), false));
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            self.message = Some((format!("Push failed: {}", err.trim()), true));
        }
        self.refresh()?;
        Ok(())
    }

    fn pull(&mut self) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["pull"])
            .output()
            .context("Failed to execute git pull")?;

        if output.status.success() {
            self.message = Some(("Pulled successfully".to_string(), false));
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            self.message = Some((format!("Pull failed: {}", err.trim()), true));
        }
        self.refresh()?;
        Ok(())
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        // Normalize full-width characters to ASCII (for Japanese IME support)
        let code = match code {
            KeyCode::Char(c) => KeyCode::Char(normalize_fullwidth(c)),
            other => other,
        };

        debug_log(&format!(
            "\n=== handle_key() | code={:?} | input_mode={:?} | tab={:?} ===",
            code, self.input_mode, self.tab
        ));

        // Clear message on any key
        self.message = None;

        match self.input_mode {
            InputMode::Insert => match code {
                KeyCode::Esc => self.input_mode = InputMode::Normal,
                KeyCode::Enter => self.commit()?,
                KeyCode::Backspace => {
                    self.commit_message.pop();
                }
                KeyCode::Char(c) => self.commit_message.push(c),
                _ => {}
            },
            InputMode::Normal => match code {
                KeyCode::Char('q') => self.running = false,
                KeyCode::Tab => {
                    self.tab = match self.tab {
                        Tab::Files => Tab::Log,
                        Tab::Log => Tab::Files,
                    };
                }
                KeyCode::Char('j') | KeyCode::Down => self.select_next(),
                KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
                KeyCode::Char(' ') if self.tab == Tab::Files => self.stage_selected()?,
                KeyCode::Char('c') if self.tab == Tab::Files => {
                    self.input_mode = InputMode::Insert;
                }
                KeyCode::Char('P') => self.push()?,
                KeyCode::Char('p') if self.tab == Tab::Log => self.pull()?,
                KeyCode::Char('r') => self.refresh()?,
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.running = false;
                }
                _ => {}
            },
        }
        Ok(())
    }

    fn select_next(&mut self) {
        match self.tab {
            Tab::Files => {
                let len = self.visual_list.len();
                if len > 0 {
                    let i = self.files_state.selected().unwrap_or(0);
                    self.files_state.select(Some((i + 1) % len));
                }
            }
            Tab::Log => {
                let len = self.commits.len();
                if len > 0 {
                    let i = self.commits_state.selected().unwrap_or(0);
                    self.commits_state.select(Some((i + 1) % len));
                }
            }
        }
    }

    fn select_prev(&mut self) {
        match self.tab {
            Tab::Files => {
                let len = self.visual_list.len();
                if len > 0 {
                    let i = self.files_state.selected().unwrap_or(0);
                    self.files_state
                        .select(Some(if i == 0 { len - 1 } else { i - 1 }));
                }
            }
            Tab::Log => {
                let len = self.commits.len();
                if len > 0 {
                    let i = self.commits_state.selected().unwrap_or(0);
                    self.commits_state
                        .select(Some(if i == 0 { len - 1 } else { i - 1 }));
                }
            }
        }
    }

    fn select_index(&mut self, index: usize) {
        debug_log(&format!("select_index({}) called", index));
        match self.tab {
            Tab::Files => {
                if index < self.visual_list.len() {
                    self.files_state.select(Some(index));
                    debug_log(&format!("files_state.selected() now = {:?}", self.files_state.selected()));
                } else {
                    debug_log(&format!("index {} >= visual_list.len() {}", index, self.visual_list.len()));
                }
            }
            Tab::Log => {
                if index < self.commits.len() {
                    self.commits_state.select(Some(index));
                }
            }
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent) -> Result<()> {
        match event.kind {
            MouseEventKind::ScrollDown => self.select_next(),
            MouseEventKind::ScrollUp => self.select_prev(),
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_click(event.column, event.row)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_click(&mut self, _x: u16, y: u16) -> Result<()> {
        debug_log(&format!("\n=== handle_click() | y={} ===", y));

        // Layout: y=0 title, y=1-3 tabs, y=4-6 commit input, y=7+ files/commits
        // Tab area (y=1-3): click to switch tabs
        if (1..=3).contains(&y) {
            debug_log("Clicked on tab area, switching tabs");
            self.tab = match self.tab {
                Tab::Files => Tab::Log,
                Tab::Log => Tab::Files,
            };
            return Ok(());
        }

        // File/commit list area
        match self.tab {
            Tab::Files => {
                // Layout: y=7 STAGED header, y=8+ files
                if y >= 8 {
                    let clicked_row = (y - 8) as usize;
                    let staged_count = self.visual_list.iter()
                        .filter(|v| self.files.get(v.file_index).is_some_and(|f| f.staged))
                        .count();

                    debug_log(&format!("clicked_row={} staged_count={}", clicked_row, staged_count));

                    // Row 0 = STAGED header (skip)
                    // Row 1..=staged_count = staged files (visual_list[0..staged_count])
                    // Row staged_count+1 = CHANGES header (skip)
                    // Row staged_count+2.. = unstaged files (visual_list[staged_count..])
                    let visual_index = if clicked_row == 0 {
                        debug_log("Clicked on STAGED header");
                        None // STAGED header
                    } else if clicked_row <= staged_count {
                        debug_log(&format!("Clicked on staged file, visual_index={}", clicked_row - 1));
                        Some(clicked_row - 1)
                    } else if clicked_row == staged_count + 1 {
                        debug_log("Clicked on CHANGES header");
                        None // CHANGES header
                    } else {
                        let idx = staged_count + (clicked_row - staged_count - 2);
                        debug_log(&format!("Clicked on unstaged file, visual_index={}", idx));
                        Some(idx)
                    };

                    if let Some(idx) = visual_index
                        && idx < self.visual_list.len()
                    {
                        debug_log(&format!("Selecting visual_index={}", idx));
                        self.select_index(idx);
                    }
                }
            }
            Tab::Log => {
                // Log tab: y=4-5 is branch info, y=6+ is commit list (2 lines per commit)
                if y >= 6 {
                    let clicked_row = (y - 6) as usize;
                    let commit_index = clicked_row / 2; // Each commit takes 2 lines
                    self.select_index(commit_index);
                }
            }
        }
        Ok(())
    }
}

/// Normalize full-width ASCII characters to half-width (for Japanese IME support)
fn normalize_fullwidth(c: char) -> char {
    match c {
        // Full-width lowercase a-z (U+FF41-U+FF5A) -> ASCII a-z (U+0061-U+007A)
        '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFF41 + 0x61).unwrap_or(c),
        // Full-width uppercase A-Z (U+FF21-U+FF3A) -> ASCII A-Z (U+0041-U+005A)
        '\u{FF21}'..='\u{FF3A}' => char::from_u32(c as u32 - 0xFF21 + 0x41).unwrap_or(c),
        // Full-width digits 0-9 (U+FF10-U+FF19) -> ASCII 0-9 (U+0030-U+0039)
        '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + 0x30).unwrap_or(c),
        // Full-width space (U+3000) -> ASCII space
        '\u{3000}' => ' ',
        // Return as-is for other characters
        _ => c,
    }
}

fn format_relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let diff = now - timestamp;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{} min ago", diff / 60)
    } else if diff < 86400 {
        format!("{} hours ago", diff / 3600)
    } else {
        format!("{} days ago", diff / 86400)
    }
}

// ============================================================================
// UI
// ============================================================================
fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Main layout
    let chunks = Layout::vertical([
        Constraint::Length(1), // Title
        Constraint::Length(3), // Tabs
        Constraint::Min(0),    // Content
        Constraint::Length(3), // Hints
    ])
    .split(area);

    // Title
    let title = Paragraph::new(Span::styled(
        "siori - minimal git",
        Style::default().fg(colors::FG_BRIGHT).bold(),
    ))
    .style(Style::default().bg(colors::BG));
    frame.render_widget(title, chunks[0]);

    // Tabs
    let tabs = Tabs::new(vec!["Files", "Log"])
        .select(match app.tab {
            Tab::Files => 0,
            Tab::Log => 1,
        })
        .style(Style::default().fg(colors::DIM))
        .highlight_style(Style::default().fg(colors::BLUE).bold())
        .divider(" ");
    frame.render_widget(tabs, chunks[1]);

    // Content
    match app.tab {
        Tab::Files => render_files_tab(frame, app, chunks[2]),
        Tab::Log => render_log_tab(frame, app, chunks[2]),
    }

    // Hints + Message
    render_hints(frame, app, chunks[3]);
}

fn render_files_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Commit input
        Constraint::Min(0),    // Files
    ])
    .split(area);

    // Commit input area
    let input_style = if app.input_mode == InputMode::Insert {
        Style::default().fg(colors::FG_BRIGHT)
    } else {
        Style::default().fg(colors::DIM)
    };

    let input_text = if app.commit_message.is_empty() && app.input_mode == InputMode::Normal {
        "Commit message...".to_string()
    } else {
        app.commit_message.clone()
    };

    let input = Paragraph::new(input_text).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.input_mode == InputMode::Insert {
                colors::BLUE
            } else {
                colors::DIM
            }))
            .title(if app.input_mode == InputMode::Insert {
                " [INSERT] "
            } else {
                " c: commit "
            }),
    );
    frame.render_widget(input, chunks[0]);

    // Set cursor position in insert mode
    if app.input_mode == InputMode::Insert {
        // Use unicode width for correct cursor positioning with CJK characters
        let cursor_x = chunks[0].x + app.commit_message.width() as u16 + 1;
        frame.set_cursor_position((cursor_x, chunks[0].y + 1));
    }

    // Files list
    let staged: Vec<_> = app.files.iter().filter(|f| f.staged).collect();
    let unstaged: Vec<_> = app.files.iter().filter(|f| !f.staged).collect();

    let mut items: Vec<ListItem> = Vec::new();

    // STAGED section
    items.push(ListItem::new(Line::from(vec![
        Span::styled("STAGED ", Style::default().fg(colors::DIM).bold()),
        Span::styled(
            format!("({})", staged.len()),
            Style::default().fg(colors::GREEN),
        ),
    ])));

    for file in &staged {
        items.push(create_file_item(file, app));
    }

    // CHANGES section
    items.push(ListItem::new(Line::from(vec![
        Span::styled("CHANGES ", Style::default().fg(colors::DIM).bold()),
        Span::styled(
            format!("({})", unstaged.len()),
            Style::default().fg(colors::YELLOW),
        ),
    ])));

    for file in &unstaged {
        items.push(create_file_item(file, app));
    }

    let items_len = items.len();
    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::SELECTED))
        .highlight_symbol("> ");

    // Adjust selected index for section headers
    let mut adjusted_state = app.files_state.clone();
    if let Some(idx) = app.files_state.selected() {
        let staged_count = staged.len();
        let adjusted_idx = if idx < staged_count {
            idx + 1 // Skip STAGED header
        } else {
            idx + 2 // Skip both headers
        };
        adjusted_state.select(Some(adjusted_idx));

        // DEBUG: Log render state (only once per second to avoid spam)
        static LAST_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = LAST_LOG.load(std::sync::atomic::Ordering::Relaxed);
        if now > last {
            LAST_LOG.store(now, std::sync::atomic::Ordering::Relaxed);
            debug_log(&format!(
                "[render] visual_idx={} staged_count={} adjusted_idx={} items_len={}",
                idx, staged_count, adjusted_idx, items_len
            ));
        }
    }

    frame.render_stateful_widget(list, chunks[1], &mut adjusted_state);
}

fn create_file_item(file: &FileEntry, _app: &App) -> ListItem<'static> {
    let (status_char, status_color) = match file.status {
        FileStatus::Added => ("A", colors::GREEN),
        FileStatus::Modified => ("M", colors::YELLOW),
        FileStatus::Deleted => ("D", colors::RED),
        FileStatus::Untracked => ("??", colors::RED),
    };

    let diff_str = match file.diff_stats {
        Some((add, del)) => format!("+{} -{}", add, del),
        None => "new".to_string(),
    };

    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{:>2} ", status_char),
            Style::default().fg(status_color),
        ),
        Span::styled(file.path.clone(), Style::default().fg(colors::FG)),
        Span::styled(format!("  {}", diff_str), Style::default().fg(colors::DIM)),
    ]))
}

fn render_log_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Branch info
        Constraint::Min(0),    // Commits
    ])
    .split(area);

    // Branch info
    let ahead_behind_str = match app.ahead_behind {
        Some((ahead, behind)) => format!(" ahead {}, behind {}", ahead, behind),
        None => String::new(),
    };

    let branch_info = Paragraph::new(Line::from(vec![
        Span::styled("on ", Style::default().fg(colors::DIM)),
        Span::styled(
            app.branch_name.clone(),
            Style::default().fg(colors::GREEN).bold(),
        ),
        Span::styled(ahead_behind_str, Style::default().fg(colors::DIM)),
    ]));
    frame.render_widget(branch_info, chunks[0]);

    // Commits list
    let items: Vec<ListItem> = app
        .commits
        .iter()
        .map(|commit| {
            let mut spans = vec![
                Span::styled(
                    if commit.is_head { "● " } else { "○ " },
                    Style::default().fg(if commit.is_head {
                        colors::BLUE
                    } else {
                        colors::PURPLE
                    }),
                ),
                Span::styled(commit.message.clone(), Style::default().fg(colors::FG)),
            ];

            if commit.is_head {
                spans.push(Span::styled(
                    " [HEAD]",
                    Style::default().fg(colors::GREEN).bold(),
                ));
            }

            for branch in &commit.remote_branches {
                spans.push(Span::styled(
                    format!(" [{}]", branch),
                    Style::default().fg(colors::BLUE),
                ));
            }

            ListItem::new(vec![
                Line::from(spans),
                Line::from(vec![Span::styled(
                    format!("    {} - {}", commit.id, commit.time),
                    Style::default().fg(colors::DIM),
                )]),
            ])
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::SELECTED))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[1], &mut app.commits_state);
}

fn render_hints(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match app.tab {
        Tab::Files => {
            if app.input_mode == InputMode::Insert {
                vec![("Enter", "commit"), ("Esc", "cancel")]
            } else {
                vec![
                    ("Space", "stage"),
                    ("c", "commit"),
                    ("P", "push"),
                    ("Tab", "log"),
                    ("q", "quit"),
                ]
            }
        }
        Tab::Log => vec![
            ("j/k", "navigate"),
            ("P", "push"),
            ("p", "pull"),
            ("Tab", "files"),
            ("q", "quit"),
        ],
    };

    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, action)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(*key, Style::default().fg(colors::BLUE)));
        spans.push(Span::styled(
            format!(" {}", action),
            Style::default().fg(colors::DIM),
        ));
    }

    // Add message if present
    let content = if let Some((msg, is_error)) = &app.message {
        vec![
            Line::from(spans),
            Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(if *is_error {
                    colors::RED
                } else {
                    colors::GREEN
                }),
            )),
        ]
    } else {
        vec![Line::from(spans)]
    };

    let hints_widget = Paragraph::new(content);
    frame.render_widget(hints_widget, area);
}

// ============================================================================
// Main
// ============================================================================
fn run() -> Result<()> {
    // Clear debug log on startup
    let _ = std::fs::write("/tmp/siori_debug.log", "=== siori started ===\n");

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = App::new()?;

    // Event loop
    while app.running {
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key.code, key.modifiers)?;
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse)?;
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(DisableMouseCapture)?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle --check flag for non-interactive testing
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

    // Handle --help flag
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
        println!("  j/k/Up/Down Navigate files");
        println!("  Tab        Switch to Log tab");
        println!("  q          Quit");
        println!();
        println!("Keybindings (Log tab):");
        println!("  j/k/Up/Down Navigate commits");
        println!("  P          Push to remote");
        println!("  p          Pull from remote");
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

/// Check mode: verify repository without starting TUI
fn check_mode() -> Result<()> {
    let repo = Repository::discover(".").context("Not a git repository")?;

    // Check branch
    let branch = match repo.head() {
        Ok(head) => head.shorthand().unwrap_or("HEAD").to_string(),
        Err(_) => "(no commits yet)".to_string(),
    };
    println!("Branch: {}", branch);

    // Check status
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let staged: Vec<_> = statuses
        .iter()
        .filter(|e| {
            e.status()
                .intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED)
        })
        .collect();

    let unstaged: Vec<_> = statuses
        .iter()
        .filter(|e| {
            e.status()
                .intersects(Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED)
        })
        .collect();

    println!("Staged: {} files", staged.len());
    println!("Changes: {} files", unstaged.len());

    // Check commits (handle unborn branch)
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

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_default() {
        assert_eq!(Tab::default(), Tab::Files);
    }

    #[test]
    fn test_input_mode_default() {
        assert_eq!(InputMode::default(), InputMode::Normal);
    }

    #[test]
    fn test_format_relative_time() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        assert_eq!(format_relative_time(now), "just now");
        assert_eq!(format_relative_time(now - 120), "2 min ago");
        assert_eq!(format_relative_time(now - 7200), "2 hours ago");
        assert_eq!(format_relative_time(now - 172800), "2 days ago");
    }

    #[test]
    fn test_file_status_display() {
        let file = FileEntry {
            path: "test.rs".to_string(),
            status: FileStatus::Added,
            staged: true,
            diff_stats: Some((10, 5)),
        };
        assert_eq!(file.path, "test.rs");
        assert!(file.staged);
    }

    #[test]
    fn test_normalize_fullwidth() {
        // Full-width lowercase -> ASCII lowercase
        assert_eq!(normalize_fullwidth('ａ'), 'a');
        assert_eq!(normalize_fullwidth('ｚ'), 'z');
        assert_eq!(normalize_fullwidth('ｃ'), 'c');
        assert_eq!(normalize_fullwidth('ｑ'), 'q');

        // Full-width uppercase -> ASCII uppercase
        assert_eq!(normalize_fullwidth('Ａ'), 'A');
        assert_eq!(normalize_fullwidth('Ｚ'), 'Z');
        assert_eq!(normalize_fullwidth('Ｐ'), 'P');

        // Full-width digits -> ASCII digits
        assert_eq!(normalize_fullwidth('０'), '0');
        assert_eq!(normalize_fullwidth('９'), '9');

        // Full-width space -> ASCII space
        assert_eq!(normalize_fullwidth('\u{3000}'), ' ');

        // ASCII characters should remain unchanged
        assert_eq!(normalize_fullwidth('a'), 'a');
        assert_eq!(normalize_fullwidth(' '), ' ');

        // Japanese characters should remain unchanged
        assert_eq!(normalize_fullwidth('あ'), 'あ');
    }
}
