use cd_model::progress::GLOBAL;
use cd_model::{Inventory, Progress, Service};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use ratatui::Terminal;

use crate::app::{App, Flow, Mode};
use crate::{theme, ui};

const FIXTURE: &str = include_str!("../tests/fixtures/inventory.json");

fn fixture() -> Inventory {
    serde_json::from_str(FIXTURE).expect("fixture is a v2 inventory")
}

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buf: &Buffer) -> String {
    let area = buf.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn screen(app: &mut App) -> String {
    text(&draw(app, 160, 48))
}

fn press(app: &mut App, code: KeyCode) -> Flow {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn goto(app: &mut App, label: &str) {
    for _ in 0..20 {
        if let Mode::Browsing(b) = &app.mode {
            if b.current().tab.label() == label {
                return;
            }
        }
        press(app, KeyCode::Tab);
    }
    panic!("no tab {label}");
}

fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in\n{screen}"))
}

fn fg_of(buf: &Buffer, row: &str, needle: &str) -> Color {
    let area = buf.area;
    for y in 0..area.height {
        let line: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
        if !line.contains(row) {
            continue;
        }
        if let Some(byte) = line.find(needle) {
            let col = line[..byte].chars().count() as u16;
            return buf[(col, y)].fg;
        }
    }
    panic!("{needle:?} not on a row with {row:?}");
}

#[test]
fn fixture_deserializes_into_every_section() {
    let inv = fixture();
    assert_eq!(inv.schema_version, 2);
    assert_eq!(inv.account.alias.as_deref(), Some("acme-prod"));
    let counts: Vec<_> = inv.summary();
    assert_eq!(counts.len(), Service::ASSETS.len());
    assert_eq!(counts.iter().map(|(_, n)| n).sum::<usize>(), 13);
    assert_eq!(inv.aws_backup.as_ref().map(Vec::len), Some(5));
    assert_eq!(inv.backup_vaults.as_ref().map(Vec::len), Some(2));
    assert_eq!(inv.rbin_rules.as_ref().map(Vec::len), Some(1));
    assert_eq!(inv.trails.as_ref().map(Vec::len), Some(1));
    assert_eq!(inv.findings.len(), 6);
    assert_eq!(inv.errors.len(), 2);
}

#[test]
fn overview_shows_counts_statuses_and_header() {
    let mut app = App::browse(fixture(), false);
    let buf = draw(&mut app, 160, 48);
    let s = text(&buf);
    assert!(s.contains("Clumio Discover · read-only"), "{s}");
    assert!(s.contains("123456789012 (acme-prod)"));
    assert!(s.contains("o-abc123def4"));
    assert!(s.contains("Regions 2: ap-southeast-2, us-east-1"));
    assert!(s.contains("tag Owner=platform"));
    assert!(s.contains("Protection by service"));

    let total = line_with(&s, "Total");
    for expected in ["13", "4  31%", "9  69%"] {
        assert!(
            total.contains(expected),
            "{expected:?} missing from {total:?}"
        );
    }
    assert!(line_with(&s, "│ EBS ").contains("600.0 GiB"));
    assert!(total.contains("602.0 GiB"));
    assert!(line_with(&s, "│ Glue Iceberg").contains("1"));

    assert!(s.contains("By region"));
    assert!(line_with(&s, "│ap-southeast-2").contains("     7  102.0 GiB"));
    assert!(line_with(&s, "│us-east-1").contains("     6  500.0 GiB"));
    assert!(s.contains("High 2"));
    assert!(s.contains("Medium 2"));
    assert!(s.contains("Backup vaults 2 · Recycle Bin rules 1 · Trails 1 · Errors 2"));

    assert_eq!(fg_of(&buf, "Not protected", "AWS Backup"), theme::OK);
    assert_eq!(fg_of(&buf, "Not protected", "Not protected"), theme::PINK);
    assert_eq!(fg_of(&buf, "High 2", "High 2"), theme::PINK);
    assert!(buf
        .content
        .iter()
        .any(|c| c.symbol() == "█" && c.fg == theme::OK));
    assert!(buf.content.iter().any(|c| c.symbol() == "▀"));
    assert!(buf.content.iter().all(|c| c.bg != Color::Reset));
}

#[test]
fn asset_tabs_list_fixture_names_with_coloured_status() {
    let mut app = App::browse(fixture(), false);
    press(&mut app, KeyCode::Char('2'));
    let buf = draw(&mut app, 160, 48);
    let s = text(&buf);
    assert!(s.contains("acme-orders"));
    assert!(s.contains("acme-logs"));
    assert!(
        line_with(&s, "acme-orders").contains("Owner=platform"),
        "{s}"
    );
    assert!(s.contains("2 of 2"));
    assert_eq!(fg_of(&buf, "acme-logs", "Not protected"), theme::ON_ACCENT);
    assert_eq!(fg_of(&buf, "acme-orders", "AWS Backup"), theme::OK);

    press(&mut app, KeyCode::Char('3'));
    let s = screen(&mut app);
    assert!(s.contains("Provisioned"));
    assert!(s.contains("Written"));
    assert!(
        line_with(&s, "app-root").contains("100.0 GiB   30.0 GiB"),
        "{s}"
    );
    assert!(line_with(&s, "app-root").contains("2026-10-23 01:00"));
    assert!(s.contains("vol-0a1b2c3d4e5f60002"));

    for (label, name) in [
        ("EC2", "bastion"),
        ("RDS", "orders-db"),
        ("DynamoDB", "Customers"),
        ("DocumentDB", "catalog-docdb"),
        ("Neptune", "graph-neptune"),
        ("S3 Tables", "analytics-tables"),
        ("Glue Iceberg", "lakehouse"),
    ] {
        goto(&mut app, label);
        assert!(screen(&mut app).contains(name), "{label} lacks {name}");
    }
}

#[test]
fn search_filter_and_sort_narrow_the_rows() {
    let mut app = App::browse(fixture(), false);
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "orders");
    let s = screen(&mut app);
    assert!(s.contains("acme-orders"));
    assert!(!s.contains("acme-logs"));
    assert!(s.contains("/orders"));
    press(&mut app, KeyCode::Enter);

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "zone=logs");
    press(&mut app, KeyCode::Enter);
    let s = screen(&mut app);
    assert!(s.contains("acme-logs") && !s.contains("acme-orders"));

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "arn:aws:s3:::acme-o");
    press(&mut app, KeyCode::Enter);
    assert!(!screen(&mut app).contains("acme-logs"));
    press(&mut app, KeyCode::Esc);

    press(&mut app, KeyCode::Char('f'));
    let s = screen(&mut app);
    assert!(s.contains("filter: AWS Backup"));
    assert!(s.contains("acme-orders") && !s.contains("acme-logs"));
    press(&mut app, KeyCode::Char('f'));
    press(&mut app, KeyCode::Char('f'));
    let s = screen(&mut app);
    assert!(s.contains("filter: region ap-southeast-2"), "{s}");
    press(&mut app, KeyCode::Esc);

    press(&mut app, KeyCode::Char('S'));
    let s = screen(&mut app);
    let logs = s.find("acme-logs").unwrap();
    let orders = s.find("acme-orders").unwrap();
    assert!(orders < logs, "reverse name sort puts orders first");
    assert!(s.contains("Name▼"));
}

