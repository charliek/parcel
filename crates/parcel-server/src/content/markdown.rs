//! Markdown pages (the entry, and any other page opened as one), rendered
//! to a styled HTML document.
//!
//! Raw HTML inside the Markdown is passed through: the result is served on
//! the sandboxed content origin like any other parcel page, so it has no more
//! power than an HTML parcel would.

use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const STYLE: &str = r#"
:root { color-scheme: light dark; --fg:#1f2328; --muted:#59636e; --bg:#ffffff; --border:#d1d9e0; --code-bg:#f6f8fa; --link:#0969da; }
@media (prefers-color-scheme: dark) { :root { --fg:#e6edf3; --muted:#9198a1; --bg:#0d1117; --border:#3d444d; --code-bg:#151b23; --link:#4493f8; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif; }
main { max-width: 860px; margin: 0 auto; padding: 32px 16px 64px; }
h1,h2 { border-bottom:1px solid var(--border); padding-bottom:.3em; }
h1,h2,h3,h4 { line-height:1.25; margin:1.5em 0 .6em; }
a { color:var(--link); }
code { font:85% ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; background:var(--code-bg); padding:.2em .4em; border-radius:6px; }
pre { background:var(--code-bg); padding:16px; border-radius:8px; overflow:auto; }
pre code { background:none; padding:0; }
blockquote { margin:0; padding:0 1em; color:var(--muted); border-left:.25em solid var(--border); }
table { border-collapse:collapse; display:block; overflow:auto; }
th,td { border:1px solid var(--border); padding:6px 13px; }
img { max-width:100%; }
hr { border:0; border-top:1px solid var(--border); margin:24px 0; }
"#;

/// The Markdown dialect pages are written in (search indexes the same one).
pub const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_FOOTNOTES);

/// Render Markdown into a complete HTML document.
pub fn render(markdown: &str, fallback_title: &str) -> String {
    let title = first_heading(markdown).unwrap_or_else(|| fallback_title.to_string());
    let mut body = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut body, Parser::new_ext(markdown, OPTIONS));
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body><main>\n{body}</main></body></html>\n",
        escape_html(&title)
    )
}

/// Text of the first level-1 heading, if any (also used as the default title).
pub fn first_heading(markdown: &str) -> Option<String> {
    let mut in_h1 = false;
    let mut text = String::new();
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => in_h1 = true,
            Event::End(TagEnd::Heading(HeadingLevel::H1)) if in_h1 => {
                let t = text.trim().to_string();
                return (!t.is_empty()).then_some(t);
            }
            Event::Text(t) | Event::Code(t) if in_h1 => text.push_str(&t),
            _ => {}
        }
    }
    None
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_document() {
        let html = render(
            "# Tom & \"Jerry\"\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
            "x",
        );
        assert!(html.starts_with("<!doctype html>"));
        assert!(
            html.contains("<title>Tom &amp; &quot;Jerry&quot;</title>"),
            "{html}"
        );
        assert!(html.contains("<table>"));
    }

    #[test]
    fn falls_back_to_given_title() {
        assert!(render("no heading", "notes.md").contains("<title>notes.md</title>"));
        assert_eq!(first_heading("## two\n# One `x`"), Some("One x".into()));
    }
}
