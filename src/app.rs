use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use git2::{DiffOptions, Repository, Status, StatusOptions};
use ratatui::widgets::ListState;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

// ============================================================================
// Constants & Helpers
// ============================================================================
pub const HEAD_LABEL: &str = "[HEAD]";

pub fn remote_label(branch: &str) -> String {
    format!("[{branch}]")
}

// ============================================================================
// Types
// ============================================================================

/// Braille spinner characters for smooth animation
pub const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Processing state for async operations
#[derive(Clone, PartialEq, Debug)]
pub enum Processing {
    None,
    Pushing,
    Pulling,
    Committing,
    PushingTags,
}

impl Processing {
    pub fn message(&self) -> &'static str {
        match self {
            Processing::None => "",
            Processing::Pushing => "Pushing...",
            Processing::Pulling => "Pulling...",
            Processing::Committing => "Committing...",
            Processing::PushingTags => "Pushing tags...",
        }
    }

    pub fn is_active(&self) -> bool {
        *self != Processing::None
    }
}

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
    TagInput,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagInfo {
    pub name: String,
    pub pushed: bool,
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
    pub full_id: git2::Oid,
    pub message: String,
    pub time: String,
    pub is_head: bool,
    pub remote_branches: Vec<String>,
    pub tags: Vec<TagInfo>,
}

/// Result from background git operations
pub type GitResult = std::result::Result<String, String>;

/// Run a git command in the specified repository directory
fn run_git(
    repo_path: &std::path::Path,
    args: &[&str],
    success_msg: &str,
    error_prefix: &str,
) -> GitResult {
    match std::process::Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
    {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);

            if o.status.success() {
                // Check if git actually did something
                let output_text = format!("{}{}", stdout, stderr);
                if output_text.contains("nothing to commit")
                    || output_text.contains("no changes added")
                {
                    return Err(format!("{}: {}", error_prefix, output_text.trim()));
                }
                Ok(success_msg.to_string())
            } else {
                Err(format!(
                    "{}: {}",
                    error_prefix,
                    if stderr.trim().is_empty() {
                        stdout.trim()
                    } else {
                        stderr.trim()
                    }
                ))
            }
        }
        Err(e) => Err(format!("{}: {}", error_prefix, e)),
    }
}

pub struct App {
    pub tab: Tab,
    pub running: bool,
    pub input_mode: InputMode,
    pub commit_message: String,
    pub cursor_pos: usize, // Cursor position in commit_message (byte index)
    pub is_amending: bool, // true when editing existing commit message
    pub remote_url: String,
    pub tag_input: String,
    pub editing_tag: Option<String>,
    pub files: Vec<FileEntry>,
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
    // Processing state
    pub processing: Processing,
    pub spinner_frame: usize,
    processing_rx: Option<mpsc::Receiver<GitResult>>,
    #[allow(dead_code)]
    processing_handle: Option<JoinHandle<()>>,
    // Status fingerprint for change detection
    status_fingerprint: Option<u64>,
}

impl App {
    pub fn new() -> Result<Self> {
        // Prioritize .git in current directory to handle nested repositories correctly
        // This ensures that when working in a subdirectory with its own .git,
        // we use that repository instead of a parent repository
        let current_dir = std::env::current_dir().unwrap_or_default();
        let git_dir = current_dir.join(".git");

        let repo = if git_dir.exists() {
            // Use current directory's .git if it exists (handles nested repos)
            eprintln!(
                "[INFO] Using repository in current directory: {:?}",
                current_dir
            );
            Repository::open(&current_dir).context("Failed to open git repository")?
        } else {
            // Fall back to discovering parent repositories
            eprintln!("[INFO] Discovering repository from current directory...");
            Repository::discover(".").context("Not a git repository")?
        };
        let repo_path = repo.workdir().unwrap_or(repo.path()).to_path_buf();
        eprintln!("[INFO] Repository path: {:?}", repo_path);
        let base_dir = std::env::current_dir().unwrap_or_default();
        let available_repos = detect_repos(&base_dir);

        let mut app = Self {
            tab: Tab::default(),
            running: true,
            input_mode: InputMode::default(),
            commit_message: String::new(),
            cursor_pos: 0,
            is_amending: false,
            remote_url: String::new(),
            tag_input: String::new(),
            editing_tag: None,
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
            processing: Processing::None,
            spinner_frame: 0,
            processing_rx: None,
            processing_handle: None,
            status_fingerprint: None,
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

    /// Lightweight refresh for auto-refresh (no network calls, no diff stats)
    pub fn refresh_status_only(&mut self) -> Result<()> {
        self.refresh_status_internal(false)?;
        self.refresh_branch_info()?;
        self.refresh_log_local()?;
        Ok(())
    }

    // ========================================================================
    // Processing state management
    // ========================================================================

    /// Advance spinner animation frame
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Get current spinner character
    pub fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame]
    }

