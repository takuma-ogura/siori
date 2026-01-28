use crate::app::{App, FileEntry, FileStatus, InputMode, Tab};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs},
};
use unicode_width::UnicodeWidthStr;

mod colors {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(26, 27, 38);
    pub const FG: Color = Color::Rgb(169, 177, 214);
    pub const FG_BRIGHT: Color = Color::Rgb(192, 202, 245);
    pub const GREEN: Color = Color::Rgb(158, 206, 106);
    pub const YELLOW: Color = Color::Rgb(224, 175, 104);
    pub const RED: Color = Color::Rgb(247, 118, 142);
    pub const BLUE: Color = Color::Rgb(122, 162, 247);
    pub const DIM: Color = Color::Rgb(86, 95, 137);
    pub const SELECTED: Color = Color::Rgb(40, 52, 87);
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
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

    // Hints
    render_hints(frame, app, chunks[3]);

    // Remote URL dialog (overlay)
    if app.input_mode == InputMode::RemoteUrl {
        render_remote_dialog(frame, app);
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
        Style::default().fg(colors::FG_BRIGHT)
    } else {
        Style::default().fg(colors::DIM)
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

    if app.input_mode == InputMode::Insert {
        let cursor_x = chunks[0].x + app.commit_message.width() as u16 + 1;
        frame.set_cursor_position((cursor_x, chunks[0].y + 1));
    }

    // Files list
    let staged: Vec<_> = app.files.iter().filter(|f| f.staged).collect();
    let unstaged: Vec<_> = app.files.iter().filter(|f| !f.staged).collect();

    let mut items: Vec<ListItem> = Vec::new();

    items.push(ListItem::new(Line::from(vec![
        Span::styled("STAGED ", Style::default().fg(colors::DIM).bold()),
        Span::styled(
            format!("({})", staged.len()),
            Style::default().fg(colors::GREEN),
        ),
    ])));
    for file in &staged {
        items.push(create_file_item(file));
    }

    items.push(ListItem::new(Line::from(vec![
        Span::styled("CHANGES ", Style::default().fg(colors::DIM).bold()),
        Span::styled(
            format!("({})", unstaged.len()),
            Style::default().fg(colors::YELLOW),
        ),
    ])));
    for file in &unstaged {
        items.push(create_file_item(file));
    }

    let list = List::new(items)
        .highlight_style(Style::default().bg(colors::SELECTED))
        .highlight_symbol("> ");

    let mut adjusted_state = app.files_state.clone();
    if let Some(idx) = app.files_state.selected() {
        let staged_count = staged.len();
        let adjusted_idx = if idx < staged_count {
            idx + 1
        } else {
            idx + 2
        };
        adjusted_state.select(Some(adjusted_idx));
    }

    frame.render_stateful_widget(list, chunks[1], &mut adjusted_state);
}

fn create_file_item(file: &FileEntry) -> ListItem<'static> {
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
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(area);

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

    let ahead = app.ahead_behind.map(|(a, _)| a).unwrap_or(0);

    let items: Vec<ListItem> = app
        .commits
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let is_unpushed = i < ahead;
            let graph_color = if is_unpushed { colors::BLUE } else { colors::DIM };

            let node = if commit.is_head {
                "● "
            } else if is_unpushed {
                "○ "
            } else {
                "● "
            };

            let mut spans = vec![
                Span::styled("│ ", Style::default().fg(graph_color)),
                Span::styled(node, Style::default().fg(graph_color)),
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
                    format!("│     {} - {}", commit.id, commit.time),
                    Style::default().fg(graph_color),
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

    frame.render_widget(Paragraph::new(content), area);
}

fn render_remote_dialog(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 5, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Add Remote Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::BLUE));

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
                colors::DIM
            } else {
                colors::FG_BRIGHT
            }),
        )),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(colors::BLUE)),
            Span::styled(" add & push  ", Style::default().fg(colors::DIM)),
            Span::styled("Esc", Style::default().fg(colors::BLUE)),
            Span::styled(" cancel", Style::default().fg(colors::DIM)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    // Cursor
    frame.set_cursor_position((
        inner.x + app.remote_url.width() as u16,
        inner.y,
    ));
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
