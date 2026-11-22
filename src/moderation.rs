//! Modération : préférences (contenu pour adultes, étiquettes, mots
//! masqués), comptes masqués et bloqués, filtre des mots masqués.
//!
//! Les préférences Bluesky sont une liste d'éléments typés, remplacée en
//! entier à chaque écriture (`putPreferences`). On relit donc toujours la
//! liste, on ne change que l'élément concerné, et on garde tous les autres
//! tels quels, même ceux que Tsubame ne connaît pas.

use std::cell::RefCell;
use std::sync::Arc;

use gettextrs::{gettext, pgettext};
use serde_json::{Value, json};

use crate::api::{self, ApiError, request_failed};
use crate::auth::Session;
use crate::data::{Actor, Post};

const ADULT: &str = "app.bsky.actor.defs#adultContentPref";
const LABEL: &str = "app.bsky.actor.defs#contentLabelPref";
const MUTED_WORDS: &str = "app.bsky.actor.defs#mutedWordsPref";
const LABELERS: &str = "app.bsky.actor.defs#labelersPref";

/// Ce que Tsubame fait d'un contenu étiqueté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Show,
    Warn,
    Hide,
}

impl Visibility {
    pub const ALL: [Visibility; 3] = [Visibility::Show, Visibility::Warn, Visibility::Hide];

    fn api_name(self) -> &'static str {
        match self {
            Visibility::Show => "ignore",
            Visibility::Warn => "warn",
            Visibility::Hide => "hide",
        }
    }

    fn from_api(name: &str) -> Option<Self> {
        match name {
            "ignore" | "show" => Some(Visibility::Show),
            "warn" => Some(Visibility::Warn),
            "hide" => Some(Visibility::Hide),
            _ => None,
        }
    }

    pub fn title(self) -> String {
        match self {
            Visibility::Show => pgettext("réglage d'une étiquette de contenu", "Afficher"),
            Visibility::Warn => pgettext("réglage d'une étiquette de contenu", "Avertir"),
            Visibility::Hide => pgettext("réglage d'une étiquette de contenu", "Masquer"),
        }
    }
}

/// Étiquettes de contenu réglables, avec leur réglage par défaut chez
/// Bluesky. Les deux dernières ne s'appliquent qu'avec le contenu pour
/// adultes activé.
pub const LABELS: [(&str, Visibility); 4] = [
    ("nudity", Visibility::Show),
    ("graphic-media", Visibility::Warn),
    ("sexual", Visibility::Warn),
    ("porn", Visibility::Hide),
];

pub fn label_title(label: &str) -> String {
    match label {
        "nudity" => gettext("Nudité non sexuelle"),
        "graphic-media" => gettext("Violence graphique"),
        "sexual" => gettext("Contenu sexuel suggestif"),
        "porn" => gettext("Pornographie"),
        other => other.to_owned(),
    }
}

pub fn is_adult_label(label: &str) -> bool {
    matches!(label, "sexual" | "porn")
}

fn of_type<'a>(preferences: &'a [Value], kind: &str) -> impl Iterator<Item = &'a Value> + 'a {
    let kind = kind.to_owned();
    preferences
        .iter()
        .filter(move |item| item.get("$type").and_then(Value::as_str) == Some(kind.as_str()))
}

pub fn adult_content(preferences: &[Value]) -> bool {
    of_type(preferences, ADULT)
        .next()
        .and_then(|item| item.get("enabled")?.as_bool())
        .unwrap_or(false)
}

pub fn set_adult_content(preferences: &mut Vec<Value>, enabled: bool) {
    preferences.retain(|item| item.get("$type").and_then(Value::as_str) != Some(ADULT));
    preferences.push(json!({"$type": ADULT, "enabled": enabled}));
}

/// Réglage d'une étiquette (celles de Bluesky, sans service d'étiquetage
/// tiers), ou son réglage par défaut.
pub fn label_visibility(preferences: &[Value], label: &str) -> Visibility {
    let default = LABELS
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, v)| *v)
        .unwrap_or(Visibility::Warn);
    of_type(preferences, LABEL)
        .find(|item| {
            item.get("label").and_then(Value::as_str) == Some(label)
                && item.get("labelerDid").is_none()
        })
        .and_then(|item| Visibility::from_api(item.get("visibility")?.as_str()?))
        .unwrap_or(default)
}

pub fn set_label_visibility(preferences: &mut Vec<Value>, label: &str, visibility: Visibility) {
    preferences.retain(|item| {
        !(item.get("$type").and_then(Value::as_str) == Some(LABEL)
            && item.get("label").and_then(Value::as_str) == Some(label)
            && item.get("labelerDid").is_none())
    });
    preferences.push(json!({"$type": LABEL, "label": label, "visibility": visibility.api_name()}));
}

