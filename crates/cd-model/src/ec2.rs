use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, RecycleBinRule, Service, Severity, Tags};

pub const MSSQL_LICENSE_INCLUDED: &str = "license_included";
pub const MSSQL_POSSIBLE: &str = "possible";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ec2Instance {
    pub id: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub instance_type: Option<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcpus: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpc_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpc_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subnet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subnet_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elastic_ip: Option<ElasticIp>,
    #[serde(default)]
    pub eni_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_groups: Vec<SecurityGroupRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_pair_exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iam_instance_profile: Option<InstanceProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_device_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ebs_optimized: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitoring: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenancy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hibernation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_options: Option<MetadataOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_reservation: Option<CapacityReservationRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_credits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssm_managed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssm: Option<SsmAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mssql: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub amis: Vec<OwnedAmi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_protection: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutdown_behavior: Option<String>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElasticIp {
    pub public_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityGroupRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_to_world: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceProfile {
    pub arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hop_limit: Option<i32>,
}

impl MetadataOptions {
    pub fn imdsv1_allowed(&self) -> bool {
        self.http_endpoint.as_deref() != Some("disabled")
            && self.http_tokens.as_deref() == Some("optional")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityReservationRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsmAgent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ping: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedAmi {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    pub tags: Tags,
}

impl Ec2Instance {
    fn reachable_ip(&self) -> Option<&str> {
        self.public_ip
            .as_deref()
            .or(self.elastic_ip.as_ref().map(|e| e.public_ip.as_str()))
    }

    fn public_admin_ports(&self) -> Vec<String> {
        let mut ports: Vec<String> = self
            .security_groups
            .iter()
            .flat_map(|g| &g.open_to_world)
            .filter(|p| exposes_admin_port(p))
            .cloned()
            .collect();
        ports.sort();
        ports.dedup();
        ports
    }
}

fn exposes_admin_port(spec: &str) -> bool {
    if spec == "all" {
        return true;
    }
    let Some(range) = spec.strip_prefix("tcp/") else {
        return false;
    };
    let (from, to) = range.split_once('-').unwrap_or((range, range));
    match (from.parse::<u16>(), to.parse::<u16>()) {
        (Ok(from), Ok(to)) => [22, 3389].iter().any(|p| (from..=to).contains(p)),
        _ => false,
    }
}

impl Asset for Ec2Instance {
    const SERVICE: Service = Service::Ec2;

    fn name(&self) -> String {
        self.tags
            .get("Name")
            .cloned()
            .unwrap_or_else(|| self.id.clone())
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
        [
            self.instance_type.clone(),
            self.state.clone(),
            Some(format!("{} vol", self.volumes.len())),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }

    fn findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        if self
            .metadata_options
            .as_ref()
            .is_some_and(MetadataOptions::imdsv1_allowed)
        {
            let message = match self.reachable_ip() {
                Some(ip) => format!("IMDSv1 allowed (HttpTokens optional) on public IP {ip}"),
                None => "IMDSv1 allowed (HttpTokens optional)".to_string(),
            };
            findings.push(self.finding(Severity::Medium, "ec2.imdsv1", message));
        }

        let impaired: Vec<&str> = [
            ("system", &self.system_status),
            ("instance", &self.instance_status),
        ]
        .into_iter()
        .filter(|(_, status)| status.as_deref() == Some("impaired"))
        .map(|(check, _)| check)
        .collect();
        if !impaired.is_empty() {
            findings.push(self.finding(
                Severity::Medium,
                "ec2.status_impaired",
                format!("{} status check impaired", impaired.join(" and ")),
            ));
        }

        if let Some(ip) = self.reachable_ip() {
            let ports = self.public_admin_ports();
            if !ports.is_empty() {
                findings.push(self.finding(
                    Severity::Medium,
                    "ec2.admin_port_public",
                    format!(
                        "security group opens {} to the internet on public IP {ip}",
                        ports.join(", ")
                    ),
                ));
            }
        }

        let public: Vec<&str> = self
            .amis
            .iter()
            .filter(|a| a.public == Some(true))
            .map(|a| a.id.as_str())
            .collect();
        if !public.is_empty() {
            findings.push(self.finding(
                Severity::High,
                "ec2.ami_public",
                format!("AMI of this instance is public: {}", public.join(", ")),
            ));
        }

        let shared: Vec<String> = self
            .amis
            .iter()
            .filter(|a| !a.shared_with.is_empty())
            .map(|a| format!("{} ({})", a.id, a.shared_with.len()))
            .collect();
        if !shared.is_empty() {
            findings.push(self.finding(
                Severity::Info,
                "ec2.ami_shared",
                format!("AMI shared with other principals: {}", shared.join(", ")),
            ));
        }

        if self.termination_protection == Some(false) {
            findings.push(self.finding(
                Severity::Low,
                "ec2.termination_protection_off",
                "termination protection disabled",
            ));
        }

        if self.shutdown_behavior.as_deref() == Some("terminate") {
            findings.push(self.finding(
                Severity::Low,
                "ec2.shutdown_terminates",
                "shutdown from the OS terminates the instance",
            ));
        }

        if self.ssm_managed == Some(false) {
            findings.push(self.finding(
                Severity::Info,
                "ec2.not_ssm_managed",
                "not an SSM managed node",
            ));
        }

        match self.mssql.as_deref() {
            Some(MSSQL_LICENSE_INCLUDED) => findings.push(self.finding(
                Severity::Info,
                "ec2.mssql_candidate",
                "SQL Server license-included AMI; Clumio can protect SQL Server on EC2",
            )),
            Some(_) => findings.push(self.finding(
                Severity::Info,
                "ec2.mssql_candidate",
                "SSM-managed Windows instance, may run SQL Server; Clumio can protect SQL Server on EC2",
            )),
            None => {}
        }

        findings
    }
}

impl Ec2Instance {
    pub fn apply_rbin_rules(&mut self, rules: &[RecycleBinRule]) {
        self.protection.rbin_rules = rules
            .iter()
            .filter(|rule| {
                self.amis
                    .iter()
                    .any(|ami| rule.retains(RecycleBinRule::EC2_IMAGE, &ami.tags))
            })
            .map(|rule| rule.id.clone())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycle_bin_rules_match_ami_tags() {
        let rule = |id: &str, key: &str| RecycleBinRule {
            id: id.into(),
            resource_type: RecycleBinRule::EC2_IMAGE.into(),
            status: Some("available".into()),
            resource_tags: vec![crate::RuleTag {
                key: key.into(),
                value: None,
            }],
            ..RecycleBinRule::default()
        };
        let mut i = instance();
        i.amis = vec![OwnedAmi {
            id: "ami-1".into(),
            tags: Tags::from([("keep".to_string(), "yes".to_string())]),
            ..OwnedAmi::default()
        }];
        i.apply_rbin_rules(&[rule("r-keep", "keep"), rule("r-other", "other")]);
        assert_eq!(i.protection.rbin_rules, vec!["r-keep"]);
    }

    fn instance() -> Ec2Instance {
        Ec2Instance {
            id: "i-0abc".into(),
            arn: "arn:aws:ec2:ap-southeast-2:1:instance/i-0abc".into(),
            region: "ap-southeast-2".into(),
            state: Some("running".into()),
            instance_type: Some("t3.micro".into()),
            volumes: vec!["vol-a".into(), "vol-b".into()],
            metadata_options: Some(MetadataOptions {
                http_tokens: Some("required".into()),
                http_endpoint: Some("enabled".into()),
                hop_limit: Some(2),
            }),
            ssm_managed: Some(true),
            ..Ec2Instance::default()
        }
    }

    fn codes(instance: &Ec2Instance) -> Vec<String> {
        instance.findings().into_iter().map(|f| f.code).collect()
    }

    #[test]
    fn summary_and_name() {
        let mut i = instance();
        assert_eq!(i.summary(), "t3.micro · running · 2 vol");
        assert_eq!(i.name(), "i-0abc");
        i.tags.insert("Name".into(), "web".into());
        assert_eq!(i.name(), "web");
        assert_eq!(i.size_bytes(), None);
    }

    #[test]
    fn a_hardened_instance_has_no_findings() {
        assert!(codes(&instance()).is_empty());
    }

    #[test]
    fn imdsv1_mentions_the_public_ip() {
        let mut i = instance();
        i.metadata_options.as_mut().unwrap().http_tokens = Some("optional".into());
        let findings = i.findings();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "ec2.imdsv1");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert!(!findings[0].message.contains("public"));

        i.public_ip = Some("203.0.113.7".into());
        assert!(i.findings()[0].message.contains("203.0.113.7"));

        i.metadata_options.as_mut().unwrap().http_endpoint = Some("disabled".into());
        assert!(codes(&i).is_empty());
    }

    #[test]
    fn admin_ports_count_only_with_a_public_address() {
        let mut i = instance();
        i.security_groups = vec![SecurityGroupRef {
            id: "sg-1".into(),
            name: Some("web".into()),
            open_to_world: vec!["tcp/443".into(), "tcp/20-23".into()],
        }];
        assert!(codes(&i).is_empty());

        i.elastic_ip = Some(ElasticIp {
            public_ip: "198.51.100.1".into(),
            allocation_id: Some("eipalloc-1".into()),
        });
        let findings = i.findings();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "ec2.admin_port_public");
        assert!(findings[0].message.contains("tcp/20-23"));
        assert!(!findings[0].message.contains("443"));
    }

    #[test]
    fn admin_port_specs() {
        assert!(exposes_admin_port("all"));
        assert!(exposes_admin_port("tcp/22"));
        assert!(exposes_admin_port("tcp/0-65535"));
        assert!(exposes_admin_port("tcp/3389"));
        assert!(!exposes_admin_port("tcp/443"));
        assert!(!exposes_admin_port("udp/22"));
        assert!(!exposes_admin_port("icmp"));
    }

    #[test]
    fn ami_exposure_findings() {
        let mut i = instance();
        i.amis = vec![
            OwnedAmi {
                id: "ami-pub".into(),
                public: Some(true),
                ..OwnedAmi::default()
            },
            OwnedAmi {
                id: "ami-shared".into(),
                public: Some(false),
                shared_with: vec!["111122223333".into()],
                ..OwnedAmi::default()
            },
        ];
        let findings = i.findings();
        let public = findings
            .iter()
            .find(|f| f.code == "ec2.ami_public")
            .unwrap();
        assert_eq!(public.severity, Severity::High);
        assert!(public.message.contains("ami-pub"));
        let shared = findings
            .iter()
            .find(|f| f.code == "ec2.ami_shared")
            .unwrap();
        assert!(shared.message.contains("ami-shared (1)"));
    }

    #[test]
    fn deep_attribute_findings_need_deep_data() {
        let mut i = instance();
        assert!(codes(&i).is_empty());
        i.termination_protection = Some(false);
        i.shutdown_behavior = Some("terminate".into());
        assert_eq!(
            codes(&i),
            vec!["ec2.termination_protection_off", "ec2.shutdown_terminates"]
        );
        i.termination_protection = Some(true);
        i.shutdown_behavior = Some("stop".into());
        assert!(codes(&i).is_empty());
    }

    #[test]
    fn status_ssm_and_mssql_findings() {
        let mut i = instance();
        i.system_status = Some("ok".into());
        i.instance_status = Some("impaired".into());
        i.ssm_managed = Some(false);
        i.mssql = Some(MSSQL_LICENSE_INCLUDED.into());
        let findings = i.findings();
        let found: Vec<_> = findings
            .iter()
            .map(|f| (f.code.as_str(), f.severity))
            .collect();
        assert_eq!(
            found,
            vec![
                ("ec2.status_impaired", Severity::Medium),
                ("ec2.not_ssm_managed", Severity::Info),
                ("ec2.mssql_candidate", Severity::Info),
            ]
        );
        assert_eq!(findings[0].message, "instance status check impaired");

        i.ssm_managed = None;
        i.mssql = None;
        assert_eq!(codes(&i), vec!["ec2.status_impaired"]);
    }

    #[test]
    fn older_json_still_loads() {
        let json = r#"{"id":"i-1","arn":"arn","region":"us-east-1","state":"running",
            "instance_type":"t3.micro","volumes":["vol-1"],"tags":{},"protection":{}}"#;
        let i: Ec2Instance = serde_json::from_str(json).unwrap();
        assert_eq!(i.eni_count, 0);
        assert!(i.amis.is_empty());
        assert!(i.findings().is_empty());
    }
}
