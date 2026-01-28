use anyhow::{Context, Result};
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

#[derive(Clone, Copy, PartialEq)]
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

struct App {
    tab: Tab,
    running: bool,
    input_mode: InputMode,
    commit_message: String,
    files: Vec<FileEntry>,
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
                let diff_stats = self.get_diff_stats(&path, true);
                self.files.push(FileEntry {
                    path: path.clone(),
                    status: file_status,
                    staged: true,
                    diff_stats,
                });
            }

            // Unstaged/untracked files
            if status.intersects(
                Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED | Status::INDEX_MODIFIED,
            ) && !status.contains(Status::INDEX_NEW)
            {
                let file_status = if status.contains(Status::WT_NEW) {
                    FileStatus::Untracked
                } else if status.contains(Status::WT_DELETED) {
                    FileStatus::Deleted
                } else {
                    FileStatus::Modified
                };
                // Avoid duplicates for modified+staged
                if !self.files.iter().any(|f| f.path == path && !f.staged) {
                    let diff_stats = self.get_diff_stats(&path, false);
                    self.files.push(FileEntry {
                        path,
                        status: file_status,
                        staged: false,
                        diff_stats,
                    });
                }
            }
        }

        // Select first item if nothing selected
        if self.files_state.selected().is_none() && !self.files.is_empty() {
            self.files_state.select(Some(0));
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
        revwalk.set_sorting(git2::Sort::TIME)?;

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
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;

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
        let Some(idx) = self.files_state.selected() else {
            return Ok(());
        };
        let Some(file) = self.files.get(idx) else {
            return Ok(());
        };

        let mut index = self.repo.index()?;
        if file.staged {
            // Unstage: reset to HEAD
            if let Ok(head) = self.repo.head().and_then(|h| h.peel_to_tree()) {
                self.repo
                    .reset_default(Some(head.as_object()), [&file.path])?;
            }
        } else {
            // Stage
            index.add_path(std::path::Path::new(&file.path))?;
            index.write()?;
        }

        self.refresh_status()?;
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
                let len = self.files.len();
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
                let len = self.files.len();
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
        match self.tab {
            Tab::Files => {
                if index < self.files.len() {
                    self.files_state.select(Some(index));
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
        // Layout: y=0 title, y=1-3 tabs, y=4-6 commit input, y=7+ files/commits
        // Tab area (y=1-3): click to switch tabs
        if (1..=3).contains(&y) {
            // Simple toggle on tab bar click
            self.tab = match self.tab {
                Tab::Files => Tab::Log,
                Tab::Log => Tab::Files,
            };
            return Ok(());
        }

        // File/commit list area
        match self.tab {
            Tab::Files => {
                // Files tab: y=4-6 is commit input, y=7 is STAGED header, y=8+ is file list
                // Account for headers: STAGED at index 0, files, CHANGES header, more files
                if y >= 8 {
                    let clicked_row = (y - 8) as usize;
                    let staged_count = self.files.iter().filter(|f| f.staged).count();

                    // Adjust for section headers
                    let file_index = if clicked_row <= staged_count {
                        // In staged section (skip STAGED header at row 0)
                        clicked_row.saturating_sub(0)
                    } else {
                        // In changes section (skip CHANGES header)
                        clicked_row.saturating_sub(1)
                    };

                    if file_index < self.files.len() {
                        self.select_index(file_index);
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
        frame.set_cursor_position((
            chunks[0].x + app.commit_message.len() as u16 + 1,
            chunks[0].y + 1,
        ));
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
}
