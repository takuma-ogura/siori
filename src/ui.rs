use crate::app::{App, FileEntry, FileStatus, HEAD_LABEL, InputMode, Tab, remote_label};
use crate::config::{Config, get_color};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

static CONFIG: OnceLock<Config> = OnceLock::new();

fn config() -> &'static Config {
    CONFIG.get_or_init(Config::load)
}

mod colors {
    use super::{config, get_color};
    use ratatui::style::Color;

    pub fn fg() -> Color {
        get_color(&config().colors.text, Color::Reset)
    }
    pub fn fg_bright() -> Color {
        get_color(&config().colors.text_bright, Color::White)
    }
    pub fn green() -> Color {
        get_color(&config().colors.staged, Color::Green)
    }
    pub fn yellow() -> Color {
        get_color(&config().colors.modified, Color::Yellow)
    }
    pub fn red() -> Color {
        get_color(&config().colors.untracked, Color::Red)
    }
    pub fn blue() -> Color {
        get_color(&config().colors.info, Color::Blue)
    }
    pub fn magenta() -> Color {
        Color::Magenta
    }
    pub fn dim() -> Color {
        get_color(&config().colors.dim, Color::DarkGray)
    }
    pub fn selected() -> Color {
        get_color(&config().colors.selected_bg, Color::DarkGray)
    }
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(2), // Tabs with underline
        Constraint::Min(0),    // Content
        Constraint::Length(3), // Hints
    ])
    .split(area);

    // Tabs with underline
    render_tabs(frame, app, chunks[0]);

    // Content
    match app.tab {
        Tab::Files => render_files_tab(frame, app, chunks[1]),
        Tab::Log => render_log_tab(frame, app, chunks[1]),
    }

    // Hints
    if config().ui.show_hints {
        render_hints(frame, app, chunks[2]);
    }

    // Dialogs (overlays)
    match app.input_mode {
        InputMode::RemoteUrl => render_remote_dialog(frame, app),
        InputMode::RepoSelect => render_repo_select_dialog(frame, app),
        InputMode::TagInput => render_tag_dialog(frame, app),
        InputMode::VersionConfirm => render_version_confirm_dialog(frame, app),
        InputMode::UncommittedWarning => render_uncommitted_warning_dialog(frame, app),
        InputMode::DiscardConfirm => render_discard_confirm_dialog(frame, app),
        InputMode::DeleteTagConfirm => render_delete_tag_confirm_dialog(frame, app),
        InputMode::DiffConfirm => render_diff_confirm_dialog(frame, app),
        _ => {}
    }

    // Processing overlay (highest priority)
    if app.processing.is_active() {
        render_processing_overlay(frame, app);
    }
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let base_dir = std::env::current_dir().unwrap_or_default();
    let repo_name = repo_display_name(&app.repo_path, &base_dir);

    // Line 1: Tabs + repo name
    let is_files = app.tab == Tab::Files;
    let files_style = if is_files {
        Style::default().fg(colors::fg_bright()).bold()
    } else {
        Style::default().fg(colors::dim())
    };
    let log_style = if !is_files {
        Style::default().fg(colors::fg_bright()).bold()
    } else {
        Style::default().fg(colors::dim())
    };

    let tabs_line = Line::from(vec![
        Span::styled(" Files", files_style),
        Span::raw("   "),
        Span::styled("Log", log_style),
        Span::styled(
            format!(
                "{:>width$}",
                format!("@ {}", repo_name),
                width = (area.width as usize).saturating_sub(15)
            ),
            Style::default().fg(colors::green()),
        ),
    ]);

    // Line 2: Underline + branch info
    // Fixed width: " Files" = 6 chars, "   " = 3 chars, "Log" = 3 chars = 12 total
    // Use fixed-width strings so branch info position stays constant
    let underline = if is_files {
        " ━━━━━━         " // Files underline + padding (16 chars total)
    } else {
        "         ━━━    " // Padding + Log underline + padding (16 chars total)
    };
    let status = app.status_label();
    let branch_info = format!("on {}  {}", app.branch_name, status);

    let underline_line = Line::from(vec![
        Span::styled(underline, Style::default().fg(colors::blue())),
        Span::styled(
            format!(
                "{:>width$}",
                branch_info,
                width = (area.width as usize).saturating_sub(16)
            ),
            Style::default().fg(colors::dim()),
        ),
    ]);

    let paragraph = Paragraph::new(vec![tabs_line, underline_line]);
    frame.render_widget(paragraph, area);
}

