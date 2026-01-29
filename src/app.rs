use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use git2::{DiffOptions, Repository, Status, StatusOptions};
use ratatui::widgets::ListState;
use std::path::PathBuf;

// ============================================================================
// Types
// ============================================================================
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    #[default]
    Files,
    Log,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,
    RemoteUrl,
    RepoSelect,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum LabelMode {
    #[default]
    Friendly, // 日本語ラベル
    Git, // Git用語
}

#[derive(Clone)]
pub struct FileEntry {
    pub path: String,
    pub status: FileStatus,
    pub staged: bool,
    pub diff_stats: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
}

#[derive(Clone)]
pub struct CommitEntry {
    pub id: String,
    pub message: String,
    pub time: String,
    pub is_head: bool,
    pub remote_branches: Vec<String>,
}

pub struct App {
    pub tab: Tab,
    pub running: bool,
    pub input_mode: InputMode,
    pub label_mode: LabelMode,
    pub commit_message: String,
    pub remote_url: String,
    pub files: Vec<FileEntry>,
    /// Index mapping for visual list order (staged files first, then unstaged)
    pub visual_list: Vec<usize>,
    pub commits: Vec<CommitEntry>,
    pub files_state: ListState,
    pub commits_state: ListState,
    pub branch_name: String,
    pub ahead_behind: Option<(usize, usize)>,
    pub message: Option<(String, bool)>,
    pub repo: Repository,
    pub repo_path: PathBuf,
    pub available_repos: Vec<PathBuf>,
    pub repo_select_state: ListState,
}

impl App {
    pub fn new() -> Result<Self> {
        let repo = Repository::discover(".").context("Not a git repository")?;
        let repo_path = repo.workdir().unwrap_or(repo.path()).to_path_buf();
        let base_dir = std::env::current_dir().unwrap_or_default();
        let available_repos = detect_repos(&base_dir);

        let mut app = Self {
            tab: Tab::default(),
            running: true,
            input_mode: InputMode::default(),
            label_mode: LabelMode::default(),
            commit_message: String::new(),
            remote_url: String::new(),
            files: Vec::new(),
            visual_list: Vec::new(),
            commits: Vec::new(),
            files_state: ListState::default(),
            commits_state: ListState::default(),
            branch_name: String::new(),
            ahead_behind: None,
            message: None,
            repo,
            repo_path,
            available_repos,
            repo_select_state: ListState::default(),
        };
        app.refresh()?;
        Ok(app)
    }

