use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::theme;

pub fn json_lines(value: &Value) -> Vec<Line<'static>> {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
    let mut lines = Vec::new();
    let mut block: Option<usize> = None;
    for raw in pretty.lines() {
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        let opens = block.is_none() && trimmed.starts_with("\"protection\":");
        if opens {
            block = Some(indent);
        }
        let highlighted = block.is_some();
        let closes = match block {
            Some(_) if opens => !(trimmed.ends_with('{') || trimmed.ends_with('[')),
            Some(level) => {
                indent == level && (trimmed.starts_with('}') || trimmed.starts_with(']'))
            }
            None => false,
        };

        let mut spans = vec![if highlighted {
            Span::styled("▌", Style::new().fg(theme::CROCUS_BRIGHT))
        } else {
            Span::raw(" ")
        }];
        spans.push(Span::raw(" ".repeat(indent)));
        spans.extend(colour(trimmed));
        let mut line = Line::from(spans);
        if highlighted {
            line = line.style(Style::new().bg(theme::ACCENT_SOFT));
        }
        lines.push(line);
        if closes {
            block = None;
        }
    }
    lines
}

fn colour(text: &str) -> Vec<Span<'static>> {
    let (key, value) = match split_key(text) {
        Some((key, value)) => (Some(key), value),
        None => (None, text),
    };
    let mut spans = Vec::new();
    if let Some(key) = key {
        let style = if key == "\"protection\"" || key == "\"status\"" {
            Style::new()
                .fg(theme::CROCUS_BRIGHT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::LAVENDER)
        };
        spans.push(Span::styled(key.to_string(), style));
        spans.push(Span::styled(": ", Style::new().fg(theme::DIM)));
    }
    let body = value.trim_end_matches(',');
    let comma = &value[body.len()..];
    let style = match body {
        "\"aws_backup\"" => Style::new().fg(theme::OK),
        "\"native_only\"" => Style::new().fg(theme::WARN),
        "\"unprotected\"" => Style::new().fg(theme::PINK),
        _ if body.starts_with('"') => Style::new().fg(theme::TEXT),
        "true" | "false" | "null" => Style::new().fg(theme::INDIGO),
        _ if body.starts_with(['{', '}', '[', ']']) => Style::new().fg(theme::DIM),
        _ => Style::new().fg(theme::GREY),
    };
    spans.push(Span::styled(body.to_string(), style));
    if !comma.is_empty() {
        spans.push(Span::styled(comma.to_string(), Style::new().fg(theme::DIM)));
    }
    spans
}

fn split_key(text: &str) -> Option<(&str, &str)> {
    if !text.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in text.char_indices().skip(1) {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => {
                let rest = &text[i + 1..];
                return rest.strip_prefix(": ").map(|value| (&text[..=i], value));
            }
            _ => escaped = false,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn highlights_only_the_protection_block() {
        let value = json!({
            "name": "a",
            "protection": {"status": "aws_backup", "native": [{"kind": "ami"}]},
            "tags": {"k": "v"}
        });
        let lines = json_lines(&value);
        let marked: Vec<String> = lines
            .iter()
            .filter(|l| l.spans[0].content == "▌")
            .map(|l| plain(l).trim_start_matches('▌').trim().to_string())
            .collect();
        assert_eq!(
            marked.first().map(String::as_str),
            Some("\"protection\": {")
        );
        assert_eq!(marked.last().map(String::as_str), Some("},"));
        assert!(!marked.iter().any(|l| l.contains("\"tags\"")));
        assert!(!marked.iter().any(|l| l.contains("\"name\"")));
    }

    #[test]
    fn splits_keys_with_escaped_quotes() {
        assert_eq!(split_key(r#""a\"b": 1,"#), Some((r#""a\"b""#, "1,")));
        assert_eq!(split_key("{"), None);
    }
}
