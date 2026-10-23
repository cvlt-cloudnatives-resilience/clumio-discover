use std::cmp::Ordering;

use cd_model::{
    AssetRow, BackupVault, DiscoveryError, Finding, ProtectedResource, ProtectionStatus, Service,
    Severity,
};
use ratatui::layout::Constraint;
use ratatui::style::Color;
use ratatui::widgets::TableState;
use serde_json::Value;

use crate::{format, theme};

pub struct Column {
    pub title: &'static str,
    pub width: Constraint,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Key {
    Missing,
    Num(f64),
    Text(String),
}

pub struct Record {
    pub cells: Vec<(String, Option<Color>)>,
    pub sort: Vec<Key>,
    pub haystack: String,
    pub facets: Vec<String>,
    pub csv: Vec<String>,
    pub json: Value,
}

pub struct Facet {
    pub key: String,
    pub label: String,
}

pub struct TableData {
    pub stem: String,
    pub title: String,
    pub columns: Vec<Column>,
    pub csv_header: Vec<&'static str>,
    pub records: Vec<Record>,
    pub facets: Vec<Facet>,
    pub detail_field: Option<&'static str>,
    pub default_sort: usize,
}

impl TableData {
    pub fn detail(&self, record: &Record) -> Value {
        match self.detail_field {
            Some(field) => record.json.get(field).cloned().unwrap_or(Value::Null),
            None => record.json.clone(),
        }
    }
}

pub struct TableView {
    pub query: String,
    pub facet: Option<usize>,
    pub sort: usize,
    pub reverse: bool,
    pub state: TableState,
    pub visible: Vec<usize>,
    pub page: usize,
}

impl TableView {
    pub fn new(data: &TableData) -> Self {
        let mut view = Self {
            query: String::new(),
            facet: None,
            sort: data.default_sort,
            reverse: false,
            state: TableState::default(),
            visible: Vec::new(),
            page: 10,
        };
        view.refresh(data);
        view
    }

    pub fn refresh(&mut self, data: &TableData) {
        let needle = self.query.to_lowercase();
        let facet = self.facet.and_then(|i| data.facets.get(i));
        let mut visible: Vec<usize> = data
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| needle.is_empty() || r.haystack.contains(&needle))
            .filter(|(_, r)| facet.is_none_or(|f| r.facets.contains(&f.key)))
            .map(|(i, _)| i)
            .collect();
        let col = self.sort;
        visible.sort_by(|&a, &b| {
            let (ka, kb) = (&data.records[a].sort[col], &data.records[b].sort[col]);
            ka.partial_cmp(kb).unwrap_or(Ordering::Equal)
        });
        if self.reverse {
            visible.reverse();
        }
        self.visible = visible;
        let selected = self.state.selected().unwrap_or(0);
        self.state.select(if self.visible.is_empty() {
            None
        } else {
            Some(selected.min(self.visible.len() - 1))
        });
    }

    pub fn selected_record<'a>(&self, data: &'a TableData) -> Option<&'a Record> {
        let i = self.state.selected()?;
        self.visible.get(i).map(|&r| &data.records[r])
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let max = self.visible.len() as isize - 1;
        let current = self.state.selected().unwrap_or(0) as isize;
        self.state
            .select(Some((current + delta).clamp(0, max) as usize));
    }

    pub fn home(&mut self) {
        if !self.visible.is_empty() {
            self.state.select(Some(0));
        }
    }

    pub fn end(&mut self) {
        if !self.visible.is_empty() {
            self.state.select(Some(self.visible.len() - 1));
        }
    }

    pub fn cycle_facet(&mut self, data: &TableData) {
        self.facet = match self.facet {
            None if !data.facets.is_empty() => Some(0),
            Some(i) if i + 1 < data.facets.len() => Some(i + 1),
            _ => None,
        };
        self.refresh(data);
    }

    pub fn cycle_sort(&mut self, data: &TableData) {
        self.sort = (self.sort + 1) % data.columns.len().max(1);
        self.refresh(data);
    }

    pub fn reverse(&mut self, data: &TableData) {
        self.reverse = !self.reverse;
        self.refresh(data);
    }

    pub fn facet_label<'a>(&self, data: &'a TableData) -> Option<&'a str> {
        self.facet
            .and_then(|i| data.facets.get(i))
            .map(|f| f.label.as_str())
    }
}

