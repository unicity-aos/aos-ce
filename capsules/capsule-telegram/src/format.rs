//! Markdown to Telegram HTML conversion and text chunking.
//!
//! Telegram supports a limited HTML subset: `<b>`, `<i>`, `<code>`, `<pre>`,
//! `<a href="...">`. This module converts LLM markdown output into that format.

use regex::Regex;
use std::sync::LazyLock;

/// Escape text for safe inclusion in Telegram HTML.
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Reverse common HTML entity escapes.
fn html_unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Convert LLM markdown to Telegram HTML.
///
/// Handles: bold, italic, inline code, code blocks, links, headings.
pub fn md_to_telegram_html(md: &str) -> String {
    static CODE_BLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"```(\w*)\n?([\s\S]*?)```").expect("invalid regex"));
    static INLINE_CODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("invalid regex"));
    static BOLD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("invalid regex"));
    static ITALIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([^*]|^)\*([^*]+)\*([^*]|$)").expect("invalid regex"));
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("invalid regex"));
    static HEADING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+(.+)$").expect("invalid regex"));

    // First pass: extract code blocks to protect them from other transforms.
    let mut protected: Vec<String> = Vec::new();
    let text = CODE_BLOCK.replace_all(md, |caps: &regex::Captures<'_>| {
        let code = html_escape(&caps[2]);
        let placeholder = format!("\x00CODE{}\x00", protected.len());
        protected.push(format!("<pre>{code}</pre>"));
        placeholder
    });

    // Second pass: extract inline code.
    let text = INLINE_CODE.replace_all(&text, |caps: &regex::Captures<'_>| {
        let code = html_escape(&caps[1]);
        let placeholder = format!("\x00CODE{}\x00", protected.len());
        protected.push(format!("<code>{code}</code>"));
        placeholder
    });

    // Escape HTML in non-code sections.
    let text = html_escape(&text);

    // Apply inline transforms.
    let text = BOLD.replace_all(&text, "<b>$1</b>");
    let text = ITALIC.replace_all(&text, "$1<i>$2</i>$3");
    let text = LINK.replace_all(&text, |caps: &regex::Captures<'_>| {
        let label = &caps[1];
        // The URL has been HTML-escaped by the html_escape() call above,
        // so we must unescape it before placing it inside href="...".
        let url = html_unescape(&caps[2]);
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("tg://")
            || url.starts_with("mailto:")
        {
            let escaped_url = html_escape(&url);
            format!("<a href=\"{escaped_url}\">{label}</a>")
        } else {
            format!("{label} ({})", html_escape(&url))
        }
    });
    let text = HEADING.replace_all(&text, "<b>$1</b>");

    // Restore protected regions.
    let mut text = text.into_owned();
    for (i, block) in protected.iter().enumerate() {
        let placeholder = format!("\x00CODE{i}\x00");
        text = text.replace(&placeholder, block);
    }

    text
}

/// Find a safe truncation boundary in HTML at or before `max_len`.
///
/// Walks backwards to avoid cutting inside a tag or entity.
pub fn find_safe_html_boundary(html: &str, max_len: usize) -> usize {
    let mut boundary = html.floor_char_boundary(max_len.min(html.len()));

    while boundary > 0 {
        let bytes = &html.as_bytes()[..boundary];
        let last_open = bytes.iter().rposition(|&b| b == b'<');
        let last_close = bytes.iter().rposition(|&b| b == b'>');
        let inside_tag = match (last_open, last_close) {
            (Some(lt), Some(gt)) => lt > gt,
            (Some(_), None) => true,
            _ => false,
        };
        let last_amp = bytes.iter().rposition(|&b| b == b'&');
        let last_semi = bytes.iter().rposition(|&b| b == b';');
        let inside_entity = match (last_amp, last_semi) {
            (Some(amp), Some(semi)) => amp > semi,
            (Some(_), None) => true,
            _ => false,
        };
        if !inside_tag && !inside_entity {
            break;
        }
        boundary = html.floor_char_boundary(boundary.saturating_sub(1));
    }

    boundary
}

/// Close any unclosed HTML tags in a truncated fragment.
pub fn close_open_tags(html: &str) -> String {
    use std::fmt::Write as _;

    static TAG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<(/?)(\w+)[^>]*>").expect("invalid regex"));

    let mut open_tags: Vec<String> = Vec::new();
    for cap in TAG_RE.captures_iter(html) {
        let is_close = &cap[1] == "/";
        let tag_name = cap[2].to_lowercase();
        if is_close {
            if let Some(pos) = open_tags.iter().rposition(|t| *t == tag_name) {
                open_tags.remove(pos);
            }
        } else {
            open_tags.push(tag_name);
        }
    }

    if open_tags.is_empty() {
        return html.to_string();
    }

    let mut result = html.to_string();
    for tag in open_tags.into_iter().rev() {
        let _ = write!(result, "</{tag}>");
    }
    result
}

