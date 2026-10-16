//! Human-readable formatting and the `--json` printer.

use std::io::Write;

use anyhow::{Context, Result};
use time::OffsetDateTime;

/// `512 B`, `12.3 KiB`, `4.0 MiB`.
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = UNITS[0];
    for next in &UNITS[1..] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

pub fn format_size(n: i64) -> String {
    format_bytes(u64::try_from(n).unwrap_or(0))
}

/// `just now`, `5m ago`, `3h ago`, `12d ago`, then the date.
pub fn format_age(at: OffsetDateTime, now: OffsetDateTime) -> String {
    let secs = (now - at).whole_seconds();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{days}d ago");
    }
    let date = at.date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

pub fn age(at: OffsetDateTime) -> String {
    format_age(at, OffsetDateTime::now_utc())
}

/// Shorten to `max` characters with a trailing `…`.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Left-aligned columns separated by two spaces; the last column is not padded.
pub fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let columns = header.len();
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(columns) {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut push_row = |cells: Vec<&str>| {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                line.push_str(cell);
            } else {
                line.push_str(cell);
                let pad = widths[i] - cell.chars().count() + 2;
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    };
    push_row(header.to_vec());
    for row in rows {
        push_row(row.iter().map(String::as_str).collect());
    }
    out
}

/// Text that came from other people (comments, names, titles of parcels
/// shared with you): spell out control characters other than line breaks
/// and tabs, so it can't drive the terminal (ESC/OSC sequences that rewrite
/// the screen or the clipboard). The server refuses them in comment bodies;
/// this also covers names, titles and anything an older server let through.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

/// Pretty-print a JSON body exactly as the server sent it (same keys, same
/// order), followed by a newline.
pub fn print_json(raw: &[u8]) -> Result<()> {
    let mut out = Vec::with_capacity(raw.len() * 2);
    {
        let mut de = serde_json::Deserializer::from_slice(raw);
        let mut ser = serde_json::Serializer::pretty(&mut out);
        serde_transcode::transcode(&mut de, &mut ser).context("server sent invalid JSON")?;
        de.end().context("server sent invalid JSON")?;
    }
    out.push(b'\n');
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&out)?;
    stdout.flush()?;
    Ok(())
}

/// Write text to stdout, surfacing a closed pipe as an error `main` ignores.
pub fn print(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(16 * 1024 * 1024), "16.0 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
        assert_eq!(format_size(-5), "0 B");
    }

    #[test]
    fn formats_ages() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(400);
        assert_eq!(format_age(now - Duration::seconds(5), now), "just now");
        assert_eq!(format_age(now - Duration::minutes(5), now), "5m ago");
        assert_eq!(format_age(now - Duration::hours(30), now), "30h ago");
        assert_eq!(format_age(now - Duration::days(3), now), "3d ago");
        assert_eq!(format_age(OffsetDateTime::UNIX_EPOCH, now), "1970-01-01");
    }

    #[test]
    fn lays_out_tables() {
        let rows = vec![
            vec!["abc".to_string(), "A title".to_string(), "1".to_string()],
            vec!["defghi".to_string(), "T".to_string(), "".to_string()],
        ];
        assert_eq!(
            table(&["ID", "TITLE", "V"], &rows),
            "ID      TITLE    V\nabc     A title  1\ndefghi  T\n"
        );
    }

    #[test]
    fn control_characters_are_spelled_out() {
        assert_eq!(
            printable("hi\u{1b}]52;c;SGVsbG8=\u{7}\r\n\tok\u{9b}2J"),
            "hi\\u{1b}]52;c;SGVsbG8=\\u{7}\\u{d}\n\tok\\u{9b}2J"
        );
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a longer title", 8), "a longe…");
    }
}