    /// Check if background operation completed and handle result
    pub fn check_processing(&mut self) -> Result<()> {
        if let Some(rx) = &self.processing_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(msg) => self.message = Some((msg, false)),
                    Err(msg) => self.message = Some((msg, true)),
                }
                self.processing = Processing::None;
                self.processing_rx = None;
                self.processing_handle = None;
                self.refresh()?;
            }
        }
        Ok(())
    }

    /// Start a background git operation
    fn start_processing<F>(&mut self, state: Processing, operation: F)
    where
        F: FnOnce() -> GitResult + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = operation();
            let _ = tx.send(result);
        });
        self.processing = state;
        self.processing_rx = Some(rx);
        self.processing_handle = Some(handle);
    }

    fn refresh_status(&mut self) -> Result<()> {
        self.refresh_status_internal(true)
    }

    fn refresh_status_internal(&mut self, compute_diff_stats: bool) -> Result<()> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);

        let statuses = self.repo.statuses(Some(&mut opts))?;

        // Quick check: compute a fingerprint of current status and compare to previous
        if !compute_diff_stats {
            let new_fingerprint = Self::compute_status_fingerprint(&statuses);
            if Some(&new_fingerprint) == self.status_fingerprint.as_ref() {
                return Ok(()); // No changes, skip rebuild
            }
            self.status_fingerprint = Some(new_fingerprint);
        }

        self.files.clear();
        self.visual_list.clear();

        let mut staged_indices = Vec::new();
        let mut unstaged_indices = Vec::new();

        // Single pass: collect all files
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            // Staged files
            if status.intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED)
            {
                let file_status = if status.contains(Status::INDEX_NEW) {
                    FileStatus::Added
                } else if status.contains(Status::INDEX_DELETED) {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                let diff_stats = if compute_diff_stats {
                    self.get_diff_stats(&path, true)
                } else {
                    None
                };
                staged_indices.push(self.files.len());
                self.files.push(FileEntry {
                    path: path.clone(),
                    status: file_status,
                    staged: true,
                    diff_stats,
                });
            }

            // Unstaged/untracked files
            if status.intersects(Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED) {
                let file_status = if status.contains(Status::WT_NEW) {
                    FileStatus::Untracked
                } else if status.contains(Status::WT_DELETED) {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                let diff_stats = if compute_diff_stats {
                    self.get_diff_stats(&path, false)
                } else {
                    None
                };
                unstaged_indices.push(self.files.len());
                self.files.push(FileEntry {
                    path,
                    status: file_status,
                    staged: false,
                    diff_stats,
                });
            }
        }

        // Build visual_list: staged first, then unstaged
        self.visual_list.extend(staged_indices);
        self.visual_list.extend(unstaged_indices);

        // Adjust selection
        if self.files_state.selected().is_none() && !self.visual_list.is_empty() {
            self.files_state.select(Some(0));
        } else if let Some(idx) = self.files_state.selected()
            && idx >= self.visual_list.len()
        {
            self.files_state
                .select(self.visual_list.len().checked_sub(1));
        }

        Ok(())
    }

    /// Compute a fingerprint of the git status for change detection.
    /// This captures path + status bits for each file.
    fn compute_status_fingerprint(statuses: &git2::Statuses) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        for entry in statuses.iter() {
            if let Some(path) = entry.path() {
                path.hash(&mut hasher);
            }
            entry.status().bits().hash(&mut hasher);
        }
        hasher.finish()
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
        self.refresh_log_internal(true)
    }

    /// Lightweight log refresh without network calls (for auto-refresh)
    fn refresh_log_local(&mut self) -> Result<()> {
        self.refresh_log_internal(false)
    }

    fn refresh_log_internal(&mut self, check_remote_tags: bool) -> Result<()> {
        // Save previous tag pushed status before clearing
        let previous_tag_status: std::collections::HashMap<String, bool> = self
            .commits
            .iter()
            .flat_map(|c| c.tags.iter())
            .map(|t| (t.name.clone(), t.pushed))
            .collect();

        self.commits.clear();
        let Ok(mut revwalk) = self.repo.revwalk() else {
            return Ok(());
        };
        if revwalk.push_head().is_err() {
            return Ok(());
        }
        let _ = revwalk.set_sorting(git2::Sort::TIME);
        let head_id = self.repo.head().ok().and_then(|h| h.target());

        // Collect remote branch refs
        let mut remote_refs: std::collections::HashMap<git2::Oid, Vec<String>> =
            std::collections::HashMap::new();
        // Collect local tags
        let mut local_tags: std::collections::HashMap<git2::Oid, Vec<String>> =
            std::collections::HashMap::new();
        // Collect remote tags (to determine pushed status)
        let mut remote_tags: std::collections::HashSet<String> = std::collections::HashSet::new();

        if let Ok(refs) = self.repo.references() {
            for reference in refs.flatten() {
                let Some(name) = reference.name() else {
                    continue;
                };
                if name.starts_with("refs/remotes/") {
                    if let Ok(commit) = reference.peel_to_commit() {
                        let short_name = name.strip_prefix("refs/remotes/").unwrap_or(name);
                        remote_refs
                            .entry(commit.id())
                            .or_default()
                            .push(short_name.to_string());
                    }
                } else if name.starts_with("refs/tags/") {
                    let tag_name = name.strip_prefix("refs/tags/").unwrap_or(name);
                    if let Ok(obj) = reference.peel(git2::ObjectType::Commit) {
                        local_tags
                            .entry(obj.id())
                            .or_default()
                            .push(tag_name.to_string());
                    }
                }
            }
        }

        // Check which tags exist on remote (only when requested - this is a network call)
        if check_remote_tags {
            if let Ok(output) = std::process::Command::new("git")
                .args(["ls-remote", "--tags", "origin"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Some(tag_ref) = line.split('\t').nth(1) {
                        let tag_name = tag_ref
                            .strip_prefix("refs/tags/")
                            .unwrap_or(tag_ref)
                            .trim_end_matches("^{}");
                        remote_tags.insert(tag_name.to_string());
                    }
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
            let tags: Vec<TagInfo> = local_tags
                .get(&oid)
                .map(|names| {
                    names
                        .iter()
                        .map(|name| TagInfo {
                            name: name.clone(),
                            pushed: if check_remote_tags {
                                remote_tags.contains(name)
                            } else {
                                // Keep previous pushed status if not checking remote
                                previous_tag_status.get(name).copied().unwrap_or(false)
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();

            self.commits.push(CommitEntry {
                id: format!("{:.7}", oid),
                full_id: oid,
                message: commit.summary().unwrap_or("").to_string(),
                time: format_relative_time(commit.time().seconds()),
                is_head: Some(oid) == head_id,
                remote_branches: remote_refs.get(&oid).cloned().unwrap_or_default(),
                tags,
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

        let file_path = file.path.clone();
        let file_status = file.status;
        let is_staged = file.staged;

        // Check if path is a directory (ends with '/' or is actually a directory)
        let is_directory = file_path.ends_with('/') || {
            let workdir = self.repo.workdir().unwrap_or(self.repo.path());
            workdir.join(&file_path).is_dir()
        };

        // 操作前のセクション情報を記録
        let old_staged_count = self.files.iter().filter(|f| f.staged).count();
        let was_in_staged = visual_idx < old_staged_count;
        let pos_in_section = if was_in_staged {
            visual_idx
        } else {
            visual_idx - old_staged_count
        };

        if is_staged {
            // Unstaging
            if is_directory {
                // Use git command for directories
                let output = std::process::Command::new("git")
                    .args(["reset", "HEAD", "--", &file_path])
                    .output();
                match output {
                    Ok(out) if out.status.success() => {
                        self.message = Some((format!("Unstaged: {}", file_path), false));
                    }
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        self.message = Some((format!("Unstage failed: {}", err.trim()), true));
                    }
                    Err(e) => {
                        self.message = Some((format!("Unstage failed: {}", e), true));
                    }
                }
            } else if file_status == FileStatus::Added {
                let mut index = self.repo.index()?;
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
            // Staging
            if is_directory {
                // Use git command for directories (handles recursive add properly)
                let output = std::process::Command::new("git")
                    .args(["add", "--", &file_path])
                    .output();
                match output {
                    Ok(out) if out.status.success() => {
                        self.message = Some((format!("Staged: {}", file_path), false));
                    }
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        self.message = Some((format!("Stage failed: {}", err.trim()), true));
                    }
                    Err(e) => {
                        self.message = Some((format!("Stage failed: {}", e), true));
                    }
                }
            } else {
                let mut index = self.repo.index()?;
                if file_status == FileStatus::Deleted {
                    index.remove_path(std::path::Path::new(&file_path))?;
                } else {
                    index.add_path(std::path::Path::new(&file_path))?;
                }
                index.write()?;
                self.message = Some((format!("Staged: {}", file_path), false));
            }
        }

        self.refresh_status()?;

        // 同じセクション内にカーソルを維持
        let new_staged_count = self.files.iter().filter(|f| f.staged).count();
        let new_changes_count = self.visual_list.len() - new_staged_count;

        let new_idx = if was_in_staged {
            if new_staged_count > 0 {
                pos_in_section.min(new_staged_count - 1)
            } else if new_changes_count > 0 {
                new_staged_count // Changesの先頭へ
            } else {
                0
            }
        } else if new_changes_count > 0 {
            new_staged_count + pos_in_section.min(new_changes_count - 1)
        } else if new_staged_count > 0 {
            new_staged_count - 1 // Stagedの末尾へ
        } else {
            0
        };

        if !self.visual_list.is_empty() {
            self.files_state.select(Some(new_idx));
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        let message = self.commit_message.trim().to_string();
        if message.is_empty() {
            self.message = Some(("Empty commit message".to_string(), true));
            return Ok(());
        }

        let is_amending = self.is_amending;
        let repo_path = self.repo_path.clone();
        self.commit_message.clear();
        self.cursor_pos = 0;
        self.is_amending = false;
        self.input_mode = InputMode::Normal;

        if is_amending {
            self.start_processing(Processing::Committing, move || {
                run_git(
                    &repo_path,
                    &["commit", "--amend", "-m", &message],
                    "Amended successfully",
                    "Amend failed",
                )
            });
        } else {
            self.start_processing(Processing::Committing, move || {
                run_git(
                    &repo_path,
                    &["commit", "-m", &message],
                    "Committed successfully",
                    "Commit failed",
                )
            });
        }
        Ok(())
    }

    fn start_amend(&mut self) -> Result<()> {
        // Only allow amending HEAD commit
        let Some(idx) = self.commits_state.selected() else {
            return Ok(());
        };
        let Some(commit) = self.commits.get(idx) else {
            return Ok(());
        };
        if !commit.is_head {
            self.message = Some(("Can only amend HEAD commit".to_string(), true));
            return Ok(());
        }

        self.commit_message = commit.message.clone();
        self.cursor_pos = self.commit_message.len();
        self.is_amending = true;
        self.input_mode = InputMode::Insert;
        self.tab = Tab::Files; // Switch to Files tab to show input
        Ok(())
    }

    fn push(&mut self) -> Result<()> {
        // Quick check for remote configuration
        let check = std::process::Command::new("git")
            .current_dir(&self.repo_path)
            .args(["remote", "get-url", "origin"])
            .output();

        if check.is_err() || !check.unwrap().status.success() {
            self.input_mode = InputMode::RemoteUrl;
            self.remote_url.clear();
            self.message = Some((
                "No remote configured. Enter repository URL:".to_string(),
                true,
            ));
            return Ok(());
        }

        // Run push in background
        let repo_path = self.repo_path.clone();
        self.start_processing(Processing::Pushing, move || {
            run_git(&repo_path, &["push"], "Pushed successfully", "Push failed")
        });
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
        let repo_path = self.repo_path.clone();
        self.start_processing(Processing::Pulling, move || {
            run_git(
                &repo_path,
                &["pull", "--no-rebase"],
                "Pulled successfully",
                "Pull failed",
            )
        });
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
    // Tag operations
    // ========================================================================
    fn open_tag_input(&mut self) {
        let Some(idx) = self.commits_state.selected() else {
            return;
        };
        let Some(commit) = self.commits.get(idx) else {
            return;
        };
        // If commit has a tag, pre-fill for editing
        if let Some(tag) = commit.tags.first() {
            self.tag_input = tag.name.clone();
            self.editing_tag = Some(tag.name.clone());
        } else {
            self.tag_input.clear();
            self.editing_tag = None;
        }
        self.input_mode = InputMode::TagInput;
    }

    fn create_or_update_tag(&mut self) -> Result<()> {
        let tag_name = self.tag_input.trim().to_string();
        if tag_name.is_empty() {
            self.input_mode = InputMode::Normal;
            self.message = Some(("Tag name is empty".to_string(), true));
            return Ok(());
        }

        let Some(idx) = self.commits_state.selected() else {
            return Ok(());
        };
        let Some(commit) = self.commits.get(idx) else {
            return Ok(());
        };
        let commit_id = commit.full_id;
        let was_pushed = commit.tags.first().is_some_and(|t| t.pushed);
        let old_tag = self.editing_tag.clone();

        // Delete old tag if editing
        if let Some(ref old) = old_tag {
            if old != &tag_name {
                self.delete_tag_by_name(old, was_pushed)?;
            }
        }

        // Create new tag using git command (avoids borrow issues with git2)
        let output = std::process::Command::new("git")
            .args(["tag", "-f", &tag_name, &commit_id.to_string()])
            .output();

        if let Err(e) = output {
            self.message = Some((format!("Failed to create tag: {e}"), true));
            self.input_mode = InputMode::Normal;
            return Ok(());
        }

        // If old tag was pushed, push new tag too
        if was_pushed {
            let push_output = std::process::Command::new("git")
                .args(["push", "origin", &tag_name])
                .output();
            if let Ok(out) = push_output {
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr);
                    self.message = Some((format!("Tag created, push failed: {err}"), true));
                    self.input_mode = InputMode::Normal;
                    self.refresh_log()?;
                    return Ok(());
                }
            }
            self.message = Some((format!("Tag updated: {} (pushed)", tag_name), false));
        } else {
            self.message = Some((format!("Created tag: {}", tag_name), false));
        }

        self.tag_input.clear();
        self.editing_tag = None;
        self.input_mode = InputMode::Normal;
        self.refresh_log()?;
        Ok(())
    }

    fn delete_tag_by_name(&mut self, tag_name: &str, delete_remote: bool) -> Result<()> {
        // Delete local tag
        let _ = std::process::Command::new("git")
            .args(["tag", "-d", tag_name])
            .output();

        // Delete remote tag if needed
        if delete_remote {
            let _ = std::process::Command::new("git")
                .args(["push", "origin", &format!(":refs/tags/{tag_name}")])
                .output();
        }
        Ok(())
    }

    fn delete_selected_tag(&mut self) -> Result<()> {
        let Some(idx) = self.commits_state.selected() else {
            return Ok(());
        };
        let Some(commit) = self.commits.get(idx) else {
            return Ok(());
        };
        let Some(tag) = commit.tags.first() else {
            self.message = Some(("No tag on this commit".to_string(), true));
            return Ok(());
        };
        let tag_name = tag.name.clone();
        let was_pushed = tag.pushed;

        self.delete_tag_by_name(&tag_name, was_pushed)?;

        let msg = if was_pushed {
            format!("Deleted tag: {tag_name} (local + remote)")
        } else {
            format!("Deleted tag: {tag_name}")
        };
        self.message = Some((msg, false));
        self.refresh_log()?;
        Ok(())
    }

    fn push_tags(&mut self) -> Result<()> {
        let repo_path = self.repo_path.clone();
        self.start_processing(Processing::PushingTags, move || {
            run_git(
                &repo_path,
                &["push", "--tags"],
                "Tags pushed successfully",
                "Push tags failed",
            )
        });
        Ok(())
    }

    pub fn unpushed_tag_count(&self) -> usize {
        self.commits
            .iter()
            .flat_map(|c| &c.tags)
            .filter(|t| !t.pushed)
            .count()
    }

    // ========================================================================
    // Label helpers
    // ========================================================================
    pub fn status_label(&self) -> String {
        match self.ahead_behind {
            None => String::new(),
            Some((0, 0)) => "synced".to_string(),
            Some((ahead, 0)) => format!("↑{}", ahead),
            Some((0, behind)) => format!("↓{}", behind),
            Some((ahead, behind)) => format!("↑{}  ↓{}", ahead, behind),
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
                    if self.cursor_pos > 0 {
                        let prev = self.cursor_prev_char();
                        self.commit_message.remove(prev);
                        self.cursor_pos = prev;
                    }
                }
                KeyCode::Delete => {
                    if self.cursor_pos < self.commit_message.len() {
                        self.commit_message.remove(self.cursor_pos);
                    }
                }
                KeyCode::Left => self.cursor_pos = self.cursor_prev_char(),
                KeyCode::Right => self.cursor_pos = self.cursor_next_char(),
                KeyCode::Home => self.cursor_pos = 0,
                KeyCode::End => self.cursor_pos = self.commit_message.len(),
                KeyCode::Char(c) => {
                    self.commit_message.insert(self.cursor_pos, c);
                    self.cursor_pos += c.len_utf8();
                }
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
            InputMode::TagInput => match code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.tag_input.clear();
                    self.editing_tag = None;
                }
                KeyCode::Enter => self.create_or_update_tag()?,
                KeyCode::Backspace => {
                    self.tag_input.pop();
                }
                KeyCode::Char(c) => self.tag_input.push(c),
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
                KeyCode::Char('t') if self.tab == Tab::Log => self.open_tag_input(),
                KeyCode::Char('T') if self.tab == Tab::Log => self.push_tags()?,
                KeyCode::Char('d') if self.tab == Tab::Log => self.delete_selected_tag()?,
                KeyCode::Char('e') if self.tab == Tab::Log => self.start_amend()?,
                KeyCode::Char('r') => self.open_repo_select(),
                KeyCode::Char('R') => {
                    self.refresh()?;
                    self.message = Some(("Refreshed".to_string(), false));
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.running = false;
                }
                _ => {}
            },
        }
        Ok(())
    }

    // ========================================================================
    // Cursor movement helpers for commit message editing
    // ========================================================================

    /// Move cursor to the start of the previous character (for Left key / Backspace)
    fn cursor_prev_char(&self) -> usize {
        if self.cursor_pos == 0 {
            return 0;
        }
        self.commit_message[..self.cursor_pos]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Move cursor to the start of the next character (for Right key)
    fn cursor_next_char(&self) -> usize {
        if self.cursor_pos >= self.commit_message.len() {
            return self.commit_message.len();
        }
        self.commit_message[self.cursor_pos..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor_pos + i)
            .unwrap_or(self.commit_message.len())
    }

    // ========================================================================
    // List navigation helpers
    // ========================================================================

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
    fn test_tag_info() {
        let pushed_tag = TagInfo {
            name: "v1.0.0".to_string(),
            pushed: true,
        };
        let unpushed_tag = TagInfo {
            name: "v2.0.0".to_string(),
            pushed: false,
        };
        assert_eq!(pushed_tag.name, "v1.0.0");
        assert!(pushed_tag.pushed);
        assert_eq!(unpushed_tag.name, "v2.0.0");
        assert!(!unpushed_tag.pushed);
    }
}
