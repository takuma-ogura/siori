//! Custom diff viewer with full file display and change highlighting

use anyhow::{Context, Result};
use crossterm::{
    ExecutableCommand,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use std::io::stdout;
use std::path::Path;
use std::process::Command;

// High contrast colors for universal design
const BG_ADDED: Color = Color::Rgb(45, 74, 62); // Dark green
const BG_DELETED: Color = Color::Rgb(74, 45, 45); // Dark red
const FG_LINE_NUM: Color = Color::Rgb(106, 106, 106);
const FG_MARKER_ADD: Color = Color::Rgb(120, 200, 120);
const FG_MARKER_DEL: Color = Color::Rgb(200, 120, 120);

#[derive(Clone, Copy, PartialEq)]
enum LineKind {
    Context,
    Added,
    Deleted,
}

struct DiffLine {
    line_number: Option<usize>,
    content: String,
    kind: LineKind,
}

struct DiffData {
    file_path: String,
    lines: Vec<DiffLine>,
    added: usize,
    deleted: usize,
    change_indices: Vec<usize>,
}

struct DiffViewer {
    data: DiffData,
    scroll: usize,
    current_change: usize,
}

impl DiffViewer {
    fn new(data: DiffData) -> Self {
        Self {
            data,
            scroll: 0,
            current_change: 0,
        }
    }

    fn scroll_down(&mut self, amount: usize) {
        let max = self.data.lines.len().saturating_sub(1);
        self.scroll = (self.scroll + amount).min(max);
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    fn next_change(&mut self) {
        if self.data.change_indices.is_empty() {
            return;
        }
        self.current_change = (self.current_change + 1) % self.data.change_indices.len();
        self.scroll = self.data.change_indices[self.current_change].saturating_sub(5);
    }

    fn prev_change(&mut self) {
        if self.data.change_indices.is_empty() {
            return;
        }
        self.current_change = if self.current_change == 0 {
            self.data.change_indices.len() - 1
        } else {
            self.current_change - 1
        };
        self.scroll = self.data.change_indices[self.current_change].saturating_sub(5);
    }
}

/// Parse unified diff output into DiffData
fn parse_diff(file_path: &str, diff_output: &str, file_content: &str) -> DiffData {
    let mut lines = Vec::new();
    let mut added = 0;
    let mut deleted = 0;
    let mut change_indices = Vec::new();

    // Parse diff to find changed line numbers and deleted content
    let mut added_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut deleted_at: std::collections::HashMap<usize, Vec<String>> =
        std::collections::HashMap::new();
    let mut current_new_line = 0usize;

    for line in diff_output.lines() {
        if line.starts_with("@@") {
            // Parse hunk header: @@ -old,count +new,count @@
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Some(new_start) = parts[2].trim_start_matches('+').split(',').next() {
                    current_new_line = new_start.parse().unwrap_or(1);
                }
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            added_lines.insert(current_new_line);
            added += 1;
            current_new_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deleted_at
                .entry(current_new_line)
                .or_default()
                .push(line[1..].to_string());
            deleted += 1;
        } else if !line.starts_with('\\') {
            current_new_line += 1;
        }
    }

    // Build display lines from file content, inserting deleted lines
    for (i, content) in file_content.lines().enumerate() {
        let line_num = i + 1;

        // Insert deleted lines before this line
        if let Some(deleted_content) = deleted_at.get(&line_num) {
            for del_line in deleted_content {
                change_indices.push(lines.len());
                lines.push(DiffLine {
                    line_number: None,
                    content: del_line.clone(),
                    kind: LineKind::Deleted,
                });
            }
        }

        let kind = if added_lines.contains(&line_num) {
            change_indices.push(lines.len());
            LineKind::Added
        } else {
            LineKind::Context
        };
        lines.push(DiffLine {
            line_number: Some(line_num),
            content: content.to_string(),
            kind,
        });
    }

    // Handle deleted lines at the end of file
    let last_line = file_content.lines().count() + 1;
    if let Some(deleted_content) = deleted_at.get(&last_line) {
        for del_line in deleted_content {
            change_indices.push(lines.len());
            lines.push(DiffLine {
                line_number: None,
                content: del_line.clone(),
                kind: LineKind::Deleted,
            });
        }
    }

    DiffData {
        file_path: file_path.to_string(),
        lines,
        added,
        deleted,
        change_indices,
    }
}

fn render(frame: &mut Frame, viewer: &DiffViewer) {
    let area = frame.area();

    // Layout: header, content, footer
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    // Header
    let changes_str = if viewer.data.change_indices.is_empty() {
        "No changes".to_string()
    } else {
        format!(
            "Change {}/{}",
            viewer.current_change + 1,
            viewer.data.change_indices.len()
        )
    };
    let header = format!(
        " Δ {}  [+{} -{}]  {}",
        viewer.data.file_path, viewer.data.added, viewer.data.deleted, changes_str
    );
    let header_widget = Paragraph::new(header)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header_widget, chunks[0]);

    // Content
    let visible_height = chunks[1].height as usize;
    let line_num_width = viewer
        .data
        .lines
        .last()
        .and_then(|l| l.line_number)
        .unwrap_or(0)
        .to_string()
        .len()
        .max(4);

    let visible_lines: Vec<Line> = viewer
        .data
        .lines
        .iter()
        .skip(viewer.scroll)
        .take(visible_height)
        .map(|diff_line| {
            let (marker, marker_style, bg) = match diff_line.kind {
                LineKind::Added => ("+", Style::default().fg(FG_MARKER_ADD), Some(BG_ADDED)),
                LineKind::Deleted => ("-", Style::default().fg(FG_MARKER_DEL), Some(BG_DELETED)),
                LineKind::Context => (" ", Style::default(), None),
            };

            let line_num_str = diff_line
                .line_number
                .map(|n| format!("{:>width$}", n, width = line_num_width))
                .unwrap_or_else(|| " ".repeat(line_num_width));

            let base_style = bg.map(|c| Style::default().bg(c)).unwrap_or_default();

            Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(
                    format!(" {} │ ", line_num_str),
                    Style::default().fg(FG_LINE_NUM),
                ),
                Span::styled(&diff_line.content, base_style.fg(Color::White)),
            ])
        })
        .collect();

    let content = Paragraph::new(visible_lines);
    frame.render_widget(content, chunks[1]);

    // Scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    let mut scrollbar_state = ScrollbarState::new(viewer.data.lines.len()).position(viewer.scroll);
    frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);

    // Footer
    let footer = " j/k: scroll  n/N: next/prev change  q: quit";
    let footer_widget = Paragraph::new(footer).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer_widget, chunks[2]);
}

