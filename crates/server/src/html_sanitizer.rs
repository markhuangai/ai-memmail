use std::borrow::Cow;
use std::collections::HashSet;

use ammonia::{Builder, UrlRelative};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedHtml {
    pub html: String,
    pub text: String,
    pub changed: bool,
    pub visually_empty: bool,
}

pub fn sanitize_email_html(input: &str) -> SanitizedHtml {
    let html = email_html_builder().clean(input).to_string();
    let text = html2text::from_read(html.as_bytes(), 80)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    SanitizedHtml {
        changed: html != input,
        visually_empty: text.trim().is_empty() && !html_contains_alt_text(&html),
        html,
        text,
    }
}

fn email_html_builder() -> Builder<'static> {
    let mut builder = Builder::default();
    builder
        .tags(hash_set(&[
            "a",
            "b",
            "blockquote",
            "br",
            "code",
            "col",
            "colgroup",
            "div",
            "em",
            "i",
            "img",
            "li",
            "ol",
            "p",
            "pre",
            "span",
            "strong",
            "table",
            "tbody",
            "td",
            "tfoot",
            "th",
            "thead",
            "tr",
            "u",
            "ul",
        ]))
        .generic_attributes(hash_set(&["style", "title"]))
        .add_tag_attributes("a", &["href"])
        .add_tag_attributes("img", &["src", "alt", "height", "width"])
        .add_tag_attributes("table", &["border", "cellpadding", "cellspacing", "width"])
        .add_tag_attributes("td", &["align", "colspan", "rowspan", "width"])
        .add_tag_attributes("th", &["align", "colspan", "rowspan", "scope", "width"])
        .url_schemes(hash_set(&["http", "https", "mailto", "tel"]))
        .url_relative(UrlRelative::Deny)
        .filter_style_properties(hash_set(&[
            "background-color",
            "border",
            "border-bottom",
            "border-collapse",
            "border-left",
            "border-right",
            "border-top",
            "color",
            "font-family",
            "font-size",
            "font-style",
            "font-weight",
            "line-height",
            "margin",
            "margin-bottom",
            "margin-left",
            "margin-right",
            "margin-top",
            "padding",
            "padding-bottom",
            "padding-left",
            "padding-right",
            "padding-top",
            "text-align",
            "text-decoration",
            "vertical-align",
            "white-space",
            "width",
        ]))
        .attribute_filter(|element, attribute, value| match (element, attribute) {
            ("img", "src") if !is_https_url(value) => None,
            ("img", "alt") if value.trim().is_empty() => None,
            (_, name) if name.starts_with("on") => None,
            _ => Some(Cow::Borrowed(value)),
        });
    builder
}

fn hash_set(values: &[&'static str]) -> HashSet<&'static str> {
    values.iter().copied().collect()
}

fn is_https_url(value: &str) -> bool {
    value
        .trim_start()
        .get(.."https://".len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

fn html_contains_alt_text(html: &str) -> bool {
    html.split("<img").skip(1).any(|chunk| {
        let Some(tag) = chunk.split('>').next() else {
            return false;
        };
        let has_src = tag
            .split("src=\"")
            .nth(1)
            .and_then(|value| value.split('"').next())
            .is_some_and(is_https_url);
        let has_alt = tag
            .split("alt=\"")
            .nth(1)
            .and_then(|value| value.split('"').next())
            .is_some_and(|alt| !alt.trim().is_empty());
        has_src && has_alt
    })
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn plain_text_to_html(value: &str) -> String {
    escape_html(value)
        .replace("\r\n", "\n")
        .replace(['\r', '\n'], "<br>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_strips_scripts_events_and_unsafe_urls() {
        let sanitized = sanitize_email_html(
            r#"<p onclick="x()">Hi<script>alert(1)</script><a href="javascript:x">bad</a><img src="http://x" alt="Logo"></p>"#,
        );

        assert!(!sanitized.html.contains("script"));
        assert!(!sanitized.html.contains("onclick"));
        assert!(!sanitized.html.contains("javascript"));
        assert!(!sanitized.html.contains("http://x"));
        assert!(sanitized.changed);
        assert!(!sanitized.visually_empty);
    }

    #[test]
    fn sanitizer_keeps_email_safe_tables_and_inline_styles() {
        let sanitized = sanitize_email_html(
            r#"<table style="border-collapse: collapse; position: fixed"><tr><td style="color: red">Hi</td></tr></table>"#,
        );

        assert!(sanitized.html.contains("<table"));
        assert!(sanitized.html.contains("border-collapse:collapse"));
        assert!(sanitized.html.contains("color:red"));
        assert!(!sanitized.html.contains("position"));
    }
}