fn text(s: impl Into<String>) -> Key {
    Key::Text(s.into().to_lowercase())
}

fn num(n: Option<f64>) -> Key {
    n.map_or(Key::Missing, Key::Num)
}

fn opt(s: Option<&str>) -> Key {
    s.map_or(Key::Missing, text)
}

fn region_facets(regions: impl Iterator<Item = String>) -> Vec<Facet> {
    let mut regions: Vec<String> = regions.collect();
    regions.sort();
    regions.dedup();
    regions
        .into_iter()
        .map(|r| Facet {
            key: format!("region:{r}"),
            label: format!("region {r}"),
        })
        .collect()
}

pub fn assets(service: Service, rows: Vec<AssetRow>) -> TableData {
    let mut facets: Vec<Facet> = [
        ProtectionStatus::AwsBackup,
        ProtectionStatus::NativeOnly,
        ProtectionStatus::Unprotected,
    ]
    .into_iter()
    .filter(|s| rows.iter().any(|r| r.status == *s))
    .map(|s| Facet {
        key: format!("status:{s:?}"),
        label: s.label().to_string(),
    })
    .collect();
    facets.extend(region_facets(rows.iter().map(|r| r.region.clone())));

    let records = rows
        .into_iter()
        .map(|row| {
            let tags = format::tags(&row.tags);
            let last = format::when(row.last_backup.as_deref());
            Record {
                cells: vec![
                    (row.name.clone(), None),
                    (row.region.clone(), Some(theme::MUTE)),
                    (format::opt_bytes(row.size_bytes), None),
                    (
                        row.status.label().to_string(),
                        Some(theme::status(row.status)),
                    ),
                    (last.clone(), None),
                    (row.summary.clone(), Some(theme::MUTE)),
                    (tags.clone(), Some(theme::DIM)),
                ],
                sort: vec![
                    text(&row.name),
                    text(&row.region),
                    num(row.size_bytes.map(|b| b as f64)),
                    Key::Num(row.status as u8 as f64),
                    opt(row.last_backup.as_deref()),
                    text(&row.summary),
                    text(&tags),
                ],
                haystack: format!("{} {} {}", row.name, row.arn, tags).to_lowercase(),
                facets: vec![
                    format!("status:{:?}", row.status),
                    format!("region:{}", row.region),
                ],
                csv: vec![
                    row.name.clone(),
                    row.arn.clone(),
                    row.region.clone(),
                    row.size_bytes.map(|b| b.to_string()).unwrap_or_default(),
                    row.status.label().to_string(),
                    row.last_backup.clone().unwrap_or_default(),
                    row.summary.clone(),
                    tags,
                ],
                json: serde_json::to_value(&row).unwrap_or(Value::Null),
            }
        })
        .collect();

    TableData {
        stem: service.as_str().to_string(),
        title: service.label().to_string(),
        columns: vec![
            Column {
                title: "Name",
                width: Constraint::Fill(3),
            },
            Column {
                title: "Region",
                width: Constraint::Length(14),
            },
            Column {
                title: "Size",
                width: Constraint::Length(10),
            },
            Column {
                title: "Status",
                width: Constraint::Length(11),
            },
            Column {
                title: "Last backup",
                width: Constraint::Length(16),
            },
            Column {
                title: "Summary",
                width: Constraint::Fill(2),
            },
            Column {
                title: "Tags",
                width: Constraint::Fill(2),
            },
        ],
        csv_header: vec![
            "name",
            "arn",
            "region",
            "size_bytes",
            "status",
            "last_backup",
            "summary",
            "tags",
        ],
        records,
        facets,
        detail_field: Some("detail"),
        default_sort: 0,
    }
}

