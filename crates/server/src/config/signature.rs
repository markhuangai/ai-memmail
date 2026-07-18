use super::{ConfigError, EmailSignatureFormat, MailboxConfig};

pub(super) fn validate_mailbox_signature(mailbox: &MailboxConfig) -> Result<(), ConfigError> {
    let Some(signature) = &mailbox.signature else {
        return Ok(());
    };
    if signature.content.trim().is_empty() {
        return Err(ConfigError::Invalid(format!(
            "mailbox {} signature.content must not be empty",
            mailbox.id
        )));
    }
    if matches!(signature.format, EmailSignatureFormat::Html) {
        if let Some(error) = html_signature_image_error(&signature.content) {
            return Err(ConfigError::Invalid(format!(
                "mailbox {} signature.content {error}",
                mailbox.id
            )));
        }
        if !html_signature_has_visible_content(&signature.content) {
            return Err(ConfigError::Invalid(format!(
                "mailbox {} signature.content must not be visually empty",
                mailbox.id
            )));
        }
    }
    Ok(())
}

fn html_signature_has_visible_content(content: &str) -> bool {
    let mut text = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for character in content.chars() {
        if in_tag {
            if character == '>' {
                if image_tag_has_visible_alt(&tag) {
                    return true;
                }
                tag.clear();
                in_tag = false;
            } else {
                tag.push(character);
            }
            continue;
        }
        if character == '<' {
            in_tag = true;
            continue;
        }
        text.push(character);
    }
    html_text_has_visible_content(&text)
}

fn html_signature_image_error(content: &str) -> Option<&'static str> {
    for tag in html_tags(content) {
        if !is_image_tag(tag) {
            continue;
        }
        let src = html_attribute_value(tag, "src").unwrap_or_default();
        if !is_https_url(&src) {
            return Some("image src must start with https://");
        }
        let alt = html_attribute_value(tag, "alt").unwrap_or_default();
        if !html_text_has_visible_content(&alt) {
            return Some("image alt text must not be empty");
        }
    }
    None
}

fn html_tags(content: &str) -> Vec<&str> {
    let mut tags = Vec::new();
    let mut start = None;
    for (index, character) in content.char_indices() {
        if character == '<' {
            start = Some(index + 1);
            continue;
        }
        if character == '>' {
            if let Some(tag_start) = start.take() {
                tags.push(&content[tag_start..index]);
            }
        }
    }
    tags
}

fn image_tag_has_visible_alt(tag: &str) -> bool {
    is_image_tag(tag)
        && html_attribute_value(tag, "alt")
            .map(|alt| html_text_has_visible_content(&alt))
            .unwrap_or(false)
}

fn is_image_tag(tag: &str) -> bool {
    let trimmed = tag.trim_start();
    if trimmed.starts_with('/') {
        return false;
    }
    let Some(prefix) = trimmed.get(..3) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case("img") {
        return false;
    }
    trimmed
        .chars()
        .nth(3)
        .map(|character| character.is_whitespace() || character == '/')
        .unwrap_or(true)
}

fn html_attribute_value(tag: &str, name: &str) -> Option<String> {
    let trimmed = tag.trim_start();
    if trimmed.starts_with('/') {
        return None;
    }
    let mut rest = trimmed
        .find(char::is_whitespace)
        .map(|index| &trimmed[index..])
        .unwrap_or("");
    while !rest.is_empty() {
        rest = rest.trim_start();
        let equals = rest.find('=')?;
        let key = rest[..equals].trim();
        rest = rest[equals + 1..].trim_start();
        if let Some(value) = rest.strip_prefix('"') {
            let end = value.find('"')?;
            if key.eq_ignore_ascii_case(name) {
                return Some(value[..end].to_string());
            }
            rest = &value[end + 1..];
            continue;
        }
        if let Some(value) = rest.strip_prefix('\'') {
            let end = value.find('\'')?;
            if key.eq_ignore_ascii_case(name) {
                return Some(value[..end].to_string());
            }
            rest = &value[end + 1..];
            continue;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if key.eq_ignore_ascii_case(name) {
            return Some(rest[..end].to_string());
        }
        rest = &rest[end..];
    }
    None
}

fn is_https_url(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed
        .get(.."https://".len())
        .map(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .unwrap_or(false)
        && trimmed.len() > "https://".len()
}

fn html_text_has_visible_content(text: &str) -> bool {
    text.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .chars()
        .any(|character| !character.is_whitespace())
}