#[test]
fn detail_pane_shows_raw_json_with_protection() {
    let mut app = App::browse(fixture(), false);
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Enter);
    let buf = draw(&mut app, 120, 40);
    let s = text(&buf);
    assert!(s.contains("acme-orders detail"));
    assert!(s.contains("\"versioning\": \"Enabled\""));
    assert!(s.contains("│▌  \"protection\": {"), "{s}");
    assert_eq!(fg_of(&buf, "\"status\"", "\"aws_backup\""), theme::OK);
    press(&mut app, KeyCode::Char('G'));
    press(&mut app, KeyCode::Char('k'));
    draw(&mut app, 120, 10);
    press(&mut app, KeyCode::Esc);
    assert!(screen(&mut app).contains("2 of 2"));
}

#[test]
fn findings_tab_colours_and_filters_by_severity() {
    let mut app = App::browse(fixture(), false);
    goto(&mut app, "Findings");
    let buf = draw(&mut app, 160, 48);
    let s = text(&buf);
    for severity in ["High", "Medium", "Low", "Info"] {
        assert!(s.contains(severity), "missing {severity}");
    }
    assert!(s.contains("cloudtrail.region_not_logged"));
    assert!(line_with(&s, "region_not_logged").contains("CloudTrail"));
    assert_eq!(fg_of(&buf, "ec2.unprotected", "High"), theme::PINK);
    assert_eq!(fg_of(&buf, "dynamodb.pitr_off", "Medium"), theme::WARN);
    let high = s.find("s3.unprotected").unwrap();
    let info = s.find("cloudtrail.region_not_logged").unwrap();
    assert!(high < info);

    press(&mut app, KeyCode::Char('f'));
    let s = screen(&mut app);
    assert!(s.contains("filter: High"));
    assert!(s.contains("ec2.unprotected"));
    assert!(!s.contains("ebs.unencrypted"));

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('G'));
    press(&mut app, KeyCode::Enter);
    assert!(screen(&mut app).contains("\"arn\": \"\""));
}