fn render_files_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    // In INSERT mode, add extra line for IME composition
    let chunks = if app.input_mode == InputMode::Insert {
        Layout::vertical([
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Commit input
            Constraint::Length(1), // IME composition line
            Constraint::Length(1), // Spacing
            Constraint::Min(0),    // Files
        ])
        .split(area)
    } else {
        Layout::vertical([
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Commit input
            Constraint::Length(1), // Spacing
            Constraint::Min(0),    // Files
        ])
        .split(area)
    };

    // Commit input area
    let input_style = if app.input_mode == InputMode::Insert {
        Style::default().fg(colors::fg_bright())
    } else {
        Style::default().fg(colors::dim())
    };

    // Build display text for input box
    let inner_width = chunks[1].width.saturating_sub(2) as usize;
    let input_text = build_input_display(
        &app.commit_message,
        app.cursor_pos,
        inner_width,
        app.input_mode,
    );

    let input = Paragraph::new(input_text).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.input_mode == InputMode::Insert {
                colors::blue()
            } else {
                colors::dim()
            }))
            .title(if app.input_mode == InputMode::Insert {
                if app.is_amending {
                    " [AMEND] "
                } else {
                    " [INSERT] "
                }
            } else {
                " c: commit "
            }),
    );
    frame.render_widget(input, chunks[1]);

    if app.input_mode == InputMode::Insert {
        // Render IME composition line: "  > " prompt for cursor positioning
        let ime_prompt = Paragraph::new(Span::styled("  > ", Style::default().fg(colors::dim())));
        frame.render_widget(ime_prompt, chunks[2]);

        // Position cursor at IME line (for Japanese input compatibility)
        frame.set_cursor_position((chunks[2].x + 4, chunks[2].y));
    }

    // Files list (chunk index differs based on INSERT mode)
    let files_chunk_idx = if app.input_mode == InputMode::Insert {
        4
    } else {
        3
    };
    let staged: Vec<_> = app.files.iter().filter(|f| f.staged).collect();
    let unstaged: Vec<_> = app.files.iter().filter(|f| !f.staged).collect();

    let mut items: Vec<ListItem> = Vec::new();

    items.push(ListItem::new(Line::from(vec![
        Span::styled("STAGED ", Style::default().fg(colors::dim()).bold()),
        Span::styled(
            format!("({})", staged.len()),
            Style::default().fg(colors::green()),
        ),
    ])));
    for file in &staged {
        items.push(create_file_item(file));
    }

    items.push(ListItem::new(Line::from(vec![
        Span::styled("CHANGES ", Style::default().fg(colors::dim()).bold()),
        Span::styled(
            format!("({})", unstaged.len()),
            Style::default().fg(colors::yellow()),
        ),
    ])));
    for file in &unstaged {
        items.push(create_file_item(file));
    }

    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::selected()))
        .highlight_symbol("> ");

    let mut adjusted_state = app.files_state.clone();
    if let Some(idx) = app.files_state.selected() {
        let staged_count = staged.len();
        let adjusted_idx = if idx < staged_count { idx + 1 } else { idx + 2 };
        adjusted_state.select(Some(adjusted_idx));
    }

    frame.render_stateful_widget(list, chunks[files_chunk_idx], &mut adjusted_state);
}

fn create_file_item(file: &FileEntry) -> ListItem<'static> {
    let (status_char, status_color) = match file.status {
        FileStatus::Added => ("A", colors::green()),
        FileStatus::Modified => ("M", colors::yellow()),
        FileStatus::Deleted => ("D", colors::red()),
        FileStatus::Untracked => ("??", colors::red()),
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
        Span::styled(file.path.clone(), Style::default().fg(colors::fg())),
        Span::styled(
            format!("  {}", diff_str),
            Style::default().fg(colors::dim()),
        ),
    ]))
}