    pub fn refresh(&mut self) -> Result<()> {
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

        // Pass 1: staged files
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

        // Pass 2: unstaged/untracked files
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

        // Build visual_list: staged first, then unstaged
        for (i, file) in self.files.iter().enumerate() {
            if file.staged {
                self.visual_list.push(i);
            }
        }
        for (i, file) in self.files.iter().enumerate() {
            if !file.staged {
                self.visual_list.push(i);
            }
        }

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
        let Ok(mut revwalk) = self.repo.revwalk() else {
            return Ok(());
        };
        if revwalk.push_head().is_err() {
            return Ok(());
        }
        let _ = revwalk.set_sorting(git2::Sort::TIME);
        let head_id = self.repo.head().ok().and_then(|h| h.target());

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
            let Ok(oid) = oid else { continue };
            let Ok(commit) = self.repo.find_commit(oid) else {
                continue;
            };
            self.commits.push(CommitEntry {
                id: format!("{:.7}", oid),
                message: commit.summary().unwrap_or("").to_string(),
                time: format_relative_time(commit.time().seconds()),
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
        let Some(visual_idx) = self.files_state.selected() else {
            self.message = Some(("No file selected".to_string(), true));
            return Ok(());
        };
        let Some(&file_index) = self.visual_list.get(visual_idx) else {
            self.message = Some(("Invalid selection".to_string(), true));
            return Ok(());
        };
        let Some(file) = self.files.get(file_index) else {
            self.message = Some(("File not found".to_string(), true));
            return Ok(());
        };

        let mut index = self.repo.index()?;
        let file_path = file.path.clone();
        let file_status = file.status;
        let is_staged = file.staged;

        if is_staged {
            if file_status == FileStatus::Added {
                index.remove_path(std::path::Path::new(&file_path))?;
                index.write()?;
                self.message = Some((format!("Unstaged (new): {}", file_path), false));
            } else if let Ok(head_commit) = self.repo.head().and_then(|h| h.peel_to_commit()) {
                match self
                    .repo
                    .reset_default(Some(head_commit.as_object()), [&file_path])
                {
                    Ok(_) => self.message = Some((format!("Unstaged: {}", file_path), false)),
                    Err(e) => self.message = Some((format!("Unstage failed: {}", e), true)),
                }
            } else {
                self.message = Some(("Cannot unstage: no HEAD".to_string(), true));
            }
        } else {
            if file_status == FileStatus::Deleted {
                index.remove_path(std::path::Path::new(&file_path))?;
            } else {
                index.add_path(std::path::Path::new(&file_path))?;
            }
            index.write()?;
            self.message = Some((format!("Staged: {}", file_path), false));
        }

        let target_staged = !is_staged;
        self.refresh_status()?;

        // Follow cursor to the file's new position
        for (i, &file_idx) in self.visual_list.iter().enumerate() {
            if let Some(f) = self.files.get(file_idx)
                && f.path == file_path
                && f.staged == target_staged
            {
                self.files_state.select(Some(i));
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

        // Scope the git2 objects to avoid borrow conflicts with refresh()
        {
            let mut index = self.repo.index()?;
            let tree_id = index.write_tree()?;
            let tree = self.repo.find_tree(tree_id)?;
            let signature = self.repo.signature()?;
            let parents: Vec<_> = self
                .repo
                .head()
                .ok()
                .and_then(|h| h.peel_to_commit().ok())
                .into_iter()
                .collect();
            let parent_refs: Vec<_> = parents.iter().collect();
            self.repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )?;
        }

        self.commit_message.clear();
        self.input_mode = InputMode::Normal;
        self.message = Some(("Committed successfully".to_string(), false));
        self.refresh()?;
        Ok(())
    }

    fn push(&mut self) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["push"])
            .output()
            .context("Failed to execute git push")?;

        if output.status.success() {
            self.message = Some(("Pushed successfully".to_string(), false));
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            if err.contains("No configured push destination")
                || err.contains("does not appear to be a git repository")
            {
                self.input_mode = InputMode::RemoteUrl;
                self.remote_url.clear();
                self.message = Some((
                    "No remote configured. Enter repository URL:".to_string(),
                    true,
                ));
                return Ok(());
            }
            self.message = Some((format!("Push failed: {}", err.trim()), true));
        }
        self.refresh()?;
        Ok(())
    }

    fn add_remote_and_push(&mut self) -> Result<()> {
        let url = self.remote_url.trim().to_string();
        if url.is_empty() {
            self.message = Some(("URL is empty".to_string(), true));
            return Ok(());
        }

        let add_output = std::process::Command::new("git")
            .args(["remote", "add", "origin", &url])
            .output()
            .context("Failed to add remote")?;

        if !add_output.status.success() {
            let err = String::from_utf8_lossy(&add_output.stderr);
            self.message = Some((format!("Failed: {}", err.trim()), true));
            self.remote_url.clear();
            self.input_mode = InputMode::Normal;
            return Ok(());
        }

        let push_output = std::process::Command::new("git")
            .args(["push", "-u", "origin", &self.branch_name])
            .output()
            .context("Failed to push")?;

        if push_output.status.success() {
            self.message = Some(("Remote added & pushed!".to_string(), false));
        } else {
            let err = String::from_utf8_lossy(&push_output.stderr);
            self.message = Some((format!("Push failed: {}", err.trim()), true));
        }

        self.remote_url.clear();
        self.input_mode = InputMode::Normal;
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

    // ========================================================================
    // Repository switcher
    // ========================================================================
    fn switch_repo(&mut self, path: PathBuf) -> Result<()> {
        self.repo = Repository::open(&path).context("Failed to open repository")?;
        self.repo_path = path.clone();
        self.input_mode = InputMode::Normal;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
        self.message = Some((format!("Switched to: {}", name), false));
        self.refresh()?;
        Ok(())
    }

    fn open_repo_select(&mut self) {
        let base_dir = std::env::current_dir().unwrap_or_default();
        self.available_repos = detect_repos(&base_dir);
        // Select current repo in the list
        let current_idx = self
            .available_repos
            .iter()
            .position(|p| p == &self.repo_path)
            .unwrap_or(0);
        self.repo_select_state.select(Some(current_idx));
        self.input_mode = InputMode::RepoSelect;
    }

    fn repo_select_next(&mut self) {
        let len = self.available_repos.len();
        if len > 0 {
            let i = self.repo_select_state.selected().unwrap_or(0);
            self.repo_select_state.select(Some((i + 1) % len));
        }
    }

    fn repo_select_prev(&mut self) {
        let len = self.available_repos.len();
        if len > 0 {
            let i = self.repo_select_state.selected().unwrap_or(0);
            self.repo_select_state
                .select(Some(if i == 0 { len - 1 } else { i - 1 }));
        }
    }

    // ========================================================================
    // Label helpers (for friendly/git mode)
    // ========================================================================
    pub fn head_label(&self) -> &'static str {
        match self.label_mode {
            LabelMode::Friendly => "[自分]",
            LabelMode::Git => "[HEAD]",
        }
    }

    pub fn remote_label(&self, branch: &str) -> String {
        match self.label_mode {
            LabelMode::Friendly => "[クラウド]".to_string(),
            LabelMode::Git => format!("[{}]", branch),
        }
    }

    pub fn status_label(&self) -> String {
        let Some((ahead, behind)) = self.ahead_behind else {
            return String::new();
        };
        let friendly = self.label_mode == LabelMode::Friendly;
        let mut parts = Vec::new();

        if ahead > 0 {
            parts.push(if friendly {
                format!("{}件 未保存", ahead)
            } else {
                format!("↑{}", ahead)
            });
        }
        if behind > 0 {
            parts.push(if friendly {
                format!("{}件 更新あり", behind)
            } else {
                format!("↓{}", behind)
            });
        }

        if parts.is_empty() {
            if friendly { "同期済み" } else { "synced" }.to_string()
        } else {
            parts.join("  ")
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        let code = match code {
            KeyCode::Char(c) => KeyCode::Char(normalize_fullwidth(c)),
            other => other,
        };
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
            InputMode::RemoteUrl => match code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.remote_url.clear();
                    self.message = Some(("Cancelled".to_string(), false));
                }
                KeyCode::Enter => self.add_remote_and_push()?,
                KeyCode::Backspace => {
                    self.remote_url.pop();
                }
                KeyCode::Char(c) => self.remote_url.push(c),
                _ => {}
            },
            InputMode::RepoSelect => match code {
                KeyCode::Esc => self.input_mode = InputMode::Normal,
                KeyCode::Enter => {
                    if let Some(idx) = self.repo_select_state.selected()
                        && let Some(path) = self.available_repos.get(idx).cloned()
                    {
                        if path != self.repo_path {
                            self.switch_repo(path)?;
                        } else {
                            self.input_mode = InputMode::Normal;
                        }
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.repo_select_next(),
                KeyCode::Char('k') | KeyCode::Up => self.repo_select_prev(),
                _ => {}
            },
            InputMode::Normal => match code {
                KeyCode::Char('q') => self.running = false,
                KeyCode::Tab => self.toggle_tab(),
                KeyCode::Char('j') | KeyCode::Down => self.select_next(),
                KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
                KeyCode::Char(' ') if self.tab == Tab::Files => self.stage_selected()?,
                KeyCode::Char('c') if self.tab == Tab::Files => {
                    self.input_mode = InputMode::Insert;
                }
                KeyCode::Char('P') => self.push()?,
                KeyCode::Char('p') if self.tab == Tab::Log => self.pull()?,
                KeyCode::Char('r') => self.refresh()?,
                KeyCode::Char('R') => self.open_repo_select(),
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.running = false;
                }
                _ => {}
            },
        }
        Ok(())
    }

