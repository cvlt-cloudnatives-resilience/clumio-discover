use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, Service, Severity, Tags};

const GIB: u64 = 1024 * 1024 * 1024;
const MIN_RETENTION_DAYS: i32 = 7;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Cluster,
    #[default]
    Instance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMember {
    pub id: String,
    #[serde(default)]
    pub writer: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publicly_accessible: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerlessV2Capacity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_acu: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_acu: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlobalRole {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalMembership {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<GlobalRole>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotStats {
    #[serde(default)]
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default)]
    pub allocated_gib: u64,
}

impl SnapshotStats {
    pub fn add(&mut self, created: Option<String>, allocated_gib: Option<i32>) {
        self.count += 1;
        self.latest = self.latest.take().max(created);
        self.allocated_gib += allocated_gib.unwrap_or(0).max(0) as u64;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Database {
    pub identifier: String,
    pub arn: String,
    pub region: String,
    pub kind: DbKind,
    pub engine: String,
    #[serde(default)]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_mode: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_class: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<ClusterMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serverless_v2: Option<ServerlessV2Capacity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_cluster: Option<GlobalMembership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replica_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_az: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publicly_accessible: Option<bool>,
    #[serde(default)]
    pub storage_encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iam_auth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_protection: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_tags_to_snapshot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated_storage_gib: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iops: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_retention_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_backup_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_restorable_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_restorable_time: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_replications: Vec<String>,
    #[serde(default, skip_serializing_if = "SnapshotStats::is_empty")]
    pub automated_snapshots: SnapshotStats,
    #[serde(default, skip_serializing_if = "SnapshotStats::is_empty")]
    pub manual_snapshots: SnapshotStats,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_snapshots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshot_shared_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_subnet_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpc_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_certificate: Option<String>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetainedBackup {
    pub identifier: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_restorable_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_restorable_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_retention_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated_storage_gib: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
    #[serde(default)]
    pub tags: Tags,
}

macro_rules! database_asset {
    ($name:ident, $service:expr) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Database);

        impl Asset for $name {
            const SERVICE: Service = $service;

            fn name(&self) -> String {
                self.0.identifier.clone()
            }
            fn arn(&self) -> &str {
                &self.0.arn
            }
            fn region(&self) -> &str {
                &self.0.region
            }
            fn tags(&self) -> &Tags {
                &self.0.tags
            }
            fn protection(&self) -> &Protection {
                &self.0.protection
            }
            fn protection_mut(&mut self) -> &mut Protection {
                &mut self.0.protection
            }
            fn size_bytes(&self) -> Option<u64> {
                self.0.size_bytes()
            }
            fn summary(&self) -> String {
                self.0.summary()
            }
            fn findings(&self) -> Vec<Finding> {
                self.0.findings(self)
            }
        }
    };
}

database_asset!(RdsDatabase, Service::Rds);
database_asset!(DocumentDbCluster, Service::Documentdb);
database_asset!(NeptuneCluster, Service::Neptune);

impl Database {
    pub fn size_bytes(&self) -> Option<u64> {
        self.used_bytes.or_else(|| {
            self.allocated_storage_gib
                .map(|gib| gib.max(0) as u64 * GIB)
        })
    }

    pub fn summary(&self) -> String {
        let mut parts = vec![match &self.engine_version {
            Some(version) => format!("{} {version}", self.engine),
            None => self.engine.clone(),
        }];
        match self.kind {
            DbKind::Cluster => {
                parts.push("cluster".to_string());
                if !self.members.is_empty() {
                    parts.push(format!("{} inst", self.members.len()));
                }
            }
            DbKind::Instance => parts.extend(self.instance_class.clone()),
        }
        parts.extend(self.size_bytes().map(human_bytes));
        parts.join(" · ")
    }

    pub fn findings<A: Asset>(&self, asset: &A) -> Vec<Finding> {
        let code = |suffix: &str| format!("{}.{suffix}", A::SERVICE.as_str());
        let mut findings = Vec::new();

        if self.storage_encrypted == Some(false) {
            findings.push(asset.finding(
                Severity::High,
                &code("unencrypted"),
                "storage is not encrypted",
            ));
        }
        let public_members: Vec<&str> = self
            .members
            .iter()
            .filter(|m| m.publicly_accessible == Some(true))
            .map(|m| m.id.as_str())
            .collect();
        if !public_members.is_empty() {
            findings.push(asset.finding(
                Severity::High,
                &code("public"),
                format!("publicly accessible members: {}", public_members.join(", ")),
            ));
        } else if self.publicly_accessible == Some(true) {
            findings.push(asset.finding(Severity::High, &code("public"), "publicly accessible"));
        }
        if !self.public_snapshots.is_empty() {
            findings.push(asset.finding(
                Severity::High,
                &code("snapshot_public"),
                format!(
                    "manual snapshots any AWS account can restore: {}",
                    self.public_snapshots.join(", ")
                ),
            ));
        }
        if let (Some(days), None) = (self.backup_retention_days, &self.replica_of) {
            if days < MIN_RETENTION_DAYS {
                let message = if days <= 0 {
                    "automated backups are disabled".to_string()
                } else {
                    format!("automated backups kept {days} day(s), under {MIN_RETENTION_DAYS}")
                };
                findings.push(asset.finding(Severity::Medium, &code("short_retention"), message));
            }
        }
        if self.deletion_protection == Some(false) {
            findings.push(asset.finding(
                Severity::Low,
                &code("no_deletion_protection"),
                "deletion protection is off",
            ));
        }
        if self.copy_tags_to_snapshot == Some(false) {
            findings.push(asset.finding(
                Severity::Info,
                &code("no_copy_tags_to_snapshot"),
                "snapshots do not inherit the database's tags",
            ));
        }
        if !self.snapshot_shared_with.is_empty() {
            findings.push(asset.finding(
                Severity::Info,
                &code("snapshot_shared"),
                format!(
                    "manual snapshots shared with accounts: {}",
                    self.snapshot_shared_with.join(", ")
                ),
            ));
        }
        findings
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 || value.fract() == 0.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster() -> Database {
        Database {
            identifier: "orders".into(),
            arn: "arn:aws:rds:ap-southeast-2:1:cluster:orders".into(),
            region: "ap-southeast-2".into(),
            kind: DbKind::Cluster,
            engine: "aurora-postgresql".into(),
            engine_version: Some("16.4".into()),
            members: vec![
                ClusterMember {
                    id: "orders-1".into(),
                    writer: true,
                    ..ClusterMember::default()
                },
                ClusterMember {
                    id: "orders-2".into(),
                    ..ClusterMember::default()
                },
            ],
            storage_encrypted: Some(true),
            deletion_protection: Some(true),
            copy_tags_to_snapshot: Some(true),
            backup_retention_days: Some(7),
            used_bytes: Some(4_402_341_478),
            ..Database::default()
        }
    }

    fn codes<A: Asset>(asset: &A) -> Vec<String> {
        asset.findings().into_iter().map(|f| f.code).collect()
    }

    #[test]
    fn summary_reads_like_the_console() {
        assert_eq!(
            cluster().summary(),
            "aurora-postgresql 16.4 · cluster · 2 inst · 4.1 GiB"
        );
        let instance = Database {
            engine: "postgres".into(),
            engine_version: Some("16.4".into()),
            instance_class: Some("db.t3.micro".into()),
            allocated_storage_gib: Some(20),
            ..Database::default()
        };
        assert_eq!(instance.summary(), "postgres 16.4 · db.t3.micro · 20 GiB");
        assert_eq!(instance.size_bytes(), Some(20 * GIB));
    }

    #[test]
    fn used_bytes_wins_over_allocated_storage() {
        let db = Database {
            allocated_storage_gib: Some(1),
            used_bytes: Some(512),
            ..Database::default()
        };
        assert_eq!(db.size_bytes(), Some(512));
        assert_eq!(Database::default().size_bytes(), None);
    }

    #[test]
    fn a_well_configured_cluster_has_no_findings() {
        assert!(codes(&RdsDatabase(cluster())).is_empty());
    }

    #[test]
    fn findings_carry_the_family_service() {
        let mut db = cluster();
        db.engine = "docdb".into();
        db.storage_encrypted = Some(false);
        db.deletion_protection = Some(false);
        db.copy_tags_to_snapshot = Some(false);
        db.backup_retention_days = Some(1);
        db.members[1].publicly_accessible = Some(true);
        db.public_snapshots = vec!["orders-final".into()];
        db.snapshot_shared_with = vec!["222222222222".into()];

        let findings = DocumentDbCluster(db).findings();
        let got: Vec<_> = findings
            .iter()
            .map(|f| (f.severity, f.code.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (Severity::High, "documentdb.unencrypted"),
                (Severity::High, "documentdb.public"),
                (Severity::High, "documentdb.snapshot_public"),
                (Severity::Medium, "documentdb.short_retention"),
                (Severity::Low, "documentdb.no_deletion_protection"),
                (Severity::Info, "documentdb.no_copy_tags_to_snapshot"),
                (Severity::Info, "documentdb.snapshot_shared"),
            ]
        );
        assert!(findings.iter().all(|f| f.service == Service::Documentdb));
        assert!(findings[1].message.contains("orders-2"));
    }

    #[test]
    fn retention_finding_distinguishes_disabled_and_skips_replicas() {
        let mut db = Database {
            backup_retention_days: Some(0),
            publicly_accessible: Some(true),
            ..Database::default()
        };
        let findings = RdsDatabase(db.clone()).findings();
        assert_eq!(
            codes(&RdsDatabase(db.clone())),
            ["rds.public", "rds.short_retention"]
        );
        assert_eq!(findings[1].message, "automated backups are disabled");

        db.replica_of = Some("primary-db".into());
        assert_eq!(codes(&RdsDatabase(db)), ["rds.public"]);
    }

    #[test]
    fn snapshot_stats_track_count_latest_and_size() {
        let mut stats = SnapshotStats::default();
        assert!(stats.is_empty());
        stats.add(Some("2026-10-02T00:00:00Z".into()), Some(20));
        stats.add(Some("2026-10-01T00:00:00Z".into()), None);
        stats.add(None, Some(30));
        assert_eq!(stats.count, 3);
        assert_eq!(stats.latest.as_deref(), Some("2026-10-02T00:00:00Z"));
        assert_eq!(stats.allocated_gib, 50);
    }

    #[test]
    fn older_json_still_loads() {
        let json = serde_json::json!({
            "identifier": "legacy",
            "arn": "arn:aws:rds:us-east-1:1:db:legacy",
            "region": "us-east-1",
            "kind": "instance",
            "engine": "mysql",
            "engine_version": "8.0.39",
            "status": "available",
            "storage_encrypted": true,
            "tags": {},
            "protection": {"status": "unprotected"}
        });
        let db: RdsDatabase = serde_json::from_value(json).unwrap();
        assert_eq!(db.0.identifier, "legacy");
        assert!(db.0.members.is_empty() && db.0.manual_snapshots.is_empty());

        let back = serde_json::to_value(&db).unwrap();
        assert!(back.get("manual_snapshots").is_none());
        assert!(back.get("members").is_none());
    }

    #[test]
    fn human_bytes_picks_a_readable_unit() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(300 * 1024 * 1024), "300 MiB");
        assert_eq!(human_bytes(GIB + GIB / 2), "1.5 GiB");
        assert_eq!(human_bytes(2048 * GIB), "2 TiB");
    }
}