fn render_log_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // Spacing
        Constraint::Min(0),    // Commits
    ])
    .split(area);

    let ahead = app.ahead_behind.map(|(a, _)| a).unwrap_or(0);

    let items: Vec<ListItem> = app
        .commits
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let is_unpushed = i < ahead;

            // Color: unpushed=white, pushed=blue
            let color = if is_unpushed {
                colors::fg_bright()
            } else {
                colors::blue()
            };

            // Node symbol: pushed=●, unpushed=○
            let node = if is_unpushed { "○" } else { "●" };

            // Line 1: node + message + labels
            let mut spans = vec![
                Span::styled(format!("{} ", node), Style::default().fg(color)),
                Span::styled(commit.message.clone(), Style::default().fg(colors::fg())),
            ];
            if commit.is_head {
                spans.push(Span::styled(
                    format!(" {}", HEAD_LABEL),
                    Style::default().fg(colors::green()).bold(),
                ));
            }
            for branch in &commit.remote_branches {
                spans.push(Span::styled(
                    format!(" {}", remote_label(branch)),
                    Style::default().fg(colors::blue()),
                ));
            }
            // Tags: pushed=magenta, unpushed=yellow
            for tag in &commit.tags {
                let tag_color = if tag.pushed {
                    colors::magenta()
                } else {
                    colors::yellow()
                };
                spans.push(Span::styled(
                    format!(" [{}]", tag.name),
                    Style::default().fg(tag_color),
                ));
            }

            // Line 2: graph line + hash + time
            ListItem::new(vec![
                Line::from(spans),
                Line::from(Span::styled(
                    format!("│ {} - {}", commit.id, commit.time),
                    Style::default().fg(color),
                )),
            ])
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::selected()))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[1], &mut app.commits_state);
}

fn render_hints(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match app.input_mode {
        InputMode::Insert => vec![("Enter", "commit"), ("Esc", "cancel")],
        InputMode::RepoSelect => vec![("j/k", "move"), ("Enter", "select"), ("Esc", "cancel")],
        InputMode::RemoteUrl => vec![("Enter", "add"), ("Esc", "cancel")],
        InputMode::TagInput => vec![("Enter", "create tag"), ("Esc", "cancel")],
        InputMode::VersionConfirm => vec![("Enter", "update & tag"), ("Esc", "cancel")],
        InputMode::UncommittedWarning => vec![("Enter", "continue"), ("Esc", "cancel")],
        InputMode::DiscardConfirm => vec![("Enter", "discard"), ("Esc", "cancel")],
        InputMode::DeleteTagConfirm => {
            vec![
                ("Enter", "delete all"),
                ("l", "local only"),
                ("Esc", "cancel"),
            ]
        }
        InputMode::DiffConfirm => vec![("Enter", "copy"), ("Esc", "cancel")],
        InputMode::Normal => match app.tab {
            Tab::Files => {
                let mut hints = vec![
                    ("⏎", "diff"),
                    ("Space", "stage"),
                    ("x", "discard"),
                    ("c", "commit"),
                    ("P", "push"),
                ];
                if app.available_repos.len() > 1 {
                    hints.push(("r", "repos"));
                }
                hints.push(("q", "quit"));
                hints
            }
            Tab::Log => {
                let mut hints = vec![
                    ("⏎", "diff"),
                    ("e", "amend"),
                    ("t", "tag"),
                    ("x", "del tag"),
                    ("P", "push"),
                    ("p", "pull"),
                ];
                if app.available_repos.len() > 1 {
                    hints.push(("r", "repos"));
                }
                hints.push(("q", "quit"));
                hints
            }
        },
    };

    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, action)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(*key, Style::default().fg(colors::blue())));
        spans.push(Span::styled(
            format!(" {}", action),
            Style::default().fg(colors::dim()),
        ));
    }

    let content = if let Some((msg, is_error)) = &app.message {
        vec![
            Line::from(spans),
            Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(if *is_error {
                    colors::red()
                } else {
                    colors::green()
                }),
            )),
        ]
    } else {
        vec![Line::from(spans)]
    };

    frame.render_widget(Paragraph::new(content), area);
}

fn render_remote_dialog(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 5, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Add Remote Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // URL input + hint
    let lines = vec![
        Line::from(Span::styled(
            if app.remote_url.is_empty() {
                "https://github.com/user/repo.git"
            } else {
                &app.remote_url
            },
            Style::default().fg(if app.remote_url.is_empty() {
                colors::dim()
            } else {
                colors::fg_bright()
            }),
        )),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(colors::blue())),
            Span::styled(" add & push  ", Style::default().fg(colors::dim())),
            Span::styled("Esc", Style::default().fg(colors::blue())),
            Span::styled(" cancel", Style::default().fg(colors::dim())),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    // Cursor
    frame.set_cursor_position((inner.x + app.remote_url.width() as u16, inner.y));
}

