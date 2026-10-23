use serde_json::json;

pub fn document() -> String {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [{
            "Sid": "ClumioDiscoverReadOnly",
            "Effect": "Allow",
            "Action": crate::actions(),
            "Resource": "*"
        }]
    });
    serde_json::to_string_pretty(&policy).expect("policy serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY_FILE: &str = include_str!("../../../policy/clumio-discover-readonly.json");

    #[test]
    fn checked_in_policy_matches_the_generated_document() {
        assert_eq!(
            POLICY_FILE.trim(),
            document().trim(),
            "regenerate policy/clumio-discover-readonly.json with `clumio-discover policy`"
        );
    }
}
