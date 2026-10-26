use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use cd_model::progress::GLOBAL;
use cd_model::{Inventory, Progress, ProtectionStatus, Service};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use serde_json::{json, Value};

use crate::table::{self, TableData, TableView};
use crate::{detail, export, format};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Assets(Service),
    Backup,
    Section(usize),
    Findings,
    Errors,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Assets(service) => service.label(),
            Tab::Backup => "AWS Backup",
            Tab::Section(i) => table::SECTIONS[i].label,
            Tab::Findings => "Findings",
            Tab::Errors => "Errors",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct Detail {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub scroll: usize,
    pub page: usize,
}

pub struct ServiceSummary {
    pub service: Service,
    pub scanned: bool,
    pub count: usize,
    pub bytes: Option<u64>,
    pub by_status: [usize; 2],
}

pub struct RegionSummary {
    pub region: String,
    pub count: usize,
    pub bytes: Option<u64>,
    pub by_status: [usize; 2],
}

pub fn status_index(status: ProtectionStatus) -> usize {
    match status {
        ProtectionStatus::Protected => 0,
        ProtectionStatus::Unprotected => 1,
    }
}

fn add_bytes(total: &mut Option<u64>, n: Option<u64>) {
    if let Some(n) = n {
        *total = Some(total.unwrap_or(0) + n);
    }
}

pub struct Overview {
    pub services: Vec<ServiceSummary>,
    pub regions: Vec<RegionSummary>,
    pub selected: usize,
}

impl Overview {
    fn new(inventory: &Inventory) -> Self {
        let mut services = Vec::new();
        let mut regions: BTreeMap<String, RegionSummary> = BTreeMap::new();
        for service in Service::ASSETS {
            let rows = inventory.rows(service);
            let mut summary = ServiceSummary {
                service,
                scanned: rows.is_some(),
                count: 0,
                bytes: None,
                by_status: [0; 2],
            };
            for row in rows.iter().flatten() {
                summary.count += 1;
                add_bytes(&mut summary.bytes, row.size_bytes);
                summary.by_status[status_index(row.status)] += 1;
                let region = regions
                    .entry(row.region.clone())
                    .or_insert_with(|| RegionSummary {
                        region: row.region.clone(),
                        count: 0,
                        bytes: None,
                        by_status: [0; 2],
                    });
                region.count += 1;
                add_bytes(&mut region.bytes, row.size_bytes);
                region.by_status[status_index(row.status)] += 1;
            }
            services.push(summary);
        }
        Self {
            services,
            regions: regions.into_values().collect(),
            selected: 0,
        }
    }

    pub fn total(&self) -> ServiceSummary {
        let mut total = ServiceSummary {
            service: Service::Account,
            scanned: true,
            count: 0,
            bytes: None,
            by_status: [0; 2],
        };
        for s in &self.services {
            total.count += s.count;
            add_bytes(&mut total.bytes, s.bytes);
            for (sum, n) in total.by_status.iter_mut().zip(s.by_status) {
                *sum += n;
            }
        }
        total
    }
}

pub struct Pane {
    pub tab: Tab,
    pub data: Option<TableData>,
    pub view: Option<TableView>,
}

pub struct Browser {
    pub inventory: Inventory,
    pub panes: Vec<Pane>,
    pub tab: usize,
    pub overview: Overview,
    pub detail: Option<Detail>,
    pub searching: bool,
    pub help: bool,
    pub toast: Option<Toast>,
    pub can_save: bool,
    pub out_dir: PathBuf,
}

