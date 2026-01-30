use crate::app::{App, FileEntry, FileStatus, HEAD_LABEL, InputMode, Tab, remote_label};
use crate::config::{Config, get_color};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs},
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
        Constraint::Length(1), // Title + Repo
        Constraint::Length(3), // Tabs
        Constraint::Min(0),    // Content
        Constraint::Length(3), // Hints
    ])
    .split(area);

    // Title with repo name
    let base_dir = std::env::current_dir().unwrap_or_default();
    let repo_display = repo_display_name(&app.repo_path, &base_dir);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("siori", Style::default().fg(colors::fg_bright()).bold()),
        Span::styled(" @ ", Style::default().fg(colors::dim())),
        Span::styled(repo_display, Style::default().fg(colors::green()).bold()),
    ]));
    frame.render_widget(title, chunks[0]);

    // Tabs
    let tabs = Tabs::new(vec!["Files", "Log"])
        .select(match app.tab {
            Tab::Files => 0,
            Tab::Log => 1,
        })
        .style(Style::default().fg(colors::dim()))
        .highlight_style(Style::default().fg(colors::blue()).bold())
        .divider(" ");
    frame.render_widget(tabs, chunks[1]);

    // Content
    match app.tab {
        Tab::Files => render_files_tab(frame, app, chunks[2]),
        Tab::Log => render_log_tab(frame, app, chunks[2]),
    }

    // Hints
    if config().ui.show_hints {
        render_hints(frame, app, chunks[3]);
    }

    // Dialogs (overlays)
    match app.input_mode {
        InputMode::RemoteUrl => render_remote_dialog(frame, app),
        InputMode::RepoSelect => render_repo_select_dialog(frame, app),
        InputMode::TagInput => render_tag_dialog(frame, app),
        _ => {}
    }

    // Processing overlay (highest priority)
    if app.processing.is_active() {
        render_processing_overlay(frame, app);
    }
}

fn render_files_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Commit input
        Constraint::Min(0),    // Files
    ])
    .split(area);

    // Commit input area
    let input_style = if app.input_mode == InputMode::Insert {
        Style::default().fg(colors::fg_bright())
    } else {
        Style::default().fg(colors::dim())
    };

    let input_text = if app.commit_message.is_empty() && app.input_mode != InputMode::Insert {
        "Commit message...".to_string()
    } else {
        app.commit_message.clone()
    };

    let input = Paragraph::new(input_text).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.input_mode == InputMode::Insert {
                colors::blue()
            } else {
                colors::dim()
            }))
            .title(if app.input_mode == InputMode::Insert {
                " [INSERT] "
            } else {
                " c: commit "
            }),
    );
    frame.render_widget(input, chunks[0]);

    if app.input_mode == InputMode::Insert {
        let cursor_x = chunks[0].x + app.commit_message.width() as u16 + 1;
        frame.set_cursor_position((cursor_x, chunks[0].y + 1));
    }

    // Files list
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

    frame.render_stateful_widget(list, chunks[1], &mut adjusted_state);
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
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

    // Branch info with status label and unpushed tag count
    let status_label = app.status_label();
    let unpushed_tags = app.unpushed_tag_count();
    let mut branch_spans = vec![
        Span::styled("on ", Style::default().fg(colors::dim())),
        Span::styled(
            app.branch_name.clone(),
            Style::default().fg(colors::green()).bold(),
        ),
        Span::styled(
            format!("  {}", status_label),
            Style::default().fg(colors::yellow()),
        ),
    ];
    if unpushed_tags > 0 {
        branch_spans.push(Span::styled(
            format!("  ● {} tag unpushed", unpushed_tags),
            Style::default().fg(colors::yellow()),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(branch_spans)), chunks[0]);

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
        InputMode::TagInput => vec![("Enter", "save"), ("Esc", "cancel")],
        InputMode::Normal => match app.tab {
            Tab::Files => vec![
                ("Space", "stage"),
                ("c", "commit"),
                ("P", "push"),
                ("R", "refresh"),
                ("q", "quit"),
            ],
            Tab::Log => vec![
                ("t", "tag"),
                ("T", "push tags"),
                ("P", "push"),
                ("p", "pull"),
                ("R", "refresh"),
                ("q", "quit"),
            ],
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
        " Edit Tag "
    } else {
        " Create Tag "
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
    let area = centered_rect(30, 3, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::blue()));

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