fn render_repo_select_dialog(frame: &mut Frame, app: &mut App) {
    let base_dir = std::env::current_dir().unwrap_or_default();
    let height = (app.available_repos.len() + 2).min(15) as u16;
    let area = centered_rect(50, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Select Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = app
        .available_repos
        .iter()
        .map(|path| {
            let name = repo_display_name(path, &base_dir);
            let is_current = path == &app.repo_path;
            let color = if is_current {
                colors::green()
            } else {
                colors::fg()
            };
            let suffix = if is_current { " (current)" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(name, Style::default().fg(color)),
                Span::styled(suffix, Style::default().fg(colors::dim())),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::selected()))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, inner, &mut app.repo_select_state);
}

fn render_tag_dialog(frame: &mut Frame, app: &App) {
    let is_editing = app.editing_tag.is_some();
    let title = if is_editing {
        " Edit Version "
    } else {
        " New Version "
    };

    let area = centered_rect(50, 6, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Get commit info
    let commit_info = app
        .commits_state
        .selected()
        .and_then(|i| app.commits.get(i))
        .map(|c| format!("on commit: {}", c.id))
        .unwrap_or_default();

    let warning = if is_editing
        && app
            .commits_state
            .selected()
            .and_then(|i| app.commits.get(i))
            .and_then(|c| c.tags.first())
            .is_some_and(|t| t.pushed)
    {
        Some("⚠ Tag already pushed - will update remote")
    } else {
        None
    };

    let mut lines = vec![Line::from(Span::styled(
        commit_info,
        Style::default().fg(colors::dim()),
    ))];

    if let Some(warn) = warning {
        lines.push(Line::from(Span::styled(
            warn,
            Style::default().fg(colors::yellow()),
        )));
    }

    lines.push(Line::from(Span::styled(
        format!("Tag: {}", app.tag_input),
        Style::default().fg(colors::fg_bright()),
    )));

    frame.render_widget(Paragraph::new(lines), inner);

    // Cursor position
    let cursor_y = inner.y + if warning.is_some() { 2 } else { 1 };
    frame.set_cursor_position((inner.x + 5 + app.tag_input.width() as u16, cursor_y));
}

fn render_processing_overlay(frame: &mut Frame, app: &App) {
    use crate::app::Processing;

    let area = centered_rect(30, 3, frame.area());
    frame.render_widget(Clear, area);

    // Use green for tag push, blue for other operations
    let border_color = match app.processing {
        Processing::PushingTags => colors::green(),
        _ => colors::blue(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = format!("{} {}", app.spinner_char(), app.processing.message());
    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(colors::fg_bright()))
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, inner);
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Build display text for commit input box.
/// Scrolls text to keep cursor position visible with ellipsis indicators.
fn build_input_display(
    text: &str,
    cursor_pos: usize,
    max_width: usize,
    input_mode: InputMode,
) -> String {
    // Show placeholder when empty and not in insert mode
    if text.is_empty() && input_mode != InputMode::Insert {
        return "Commit message...".to_string();
    }

    let total_width = text.width();
    if total_width <= max_width {
        // Insert visual cursor in INSERT mode
        if input_mode == InputMode::Insert && total_width < max_width {
            let mut result = String::new();
            result.push_str(&text[..cursor_pos]);
            result.push('│');
            result.push_str(&text[cursor_pos..]);
            return result;
        }
        return text.to_string();
    }

    // Calculate cursor position in display width
    let cursor_display_pos = text[..cursor_pos].width();

    // Determine scroll offset based on cursor position
    // Goal: use full width of input box, show text ending at right edge when typing at end
    let scroll_offset = if cursor_display_pos <= max_width.saturating_sub(1) {
        // Cursor fits without scrolling - show from beginning
        0
    } else {
        // Scroll to show cursor at the right edge (with 1 char margin)
        cursor_display_pos.saturating_sub(max_width.saturating_sub(2))
    };

    // Determine ellipsis needs
    let needs_start_ellipsis = scroll_offset > 0;
    let needs_end_ellipsis = scroll_offset + max_width < total_width;

    // Available width for actual text (minus ellipsis)
    let available_width = max_width
        .saturating_sub(if needs_start_ellipsis { 1 } else { 0 })
        .saturating_sub(if needs_end_ellipsis { 1 } else { 0 });

    // Extract visible portion
    let mut result = String::new();
    let mut current_width = 0;
    let mut skip_remaining = scroll_offset;

    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);

        // Skip characters before scroll offset
        if skip_remaining > 0 {
            if skip_remaining >= ch_width {
                skip_remaining -= ch_width;
                continue;
            }
            skip_remaining = 0;
        }

        // Stop if we've filled the available width
        if current_width + ch_width > available_width {
            break;
        }

        result.push(ch);
        current_width += ch_width;
    }

    // Build final string with ellipsis
    let mut output = String::new();
    if needs_start_ellipsis {
        output.push('…');
    }
    output.push_str(&result);
    if needs_end_ellipsis {
        output.push('…');
    }

    // Insert visual cursor in INSERT mode
    if input_mode == InputMode::Insert {
        let cursor_screen_x = if needs_start_ellipsis {
            1 + cursor_display_pos.saturating_sub(scroll_offset)
        } else {
            cursor_display_pos
        };

        let mut chars: Vec<char> = output.chars().collect();
        let insert_pos = cursor_screen_x.min(chars.len());
        chars.insert(insert_pos, '│');
        output = chars.into_iter().collect();
    }

    output
}

/// Get display name for a repository path relative to base directory
fn repo_display_name(path: &std::path::Path, base_dir: &std::path::Path) -> String {
    path.strip_prefix(base_dir)
        .map(|p| {
            let s = p.display().to_string();
            if s.is_empty() {
                // Current directory - show its name
                base_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(".")
                    .to_string()
            } else {
                s
            }
        })
        .unwrap_or_else(|_| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("repo")
                .to_string()
        })
}