#[test]
fn backup_and_errors_tabs() {
    let mut app = App::browse(fixture(), false);
    goto(&mut app, "AWS Backup");
    let s = screen(&mut app);
    assert!(s.contains("AWS Backup protected resources"));
    assert!(line_with(&s, "Customers").contains("2.0 GiB"));
    assert!(s.contains("Backup vaults"));
    assert!(line_with(&s, "│dr-vault").contains("locked"));
    assert!(s.contains("vol-0a1b2c3d4e5f60001"));

    goto(&mut app, "Errors");
    let s = screen(&mut app);
    assert!(line_with(&s, "GetTableOptimizer").contains("us-east-1"));
    assert!(line_with(&s, "ListRegions").contains("global"));
}

#[test]
fn unscanned_sections_show_as_dimmed_tabs() {
    let mut inv = fixture();
    inv.s3tables = None;
    inv.aws_backup = None;
    let mut app = App::browse(inv, false);
    let s = screen(&mut app);
    assert!(line_with(&s, "│ S3 Tables").contains("not scanned"));
    goto(&mut app, "S3 Tables");
    assert!(screen(&mut app).contains("Not scanned"));
    goto(&mut app, "AWS Backup");
    assert!(screen(&mut app).contains("Not scanned"));
}

#[test]
fn overview_enter_opens_the_service_tab() {
    let mut app = App::browse(fixture(), false);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    let Mode::Browsing(b) = &app.mode else {
        unreachable!()
    };
    assert_eq!(b.current().tab.label(), "EBS");
}

#[test]
fn export_writes_filtered_csv_and_json() {
    let dir = std::env::temp_dir().join(format!("cd-tui-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App::browse(fixture(), true);
    if let Mode::Browsing(b) = &mut app.mode {
        b.out_dir = dir.clone();
    }
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "logs");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('e'));
    let s = screen(&mut app);
    assert!(s.contains("Exported"), "{s}");
    press(&mut app, KeyCode::Char('w'));
    assert!(screen(&mut app).contains("Saved"));

    let mut csv = None;
    let mut json = None;
    let mut saved = None;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let body = std::fs::read_to_string(&path).unwrap();
        if name.starts_with("clumio-discover-s3-") && name.ends_with(".csv") {
            csv = Some(body);
        } else if name.starts_with("clumio-discover-s3-") && name.ends_with(".json") {
            json = Some(body);
        } else if name.starts_with("inventory-123456789012-") {
            saved = Some(body);
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();

    let csv = csv.expect("csv written");
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some("name,arn,region,size_bytes,protection,last_backup,summary,tags")
    );
    assert!(lines
        .next()
        .unwrap()
        .starts_with("acme-logs,arn:aws:s3:::acme-logs,us-east-1"));
    assert_eq!(lines.next(), None);
    let json: serde_json::Value = serde_json::from_str(&json.expect("json written")).unwrap();
    assert_eq!(json.as_array().map(Vec::len), Some(1));
    assert_eq!(json[0]["detail"]["versioning"], "Suspended");
    let saved: Inventory = serde_json::from_str(&saved.expect("inventory saved")).unwrap();
    assert_eq!(saved, fixture());
}

#[test]
fn view_mode_has_no_save_key() {
    let mut app = App::browse(fixture(), false);
    press(&mut app, KeyCode::Char('w'));
    let s = screen(&mut app);
    assert!(!s.contains("Saved"));
    assert!(!s.contains("w save"));
}

#[test]
fn help_overlay_and_quit() {
    let mut app = App::browse(fixture(), true);
    press(&mut app, KeyCode::Char('?'));
    let s = screen(&mut app);
    assert!(s.contains("Keys"));
    assert!(s.contains("save the full inventory JSON here"));
    assert_eq!(press(&mut app, KeyCode::Char('q')), Flow::Continue);
    assert!(!screen(&mut app).contains("any key closes"));
    assert_eq!(press(&mut app, KeyCode::Char('q')), Flow::Quit);
    assert!(app.into_inventory().is_some());
}

#[test]
fn scan_grid_tracks_progress_then_switches_to_browser() {
    let mut app = App::scanning();
    let region = |r: &str| r.to_string();
    app.progress(Progress::Started {
        region: region(GLOBAL),
        service: Service::Account,
    });
    app.progress(Progress::Finished {
        region: region(GLOBAL),
        service: Service::Account,
        count: Some(1),
        errors: 0,
    });
    app.progress(Progress::Regions(vec![
        region("ap-southeast-2"),
        region("us-east-1"),
    ]));
    for r in ["ap-southeast-2", "us-east-1"] {
        app.progress(Progress::Started {
            region: region(r),
            service: Service::Ebs,
        });
    }
    app.progress(Progress::Finished {
        region: region("ap-southeast-2"),
        service: Service::Ebs,
        count: Some(42),
        errors: 3,
    });
    let buf = draw(&mut app, 120, 40);
    let s = text(&buf);
    assert!(s.contains("Clumio Discover · read-only"));
    assert!(buf.content.iter().any(|c| c.symbol() == "▀"));
    assert!(s.contains("account-wide"));
    assert!(line_with(&s, "ap-southeast-2").contains("42 ✗3"));
    assert!(s.contains("2/3 sections"));
    assert!(s.contains("3 errors"));
    assert!(s.contains("67%"));
    assert_eq!(fg_of(&buf, "ap-southeast-2", "✗3"), theme::PINK);

    app.finished(fixture());
    let s = screen(&mut app);
    assert!(s.contains("Scan complete: 13 assets, 6 findings. Press w to save."));
    if let Mode::Browsing(b) = &mut app.mode {
        b.toast = None;
    }
    assert!(screen(&mut app).contains("w save"));
}