pub fn aws_backup(resources: &[ProtectedResource]) -> TableData {
    let records = resources
        .iter()
        .map(|r| {
            let name = r
                .resource_name
                .clone()
                .unwrap_or_else(|| r.arn.rsplit([':', '/']).next().unwrap_or("").to_string());
            let kind = r.resource_type.clone().unwrap_or_default();
            let c = &r.coverage;
            let vaults = c.vaults.join(", ");
            Record {
                cells: vec![
                    (name.clone(), None),
                    (kind.clone(), Some(theme::LAVENDER)),
                    (r.region.clone(), Some(theme::MUTE)),
                    (format::when(c.last_backup.as_deref()), None),
                    (c.recovery_points.to_string(), None),
                    (format::opt_bytes(c.recovery_point_bytes), None),
                    (vaults.clone(), Some(theme::MUTE)),
                ],
                sort: vec![
                    text(&name),
                    text(&kind),
                    text(&r.region),
                    opt(c.last_backup.as_deref()),
                    Key::Num(c.recovery_points as f64),
                    num(c.recovery_point_bytes.map(|b| b as f64)),
                    text(&vaults),
                ],
                haystack: format!("{name} {} {kind} {vaults}", r.arn).to_lowercase(),
                facets: vec![format!("region:{}", r.region)],
                csv: vec![
                    r.arn.clone(),
                    name,
                    kind,
                    r.region.clone(),
                    c.last_backup.clone().unwrap_or_default(),
                    c.recovery_points.to_string(),
                    c.recovery_point_bytes
                        .map(|b| b.to_string())
                        .unwrap_or_default(),
                    vaults,
                ],
                json: serde_json::to_value(r).unwrap_or(Value::Null),
            }
        })
        .collect();

    TableData {
        stem: "aws_backup".into(),
        title: "AWS Backup protected resources".into(),
        columns: vec![
            Column {
                title: "Resource",
                width: Constraint::Fill(3),
            },
            Column {
                title: "Type",
                width: Constraint::Length(12),
            },
            Column {
                title: "Region",
                width: Constraint::Length(14),
            },
            Column {
                title: "Last backup",
                width: Constraint::Length(16),
            },
            Column {
                title: "Points",
                width: Constraint::Length(7),
            },
            Column {
                title: "Size",
                width: Constraint::Length(10),
            },
            Column {
                title: "Vaults",
                width: Constraint::Fill(2),
            },
        ],
        csv_header: vec![
            "arn",
            "name",
            "type",
            "region",
            "last_backup",
            "recovery_points",
            "recovery_point_bytes",
            "vaults",
        ],
        records,
        facets: region_facets(resources.iter().map(|r| r.region.clone())),
        detail_field: None,
        default_sort: 0,
    }
}

pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::High => "High",
        Severity::Medium => "Medium",
        Severity::Low => "Low",
        Severity::Info => "Info",
    }
}

pub fn findings(findings: &[Finding]) -> TableData {
    let facets = [
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ]
    .into_iter()
    .filter(|s| findings.iter().any(|f| f.severity == *s))
    .map(|s| Facet {
        key: format!("severity:{s:?}"),
        label: severity_label(s).to_string(),
    })
    .collect();

    let records = findings
        .iter()
        .map(|f| Record {
            cells: vec![
                (
                    severity_label(f.severity).to_string(),
                    Some(theme::severity(f.severity)),
                ),
                (f.service.label().to_string(), None),
                (f.region.clone(), Some(theme::MUTE)),
                (f.name.clone(), None),
                (f.code.clone(), Some(theme::LAVENDER)),
                (f.message.clone(), Some(theme::MUTE)),
            ],
            sort: vec![
                Key::Num(f.severity as u8 as f64),
                text(f.service.label()),
                text(&f.region),
                text(&f.name),
                text(&f.code),
                text(&f.message),
            ],
            haystack: format!("{} {} {} {}", f.name, f.arn, f.code, f.message).to_lowercase(),
            facets: vec![format!("severity:{:?}", f.severity)],
            csv: vec![
                severity_label(f.severity).to_string(),
                f.service.as_str().to_string(),
                f.region.clone(),
                f.name.clone(),
                f.arn.clone(),
                f.code.clone(),
                f.message.clone(),
            ],
            json: serde_json::to_value(f).unwrap_or(Value::Null),
        })
        .collect();

    TableData {
        stem: "findings".into(),
        title: "Findings".into(),
        columns: vec![
            Column {
                title: "Severity",
                width: Constraint::Length(9),
            },
            Column {
                title: "Service",
                width: Constraint::Length(12),
            },
            Column {
                title: "Region",
                width: Constraint::Length(14),
            },
            Column {
                title: "Name",
                width: Constraint::Fill(2),
            },
            Column {
                title: "Code",
                width: Constraint::Length(30),
            },
            Column {
                title: "Message",
                width: Constraint::Fill(3),
            },
        ],
        csv_header: vec![
            "severity", "service", "region", "name", "arn", "code", "message",
        ],
        records,
        facets,
        detail_field: None,
        default_sort: 0,
    }
}