    fn current_list_len(&self) -> usize {
        match self.tab {
            Tab::Files => self.visual_list.len(),
            Tab::Log => self.commits.len(),
        }
    }

    fn current_state(&mut self) -> &mut ListState {
        match self.tab {
            Tab::Files => &mut self.files_state,
            Tab::Log => &mut self.commits_state,
        }
    }

    fn select_next(&mut self) {
        let len = self.current_list_len();
        if len > 0 {
            let i = self.current_state().selected().unwrap_or(0);
            self.current_state().select(Some((i + 1) % len));
        }
    }

    fn select_prev(&mut self) {
        let len = self.current_list_len();
        if len > 0 {
            let i = self.current_state().selected().unwrap_or(0);
            self.current_state()
                .select(Some(if i == 0 { len - 1 } else { i - 1 }));
        }
    }

    fn select_index(&mut self, index: usize) {
        if index < self.current_list_len() {
            self.current_state().select(Some(index));
        }
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) -> Result<()> {
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

    fn toggle_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Files => Tab::Log,
            Tab::Log => Tab::Files,
        };
    }

    fn handle_click(&mut self, _x: u16, y: u16) -> Result<()> {
        // Title bar (repo name) click
        if y == 0 {
            self.open_repo_select();
            return Ok(());
        }

        // Tabs click
        if (1..=3).contains(&y) {
            self.toggle_tab();
            return Ok(());
        }

        match self.tab {
            Tab::Files => {
                if y >= 8 {
                    let clicked_row = (y - 8) as usize;
                    let staged_count = self
                        .visual_list
                        .iter()
                        .filter(|&&idx| self.files.get(idx).is_some_and(|f| f.staged))
                        .count();

                    let visual_index = if clicked_row == 0 {
                        None
                    } else if clicked_row <= staged_count {
                        Some(clicked_row - 1)
                    } else if clicked_row == staged_count + 1 {
                        None
                    } else {
                        Some(staged_count + (clicked_row - staged_count - 2))
                    };

                    if let Some(idx) = visual_index
                        && idx < self.visual_list.len()
                    {
                        self.select_index(idx);
                    }
                }
            }
            Tab::Log => {
                if y >= 6 {
                    let clicked_row = (y - 6) as usize;
                    self.select_index(clicked_row / 2);
                }
            }
        }
        Ok(())
    }
}