#[test]
fn finished_scan_saves_the_inventory_when_asked() {
    let dir = std::env::temp_dir().join(format!("cd-tui-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App::scanning();
    app.auto_save = true;
    app.out_dir = dir.clone();
    app.finished(fixture());

    let saved = app.saved.clone().expect("inventory saved");
    assert!(saved.starts_with(&dir));
    let back: Inventory = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert_eq!(back.account.id, "123456789012");
    let s = screen(&mut app);
    assert!(s.contains("Scan complete: 13 assets, 6 findings. Saved"));
    if let Mode::Browsing(b) = &mut app.mode {
        b.toast = None;
    }
    assert!(!screen(&mut app).contains("w save"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scan_quit_returns_nothing() {
    let mut app = App::scanning();
    assert_eq!(press(&mut app, KeyCode::Char('x')), Flow::Continue);
    assert_eq!(press(&mut app, KeyCode::Char('q')), Flow::Quit);
    assert!(app.into_inventory().is_none());
}

#[test]
fn narrow_and_tiny_terminals_do_not_panic() {
    let sizes = [
        (1, 1),
        (8, 3),
        (20, 5),
        (40, 10),
        (60, 20),
        (80, 24),
        (100, 30),
    ];
    let mut scanning = App::scanning();
    scanning.progress(Progress::Regions(vec!["us-east-1".into()]));
    scanning.progress(Progress::Started {
        region: "us-east-1".into(),
        service: Service::Glue,
    });
    for (w, h) in sizes {
        draw(&mut scanning, w, h);
    }

    let mut app = App::browse(fixture(), true);
    let tabs = match &app.mode {
        Mode::Browsing(b) => b.panes.len(),
        Mode::Scanning(_) => unreachable!(),
    };
    for _ in 0..tabs {
        for (w, h) in sizes {
            draw(&mut app, w, h);
        }
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::PageDown);
        for (w, h) in sizes {
            draw(&mut app, w, h);
        }
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('?'));
        for (w, h) in sizes {
            draw(&mut app, w, h);
        }
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Tab);
    }

    let s = text(&draw(&mut app, 50, 12));
    assert!(s.contains("Clumio Discover"));
}

#[test]
fn non_truecolor_terminals_get_256_colour_indices() {
    let mut app = App::browse(fixture(), false);
    app.truecolor = false;
    let buf = draw(&mut app, 120, 40);
    assert!(buf
        .content
        .iter()
        .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))));
    assert!(buf.content.iter().any(|c| c.bg == Color::Indexed(232)));
}

fn ansi(buf: &Buffer) -> String {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("{r};{g};{b}"),
        _ => "0;0;0".into(),
    };
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            out += &format!(
                "\x1b[38;2;{}m\x1b[48;2;{}m{}",
                rgb(cell.fg),
                rgb(cell.bg),
                cell.symbol()
            );
        }
        out += "\x1b[0m\n";
    }
    out
}

#[test]
#[ignore]
fn print_screens() {
    let mut scan = App::scanning();
    scan.progress(Progress::Regions(vec![
        "ap-southeast-2".into(),
        "us-east-1".into(),
    ]));
    for service in [Service::Ebs, Service::Ec2, Service::Rds, Service::AwsBackup] {
        for region in ["ap-southeast-2", "us-east-1"] {
            scan.progress(Progress::Started {
                region: region.into(),
                service,
            });
        }
        scan.progress(Progress::Finished {
            region: "ap-southeast-2".into(),
            service,
            count: Some(3),
            errors: usize::from(service == Service::Rds),
        });
    }
    println!("{}", ansi(&draw(&mut scan, 120, 30)));

    let mut app = App::browse(fixture(), true);
    let tabs = match &app.mode {
        Mode::Browsing(b) => b.panes.len(),
        Mode::Scanning(_) => unreachable!(),
    };
    for _ in 0..tabs {
        println!("{}", ansi(&draw(&mut app, 140, 36)));
        press(&mut app, KeyCode::Tab);
    }
    println!("{}", ansi(&draw(&mut app, 80, 24)));
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Enter);
    println!("{}", ansi(&draw(&mut app, 100, 30)));
}
