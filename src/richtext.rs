//! Texte d'un post à publier : longueur au sens de Bluesky et repérage des
//! liens, mots-dièse et mentions (facettes).
//!
//! Bluesky limite un post à 300 graphèmes (caractères tels qu'on les voit :
//! un émoji composé compte pour un). Les facettes désignent des morceaux du
//! texte par leurs positions en octets UTF-8.

use std::sync::LazyLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_GRAPHEMES: usize = 300;
/// Longueur maximale d'un mot-dièse.
const MAX_TAG: usize = 64;

pub fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feature {
    Link(String),
    /// Mot-dièse, sans le « # ».
    Tag(String),
    /// Mention, par l'identifiant (sans le « @ ») ; le DID est résolu à la
    /// publication.
    Mention(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facet {
    /// Positions en octets UTF-8 : début inclus, fin exclue.
    pub start: usize,
    pub end: usize,
    pub feature: Feature,
}

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"]+"#).unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)([#＃][^\s#＃]+)").unwrap());
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(@[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?)+)")
        .unwrap()
});

/// Retire la ponctuation finale, qui termine la phrase plutôt que le lien
/// ou le mot-dièse.
fn trim_end_punctuation(text: &str) -> &str {
    text.trim_end_matches(|c: char| ".,;:!?»…\"')]}".contains(c))
}

pub fn detect(text: &str) -> Vec<Facet> {
    let mut facets = Vec::new();
    for m in LINK.find_iter(text) {
        let link = trim_end_punctuation(m.as_str());
        facets.push(Facet {
            start: m.start(),
            end: m.start() + link.len(),
            feature: Feature::Link(link.to_owned()),
        });
    }
    for captures in TAG.captures_iter(text) {
        let m = captures.get(1).unwrap();
        let tag = trim_end_punctuation(m.as_str());
        // Sans le « # » (un ou trois octets selon sa forme).
        let name = tag.trim_start_matches(['#', '＃']);
        let only_digits = name.chars().all(|c| c.is_ascii_digit());
        if name.is_empty() || only_digits || grapheme_count(name) > MAX_TAG {
            continue;
        }
        facets.push(Facet {
            start: m.start(),
            end: m.start() + tag.len(),
            feature: Feature::Tag(name.to_owned()),
        });
    }
    for captures in MENTION.captures_iter(text) {
        let m = captures.get(1).unwrap();
        let mention = m.as_str();
        facets.push(Facet {
            start: m.start(),
            end: m.end(),
            feature: Feature::Mention(mention[1..].to_lowercase()),
        });
    }
    facets.sort_by_key(|facet| facet.start);
    facets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, facet: &Facet) -> String {
        text[facet.start..facet.end].to_owned()
    }

    #[test]
    fn counts_graphemes_not_bytes() {
        assert_eq!(grapheme_count("été"), 3);
        // Famille : plusieurs points de code, un seul graphème.
        assert_eq!(grapheme_count("👨‍👩‍👧"), 1);
    }

    #[test]
    fn finds_links_tags_and_mentions() {
        let text =
            "Merci @lucie.example.social pour #GNOME, voir https://gnome.org/49. Été #2026 !";
        let facets = detect(text);
        let found: Vec<_> = facets
            .iter()
            .map(|f| (slice(text, f), f.feature.clone()))
            .collect();
        assert_eq!(
            found,
            vec![
                (
                    "@lucie.example.social".into(),
                    Feature::Mention("lucie.example.social".into())
                ),
                ("#GNOME".into(), Feature::Tag("GNOME".into())),
                (
                    "https://gnome.org/49".into(),
                    Feature::Link("https://gnome.org/49".into())
                ),
            ]
        );
    }

    #[test]
    fn positions_are_utf8_bytes() {
        let text = "Café #déjà";
        let facets = detect(text);
        assert_eq!(facets.len(), 1);
        // « Café » fait 5 octets ; « #déjà » commence au 7e.
        assert_eq!((facets[0].start, facets[0].end), (6, 13));
        assert_eq!(facets[0].feature, Feature::Tag("déjà".into()));
    }

    #[test]
    fn ignores_what_is_not_a_tag_or_mention() {
        assert!(detect("C#, prix #12 et adresse@exemple.fr").is_empty());
        assert!(detect("#").is_empty());
    }
}