impl Browser {
    pub fn new(inventory: Inventory, can_save: bool) -> Self {
        let mut tabs = vec![Tab::Overview];
        tabs.extend(Service::ASSETS.map(Tab::Assets));
        tabs.push(Tab::Backup);
        tabs.extend(
            table::SECTIONS
                .iter()
                .enumerate()
                .map(|(i, _)| Tab::Section(i)),
        );
        tabs.extend([Tab::Findings, Tab::Errors]);
        let raw = if table::SECTIONS.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(&inventory).unwrap_or(Value::Null)
        };
        let panes = tabs
            .into_iter()
            .map(|tab| {
                let data = match tab {
                    Tab::Overview => None,
                    Tab::Assets(service) => inventory
                        .rows(service)
                        .map(|rows| table::assets(service, rows)),
                    Tab::Backup => inventory.aws_backup.as_deref().map(table::aws_backup),
                    Tab::Section(i) => {
                        let section = &table::SECTIONS[i];
                        raw.get(section.field)
                            .and_then(Value::as_array)
                            .map(|items| table::json_table(section, items))
                    }
                    Tab::Findings => Some(table::findings(&inventory.findings)),
                    Tab::Errors => Some(table::errors(&inventory.errors)),
                };
                let view = data.as_ref().map(TableView::new);
                Pane { tab, data, view }
            })
            .collect();
        Self {
            overview: Overview::new(&inventory),
            inventory,
            panes,
            tab: 0,
            detail: None,
            searching: false,
            help: false,
            toast: None,
            can_save,
            out_dir: PathBuf::from("."),
        }
    }

    pub fn current(&self) -> &Pane {
        &self.panes[self.tab]
    }

    pub fn scanned(&self, pane: &Pane) -> bool {
        match pane.tab {
            Tab::Overview => true,
            _ => pane.data.is_some(),
        }
    }

    pub fn count(&self, pane: &Pane) -> Option<usize> {
        pane.data.as_ref().map(|d| d.records.len())
    }

    fn select_tab(&mut self, index: usize) {
        if index < self.panes.len() {
            self.tab = index;
            self.detail = None;
            self.searching = false;
        }
    }

    fn step_tab(&mut self, forward: bool) {
        let n = self.panes.len();
        let next = if forward {
            (self.tab + 1) % n
        } else {
            (self.tab + n - 1) % n
        };
        self.select_tab(next);
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    pub fn key(&mut self, key: KeyEvent) -> Flow {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Flow::Quit;
        }
        if self.help {
            self.help = false;
            return Flow::Continue;
        }
        if self.searching {
            self.search_key(key);
            return Flow::Continue;
        }
        if self.detail.is_some() {
            return self.detail_key(key);
        }
        match key.code {
            KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.step_tab(true),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.step_tab(false),
            KeyCode::Char(c @ '1'..='9') => self.select_tab(c as usize - '1' as usize),
            KeyCode::Char('0') => self.select_tab(9),
            KeyCode::Char('e') => self.export(),
            KeyCode::Char('w') if self.can_save => self.save_inventory(),
            _ if self.current().tab == Tab::Overview => self.overview_key(key),
            _ => self.table_key(key),
        }
        Flow::Continue
    }

    fn overview_key(&mut self, key: KeyEvent) {
        let n = self.overview.services.len();
        let o = &mut self.overview;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => o.selected = (o.selected + 1).min(n - 1),
            KeyCode::Up | KeyCode::Char('k') => o.selected = o.selected.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => o.selected = 0,
            KeyCode::Char('G') | KeyCode::End => o.selected = n - 1,
            KeyCode::Enter => {
                let service = o.services[o.selected].service;
                if let Some(i) = self
                    .panes
                    .iter()
                    .position(|p| p.tab == Tab::Assets(service))
                {
                    self.select_tab(i);
                }
            }
            _ => {}
        }
    }

    fn table_key(&mut self, key: KeyEvent) {
        let pane = &mut self.panes[self.tab];
        let (Some(data), Some(view)) = (pane.data.as_ref(), pane.view.as_mut()) else {
            return;
        };
        let page = view.page.max(1) as isize;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => view.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => view.move_by(-1),
            KeyCode::PageDown => view.move_by(page),
            KeyCode::PageUp => view.move_by(-page),
            KeyCode::Char('g') | KeyCode::Home => view.home(),
            KeyCode::Char('G') | KeyCode::End => view.end(),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('f') => view.cycle_facet(data),
            KeyCode::Char('s') => view.cycle_sort(data),
            KeyCode::Char('S') => view.reverse(data),
            KeyCode::Esc => {
                view.query.clear();
                view.facet = None;
                view.refresh(data);
            }
            KeyCode::Enter => {
                if let Some(record) = view.selected_record(data) {
                    let title = record.cells[0].0.clone();
                    let value = data.detail(record);
                    self.detail = Some(Detail {
                        title,
                        lines: detail::json_lines(&value),
                        scroll: 0,
                        page: 10,
                    });
                }
            }
            _ => {}
        }
    }

    fn search_key(&mut self, key: KeyEvent) {
        let pane = &mut self.panes[self.tab];
        let (Some(data), Some(view)) = (pane.data.as_ref(), pane.view.as_mut()) else {
            self.searching = false;
            return;
        };
        match key.code {
            KeyCode::Enter => self.searching = false,
            KeyCode::Esc => {
                self.searching = false;
                view.query.clear();
            }
            KeyCode::Backspace => {
                view.query.pop();
            }
            KeyCode::Down => view.move_by(1),
            KeyCode::Up => view.move_by(-1),
            KeyCode::Char(c) => view.query.push(c),
            _ => return,
        }
        view.refresh(data);
    }

    fn detail_key(&mut self, key: KeyEvent) -> Flow {
        let Some(detail) = self.detail.as_mut() else {
            return Flow::Continue;
        };
        let max = detail.lines.len().saturating_sub(1);
        let page = detail.page.max(1);
        match key.code {
            KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace => self.detail = None,
            KeyCode::Down | KeyCode::Char('j') => detail.scroll = (detail.scroll + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => detail.scroll = detail.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                detail.scroll = (detail.scroll + page).min(max)
            }
            KeyCode::PageUp => detail.scroll = detail.scroll.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => detail.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => detail.scroll = max,
            _ => {}
        }
        Flow::Continue
    }

    fn export(&mut self) {
        let stamp = format::file_stamp(SystemTime::now());
        let pane = &self.panes[self.tab];
        let result = match (pane.tab, &pane.data, &pane.view) {
            (Tab::Overview, _, _) => {
                let (rows, json) = self.overview_export();
                export::write_view(
                    &self.out_dir,
                    "overview",
                    &stamp,
                    &[
                        "service",
                        "scanned",
                        "assets",
                        "size_bytes",
                        "protected",
                        "unprotected",
                    ],
                    &rows,
                    json,
                )
            }
            (_, Some(data), Some(view)) => {
                let records: Vec<_> = view.visible.iter().map(|&i| &data.records[i]).collect();
                let rows: Vec<Vec<String>> = records.iter().map(|r| r.csv.clone()).collect();
                let json = records.iter().map(|r| r.json.clone()).collect();
                export::write_view(
                    &self.out_dir,
                    &data.stem,
                    &stamp,
                    &data.csv_header,
                    &rows,
                    json,
                )
            }
            _ => {
                self.toast("Nothing to export: not scanned", true);
                return;
            }
        };
        match result {
            Ok((csv, json)) => self.toast(
                format!("Exported {} and {}", csv.display(), json.display()),
                false,
            ),
            Err(e) => self.toast(format!("Export failed: {e}"), true),
        }
    }

    fn overview_export(&self) -> (Vec<Vec<String>>, Vec<Value>) {
        self.overview
            .services
            .iter()
            .map(|s| {
                let [protected, none] = s.by_status;
                (
                    vec![
                        s.service.as_str().to_string(),
                        s.scanned.to_string(),
                        s.count.to_string(),
                        s.bytes.map(|b| b.to_string()).unwrap_or_default(),
                        protected.to_string(),
                        none.to_string(),
                    ],
                    json!({
                        "service": s.service,
                        "scanned": s.scanned,
                        "assets": s.count,
                        "size_bytes": s.bytes,
                        "protected": protected,
                        "unprotected": none,
                    }),
                )
            })
            .unzip()
    }

    fn save_inventory(&mut self) {
        match self.write_inventory() {
            Ok(path) => self.toast(format!("Saved {}", path.display()), false),
            Err(e) => self.toast(format!("Save failed: {e}"), true),
        }
    }

    fn write_inventory(&self) -> std::io::Result<PathBuf> {
        let stamp = format::file_stamp(SystemTime::now());
        let account = if self.inventory.account.id.is_empty() {
            "unknown"
        } else {
            &self.inventory.account.id
        };
        let path = self
            .out_dir
            .join(format!("inventory-{account}-{stamp}.json"));
        let body = serde_json::to_string_pretty(&self.inventory).map_err(std::io::Error::other)?;
        std::fs::write(&path, format!("{body}\n"))?;
        Ok(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    Running,
    Done { count: Option<usize>, errors: usize },
}

pub struct ScanView {
    pub regions: Vec<String>,
    pub services: BTreeSet<Service>,
    pub cells: HashMap<(String, Service), CellState>,
    pub started: Instant,
    pub tick: usize,
}

impl ScanView {
    pub fn new() -> Self {
        Self {
            regions: vec![GLOBAL.to_string()],
            services: BTreeSet::new(),
            cells: HashMap::new(),
            started: Instant::now(),
            tick: 0,
        }
    }

    pub fn progress(&mut self, event: Progress) {
        match event {
            Progress::Regions(regions) => {
                for region in regions {
                    if !self.regions.contains(&region) {
                        self.regions.push(region);
                    }
                }
            }
            Progress::Started { region, service } => {
                self.see(&region, service);
                self.cells.insert((region, service), CellState::Running);
            }
            Progress::Finished {
                region,
                service,
                count,
                errors,
            } => {
                self.see(&region, service);
                self.cells
                    .insert((region, service), CellState::Done { count, errors });
            }
        }
    }

    fn see(&mut self, region: &str, service: Service) {
        if !self.regions.iter().any(|r| r == region) {
            self.regions.push(region.to_string());
        }
        self.services.insert(service);
    }

    pub fn cell(&self, region: &str, service: Service) -> Option<CellState> {
        self.cells.get(&(region.to_string(), service)).copied()
    }

    pub fn expected(&self) -> usize {
        let regional: BTreeSet<Service> = self
            .cells
            .keys()
            .filter(|(r, _)| r != GLOBAL)
            .map(|(_, s)| *s)
            .collect();
        let global = self.cells.keys().filter(|(r, _)| r == GLOBAL).count();
        let regions = self.regions.iter().filter(|r| *r != GLOBAL).count();
        (global + regions * regional.len()).max(self.cells.len())
    }

    pub fn done(&self) -> usize {
        self.cells
            .values()
            .filter(|c| matches!(c, CellState::Done { .. }))
            .count()
    }

    pub fn found(&self) -> usize {
        self.cells
            .iter()
            .filter(|((_, s), _)| !matches!(s, Service::Account | Service::Cloudtrail))
            .filter_map(|(_, c)| match c {
                CellState::Done { count, .. } => *count,
                CellState::Running => None,
            })
            .sum()
    }

    pub fn errors(&self) -> usize {
        self.cells
            .values()
            .map(|c| match c {
                CellState::Done { errors, .. } => *errors,
                CellState::Running => 0,
            })
            .sum()
    }
}

pub enum Mode {
    Scanning(ScanView),
    Browsing(Box<Browser>),
}

pub struct App {
    pub mode: Mode,
    pub truecolor: bool,
    pub auto_save: bool,
    pub out_dir: PathBuf,
    pub saved: Option<PathBuf>,
}

impl App {
    pub fn browse(inventory: Inventory, can_save: bool) -> Self {
        Self {
            mode: Mode::Browsing(Box::new(Browser::new(inventory, can_save))),
            truecolor: true,
            auto_save: false,
            out_dir: PathBuf::from("."),
            saved: None,
        }
    }

    pub fn scanning() -> Self {
        Self {
            mode: Mode::Scanning(ScanView::new()),
            truecolor: true,
            auto_save: false,
            out_dir: PathBuf::from("."),
            saved: None,
        }
    }

    pub fn progress(&mut self, event: Progress) {
        if let Mode::Scanning(scan) = &mut self.mode {
            scan.progress(event);
        }
    }

    pub fn finished(&mut self, inventory: Inventory) {
        let mut browser = Browser::new(inventory, !self.auto_save);
        browser.out_dir = self.out_dir.clone();
        let total = browser.overview.total().count;
        let findings = browser.inventory.findings.len();
        let done = format!("Scan complete: {total} assets, {findings} findings.");
        if !self.auto_save {
            browser.toast(format!("{done} Press w to save."), false);
        } else {
            match browser.write_inventory() {
                Ok(path) => {
                    browser.toast(format!("{done} Saved {}", path.display()), false);
                    self.saved = Some(path);
                }
                Err(e) => {
                    browser.can_save = true;
                    browser.toast(format!("{done} Save failed: {e}. Press w to retry."), true);
                }
            }
        }
        self.mode = Mode::Browsing(Box::new(browser));
    }

    pub fn tick(&mut self) {
        match &mut self.mode {
            Mode::Scanning(scan) => scan.tick = scan.tick.wrapping_add(1),
            Mode::Browsing(browser) => {
                if browser
                    .toast
                    .as_ref()
                    .is_some_and(|t| t.at.elapsed().as_secs() >= 6)
                {
                    browser.toast = None;
                }
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Flow {
        match &mut self.mode {
            Mode::Scanning(_) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Flow::Quit,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Flow::Quit,
                _ => Flow::Continue,
            },
            Mode::Browsing(browser) => browser.key(key),
        }
    }

    pub fn into_inventory(self) -> Option<Inventory> {
        match self.mode {
            Mode::Scanning(_) => None,
            Mode::Browsing(browser) => Some(browser.inventory),
        }
    }
}