fn render_version_confirm_dialog(frame: &mut Frame, app: &App) {
    let Some(pending) = &app.pending_version_update else {
        return;
    };

    let height = 6 + pending.files.len() as u16;
    let area = centered_rect(50, height.min(15), frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Version Update ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(format!(
            "Version: {} → Tag: {}",
            pending.new_version, pending.tag_name
        )),
        Line::from(""),
        Line::from("Files to update:"),
    ];

    for file in &pending.files {
        lines.push(Line::from(format!(
            "  {} ({} → {})",
            file.path, file.current_version, pending.new_version
        )));
    }

    let paragraph = Paragraph::new(lines).style(Style::default().fg(colors::fg()));
    frame.render_widget(paragraph, inner);
}

fn render_uncommitted_warning_dialog(frame: &mut Frame, _app: &App) {
    let area = centered_rect(45, 5, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Warning ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::yellow()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = "You have uncommitted changes.\nContinue anyway?";
    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(colors::yellow()))
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, inner);
}

fn render_discard_confirm_dialog(frame: &mut Frame, app: &App) {
    let Some(path) = &app.pending_discard_file else {
        return;
    };

    let area = centered_rect(45, 6, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Discard Changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::red()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from("Discard changes to:"),
        Line::from(Span::styled(
            path.as_str(),
            Style::default().fg(colors::yellow()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "This cannot be undone!",
            Style::default().fg(colors::red()),
        )),
    ];

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn render_delete_tag_confirm_dialog(frame: &mut Frame, app: &App) {
    let Some((tag_name, was_pushed)) = &app.pending_delete_tag else {
        return;
    };

    let area = centered_rect(45, 6, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Delete Tag ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::red()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let hint = if *was_pushed {
        "Enter: local+remote  l: local only"
    } else {
        "Enter: delete local tag"
    };

    let lines = vec![
        Line::from("Delete tag:"),
        Line::from(Span::styled(
            tag_name.as_str(),
            Style::default().fg(colors::yellow()),
        )),
        Line::from(""),
        Line::from(Span::styled(hint, Style::default().fg(colors::dim()))),
    ];

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn render_diff_confirm_dialog(frame: &mut Frame, app: &App) {
    let Some(cmd) = &app.pending_diff_command else {
        return;
    };

    let area = centered_rect(60, 5, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Copy Command ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            cmd.as_str(),
            Style::default().fg(colors::fg_bright()),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(colors::blue())),
            Span::styled(" copy  ", Style::default().fg(colors::dim())),
            Span::styled("Esc", Style::default().fg(colors::blue())),
            Span::styled(" cancel", Style::default().fg(colors::dim())),
        ]),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
