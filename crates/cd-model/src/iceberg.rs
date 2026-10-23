use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, Service, Severity, Tags};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableBucket {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreferenced_file_removal: Option<S3TableFileRemoval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<S3TableNamespace>,
    #[serde(default)]
    pub tables: Vec<S3Table>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3TableNamespace {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_account_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Table {
    pub namespace: String,
    pub name: String,
    pub arn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by_service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warehouse_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<S3TableCompaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_management: Option<S3TableSnapshotManagement>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3TableFileRemoval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreferenced_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_current_days: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3TableCompaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_file_size_mb: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3TableSnapshotManagement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_snapshots_to_keep: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_snapshot_age_hours: Option<i32>,
}

impl S3Table {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlueDatabase {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub iceberg_tables: Vec<GlueTable>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlueTable {
    pub name: String,
    pub arn: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_metadata_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_count: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub version_count_capped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizers: Option<Vec<GlueOptimizer>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlueOptimizer {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Asset for TableBucket {
    const SERVICE: Service = Service::S3tables;

    fn name(&self) -> String {
        self.name.clone()
    }
    fn arn(&self) -> &str {
        &self.arn
    }
    fn region(&self) -> &str {
        &self.region
    }
    fn tags(&self) -> &Tags {
        &self.tags
    }
    fn protection(&self) -> &Protection {
        &self.protection
    }
    fn protection_mut(&mut self) -> &mut Protection {
        &mut self.protection
    }
    fn summary(&self) -> String {
        format!(
            "{} · {}",
            plural(self.tables.len(), "table", "tables"),
            plural(self.namespaces.len(), "namespace", "namespaces")
        )
    }

    fn findings(&self) -> Vec<Finding> {
        self.tables
            .iter()
            .filter(|t| {
                t.snapshot_management
                    .as_ref()
                    .and_then(|s| s.status.as_deref())
                    == Some("disabled")
            })
            .map(|t| Finding {
                arn: t.arn.clone(),
                name: format!("{}/{}", self.name, t.qualified_name()),
                ..self.finding(
                    Severity::Low,
                    "s3tables.snapshot_management_disabled",
                    "snapshot management is off: snapshots never expire and storage grows unbounded",
                )
            })
            .collect()
    }
}

impl Asset for GlueDatabase {
    const SERVICE: Service = Service::Glue;

    fn name(&self) -> String {
        self.name.clone()
    }
    fn arn(&self) -> &str {
        &self.arn
    }
    fn region(&self) -> &str {
        &self.region
    }
    fn tags(&self) -> &Tags {
        &self.tags
    }
    fn protection(&self) -> &Protection {
        &self.protection
    }
    fn protection_mut(&mut self) -> &mut Protection {
        &mut self.protection
    }
    fn summary(&self) -> String {
        plural(self.iceberg_tables.len(), "Iceberg table", "Iceberg tables")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(tables: Vec<S3Table>) -> TableBucket {
        TableBucket {
            name: "lake".into(),
            arn: "arn:aws:s3tables:ap-southeast-2:1:bucket/lake".into(),
            region: "ap-southeast-2".into(),
            created_at: None,
            bucket_type: None,
            owner_account_id: None,
            unreferenced_file_removal: None,
            namespaces: Vec::new(),
            tables,
            tags: Tags::new(),
            protection: Protection::default(),
        }
    }

    fn table(name: &str, snapshots: Option<&str>) -> S3Table {
        S3Table {
            namespace: "sales".into(),
            name: name.into(),
            arn: format!("arn:aws:s3tables:ap-southeast-2:1:bucket/lake/table/{name}"),
            snapshot_management: snapshots.map(|status| S3TableSnapshotManagement {
                status: Some(status.into()),
                ..S3TableSnapshotManagement::default()
            }),
            ..S3Table::default()
        }
    }

    #[test]
    fn minimal_json_still_loads() {
        let b: TableBucket = serde_json::from_value(serde_json::json!({
            "name": "lake",
            "arn": "arn:aws:s3tables:ap-southeast-2:1:bucket/lake",
            "region": "ap-southeast-2",
            "tables": [{"namespace": "sales", "name": "orders", "arn": "arn:t"}]
        }))
        .unwrap();
        assert_eq!(b.tables[0].qualified_name(), "sales.orders");
        assert!(b.namespaces.is_empty());

        let g: GlueDatabase = serde_json::from_value(serde_json::json!({
            "name": "analytics",
            "arn": "arn:aws:glue:ap-southeast-2:1:database/analytics",
            "region": "ap-southeast-2",
            "iceberg_tables": [{"name": "events", "arn": "arn:t", "location": "s3://x"}]
        }))
        .unwrap();
        assert_eq!(g.iceberg_tables[0].optimizers, None);
        assert!(!g.iceberg_tables[0].version_count_capped);
    }

    #[test]
    fn summaries_count_tables_and_namespaces() {
        let mut b = bucket(vec![table("a", None), table("b", None)]);
        b.namespaces = vec![S3TableNamespace {
            name: "sales".into(),
            ..S3TableNamespace::default()
        }];
        assert_eq!(b.summary(), "2 tables · 1 namespace");

        let g = GlueDatabase {
            name: "analytics".into(),
            arn: "arn".into(),
            region: "ap-southeast-2".into(),
            description: None,
            location_uri: None,
            created_at: None,
            iceberg_tables: vec![GlueTable::default()],
            tags: Tags::new(),
            protection: Protection::default(),
        };
        assert_eq!(g.summary(), "1 Iceberg table");
    }

    #[test]
    fn disabled_snapshot_management_is_flagged_per_table() {
        let b = bucket(vec![
            table("on", Some("enabled")),
            table("off", Some("disabled")),
            table("unknown", None),
        ]);
        let findings = b.findings();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.code, "s3tables.snapshot_management_disabled");
        assert_eq!(f.severity, Severity::Low);
        assert_eq!(f.service, Service::S3tables);
        assert_eq!(f.name, "lake/sales.off");
        assert!(f.arn.ends_with("/table/off"));
        assert_eq!(f.region, "ap-southeast-2");
    }
}
