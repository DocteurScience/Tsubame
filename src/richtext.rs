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

/// Adresse interne d'un lien vers un mot-dièse.
pub const TAG_LINK: &str = "tsubame-tag:";
/// Adresse interne d'un lien vers un profil (DID ou identifiant).
pub const PROFILE_LINK: &str = "tsubame-profile:";

/// Facettes d'un post reçu (champ `facets` de son enregistrement). Les
/// mentions portent le DID du compte.
pub fn from_record(record: &serde_json::Value) -> Vec<Facet> {
    let Some(facets) = record.get("facets").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    facets
        .iter()
        .filter_map(|facet| {
            let index = facet.get("index")?;
            let start = usize::try_from(index.get("byteStart")?.as_u64()?).ok()?;
            let end = usize::try_from(index.get("byteEnd")?.as_u64()?).ok()?;
            let feature = facet.get("features")?.as_array()?.first()?;
            let feature = match feature.get("$type")?.as_str()? {
                "app.bsky.richtext.facet#link" => {
                    Feature::Link(feature.get("uri")?.as_str()?.to_owned())
                }
                "app.bsky.richtext.facet#tag" => {
                    Feature::Tag(feature.get("tag")?.as_str()?.to_owned())
                }
                "app.bsky.richtext.facet#mention" => {
                    Feature::Mention(feature.get("did")?.as_str()?.to_owned())
                }
                _ => return None,
            };
            Some(Facet {
                start,
                end,
                feature,
            })
        })
        .collect()
}

/// Texte en balisage Pango, les facettes devenant des liens : adresse web,
/// ou adresse interne (`TAG_LINK`, `PROFILE_LINK`). Une facette mal placée
/// (hors du texte, au milieu d'un caractère, chevauchant une autre) reste
/// du texte simple.
pub fn markup(text: &str, facets: &[Facet]) -> String {
    use gtk::glib::markup_escape_text as escape;

    let mut facets: Vec<&Facet> = facets.iter().collect();
    facets.sort_by_key(|f| f.start);
    let mut out = String::new();
    let mut at = 0;
    for facet in facets {
        if facet.start < at
            || facet.end <= facet.start
            || facet.end > text.len()
            || !text.is_char_boundary(facet.start)
            || !text.is_char_boundary(facet.end)
        {
            continue;
        }
        let href = match &facet.feature {
            Feature::Link(uri) if uri.starts_with("https://") || uri.starts_with("http://") => {
                uri.clone()
            }
            Feature::Link(_) => continue,
            Feature::Tag(tag) => format!("{TAG_LINK}{tag}"),
            Feature::Mention(actor) => format!("{PROFILE_LINK}{actor}"),
        };
        out.push_str(&escape(&text[at..facet.start]));
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape(&href),
            escape(&text[facet.start..facet.end])
        ));
        at = facet.end;
    }
    out.push_str(&escape(&text[at..]));
    out
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

    #[test]
    fn received_facets_become_links() {
        let text = "Salut @lucie ! #GNOME <3 https://gnome.org";
        let record = serde_json::json!({"text": text, "facets": [
            {"index": {"byteStart": 6, "byteEnd": 12},
             "features": [{"$type": "app.bsky.richtext.facet#mention", "did": "did:plc:lucie"}]},
            {"index": {"byteStart": 15, "byteEnd": 21},
             "features": [{"$type": "app.bsky.richtext.facet#tag", "tag": "GNOME"}]},
            {"index": {"byteStart": 25, "byteEnd": 42},
             "features": [{"$type": "app.bsky.richtext.facet#link", "uri": "https://gnome.org"}]},
            {"index": {"byteStart": 8, "byteEnd": 14},
             "features": [{"$type": "app.bsky.richtext.facet#link", "uri": "https://chevauche.example"}]},
        ]});
        let facets = from_record(&record);
        assert_eq!(facets.len(), 4);
        assert_eq!(
            markup(text, &facets),
            "Salut <a href=\"tsubame-profile:did:plc:lucie\">@lucie</a> ! \
             <a href=\"tsubame-tag:GNOME\">#GNOME</a> &lt;3 \
             <a href=\"https://gnome.org\">https://gnome.org</a>"
        );
    }

    #[test]
    fn bad_facets_stay_text() {
        let text = "é";
        let facets = [Facet {
            start: 1,
            end: 2,
            feature: Feature::Link("javascript:alert(1)".into()),
        }];
        assert_eq!(markup(text, &facets), "é");
    }
}