/// Normalize full-width ASCII characters to half-width (for Japanese IME support)
pub fn normalize_fullwidth(c: char) -> char {
    match c {
        '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFF41 + 0x61).unwrap_or(c),
        '\u{FF21}'..='\u{FF3A}' => char::from_u32(c as u32 - 0xFF21 + 0x41).unwrap_or(c),
        '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + 0x30).unwrap_or(c),
        '\u{3000}' => ' ',
        _ => c,
    }
}

pub fn format_relative_time(timestamp: i64) -> String {
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

/// Detect git repositories in base directory and subdirectories (up to 2 levels)
pub fn detect_repos(base: &std::path::Path) -> Vec<PathBuf> {
    let mut repos = Vec::new();

    // Current directory
    if base.join(".git").exists() {
        repos.push(base.to_path_buf());
    }

    // Scan subdirectories (2 levels deep)
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Level 1: direct subdirectory
            if path.join(".git").exists() {
                repos.push(path.clone());
            }
            // Level 2: subdirectory of subdirectory
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.is_dir() && sub_path.join(".git").exists() {
                        repos.push(sub_path);
                    }
                }
            }
        }
    }

    repos.sort();
    repos
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
        assert_eq!(normalize_fullwidth('ａ'), 'a');
        assert_eq!(normalize_fullwidth('ｚ'), 'z');
        assert_eq!(normalize_fullwidth('ｃ'), 'c');
        assert_eq!(normalize_fullwidth('Ｐ'), 'P');
        assert_eq!(normalize_fullwidth('０'), '0');
        assert_eq!(normalize_fullwidth('９'), '9');
        assert_eq!(normalize_fullwidth('\u{3000}'), ' ');
        assert_eq!(normalize_fullwidth('a'), 'a');
        assert_eq!(normalize_fullwidth('あ'), 'あ');
    }

    #[test]
    fn test_centered_rect() {
        use ratatui::prelude::Rect;
        let area = Rect::new(0, 0, 100, 40);
        let result = super::super::ui::centered_rect(60, 7, area);
        assert_eq!(result.width, 60);
        assert_eq!(result.height, 7);
        assert_eq!(result.x, 20); // (100 - 60) / 2
        assert_eq!(result.y, 16); // (40 - 7) / 2
    }

    #[test]
    fn test_label_mode_default() {
        assert_eq!(LabelMode::default(), LabelMode::Friendly);
    }
}