pub fn errors(errors: &[DiscoveryError]) -> TableData {
    let records = errors
        .iter()
        .map(|e| {
            let region = e.region.clone().unwrap_or_else(|| "global".into());
            Record {
                cells: vec![
                    (e.service.clone(), None),
                    (e.operation.clone(), Some(theme::LAVENDER)),
                    (region.clone(), Some(theme::MUTE)),
                    (e.message.clone(), Some(theme::PINK)),
                ],
                sort: vec![
                    text(&e.service),
                    text(&e.operation),
                    text(&region),
                    text(&e.message),
                ],
                haystack: format!("{} {} {region} {}", e.service, e.operation, e.message)
                    .to_lowercase(),
                facets: vec![format!("region:{region}")],
                csv: vec![
                    e.service.clone(),
                    e.operation.clone(),
                    region,
                    e.message.clone(),
                ],
                json: serde_json::to_value(e).unwrap_or(Value::Null),
            }
        })
        .collect();

    TableData {
        stem: "errors".into(),
        title: "Errors".into(),
        columns: vec![
            Column {
                title: "Service",
                width: Constraint::Length(12),
            },
            Column {
                title: "Operation",
                width: Constraint::Length(30),
            },
            Column {
                title: "Region",
                width: Constraint::Length(14),
            },
            Column {
                title: "Message",
                width: Constraint::Fill(1),
            },
        ],
        csv_header: vec!["service", "operation", "region", "message"],
        records,
        facets: region_facets(
            errors
                .iter()
                .map(|e| e.region.clone().unwrap_or_else(|| "global".into())),
        ),
        detail_field: None,
        default_sort: 0,
    }
}

pub struct Section {
    pub label: &'static str,
    pub field: &'static str,
    pub columns: &'static [(&'static str, &'static str)],
}

pub const SECTIONS: &[Section] = &[
    Section {
        label: "RDS retained",
        field: "rds_retained_backups",
        columns: &[
            ("Identifier", "identifier"),
            ("Region", "region"),
            ("Engine", "engine"),
            ("Latest restorable", "latest_restorable_time"),
            ("Retention days", "backup_retention_days"),
            ("GiB", "allocated_storage_gib"),
        ],
    },
    Section {
        label: "Recycle Bin",
        field: "rbin_rules",
        columns: &[
            ("Rule", "id"),
            ("Region", "region"),
            ("Type", "resource_type"),
            ("Status", "status"),
            ("Retention days", "retention_days"),
            ("Lock", "lock_state"),
        ],
    },
    Section {
        label: "CloudTrail",
        field: "trails",
        columns: &[
            ("Trail", "name"),
            ("Region", "region"),
            ("Home", "home_region"),
            ("Multi-region", "multi_region"),
            ("Org trail", "organization_trail"),
            ("Bucket", "s3_bucket"),
        ],
    },
];

fn json_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "-".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

pub fn json_table(section: &Section, items: &[Value]) -> TableData {
    let records = items
        .iter()
        .map(|item| {
            let cells: Vec<String> = section
                .columns
                .iter()
                .map(|(_, field)| json_text(item.get(*field)))
                .collect();
            let region = json_text(item.get("region"));
            Record {
                sort: section
                    .columns
                    .iter()
                    .map(|(_, field)| match item.get(*field) {
                        Some(Value::Number(n)) => num(n.as_f64()),
                        None | Some(Value::Null) => Key::Missing,
                        Some(v) => text(json_text(Some(v))),
                    })
                    .collect(),
                haystack: item.to_string().to_lowercase(),
                facets: vec![format!("region:{region}")],
                csv: cells.clone(),
                cells: cells.into_iter().map(|c| (c, None)).collect(),
                json: item.clone(),
            }
        })
        .collect();
    TableData {
        stem: section.field.to_string(),
        title: section.label.to_string(),
        columns: section
            .columns
            .iter()
            .map(|&(title, _)| Column {
                title,
                width: Constraint::Fill(1),
            })
            .collect(),
        csv_header: section.columns.iter().map(|(_, field)| *field).collect(),
        records,
        facets: region_facets(
            items
                .iter()
                .filter_map(|i| i.get("region").and_then(Value::as_str))
                .map(str::to_string),
        ),
        detail_field: None,
        default_sort: 0,
    }
}

