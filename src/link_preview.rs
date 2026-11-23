//! Aperçu d'un lien à la rédaction : titre, description et image de la
//! page, lus dans ses balises Open Graph (`og:title`…), sinon `<title>`.

use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;

use crate::photo::{self, Prepared};

/// Page lue au plus : l'en-tête suffit.
const MAX_HTML: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    pub uri: String,
    pub title: String,
    pub description: String,
    /// Adresse de l'image, absolue.
    pub image: Option<String>,
}

fn meta_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"(?is)<meta\s[^>]*>"#).unwrap())
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r#"(?is)\b{}\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
        regex::escape(name)
    ))
    .ok()?;
    let captures = pattern.captures(tag)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .map(|m| m.as_str().to_owned())
}

/// Entités HTML courantes des balises.
fn decode(text: &str) -> String {
    let mut out = text
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}");
    out = out.replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Adresse absolue de `link`, vue depuis la page `base`.
pub fn absolute(link: &str, base: &str) -> Option<String> {
    let link = link.trim();
    if link.starts_with("http://") || link.starts_with("https://") {
        return Some(link.to_owned());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(rest) = link.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    let host = rest.split(['/', '?', '#']).next()?;
    if link.starts_with('/') {
        return Some(format!("{scheme}://{host}{link}"));
    }
    // Relative au dossier de la page.
    let path = rest[host.len()..].split(['?', '#']).next().unwrap_or("");
    let folder = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(0)];
    let folder = if folder.is_empty() { "/" } else { folder };
    Some(format!("{scheme}://{host}{folder}{link}"))
}

/// Aperçu tiré du HTML de la page `uri`.
pub fn parse(html: &str, uri: &str) -> Preview {
    let mut preview = Preview {
        uri: uri.to_owned(),
        ..Default::default()
    };
    let (mut title, mut description, mut image) = (None, None, None);
    let (mut plain_description, mut twitter_image) = (None, None);
    for tag in meta_pattern().find_iter(html) {
        let tag = tag.as_str();
        let key = attribute(tag, "property")
            .or_else(|| attribute(tag, "name"))
            .unwrap_or_default()
            .to_lowercase();
        let Some(content) = attribute(tag, "content") else {
            continue;
        };
        let slot = match key.as_str() {
            "og:title" => &mut title,
            "og:description" => &mut description,
            "og:image" | "og:image:url" | "og:image:secure_url" => &mut image,
            "description" => &mut plain_description,
            "twitter:image" => &mut twitter_image,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(decode(&content));
        }
    }
    if title.is_none() {
        static TITLE: OnceLock<Regex> = OnceLock::new();
        let pattern = TITLE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
        title = pattern.captures(html).map(|c| decode(&c[1]));
    }
    preview.title = title.unwrap_or_default();
    preview.description = description.or(plain_description).unwrap_or_default();
    preview.image = image
        .or(twitter_image)
        .and_then(|link| absolute(&link, uri));
    preview
}

/// Lit la page et prépare son image (redressée, réduite, sous 1 Mo).
pub async fn fetch(uri: String) -> Result<(Preview, Option<Prepared>), String> {
    let mut response = crate::images::client()
        .get(&uri)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.contains("html"));
    if !is_html {
        return Err("pas une page HTML".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= MAX_HTML {
            break;
        }
    }
    let html = String::from_utf8_lossy(&bytes);
    let preview = parse(&html, response.url().as_str());
    let preview = Preview { uri, ..preview };
    if preview.title.is_empty() && preview.description.is_empty() {
        return Err("aucun titre".into());
    }
    let image = match &preview.image {
        Some(url) => match crate::images::download(url).await {
            Ok(bytes) => tokio::task::spawn_blocking(move || photo::prepare(&bytes))
                .await
                .ok()
                .and_then(Result::ok),
            Err(_) => None,
        },
        None => None,
    };
    Ok((preview, image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_tags_are_read() {
        let html = r#"<html><head>
            <title>Titre de secours</title>
            <meta property="og:title" content="Ce que Orca lit &amp; dit">
            <meta content='Une description.' property='og:description'>
            <meta name="description" content="Autre description">
            <meta property="og:image" content="/images/carte.png">
        </head></html>"#;
        let preview = parse(html, "https://example.org/blog/billet");
        assert_eq!(preview.title, "Ce que Orca lit & dit");
        assert_eq!(preview.description, "Une description.");
        assert_eq!(
            preview.image.as_deref(),
            Some("https://example.org/images/carte.png")
        );
    }

    #[test]
    fn title_and_description_have_fallbacks() {
        let html = "<title>\n  Page   simple </title><meta name=\"description\" content=\"Texte\">";
        let preview = parse(html, "https://example.org/");
        assert_eq!(preview.title, "Page simple");
        assert_eq!(preview.description, "Texte");
        assert_eq!(preview.image, None);
    }

    #[test]
    fn relative_links_become_absolute() {
        let base = "https://example.org/blog/billet?x=1";
        assert_eq!(
            absolute("//cdn.example/a.png", base).unwrap(),
            "https://cdn.example/a.png"
        );
        assert_eq!(
            absolute("a.png", base).unwrap(),
            "https://example.org/blog/a.png"
        );
        assert_eq!(
            absolute("https://autre.example/b.png", base).unwrap(),
            "https://autre.example/b.png"
        );
        assert_eq!(
            absolute("c.png", "https://example.org").unwrap(),
            "https://example.org/c.png"
        );
    }
}
