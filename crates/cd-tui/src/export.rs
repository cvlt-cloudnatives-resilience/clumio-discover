use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn csv_line<S: AsRef<str>>(fields: &[S]) -> String {
    fields
        .iter()
        .map(|f| {
            let f = f.as_ref();
            if f.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", f.replace('"', "\"\""))
            } else {
                f.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn write_view(
    dir: &Path,
    stem: &str,
    stamp: &str,
    header: &[&str],
    rows: &[Vec<String>],
    json: Vec<Value>,
) -> io::Result<(PathBuf, PathBuf)> {
    let base = dir.join(format!("clumio-discover-{stem}-{stamp}"));
    let csv_path = base.with_extension("csv");
    let json_path = base.with_extension("json");

    let mut csv = csv_line(header);
    csv.push('\n');
    for row in rows {
        csv.push_str(&csv_line(row));
        csv.push('\n');
    }
    std::fs::write(&csv_path, csv)?;
    let body = serde_json::to_string_pretty(&Value::Array(json)).map_err(io::Error::other)?;
    std::fs::write(&json_path, format!("{body}\n"))?;
    Ok((csv_path, json_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(
            csv_line(&["plain", "a,b", "say \"hi\"", "two\nlines"]),
            "plain,\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\""
        );
    }
}