pub fn vault_rows(vaults: &[BackupVault]) -> Vec<[String; 4]> {
    vaults
        .iter()
        .map(|v| {
            [
                v.name.clone(),
                v.region.clone(),
                v.recovery_points
                    .map_or_else(|| "-".into(), |n| n.to_string()),
                match v.locked {
                    Some(true) => "locked".into(),
                    Some(false) => "unlocked".into(),
                    None => "-".into(),
                },
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, region: &str, status: ProtectionStatus, size: Option<u64>) -> AssetRow {
        AssetRow {
            service: Service::S3,
            name: name.into(),
            arn: format!("arn:aws:s3:::{name}"),
            region: region.into(),
            size_bytes: size,
            status,
            last_backup: None,
            summary: String::new(),
            tags: [("team".to_string(), name.to_uppercase())].into(),
            detail: Value::Null,
        }
    }

    fn names(view: &TableView, data: &TableData) -> Vec<String> {
        view.visible
            .iter()
            .map(|&i| data.records[i].cells[0].0.clone())
            .collect()
    }

    #[test]
    fn search_facets_and_sort_compose() {
        let data = assets(
            Service::S3,
            vec![
                row("logs", "us-east-1", ProtectionStatus::Unprotected, Some(5)),
                row("app", "ap-southeast-2", ProtectionStatus::AwsBackup, None),
                row("data", "us-east-1", ProtectionStatus::NativeOnly, Some(9)),
            ],
        );
        let mut view = TableView::new(&data);
        assert_eq!(names(&view, &data), ["app", "data", "logs"]);

        view.sort = 2;
        view.reverse = true;
        view.refresh(&data);
        assert_eq!(names(&view, &data), ["data", "logs", "app"]);

        view.query = "TEAM=LO".into();
        view.refresh(&data);
        assert_eq!(names(&view, &data), ["logs"]);

        view.query.clear();
        let labels: Vec<_> = data.facets.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "AWS Backup",
                "Native only",
                "Unprotected",
                "region ap-southeast-2",
                "region us-east-1"
            ]
        );
        view.facet = Some(4);
        view.refresh(&data);
        assert_eq!(names(&view, &data), ["data", "logs"]);
        for _ in 0..data.facets.len() - 4 {
            view.cycle_facet(&data);
        }
        assert_eq!(view.facet, None);
        assert_eq!(view.visible.len(), 3);
    }

    #[test]
    fn json_sections_render_any_inventory_array() {
        const TRAILS: Section = Section {
            label: "Trails",
            field: "trails",
            columns: &[
                ("Name", "name"),
                ("Region", "region"),
                ("Multi", "multi_region"),
            ],
        };
        let items = vec![
            serde_json::json!({"name": "b", "region": "us-east-1", "multi_region": true}),
            serde_json::json!({"name": "a", "region": "ap-southeast-2"}),
        ];
        let data = json_table(&TRAILS, &items);
        let view = TableView::new(&data);
        assert_eq!(names(&view, &data), ["a", "b"]);
        assert_eq!(data.records[0].cells[2].0, "true");
        assert_eq!(data.records[1].cells[2].0, "-");
        assert_eq!(data.csv_header, ["name", "region", "multi_region"]);
        assert_eq!(data.facets.len(), 2);
    }

    #[test]
    fn selection_stays_in_bounds() {
        let data = assets(
            Service::S3,
            vec![row("a", "r", ProtectionStatus::Unprotected, None)],
        );
        let mut view = TableView::new(&data);
        view.move_by(10);
        assert_eq!(view.state.selected(), Some(0));
        view.query = "zzz".into();
        view.refresh(&data);
        assert_eq!(view.state.selected(), None);
        view.move_by(-1);
        view.end();
        assert!(view.selected_record(&data).is_none());
    }
}
