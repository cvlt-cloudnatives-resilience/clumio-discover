use cd_model::progress::GLOBAL;
use cd_model::{Depth, Inventory, Severity};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, Gauge, HighlightSpacing, Paragraph, Row, Table, TableState,
};
use ratatui::Frame;

use crate::app::{App, Browser, CellState, Detail, Mode, Pane, ScanView, ServiceSummary, Tab};
use crate::logo::{Logo, CLUMIO_MARK, CLUMIO_MARK_LARGE, CLUMIO_MARK_SMALL};
use crate::table::{severity_label, TableData, TableView};
use crate::{format, theme};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::new().style(theme::base()), area);
    match &mut app.mode {
        Mode::Scanning(scan) => draw_scan(frame, area, scan),
        Mode::Browsing(browser) => draw_browser(frame, area, browser),
    }
    if !app.truecolor {
        theme::downsample(frame.buffer_mut());
    }
}

fn panel<'a>(title: Line<'a>, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border(focused))
        .style(theme::panel())
        .title(title)
}

fn muted(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(theme::MUTE))
}

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(theme::DIM))
}

fn brand_title() -> Vec<Span<'static>> {
    vec![
        Span::styled(
            "Clumio Discover",
            Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD),
        ),
        muted(" · read-only"),
    ]
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn stacked_bar(parts: [usize; 3], width: usize) -> Line<'static> {
    let total: usize = parts.iter().sum();
    if width == 0 {
        return Line::default();
    }
    if total == 0 {
        return Line::from(Span::styled(
            "─".repeat(width),
            Style::new().fg(theme::HAIRLINE),
        ));
    }
    let exact: Vec<f64> = parts
        .iter()
        .map(|&p| p as f64 * width as f64 / total as f64)
        .collect();
    let mut cells: Vec<usize> = exact.iter().map(|e| e.floor() as usize).collect();
    let mut order: Vec<usize> = (0..3).collect();
    order.sort_by(|&a, &b| {
        (exact[b] - exact[b].floor())
            .partial_cmp(&(exact[a] - exact[a].floor()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let left = width - cells.iter().sum::<usize>();
    for &i in order.iter().take(left) {
        cells[i] += 1;
    }
    for i in 0..3 {
        if parts[i] > 0 && cells[i] == 0 {
            if let Some(big) = (0..3).filter(|&j| cells[j] > 1).max_by_key(|&j| cells[j]) {
                cells[big] -= 1;
                cells[i] = 1;
            }
        }
    }
    let colors = [theme::OK, theme::WARN, theme::PINK];
    Line::from(
        (0..3)
            .filter(|&i| cells[i] > 0)
            .map(|i| Span::styled("█".repeat(cells[i]), Style::new().fg(colors[i])))
            .collect::<Vec<_>>(),
    )
}

fn draw_browser(frame: &mut Frame, area: Rect, b: &mut Browser) {
    let header_h = match area.height {
        h if h >= 20 => 4,
        h if h >= 8 => 1,
        _ => 0,
    };
    let [header, tabs, body, status] = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header, &b.inventory);
    draw_tabs(frame, tabs, b);
    if let Some(detail) = b.detail.as_mut() {
        draw_detail(frame, body, detail);
    } else {
        let searching = b.searching;
        let pane = &mut b.panes[b.tab];
        match (pane.tab, pane.data.as_ref(), pane.view.as_mut()) {
            (Tab::Overview, _, _) => draw_overview(frame, body, b),
            (Tab::Backup, Some(data), Some(view)) => {
                let vaults = b.inventory.backup_vaults.as_deref().unwrap_or_default();
                let vault_h = (vaults.len() as u16 + 3).clamp(4, 10);
                if body.height >= vault_h + 6 {
                    let [top, bottom] =
                        Layout::vertical([Constraint::Fill(1), Constraint::Length(vault_h)])
                            .areas(body);
                    draw_table(frame, top, data, view, searching);
                    draw_vaults(frame, bottom, &b.inventory);
                } else {
                    draw_table(frame, body, data, view, searching);
                }
            }
            (_, Some(data), Some(view)) => draw_table(frame, body, data, view, searching),
            (tab, _, _) => draw_not_scanned(frame, body, tab),
        }
    }
    draw_status(frame, status, b);
    if b.help {
        draw_help(frame, area, b.can_save);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, inv: &Inventory) {
    if area.height == 0 {
        return;
    }
    let account = &inv.account;
    let mut account_line = vec![
        muted("Account "),
        Span::styled(
            if account.id.is_empty() {
                "unknown".to_string()
            } else {
                account.id.clone()
            },
            Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(alias) = &account.alias {
        account_line.push(Span::styled(
            format!(" ({alias})"),
            Style::new().fg(theme::LAVENDER),
        ));
    }
    account_line.push(muted("  ·  Org "));
    account_line.push(Span::raw(
        account.org_id.clone().unwrap_or_else(|| "none".into()),
    ));

    if area.height < 4 {
        let mut line = brand_title();
        line.push(dim("  │  "));
        line.extend(account_line);
        line.push(muted(format!("  ·  {} regions", inv.regions.len())));
        frame.render_widget(Paragraph::new(Line::from(line)), area);
        return;
    }

    let mark = &CLUMIO_MARK_SMALL;
    let (mark_w, _) = mark.cells();
    let text_area = if area.width >= mark_w + 40 {
        mark.render(
            Rect::new(area.x + 1, area.y, mark_w, area.height),
            frame.buffer_mut(),
            theme::BG,
        );
        Rect::new(
            area.x + mark_w + 3,
            area.y,
            area.width - mark_w - 3,
            area.height,
        )
    } else {
        area
    };

    let regions = Line::from(vec![
        muted(format!("Regions {}: ", inv.regions.len())),
        Span::raw(inv.regions.join(", ")),
    ]);
    let depth = match inv.depth {
        Depth::Standard => "standard",
        Depth::Deep => "deep",
    };
    let mut filters = Vec::new();
    if let Some(tag) = &inv.filters.aws_tag {
        filters.push(format!("tag {}={}", tag.key, tag.value));
    }
    let meta = Line::from(vec![
        muted("Generated "),
        Span::raw(inv.generated_at.clone()),
        muted("  ·  Depth "),
        Span::raw(depth),
        if inv.depth == Depth::Standard {
            Span::styled(
                " (run with --deep to check snapshot sharing)",
                Style::new().fg(theme::WARN),
            )
        } else {
            Span::raw("")
        },
        muted("  ·  Filters "),
        Span::raw(if filters.is_empty() {
            "none".to_string()
        } else {
            filters.join("; ")
        }),
    ]);
    let lines = vec![
        Line::from(brand_title()),
        Line::from(account_line),
        regions,
        meta,
    ];
    frame.render_widget(Paragraph::new(lines), text_area);
}

fn draw_tabs(frame: &mut Frame, area: Rect, b: &Browser) {
    let labels: Vec<(String, Style)> = b
        .panes
        .iter()
        .enumerate()
        .map(|(i, pane)| tab_label(b, i, pane))
        .collect();
    let widths: Vec<usize> = labels.iter().map(|(l, _)| l.chars().count() + 1).collect();
    let mut start = 0;
    while start < b.tab && widths[start..=b.tab].iter().sum::<usize>() + 2 > area.width as usize {
        start += 1;
    }
    let mut spans = vec![Span::raw(" ")];
    if start > 0 {
        spans[0] = dim("‹");
    }
    for (label, style) in labels.into_iter().skip(start) {
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn tab_label(b: &Browser, index: usize, pane: &Pane) -> (String, Style) {
    let scanned = b.scanned(pane);
    let count = match pane.tab {
        Tab::Overview => None,
        _ => b.count(pane),
    };
    let label = match count {
        Some(n) => format!(" {} {n} ", pane.tab.label()),
        None => format!(" {} ", pane.tab.label()),
    };
    let alarm = match pane.tab {
        Tab::Errors => count.unwrap_or(0) > 0,
        Tab::Findings => b
            .inventory
            .findings
            .iter()
            .any(|f| f.severity == Severity::High),
        _ => false,
    };
    let style = if index == b.tab {
        theme::selected()
    } else if !scanned {
        Style::new().fg(theme::DIM)
    } else if alarm {
        Style::new().fg(theme::PINK).bg(theme::SURFACE_0)
    } else {
        Style::new().fg(theme::MUTE).bg(theme::SURFACE_0)
    };
    (label, style)
}

fn draw_not_scanned(frame: &mut Frame, area: Rect, tab: Tab) {
    let block = panel(
        Line::from(Span::styled(format!(" {} ", tab.label()), theme::title())),
        false,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = vec![
        Line::from(Span::styled(
            "Not scanned",
            Style::new().fg(theme::MUTE).add_modifier(Modifier::BOLD),
        )),
        Line::from(dim(
            "Turned off in the config, or its list call failed in every region.",
        )),
    ];
    frame.render_widget(
        Paragraph::new(text).centered(),
        centered(inner, inner.width, 2),
    );
}

fn draw_table(
    frame: &mut Frame,
    area: Rect,
    data: &TableData,
    view: &mut TableView,
    searching: bool,
) {
    let mut title = vec![
        Span::styled(format!(" {} ", data.title), theme::title()),
        muted(format!("{} of {} ", view.visible.len(), data.records.len())),
    ];
    if searching || !view.query.is_empty() {
        title.push(Span::styled(
            format!(" /{}{} ", view.query, if searching { "▏" } else { "" }),
            Style::new().fg(theme::LAVENDER),
        ));
    }
    if let Some(label) = view.facet_label(data) {
        title.push(Span::styled(
            format!(" filter: {label} "),
            Style::new().fg(theme::CROCUS_BRIGHT),
        ));
    }
    let block = panel(Line::from(title), true);
    let inner = block.inner(area);
    view.page = inner.height.saturating_sub(1).max(1) as usize;

    let header = Row::new(data.columns.iter().enumerate().map(|(i, c)| {
        let mut title = c.title.to_string();
        if i == view.sort {
            title.push(if view.reverse { '▼' } else { '▲' });
        }
        Cell::from(title)
    }))
    .style(theme::header());
    let rows = view.visible.iter().map(|&i| {
        Row::new(data.records[i].cells.iter().map(|(text, color)| {
            let cell = Cell::from(text.as_str());
            match color {
                Some(c) => cell.style(Style::new().fg(*c)),
                None => cell,
            }
        }))
    });
    let table = Table::new(rows, data.columns.iter().map(|c| c.width))
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌")
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut view.state);

    if view.visible.is_empty() && inner.height > 2 {
        let message = if data.records.is_empty() {
            "Nothing found"
        } else {
            "No rows match; Esc clears search and filter"
        };
        frame.render_widget(
            Paragraph::new(dim(message)).centered(),
            centered(inner, inner.width, 1),
        );
    }
}

fn draw_vaults(frame: &mut Frame, area: Rect, inv: &Inventory) {
    let vaults = inv.backup_vaults.as_deref().unwrap_or_default();
    let block = panel(
        Line::from(vec![
            Span::styled(" Backup vaults ", theme::title()),
            muted(format!("{} ", vaults.len())),
        ]),
        false,
    );
    let rows = crate::table::vault_rows(vaults)
        .into_iter()
        .map(|[name, region, points, lock]| {
            let lock_style = Style::new().fg(if lock == "locked" {
                theme::OK
            } else {
                theme::MUTE
            });
            Row::new(vec![
                Cell::from(name),
                Cell::from(region).style(Style::new().fg(theme::MUTE)),
                Cell::from(points),
                Cell::from(lock).style(lock_style),
            ])
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(40),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["Vault", "Region", "Recovery points", "Lock"]).style(theme::header()))
    .block(block)
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn wrap(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if width < 4 || total <= width {
        return vec![line.clone()];
    }
    let gutter = line.spans.first().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in &line.spans {
        let mut rest: &str = &span.content;
        while !rest.is_empty() {
            if used == width {
                out.push(Line::from(std::mem::take(&mut current)).style(line.style));
                current.push(gutter.clone());
                current.push(Span::raw("  "));
                used = gutter.content.chars().count() + 2;
            }
            let take = (width - used).min(rest.chars().count());
            let split = rest.char_indices().nth(take).map_or(rest.len(), |(i, _)| i);
            current.push(Span::styled(rest[..split].to_string(), span.style));
            used += take;
            rest = &rest[split..];
        }
    }
    if !current.is_empty() {
        out.push(Line::from(current).style(line.style));
    }
    out
}

fn draw_detail(frame: &mut Frame, area: Rect, d: &mut Detail) {
    let block = panel(
        Line::from(vec![
            Span::styled(format!(" {} ", d.title), theme::title()),
            muted("detail "),
        ]),
        true,
    )
    .title_bottom(Line::from(vec![dim(" Esc back · ↑↓ PgUp PgDn scroll ")]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Line<'static>> = d
        .lines
        .iter()
        .flat_map(|l| wrap(l, inner.width as usize))
        .collect();
    let height = inner.height as usize;
    d.page = height.max(1);
    d.scroll = d.scroll.min(rows.len().saturating_sub(height));
    let visible: Vec<Line> = rows.into_iter().skip(d.scroll).take(height).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn draw_overview(frame: &mut Frame, area: Rect, b: &Browser) {
    let services_h = b.overview.services.len() as u16 + 4;
    let [top, bottom] =
        Layout::vertical([Constraint::Length(services_h), Constraint::Fill(1)]).areas(area);
    draw_services(frame, top, b);
    if bottom.height < 4 {
        return;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(bottom);
    draw_regions(frame, left, b);
    draw_findings_summary(frame, right, &b.inventory);
}

fn count_cell(n: usize, total: usize, color: ratatui::style::Color) -> Cell<'static> {
    if total == 0 {
        return Cell::from(dim("-"));
    }
    Cell::from(Line::from(vec![
        Span::styled(format!("{n:>4} "), Style::new().fg(color)),
        dim(format!("{:>4}", format::pct(n, total))),
    ]))
}

fn summary_row(s: &ServiceSummary, label: String, bar_w: usize, bold: bool) -> Row<'static> {
    let name_style = if bold {
        theme::title()
    } else if s.scanned {
        Style::new().fg(theme::TEXT)
    } else {
        Style::new().fg(theme::DIM)
    };
    if !s.scanned {
        return Row::new(vec![
            Cell::from(label).style(name_style),
            Cell::from(dim(format!("{:>6}", "-"))),
            Cell::from(dim(format!("{:>10}", "-"))),
            Cell::from(dim("not scanned")),
        ]);
    }
    let [aws, native, none] = s.by_status;
    Row::new(vec![
        Cell::from(label).style(name_style),
        Cell::from(format!("{:>6}", s.count)),
        Cell::from(format!("{:>10}", format::opt_bytes(s.bytes))),
        Cell::from(stacked_bar(s.by_status, bar_w)),
        count_cell(aws, s.count, theme::OK),
        count_cell(native, s.count, theme::WARN),
        count_cell(none, s.count, theme::PINK),
    ])
    .style(if bold {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    })
}

fn draw_services(frame: &mut Frame, area: Rect, b: &Browser) {
    let inv = &b.inventory;
    let context = format!(
        " Backup vaults {} · Recycle Bin rules {} · Trails {} · Errors {} ",
        inv.backup_vaults.as_ref().map_or(0, Vec::len),
        inv.rbin_rules.as_ref().map_or(0, Vec::len),
        inv.trails.as_ref().map_or(0, Vec::len),
        inv.errors.len(),
    );
    let block = panel(
        Line::from(Span::styled(" Protection by service ", theme::title())),
        true,
    )
    .title_bottom(Line::from(dim(context)).right_aligned());
    let inner = block.inner(area);

    let fixed = [14u16, 6, 10, 11, 11, 11];
    let spacing = fixed.len() as u16;
    let bar_w = inner
        .width
        .saturating_sub(fixed.iter().sum::<u16>() + spacing + 1)
        .min(48);
    let widths = [
        Constraint::Length(fixed[0]),
        Constraint::Length(fixed[1]),
        Constraint::Length(fixed[2]),
        Constraint::Length(bar_w),
        Constraint::Length(fixed[3]),
        Constraint::Length(fixed[4]),
        Constraint::Length(fixed[5]),
    ];
    let header = Row::new(vec![
        Cell::from("Service"),
        Cell::from("Assets"),
        Cell::from(format!("{:>10}", "Size")),
        Cell::from("Protection"),
        Cell::from(Span::styled("AWS Backup", Style::new().fg(theme::OK))),
        Cell::from(Span::styled("Native only", Style::new().fg(theme::WARN))),
        Cell::from(Span::styled("Unprotected", Style::new().fg(theme::PINK))),
    ])
    .style(theme::header());

    let mut rows: Vec<Row> = b
        .overview
        .services
        .iter()
        .map(|s| summary_row(s, s.service.label().to_string(), bar_w as usize, false))
        .collect();
    rows.push(summary_row(
        &b.overview.total(),
        "Total".into(),
        bar_w as usize,
        true,
    ));
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(theme::selected_soft())
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme::CROCUS_BRIGHT)))
        .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default().with_selected(Some(b.overview.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_regions(frame: &mut Frame, area: Rect, b: &Browser) {
    let block = panel(
        Line::from(Span::styled(" By region ", theme::title())),
        false,
    );
    let inner = block.inner(area);
    let fixed = [16u16, 6, 10];
    let bar_w = inner
        .width
        .saturating_sub(fixed.iter().sum::<u16>() + 3)
        .min(48);
    let rows = b.overview.regions.iter().map(|r| {
        Row::new(vec![
            Cell::from(r.region.clone()),
            Cell::from(format!("{:>6}", r.count)),
            Cell::from(format!("{:>10}", format::opt_bytes(r.bytes))),
            Cell::from(stacked_bar(r.by_status, bar_w as usize)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(fixed[0]),
            Constraint::Length(fixed[1]),
            Constraint::Length(fixed[2]),
            Constraint::Length(bar_w),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("Region"),
            Cell::from("Assets"),
            Cell::from(format!("{:>10}", "Size")),
            Cell::from("Protection"),
        ])
        .style(theme::header()),
    )
    .block(block)
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn draw_findings_summary(frame: &mut Frame, area: Rect, inv: &Inventory) {
    let block = panel(
        Line::from(vec![
            Span::styled(" Findings ", theme::title()),
            muted(format!("{} ", inv.findings.len())),
        ]),
        false,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut counts = vec![Span::raw(" ")];
    for severity in [
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ] {
        let n = inv
            .findings
            .iter()
            .filter(|f| f.severity == severity)
            .count();
        counts.push(Span::styled(
            format!("{} {n}", severity_label(severity)),
            Style::new()
                .fg(theme::severity(severity))
                .add_modifier(Modifier::BOLD),
        ));
        counts.push(Span::raw("   "));
    }
    let mut lines = vec![Line::from(counts), Line::default()];
    if inv.findings.is_empty() {
        lines.push(Line::from(Span::styled(
            " No findings",
            Style::new().fg(theme::OK),
        )));
    }
    let mut top: Vec<_> = inv.findings.iter().collect();
    top.sort_by_key(|f| f.severity);
    let room = (inner.height as usize).saturating_sub(lines.len());
    for f in top.into_iter().take(room) {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<7}", severity_label(f.severity)),
                Style::new().fg(theme::severity(f.severity)),
            ),
            muted(format!("{:<11}", f.service.label())),
            Span::raw(f.name.clone()),
            dim(format!("  {}", f.message)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn hint(key: &str, label: &str) -> [Span<'static>; 2] {
    [
        Span::styled(format!(" {key}"), theme::key()),
        dim(format!(" {label} ")),
    ]
}

fn draw_status(frame: &mut Frame, area: Rect, b: &Browser) {
    let line = if let Some(toast) = &b.toast {
        Line::from(Span::styled(
            format!(" {}", toast.text),
            Style::new()
                .fg(if toast.error { theme::PINK } else { theme::OK })
                .add_modifier(Modifier::BOLD),
        ))
    } else if b.searching {
        Line::from(vec![
            Span::styled(" Search ", theme::key()),
            dim("name, ARN, tags · Enter keep · Esc clear"),
        ])
    } else {
        let keys: &[(&str, &str)] = if b.detail.is_some() {
            &[("Esc", "back"), ("↑↓", "scroll"), ("q", "quit")]
        } else if b.current().tab == Tab::Overview {
            &[
                ("↑↓", "select"),
                ("Enter", "open"),
                ("Tab", "next tab"),
                ("e", "export"),
                ("?", "help"),
                ("q", "quit"),
            ]
        } else {
            &[
                ("↑↓", "move"),
                ("/", "search"),
                ("f", "filter"),
                ("s/S", "sort"),
                ("Enter", "detail"),
                ("e", "export"),
                ("?", "help"),
                ("q", "quit"),
            ]
        };
        let mut spans: Vec<Span> = keys.iter().flat_map(|(k, l)| hint(k, l)).collect();
        if b.can_save && b.detail.is_none() {
            spans.extend(hint("w", "save"));
        }
        Line::from(spans)
    };
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(theme::SURFACE_0)),
        area,
    );
}

fn draw_help(frame: &mut Frame, area: Rect, can_save: bool) {
    let mut keys = vec![
        ("Tab ⇧Tab ← →", "switch tab (or 1-9, 0)"),
        ("↑ ↓  j k", "move"),
        ("PgUp PgDn", "page"),
        ("g G", "first, last"),
        ("/", "search name, ARN, tags"),
        ("f", "cycle filter: status, region, severity"),
        ("s  S", "next sort column, reverse"),
        ("Enter", "detail (raw JSON); on Overview, open service"),
        ("Esc", "back; clears search and filter"),
        ("e", "export this view to CSV and JSON here"),
    ];
    if can_save {
        keys.push(("w", "save the full inventory JSON here"));
    }
    keys.extend([("?", "this help"), ("q", "quit")]);

    let popup = centered(area, 64, keys.len() as u16 + 4);
    frame.render_widget(Clear, popup);
    let block = panel(Line::from(Span::styled(" Keys ", theme::title())), true)
        .title_bottom(Line::from(dim(" any key closes ")).right_aligned());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let mut lines = vec![Line::default()];
    lines.extend(keys.into_iter().map(|(k, d)| {
        Line::from(vec![
            Span::styled(format!("  {k:<14}"), theme::key()),
            Span::raw(d),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), inner);
}

const SCAN_MIN_ROWS: u16 = 12;

fn splash_mark(area: Rect) -> Option<&'static Logo> {
    let title_w = Line::from(brand_title()).width() as u16;
    [&CLUMIO_MARK_LARGE, &CLUMIO_MARK].into_iter().find(|mark| {
        let (w, h) = mark.cells();
        area.width >= w + 3 + title_w + 2 && area.height >= h + 1 + SCAN_MIN_ROWS
    })
}

fn draw_splash(frame: &mut Frame, area: Rect, mark: Option<&Logo>) {
    if area.height == 0 {
        return;
    }
    let title = Line::from(brand_title());
    let Some(mark) = mark else {
        frame.render_widget(
            Paragraph::new(title).centered(),
            Rect::new(area.x, area.y, area.width, 1),
        );
        return;
    };
    let (mark_w, mark_h) = mark.cells();
    let title_w = title.width() as u16;
    let x = area.x + (area.width - mark_w - 3 - title_w) / 2;
    mark.render(
        Rect::new(x, area.y, mark_w, mark_h),
        frame.buffer_mut(),
        theme::BG,
    );
    frame.render_widget(
        Paragraph::new(title),
        Rect::new(x + mark_w + 3, area.y + mark_h / 2, title_w, 1),
    );
}

fn draw_scan(frame: &mut Frame, area: Rect, s: &ScanView) {
    let mark = splash_mark(area);
    let splash_h = mark.map_or(1, |m| m.cells().1 + 1);
    let [splash, info, gauge, grid, status] = Layout::vertical([
        Constraint::Length(splash_h),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_splash(frame, splash, mark);

    let spinner = SPINNER[s.tick % SPINNER.len()];
    let (done, expected, errors) = (s.done(), s.expected(), s.errors());
    let mut line = vec![
        Span::styled(format!(" {spinner} "), Style::new().fg(theme::LAVENDER)),
        Span::styled("Scanning", theme::title()),
        muted(format!("  ·  {done}/{expected} sections")),
        muted(format!("  ·  {} assets", s.found())),
        muted(format!("  ·  {} s", s.started.elapsed().as_secs())),
    ];
    if errors > 0 {
        line.push(Span::styled(
            format!("  ·  {errors} errors"),
            Style::new().fg(theme::PINK),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(line)), info);

    let ratio = if expected == 0 {
        0.0
    } else {
        (done as f64 / expected as f64).min(1.0)
    };
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(theme::CROCUS).bg(theme::SURFACE_2))
            .ratio(ratio)
            .label(Span::styled(
                format!("{:.0}%", ratio * 100.0),
                Style::new()
                    .fg(theme::ON_ACCENT)
                    .add_modifier(Modifier::BOLD),
            )),
        Rect::new(
            gauge.x + 1,
            gauge.y,
            gauge.width.saturating_sub(2),
            gauge.height,
        ),
    );

    let services: Vec<_> = s.services.iter().copied().collect();
    let mut widths = vec![Constraint::Length(16)];
    widths.extend(
        services
            .iter()
            .map(|svc| Constraint::Length(svc.label().len().max(7) as u16)),
    );
    let header = Row::new(
        std::iter::once(Cell::from("Region"))
            .chain(services.iter().map(|svc| Cell::from(svc.label()))),
    )
    .style(theme::header());
    let rows = s.regions.iter().map(|region| {
        let label = if region == GLOBAL {
            Cell::from(muted("account-wide"))
        } else {
            Cell::from(region.clone())
        };
        let cells = services.iter().map(|&svc| match s.cell(region, svc) {
            None => Cell::from(dim("·")),
            Some(CellState::Running) => Cell::from(Span::styled(
                spinner.to_string(),
                Style::new().fg(theme::LAVENDER),
            )),
            Some(CellState::Done { count, errors }) => {
                let mut spans = vec![match count {
                    Some(0) => dim("0"),
                    Some(n) => Span::raw(n.to_string()),
                    None => dim("-"),
                }];
                if errors > 0 {
                    spans.push(Span::styled(
                        format!(" ✗{errors}"),
                        Style::new().fg(theme::PINK),
                    ));
                }
                Cell::from(Line::from(spans))
            }
        });
        Row::new(std::iter::once(label).chain(cells))
    });
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(panel(
            Line::from(Span::styled(" Regions × services ", theme::title())),
            true,
        ));
    frame.render_widget(table, grid);

    let hints: Vec<Span> = hint("q", "abort")
        .into_iter()
        .chain([dim(" read-only: Describe, List and Get calls only")])
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(hints)).style(Style::new().bg(theme::SURFACE_0)),
        status,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(line: &Line) -> Vec<(usize, ratatui::style::Color)> {
        line.spans
            .iter()
            .map(|s| (s.content.chars().count(), s.style.fg.unwrap()))
            .collect()
    }

    #[test]
    fn stacked_bar_fills_width_and_keeps_small_shares() {
        assert_eq!(
            widths(&stacked_bar([1, 1, 2], 8)),
            vec![(2, theme::OK), (2, theme::WARN), (4, theme::PINK)]
        );
        assert_eq!(
            widths(&stacked_bar([100, 0, 1], 10)),
            vec![(9, theme::OK), (1, theme::PINK)]
        );
        assert_eq!(
            widths(&stacked_bar([1, 1, 1], 10))
                .iter()
                .map(|w| w.0)
                .sum::<usize>(),
            10
        );
        assert_eq!(stacked_bar([0, 0, 0], 5).spans[0].content, "─────");
        assert!(stacked_bar([1, 2, 3], 0).spans.is_empty());
    }

    #[test]
    fn wraps_long_detail_lines_with_gutter() {
        let line = Line::from(vec![Span::raw("▌"), Span::raw("x".repeat(25))]);
        let rows = wrap(&line, 10);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.width() <= 10));
        assert_eq!(rows[1].spans[0].content, "▌");
    }
}