pub fn muted_words(preferences: &[Value]) -> Vec<String> {
    of_type(preferences, MUTED_WORDS)
        .next()
        .and_then(|item| item.get("items")?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|word| word.get("value")?.as_str().map(str::to_owned))
        .collect()
}

fn muted_words_item(preferences: &mut Vec<Value>) -> &mut Value {
    let index = match preferences
        .iter()
        .position(|item| item.get("$type").and_then(Value::as_str) == Some(MUTED_WORDS))
    {
        Some(index) => index,
        None => {
            preferences.push(json!({"$type": MUTED_WORDS, "items": []}));
            preferences.len() - 1
        }
    };
    &mut preferences[index]
}

/// Ajoute un mot (ou un mot-clic « #… ») masqué partout : textes et
/// mots-clics, de tous les comptes. Sans effet s'il l'est déjà.
pub fn add_muted_word(preferences: &mut Vec<Value>, word: &str) {
    let word = word.trim().trim_start_matches(['#', '＃']).trim();
    if word.is_empty()
        || muted_words(preferences)
            .iter()
            .any(|w| w.eq_ignore_ascii_case(word))
    {
        return;
    }
    let item = muted_words_item(preferences);
    if let Some(items) = item.get_mut("items").and_then(Value::as_array_mut) {
        items.push(json!({"value": word, "targets": ["content", "tag"], "actorTarget": "all"}));
    }
}

pub fn remove_muted_word(preferences: &mut Vec<Value>, word: &str) {
    let item = muted_words_item(preferences);
    if let Some(items) = item.get_mut("items").and_then(Value::as_array_mut) {
        items.retain(|w| w.get("value").and_then(Value::as_str) != Some(word));
    }
}

/// Services d'étiquetage auxquels le compte est abonné (DID).
pub fn labelers(preferences: &[Value]) -> Vec<String> {
    of_type(preferences, LABELERS)
        .next()
        .and_then(|item| item.get("labelers")?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|labeler| labeler.get("did")?.as_str().map(str::to_owned))
        .collect()
}

/// Le texte contient-il un mot masqué ? Un mot seul se compare aux mots du
/// texte (sans tenir compte de la casse ni du « # ») ; une expression de
/// plusieurs mots, à sa présence dans le texte.
pub fn contains_muted_word(text: &str, muted: &[String]) -> bool {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    muted.iter().any(|m| {
        let m = m.to_lowercase();
        if m.contains(char::is_whitespace) {
            lower.contains(&m)
        } else {
            words.iter().any(|w| *w == m)
        }
    })
}

/// Ce que demandent les étiquettes d'un contenu, avec l'étiquette qui
/// décide. « !hide » et « !warn » viennent de la modération de Bluesky et
/// ne se règlent pas ; le contenu pour adultes désactivé masque ses
/// étiquettes. Les étiquettes inconnues sont ignorées.
pub fn decide(preferences: &[Value], labels: &[String]) -> Option<(Visibility, String)> {
    let adult = adult_content(preferences);
    labels
        .iter()
        .filter_map(|label| {
            let visibility = match label.as_str() {
                "!hide" => Visibility::Hide,
                "!warn" => Visibility::Warn,
                l if is_adult_label(l) && !adult => Visibility::Hide,
                l if LABELS.iter().any(|(known, _)| *known == l) => {
                    label_visibility(preferences, l)
                }
                _ => return None,
            };
            Some((visibility, label.clone()))
        })
        .max_by_key(|(visibility, _)| *visibility as u8)
        .filter(|(visibility, _)| *visibility != Visibility::Show)
}

/// Titre de l'avertissement affiché sur un contenu étiqueté.
pub fn warning_title(label: &str) -> String {
    match label {
        "!warn" | "!hide" => gettext("Contenu signalé par la modération"),
        other => label_title(other),
    }
}

thread_local! {
    /// Préférences du compte connecté (ou celles par défaut), appliquées à
    /// l'affichage des posts.
    static PREFERENCES: RefCell<Vec<Value>> = RefCell::default();
    /// Mots masqués tirés de ces préférences.
    static MUTED: RefCell<Vec<String>> = RefCell::default();
}

/// Préférences à appliquer aux fils ; vide au départ et sans compte.
pub fn set_preferences(preferences: Vec<Value>) {
    MUTED.with_borrow_mut(|muted| *muted = muted_words(&preferences));
    PREFERENCES.with_borrow_mut(|current| *current = preferences);
}

/// Post à cacher : il contient un mot masqué, ou une étiquette le masque.
pub fn is_hidden(post: &Post) -> bool {
    MUTED.with_borrow(|muted| !muted.is_empty() && contains_muted_word(&post.text, muted))
        || matches!(media_warning_visibility(post), Some((Visibility::Hide, _)))
}