/// Truncate HTML for in-progress Telegram message edits.
///
/// Stays within Telegram's 4096-byte limit and ensures valid HTML.
pub fn truncate_for_edit(html: &str) -> String {
    const MAX_HTML_LEN: usize = 4000;
    const TRUNCATED_TARGET: usize = 3940;

    if html.len() <= MAX_HTML_LEN {
        html.to_string()
    } else {
        let boundary = find_safe_html_boundary(html, TRUNCATED_TARGET);
        let mut s = close_open_tags(&html[..boundary]);
        s.push_str("...");
        s
    }
}

/// Split HTML into chunks that fit within Telegram's message size limit.
///
/// HTML-aware: avoids splitting inside tags or entities, and closes any
/// unclosed tags in each chunk.
pub fn chunk_html(html: &str, max_len: usize) -> Vec<String> {
    const CLOSING_TAG_HEADROOM: usize = 50;

    let max_len = if max_len == 0 { 4000 } else { max_len };
    let split_limit = max_len.saturating_sub(CLOSING_TAG_HEADROOM);

    if html.len() <= max_len {
        return vec![html.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = html;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let hard_cut = find_safe_html_boundary(remaining, split_limit);
        let split_at = find_split_point(remaining, hard_cut, "\n\n")
            .or_else(|| find_split_point(remaining, hard_cut, "\n"))
            .unwrap_or(hard_cut);

        let split_at = if split_at == 0 {
            remaining.floor_char_boundary(max_len.max(1))
        } else {
            split_at
        };

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(close_open_tags(chunk));
        remaining = rest.trim_start_matches('\n');
    }

    chunks
}

fn find_split_point(text: &str, boundary: usize, delimiter: &str) -> Option<usize> {
    let search_region = &text[..boundary];
    search_region
        .rfind(delimiter)
        .map(|pos| pos.saturating_add(delimiter.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("<b>&test</b>"), "&lt;b&gt;&amp;test&lt;/b&gt;");
    }

    #[test]
    fn html_unescape_roundtrip() {
        let original = "<b>&test</b>";
        assert_eq!(html_unescape(&html_escape(original)), original);
    }

    #[test]
    fn md_bold() {
        let result = md_to_telegram_html("Hello **world**");
        assert!(result.contains("<b>world</b>"));
    }

    #[test]
    fn md_code_block_escapes_html() {
        let result = md_to_telegram_html("```\n<div>test</div>\n```");
        assert!(result.contains("&lt;div&gt;"));
    }

    #[test]
    fn md_inline_code_not_affected_by_bold() {
        let result = md_to_telegram_html("Use `**not bold**` here");
        assert!(result.contains("<code>**not bold**</code>"));
    }

    #[test]
    fn md_link_unsafe_scheme_rejected() {
        let result = md_to_telegram_html("Click [here](javascript:alert(1))");
        assert!(!result.contains("<a href"));
    }

    #[test]
    fn md_link_url_not_double_escaped() {
        let result = md_to_telegram_html("Visit [docs](https://example.com/search?a=1&b=2)");
        // The href should contain properly escaped &amp; (single level), not &amp;amp;
        assert!(
            result.contains("href=\"https://example.com/search?a=1&amp;b=2\""),
            "got: {result}"
        );
        // Must NOT contain double-escaped entities.
        assert!(!result.contains("&amp;amp;"), "double-escaped: {result}");
    }

    #[test]
    fn md_link_preserves_valid_url() {
        let result = md_to_telegram_html("[click](https://example.com/path)");
        assert!(result.contains("<a href=\"https://example.com/path\">click</a>"));
    }

    #[test]
    fn close_open_tags_nested() {
        assert_eq!(close_open_tags("<b><i>text"), "<b><i>text</i></b>");
    }

    #[test]
    fn truncate_for_edit_short_text() {
        assert_eq!(truncate_for_edit("Hello world"), "Hello world");
    }

    #[test]
    fn truncate_for_edit_long_text() {
        let text = "x".repeat(5000);
        let result = truncate_for_edit(&text);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 4000);
    }

    #[test]
    fn chunk_html_short() {
        let chunks = chunk_html("<b>hello</b>", 100);
        assert_eq!(chunks.len(), 1);
    }
}
