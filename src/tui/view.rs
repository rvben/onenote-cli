use super::{App, Compose, InputKind, Location, Overlay, WriteKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

struct Theme {
    ink: Color,
    muted: Color,
    edge: Color,
    accent: Color,
    selected: Color,
    bg: Color,
}
impl Theme {
    fn new(color: bool) -> Self {
        if color {
            Self {
                ink: Color::Rgb(237, 231, 246),
                muted: Color::Rgb(172, 158, 187),
                edge: Color::Rgb(75, 61, 89),
                accent: Color::Rgb(202, 164, 250),
                selected: Color::Rgb(67, 48, 86),
                bg: Color::Rgb(27, 21, 35),
            }
        } else {
            Self {
                ink: Color::White,
                muted: Color::Gray,
                edge: Color::DarkGray,
                accent: Color::White,
                selected: Color::DarkGray,
                bg: Color::Black,
            }
        }
    }
    fn text(&self) -> Style {
        Style::default().fg(self.ink).bg(self.bg)
    }
    fn muted(&self) -> Style {
        self.text().fg(self.muted)
    }
    fn accent(&self) -> Style {
        self.text().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    fn panel(&self) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.edge))
            .style(self.text())
    }
}
fn safe(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let len = text.chars().count();
    if len <= width {
        text.to_owned()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(width.saturating_sub(1))
                .collect::<String>()
        )
    }
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let t = Theme::new(app.color);
    let area = frame.area();
    frame.render_widget(Block::default().style(t.text()), area);
    if area.width < 42 || area.height < 12 {
        frame.render_widget(
            Paragraph::new(
                "ONENOTE  /  WORKSPACE\n\nEnlarge terminal to at least 42 × 12.\n\nq  quit",
            )
            .style(t.text()),
            area,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    header(frame, rows[0], app, &t);
    if app.full_page {
        preview(frame, rows[1], app, &t);
    } else if area.width >= 100 {
        let columns = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(rows[1]);
        list(frame, columns[0], app, &t);
        preview(frame, columns[1], app, &t);
    } else if area.width >= 74 && area.height >= 24 {
        let panes = Layout::vertical([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(rows[1]);
        list(frame, panes[0], app, &t);
        preview(frame, panes[1], app, &t);
    } else {
        list(frame, rows[1], app, &t);
    }
    footer(frame, rows[2], app, &t);
    overlay(frame, app, &t);
}
fn header(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let profile = safe(&app.profile_name);
    let target = safe(app.profile.target());
    let status = if app.profile.read_only {
        "  READ ONLY"
    } else {
        ""
    };
    let brand = if area.width < 70 {
        "  ONENOTE"
    } else {
        "  ONENOTE  /  WORKSPACE"
    };
    let context = format!("  {}  ·  {}  ·  {}", app.location.label(), profile, target);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(brand, t.accent()),
                Span::styled(status, t.muted()),
            ]),
            Line::styled(
                fit(&safe(&context), area.width.saturating_sub(2) as usize),
                t.muted(),
            ),
            Line::styled(
                "  b notebooks   / indexed search   s literal scan   ? help",
                t.muted(),
            ),
        ])
        .style(t.text()),
        area,
    );
}
fn list(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let title = format!(
        " {}  ·  {} / {}{} ",
        app.location.heading(),
        app.rows.len(),
        app.total,
        if app.next.is_some() {
            "  ·  n more"
        } else {
            ""
        }
    );
    let block = t.panel().title(Span::styled(title, t.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.rows.is_empty() {
        let copy = if app.loading {
            "Loading from OneNote…"
        } else if app.error.is_some() {
            "Could not load items. Press r to retry."
        } else {
            match app.location {
                Location::Notebooks => {
                    "No open notebooks. Open a notebook in OneNote desktop, then refresh."
                }
                Location::Sections(_) => "No sections in this notebook.",
                Location::Pages { .. } => "No pages yet. Press c to create one.",
                Location::Search { .. } => "No matches. Try another query or literal scan (s).",
            }
        };
        frame.render_widget(
            Paragraph::new(format!("\n  {copy}"))
                .style(t.muted())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let visible = app.visible_indices();
    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new("\n  No rows match this filter. Press f to edit or Esc to clear.")
                .style(t.muted())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let items: Vec<ListItem> = visible
        .iter()
        .map(|index| {
            let row = &app.rows[*index];
            let label = if row.locked {
                format!("  {}  [locked]", safe(&row.name))
            } else {
                format!("  {}", safe(&row.name))
            };
            let meta = if matches!(app.location, Location::Search { .. }) {
                safe(&row.path)
            } else if let Some(modified) = &row.modified {
                format!("Updated {}", modified.chars().take(10).collect::<String>())
            } else {
                safe(&row.path)
            };
            ListItem::new(vec![
                Line::styled(
                    fit(&label, inner.width.saturating_sub(1) as usize),
                    t.text().add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    format!("  {}", fit(&meta, inner.width.saturating_sub(3) as usize)),
                    t.muted(),
                ),
            ])
        })
        .collect();
    let mut state = ListState::default();
    state.select(visible.iter().position(|index| *index == app.selected));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(t.text().bg(t.selected))
            .highlight_symbol("▌"),
        inner,
        &mut state,
    );
}
fn preview(frame: &mut Frame, area: Rect, app: &mut App, t: &Theme) {
    let title = if app.full_page {
        " PAGE  ·  Esc back "
    } else {
        " PAGE PREVIEW "
    };
    let block = t.panel().title(Span::styled(title, t.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if !app.visible_indices().contains(&app.selected) {
        app.page_max_scroll = 0;
        frame.render_widget(
            Paragraph::new("\n  No page matches this filter.").style(t.muted()),
            inner,
        );
        return;
    }
    let Some(row) = app.selected_entry() else {
        app.page_max_scroll = 0;
        frame.render_widget(
            Paragraph::new("\n  Choose an item to see it here.").style(t.muted()),
            inner,
        );
        return;
    };
    if !matches!(
        app.location,
        Location::Pages { .. } | Location::Search { .. }
    ) {
        let copy = match app.location {
            Location::Notebooks => "Enter opens this notebook's sections.",
            Location::Sections(_) => "Enter opens this section's pages.",
            _ => "",
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw(""),
                Line::styled(format!("  {}", safe(&row.name)), t.accent()),
                Line::raw(""),
                Line::styled(format!("  {copy}"), t.muted()),
            ])
            .style(t.text())
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let mut lines = vec![
        Line::styled(format!("  {}", safe(&row.name)), t.accent()),
        Line::styled(format!("  {}", safe(&row.path)), t.muted()),
        Line::raw(""),
    ];
    if let Some(value) = app
        .page
        .as_ref()
        .filter(|v| v["id"].as_str() == Some(row.id.as_str()))
    {
        let markdown = value["markdown"].as_str().unwrap_or("");
        if markdown.trim().is_empty() {
            lines.push(Line::styled("  No text content on this page", t.muted()));
        } else {
            for line in markdown.lines() {
                lines.push(Line::styled(format!("  {}", safe(line)), t.text()));
            }
        }
        if value["has_images"] == true || value["has_attachments"] == true {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "  Images and attachments remain in OneNote desktop",
                t.muted(),
            ));
        }
    } else if app.loading {
        lines.push(Line::styled("  Reading page…", t.muted()));
    } else {
        lines.push(Line::styled("  Press Enter to read this page", t.muted()));
    }
    let paragraph = Paragraph::new(lines)
        .style(t.text())
        .wrap(Wrap { trim: false });
    app.page_max_scroll = paragraph
        .line_count(inner.width)
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    app.page_scroll = app.page_scroll.min(app.page_max_scroll);
    frame.render_widget(paragraph.scroll((app.page_scroll, 0)), inner);
}
fn footer(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let message = if let Some(error) = &app.error {
        format!("{} · {}", app.notice, safe(error))
    } else {
        app.notice.clone()
    };
    let keys = if area.width < 70 {
        "j/k move  Enter open  Esc back  ? keys  q quit"
    } else if app.profile.read_only {
        "j/k move  Enter open  Esc back  / search  s scan  f filter  r refresh  ? help  q quit"
    } else {
        "j/k move  Enter open  Esc back  / search  s scan  c create  a append  r refresh  ? help  q quit"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!("  {}", fit(&message, area.width.saturating_sub(2) as usize)),
                if app.error.is_some() {
                    t.accent()
                } else {
                    t.muted()
                },
            ),
            Line::styled(
                format!("  {}", fit(keys, area.width.saturating_sub(2) as usize)),
                t.muted(),
            ),
        ])
        .style(t.text()),
        area,
    );
}
fn overlay(frame: &mut Frame, app: &mut App, t: &Theme) {
    let error = app.error.as_deref();
    match &mut app.overlay {
        Overlay::None => {}
        Overlay::Help => {
            frame.render_widget(Clear, frame.area());
            frame.render_widget(Block::default().style(t.text()), frame.area());
            let area = centered(frame.area(), 78, 20);
            let block = t
                .panel()
                .title(Span::styled(" KEYBOARD GUIDE ", t.accent()));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let lines = if frame.area().height < 22 {
                vec![
                    "  Enter open  ·  Esc back",
                    "  j/k move  ·  b notebooks",
                    "  / indexed search  ·  s literal scan",
                    "  f filter  ·  n more  ·  r refresh",
                    "  c create  ·  a append",
                    "  Ctrl+S review  ·  Esc keep draft",
                    "  PgUp/PgDn scroll  ·  q quit",
                ]
            } else {
                vec![
                    "",
                    "  NAVIGATE",
                    "  Enter / →    Open notebook, section, or page",
                    "  Esc / ←      Go back · Esc in full page returns to list",
                    "  j / k        Move selection",
                    "  b            Return to notebooks",
                    "  n / r        More items / refresh",
                    "",
                    "  FIND",
                    "  /            Indexed OneNote search",
                    "  s            Literal title and text scan (first 100 pages)",
                    "  f            Filter loaded items",
                    "",
                    "  WRITE",
                    "  c / a        Create page / append to selected page",
                    "  Tab          Switch title and body in create editor",
                    "  Ctrl+S       Review before writing · Esc keeps draft",
                    "",
                    "  PgUp/PgDn    Scroll page · q quit · any key closes help",
                ]
            };
            frame.render_widget(
                Paragraph::new(lines.join("\n"))
                    .style(t.text())
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
        Overlay::Input { kind, editor } => {
            frame.render_widget(Clear, frame.area());
            frame.render_widget(Block::default().style(t.text()), frame.area());
            let area = centered(frame.area(), 78, 8);
            let title = match kind {
                InputKind::Indexed => " INDEXED SEARCH ",
                InputKind::Literal => " LITERAL SCAN ",
                InputKind::Filter => " FILTER LOADED ITEMS ",
            };
            let hint = match kind {
                InputKind::Indexed => "OneNote search syntax · results may omit unindexed pages",
                InputKind::Literal => "Case-insensitive text scan · first 100 pages",
                InputKind::Filter => "Match item name or hierarchy path",
            };
            let block = t.panel().title(Span::styled(title, t.accent()));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            frame.render_widget(
                Paragraph::new(format!(
                    "\n  {hint}\n\n  {}\n\n  Enter apply  ·  Esc cancel  ·  Ctrl+U clear",
                    safe(&editor.display())
                ))
                .style(t.text())
                .wrap(Wrap { trim: false }),
                inner,
            );
        }
        Overlay::Compose(draft) => compose(frame, t, draft, false, error),
        Overlay::Review(draft) => compose(frame, t, draft, true, error),
    }
}
fn compose(frame: &mut Frame, t: &Theme, draft: &mut Compose, review: bool, error: Option<&str>) {
    frame.render_widget(Clear, frame.area());
    frame.render_widget(Block::default().style(t.text()), frame.area());
    let area = centered(frame.area(), 86, frame.area().height.min(26));
    let title = if review {
        " REVIEW WRITE "
    } else if draft.kind == WriteKind::Create {
        " CREATE PAGE "
    } else {
        " APPEND TO PAGE "
    };
    let block = t.panel().title(Span::styled(title, t.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 5 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if draft.kind == WriteKind::Create {
            3
        } else {
            0
        }),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let action = if draft.kind == WriteKind::Create {
        "Create in"
    } else {
        "Append to"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "  {action} {}\n  Plain text · OneNote verifies the result",
            safe(&draft.target_name)
        ))
        .style(t.muted()),
        rows[0],
    );
    if draft.kind == WriteKind::Create {
        let mark = if !review && draft.title_focus {
            "▸"
        } else {
            " "
        };
        let title_text = if review {
            draft.title.text().to_owned()
        } else {
            draft.title.display()
        };
        frame.render_widget(
            Paragraph::new(format!("  {mark} TITLE\n  {}", safe(&title_text))).style(
                if draft.title_focus && !review {
                    t.accent()
                } else {
                    t.text()
                },
            ),
            rows[1],
        );
    }
    let mark = if !review && !draft.title_focus {
        "▸"
    } else {
        " "
    };
    let body = if review {
        draft.body.text().to_owned()
    } else {
        draft.body.display()
    };
    let display = body
        .split('\n')
        .map(|line| format!("  {}", safe(line)))
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("  {mark} CONTENT\n{display}");
    let para = Paragraph::new(text)
        .style(if !review && !draft.title_focus {
            t.accent()
        } else {
            t.text()
        })
        .wrap(Wrap { trim: false });
    let scroll = if review {
        let max = para
            .line_count(rows[2].width)
            .saturating_sub(rows[2].height as usize)
            .min(u16::MAX as usize) as u16;
        draft.review_scroll = draft.review_scroll.min(max);
        draft.review_scroll
    } else {
        let prefix = draft
            .body
            .cursor_prefix()
            .split('\n')
            .map(|line| format!("  {}", safe(line)))
            .collect::<Vec<_>>()
            .join("\n");
        let before =
            Paragraph::new(format!("  {mark} CONTENT\n{prefix}")).wrap(Wrap { trim: false });
        before
            .line_count(rows[2].width)
            .saturating_sub(rows[2].height.saturating_sub(1) as usize)
            .min(u16::MAX as usize) as u16
    };
    frame.render_widget(para.scroll((scroll, 0)), rows[2]);
    let hint = if review {
        "  Enter write  ·  Esc edit  ·  ↑/↓ scroll"
    } else {
        "  Ctrl+S review  ·  Tab switch field  ·  Esc keep draft"
    };
    frame.render_widget(Paragraph::new(hint).style(t.accent()), rows[3]);
    if let Some(error) = error {
        frame.render_widget(
            Paragraph::new(format!(
                "  {}",
                fit(&safe(error), inner.width.saturating_sub(3) as usize)
            ))
            .style(t.accent()),
            rows[0],
        );
    }
}