fn media_warning_visibility(post: &Post) -> Option<(Visibility, String)> {
    if post.labels.is_empty() {
        return None;
    }
    PREFERENCES.with_borrow(|preferences| decide(preferences, &post.labels))
}

/// Avertissement à mettre sur les images du post, s'il en faut un.
pub fn media_warning(post: &Post) -> Option<String> {
    match media_warning_visibility(post) {
        Some((Visibility::Warn, label)) => Some(warning_title(&label)),
        _ => None,
    }
}

// --- Appels à l'API ---

/// Préférences du compte, en JSON.
pub async fn preferences(session: Arc<Session>) -> Result<Vec<Value>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .actor
        .get_preferences(atrium_api::app::bsky::actor::get_preferences::ParametersData {}.into())
        .await
        .map_err(request_failed)?;
    Ok(serde_json::to_value(&output.data.preferences)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default())
}

/// Relit les préférences, applique `change`, réécrit le tout. Renvoie les
/// préférences écrites.
pub async fn update_preferences(
    session: Arc<Session>,
    change: impl FnOnce(&mut Vec<Value>) + Send,
) -> Result<Vec<Value>, ApiError> {
    let mut current = preferences(session.clone()).await?;
    change(&mut current);
    let api = session.api().await?;
    let typed = serde_json::from_value(Value::Array(current.clone()))
        .map_err(|_| ApiError::new(gettext("Préférences illisibles.")))?;
    api.api
        .app
        .bsky
        .actor
        .put_preferences(
            atrium_api::app::bsky::actor::put_preferences::InputData { preferences: typed }.into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(current)
}

pub async fn mutes(session: Arc<Session>) -> Result<Vec<Actor>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_mutes(
            atrium_api::app::bsky::graph::get_mutes::ParametersData {
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.mutes.iter().map(api::actor_from_view).collect())
}

/// Comptes bloqués, avec l'adresse de chaque blocage (pour l'annuler).
pub async fn blocks(session: Arc<Session>) -> Result<Vec<(Actor, Option<String>)>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_blocks(
            atrium_api::app::bsky::graph::get_blocks::ParametersData {
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output
        .data
        .blocks
        .iter()
        .map(|view| {
            let blocking = view.viewer.as_ref().and_then(|v| v.blocking.clone());
            (api::actor_from_view(view), blocking)
        })
        .collect())
}

fn invalid_actor() -> ApiError {
    ApiError::new(gettext("Ce compte n'existe pas."))
}

pub async fn mute(session: Arc<Session>, actor: String, mute: bool) -> Result<(), ApiError> {
    let api = session.api().await?;
    let actor = actor.parse().map_err(|_| invalid_actor())?;
    let graph = &api.api.app.bsky.graph;
    if mute {
        graph
            .mute_actor(
                atrium_api::app::bsky::graph::mute_actor::InputData {
                    actor,
                    only_quoteposts: None,
                    only_reposts: None,
                }
                .into(),
            )
            .await
            .map_err(request_failed)
    } else {
        graph
            .unmute_actor(atrium_api::app::bsky::graph::unmute_actor::InputData { actor }.into())
            .await
            .map_err(request_failed)
    }
}

/// Bloque un compte (DID ou identifiant) : enregistrement
/// `app.bsky.graph.block`.
pub async fn block(session: Arc<Session>, actor: String) -> Result<String, ApiError> {
    use atrium_api::com::atproto::repo::create_record;
    use atrium_api::types::string::{Datetime, Did, Nsid};
    let did = if actor.starts_with("did:") {
        actor
    } else {
        api::profile(session.clone(), actor).await?.did
    };
    let api = session.api().await?;
    let record = json!({
        "$type": "app.bsky.graph.block",
        "subject": did,
        "createdAt": Datetime::now().as_str(),
    });
    let output = api
        .api
        .com
        .atproto
        .repo
        .create_record(
            create_record::InputData {
                collection: Nsid::new("app.bsky.graph.block".to_owned())
                    .map_err(|_| invalid_actor())?,
                record: serde_json::from_value(record).map_err(|_| invalid_actor())?,
                repo: Did::new(session.account.did.clone())
                    .map_err(|_| invalid_actor())?
                    .into(),
                rkey: None,
                swap_commit: None,
                validate: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.uri)
}

/// Noms des services d'étiquetage.
pub async fn labeler_names(
    session: Arc<Session>,
    dids: Vec<String>,
) -> Result<Vec<(String, String)>, ApiError> {
    use atrium_api::types::string::Did;
    if dids.is_empty() {
        return Ok(Vec::new());
    }
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .labeler
        .get_services(
            atrium_api::app::bsky::labeler::get_services::ParametersData {
                detailed: Some(false),
                dids: dids
                    .iter()
                    .filter_map(|d| Did::new(d.clone()).ok())
                    .collect(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let views = serde_json::to_value(&output.data.views).unwrap_or_default();
    Ok(views
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|view| {
            let did = view.get("creator")?.get("did")?.as_str()?.to_owned();
            let creator = view.get("creator")?;
            let name = api::name_or_handle(
                creator.get("displayName").and_then(Value::as_str),
                creator
                    .get("handle")
                    .and_then(Value::as_str)
                    .unwrap_or(&did),
            );
            Some((did, name))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn labels_follow_the_preferences() {
        let none: Vec<Value> = Vec::new();
        assert_eq!(decide(&none, &labels(&["nudity"])), None);
        assert_eq!(
            decide(&none, &labels(&["graphic-media"])),
            Some((Visibility::Warn, "graphic-media".into()))
        );
        // Contenu pour adultes désactivé : masqué quel que soit le réglage.
        assert_eq!(
            decide(&none, &labels(&["sexual"])).unwrap().0,
            Visibility::Hide
        );
        let mut prefs = Vec::new();
        set_adult_content(&mut prefs, true);
        assert_eq!(
            decide(&prefs, &labels(&["sexual"])).unwrap().0,
            Visibility::Warn
        );
        set_label_visibility(&mut prefs, "graphic-media", Visibility::Show);
        assert_eq!(decide(&prefs, &labels(&["graphic-media", "custom"])), None);
        // L'étiquette la plus forte l'emporte.
        assert_eq!(
            decide(&prefs, &labels(&["sexual", "!hide"])),
            Some((Visibility::Hide, "!hide".into()))
        );
    }

    fn prefs() -> Vec<Value> {
        serde_json::from_value(json!([
            {"$type": "app.bsky.actor.defs#savedFeedsPrefV2", "items": []},
            {"$type": ADULT, "enabled": true},
            {"$type": LABEL, "label": "graphic-media", "visibility": "hide"},
            {"$type": LABEL, "label": "nudity", "visibility": "warn",
             "labelerDid": "did:plc:tiers"},
            {"$type": MUTED_WORDS, "items": [
                {"value": "spoiler", "targets": ["content", "tag"], "actorTarget": "all"}
            ]},
            {"$type": LABELERS, "labelers": [{"did": "did:plc:veille"}]}
        ]))
        .unwrap()
    }

    #[test]
    fn preferences_are_read_with_defaults() {
        let p = prefs();
        assert!(adult_content(&p));
        assert_eq!(label_visibility(&p, "graphic-media"), Visibility::Hide);
        // Réglage d'un service tiers : ignoré ; défaut de Bluesky.
        assert_eq!(label_visibility(&p, "nudity"), Visibility::Show);
        assert_eq!(label_visibility(&p, "porn"), Visibility::Hide);
        assert_eq!(muted_words(&p), vec!["spoiler"]);
        assert_eq!(labelers(&p), vec!["did:plc:veille"]);
        assert!(!adult_content(&[]));
    }

    #[test]
    fn changes_keep_the_other_preferences() {
        let mut p = prefs();
        set_adult_content(&mut p, false);
        set_label_visibility(&mut p, "graphic-media", Visibility::Warn);
        add_muted_word(&mut p, "#GNOME");
        add_muted_word(&mut p, "Spoiler");
        remove_muted_word(&mut p, "spoiler");
        assert!(!adult_content(&p));
        assert_eq!(label_visibility(&p, "graphic-media"), Visibility::Warn);
        assert_eq!(muted_words(&p), vec!["GNOME"]);
        // Éléments inconnus et réglage du service tiers conservés.
        assert!(
            p.iter()
                .any(|i| i["$type"] == "app.bsky.actor.defs#savedFeedsPrefV2")
        );
        assert!(p.iter().any(|i| i.get("labelerDid").is_some()));
        assert_eq!(p.iter().filter(|i| i["$type"] == ADULT).count(), 1);
        // Les préférences restent acceptées par les types du protocole.
        let _: atrium_api::app::bsky::actor::defs::Preferences =
            serde_json::from_value(Value::Array(p)).unwrap();
    }

    #[test]
    fn muted_words_match_words_and_tags() {
        let muted = vec!["gnome".to_owned(), "fin de saison".to_owned()];
        assert!(contains_muted_word("Vive #GNOME 49 !", &muted));
        assert!(contains_muted_word("Gnome, encore", &muted));
        assert!(!contains_muted_word("gnomes et lutins", &muted));
        assert!(contains_muted_word(
            "Spoiler : la fin de saison arrive",
            &muted
        ));
        assert!(!contains_muted_word("Rien à voir", &muted));
        assert!(!contains_muted_word("gnome", &[]));
    }
}