pub fn run(repo_path: &Path, file_path: &str, staged: bool) -> Result<()> {
    // Get diff output
    let diff_args = if staged {
        vec!["diff", "--cached", "-U0", "--", file_path]
    } else {
        vec!["diff", "-U0", "--", file_path]
    };

    let diff_output = Command::new("git")
        .current_dir(repo_path)
        .args(&diff_args)
        .output()
        .context("Failed to run git diff")?;

    let diff_str = String::from_utf8_lossy(&diff_output.stdout);

    // Read file content
    let full_path = repo_path.join(file_path);
    let file_content = std::fs::read_to_string(&full_path).unwrap_or_else(|_| String::new());

    let data = parse_diff(file_path, &diff_str, &file_content);

    if data.lines.is_empty() {
        println!("No content to display");
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut viewer = DiffViewer::new(data);

    // Jump to first change if exists
    if !viewer.data.change_indices.is_empty() {
        viewer.scroll = viewer.data.change_indices[0].saturating_sub(5);
    }

    loop {
        terminal.draw(|f| render(f, &viewer))?;

        // 16ms polling for ~60fps responsiveness
        if event::poll(std::time::Duration::from_millis(16))? {
            // Drain all pending events before redrawing
            loop {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            disable_raw_mode()?;
                            stdout().execute(DisableMouseCapture)?;
                            stdout().execute(LeaveAlternateScreen)?;
                            return Ok(());
                        }
                        KeyCode::Char('j') | KeyCode::Down => viewer.scroll_down(1),
                        KeyCode::Char('k') | KeyCode::Up => viewer.scroll_up(1),
                        KeyCode::Char('d') | KeyCode::PageDown => viewer.scroll_down(20),
                        KeyCode::Char('u') | KeyCode::PageUp => viewer.scroll_up(20),
                        KeyCode::Char('n') => viewer.next_change(),
                        KeyCode::Char('N') => viewer.prev_change(),
                        KeyCode::Char('g') | KeyCode::Home => viewer.scroll = 0,
                        KeyCode::Char('G') | KeyCode::End => {
                            viewer.scroll = viewer.data.lines.len().saturating_sub(1);
                        }
                        _ => {}
                    },
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollDown => viewer.scroll_down(3),
                        MouseEventKind::ScrollUp => viewer.scroll_up(3),
                        _ => {}
                    },
                    _ => {}
                }
                // Process remaining queued events without waiting
                if !event::poll(std::time::Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

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
