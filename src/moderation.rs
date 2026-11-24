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

/// Étiquette définie par un service tiers (`labelValueDefinitions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDefinition {
    pub labeler: String,
    pub value: String,
    /// Nom dans la langue de l'interface (ou le premier donné).
    pub name: String,
    /// Réglage par défaut du service.
    pub default: Visibility,
    pub adult_only: bool,
    /// Étiquette purement informative (rien à flouter ni masquer).
    pub inform_only: bool,
}

/// Séparateur entre le service et la valeur d'une étiquette tierce
/// (« did:plc:…#valeur »).
pub const LABELER_SEPARATOR: char = '#';

/// Définitions d'étiquettes d'un service, d'après sa vue détaillée
/// (`getServices`).
pub fn definitions_from_view(view: &Value, lang: &str) -> Vec<LabelDefinition> {
    let Some(labeler) = view
        .get("creator")
        .and_then(|c| c.get("did"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    view.get("policies")
        .and_then(|p| p.get("labelValueDefinitions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|def| {
            let value = def.get("identifier")?.as_str()?.to_owned();
            let locales: Vec<&Value> = def
                .get("locales")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .collect();
            let locale = locales
                .iter()
                .find(|l| {
                    l.get("lang")
                        .and_then(Value::as_str)
                        .is_some_and(|code| code.split('-').next() == Some(lang))
                })
                .or(locales.first());
            let name = locale
                .and_then(|l| l.get("name")?.as_str())
                .unwrap_or(&value)
                .to_owned();
            let severity = def
                .get("severity")
                .and_then(Value::as_str)
                .unwrap_or("none");
            let blurs = def.get("blurs").and_then(Value::as_str).unwrap_or("none");
            Some(LabelDefinition {
                labeler: labeler.to_owned(),
                default: def
                    .get("defaultSetting")
                    .and_then(Value::as_str)
                    .and_then(Visibility::from_api)
                    .unwrap_or(Visibility::Warn),
                adult_only: def
                    .get("adultOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                inform_only: severity == "inform" || (severity == "none" && blurs == "none"),
                value,
                name,
            })
        })
        .collect()
}

/// Réglage d'une étiquette d'un service tiers, ou son défaut.
pub fn labeler_label_visibility(preferences: &[Value], definition: &LabelDefinition) -> Visibility {
    of_type(preferences, LABEL)
        .find(|item| {
            item.get("label").and_then(Value::as_str) == Some(definition.value.as_str())
                && item.get("labelerDid").and_then(Value::as_str)
                    == Some(definition.labeler.as_str())
        })
        .and_then(|item| Visibility::from_api(item.get("visibility")?.as_str()?))
        .unwrap_or(definition.default)
}

pub fn set_labeler_label_visibility(
    preferences: &mut Vec<Value>,
    labeler: &str,
    label: &str,
    visibility: Visibility,
) {
    preferences.retain(|item| {
        !(item.get("$type").and_then(Value::as_str) == Some(LABEL)
            && item.get("label").and_then(Value::as_str) == Some(label)
            && item.get("labelerDid").and_then(Value::as_str) == Some(labeler))
    });
    preferences.push(json!({
        "$type": LABEL,
        "labelerDid": labeler,
        "label": label,
        "visibility": visibility.api_name(),
    }));
}

/// Mot masqué, avec ses réglages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutedWord {
    pub value: String,
    /// Seulement dans les mots-clics (pas dans le texte).
    pub tags_only: bool,
    /// Fin du masquage, s'il est temporaire.
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
}

impl MutedWord {
    pub fn active(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.expires.is_none_or(|end| end > now)
    }
}

/// Mots masqués, tels qu'enregistrés (y compris ceux qui ont expiré).
pub fn muted_word_items(preferences: &[Value]) -> Vec<MutedWord> {
    of_type(preferences, MUTED_WORDS)
        .next()
        .and_then(|item| item.get("items")?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|word| {
            let targets: Vec<&str> = word
                .get("targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            Some(MutedWord {
                value: word.get("value")?.as_str()?.to_owned(),
                tags_only: !targets.is_empty() && !targets.contains(&"content"),
                expires: word
                    .get("expiresAt")
                    .and_then(Value::as_str)
                    .and_then(crate::api::parse_datetime)
                    .map(|t| t.with_timezone(&chrono::Utc)),
            })
        })
        .collect()
}

pub fn muted_words(preferences: &[Value]) -> Vec<String> {
    muted_word_items(preferences)
        .into_iter()
        .map(|w| w.value)
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
#[cfg(test)]
pub fn add_muted_word(preferences: &mut Vec<Value>, word: &str) {
    add_muted_word_with(preferences, word, false, None);
}

/// Ajoute un mot masqué : dans les mots-clics seulement (`tags_only`), et
/// jusqu'à `expires` s'il est temporaire.
pub fn add_muted_word_with(
    preferences: &mut Vec<Value>,
    word: &str,
    tags_only: bool,
    expires: Option<chrono::DateTime<chrono::Utc>>,
) {
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
        let targets = if tags_only {
            json!(["tag"])
        } else {
            json!(["content", "tag"])
        };
        let mut entry = json!({"value": word, "targets": targets, "actorTarget": "all"});
        if let Some(end) = expires {
            entry["expiresAt"] = json!(end.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        }
        items.push(entry);
    }
}

pub fn remove_muted_word(preferences: &mut Vec<Value>, word: &str) {
    let item = muted_words_item(preferences);
    if let Some(items) = item.get_mut("items").and_then(Value::as_array_mut) {
        items.retain(|w| w.get("value").and_then(Value::as_str) != Some(word));
    }
}

/// Service de modération de Bluesky, toujours actif.
pub const BLUESKY_LABELER: &str = "did:plc:ar7c4by46qjdydhdevvrndac";
/// Abonnements à des services d'étiquetage acceptés par Bluesky, au plus.
pub const MAX_LABELERS: usize = 20;

/// Abonne (ou désabonne) le compte au service d'étiquetage `did`.
pub fn set_labeler(preferences: &mut Vec<Value>, did: &str, subscribed: bool) {
    let index = match preferences
        .iter()
        .position(|item| item.get("$type").and_then(Value::as_str) == Some(LABELERS))
    {
        Some(index) => index,
        None => {
            preferences.push(json!({"$type": LABELERS, "labelers": []}));
            preferences.len() - 1
        }
    };
    let item = &mut preferences[index];
    if !item.get("labelers").is_some_and(Value::is_array) {
        item["labelers"] = json!([]);
    }
    let list = item["labelers"].as_array_mut().expect("tableau");
    list.retain(|l| l.get("did").and_then(Value::as_str) != Some(did));
    if subscribed {
        list.push(json!({"did": did}));
    }
}

/// Langues des posts à afficher dans les fils (vide : toutes).
pub fn set_content_languages(languages: Vec<String>) {
    CONTENT_LANGUAGES.with_borrow_mut(|current| *current = languages);
}

/// Le post est-il dans une langue écartée ? Un post sans langue déclarée,
/// ou un choix vide, ne l'est jamais. Seule la langue principale compte
/// (« en-US » vaut « en »).
pub fn outside_languages(post_langs: &[String], wanted: &[String]) -> bool {
    let primary = |code: &str| code.split('-').next().unwrap_or(code).to_lowercase();
    !wanted.is_empty()
        && !post_langs.is_empty()
        && !post_langs
            .iter()
            .any(|l| wanted.iter().any(|w| primary(w) == primary(l)))
}

/// Définitions des étiquettes des services suivis, pour les fils.
pub fn set_definitions(definitions: Vec<LabelDefinition>) {
    DEFINITIONS.with_borrow_mut(|current| *current = definitions);
}

/// Noms des services tiers suivis, pour les choisir (signalement).
pub fn set_labeler_names(names: Vec<(String, String)>) {
    LABELER_NAMES.with_borrow_mut(|current| *current = names);
}

pub fn labeler_names() -> Vec<(String, String)> {
    LABELER_NAMES.with_borrow(Clone::clone)
}

/// Langue de l'interface (« fr », « en »…), pour les noms d'étiquettes.
pub fn interface_language() -> String {
    gtk::glib::language_names()
        .first()
        .and_then(|l| l.split(['_', '.', '@']).next())
        .filter(|l| *l != "C")
        .unwrap_or("en")
        .to_owned()
}

/// Services d'étiquetage, avec leur nom et leurs étiquettes
/// (`getServices` détaillé).
pub async fn labeler_details(
    session: Arc<Session>,
    dids: Vec<String>,
    lang: String,
) -> Result<Vec<(String, String, Vec<LabelDefinition>)>, ApiError> {
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
                detailed: Some(true),
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
            let creator = view.get("creator")?;
            let did = creator.get("did")?.as_str()?.to_owned();
            let name = api::name_or_handle(
                creator.get("displayName").and_then(Value::as_str),
                creator
                    .get("handle")
                    .and_then(Value::as_str)
                    .unwrap_or(&did),
            );
            Some((did, name, definitions_from_view(view, &lang)))
        })
        .collect())
}

/// Préférences du compte connecté, telles que chargées.
pub fn current_preferences() -> Vec<Value> {
    PREFERENCES.with_borrow(Clone::clone)
}

/// Services d'étiquetage du compte connecté (préférences chargées).
pub fn subscribed_labelers() -> Vec<String> {
    PREFERENCES.with_borrow(|p| labelers(p))
}

/// Demande à Bluesky les étiquettes de ces services en plus des siennes
/// (en-tête `atproto-accept-labelers`) pour les requêtes de cette session.
pub async fn configure_labelers(session: Arc<Session>, labelers: Vec<String>) {
    use atrium_api::agent::Configure;
    use atrium_api::types::string::Did;
    let Ok(api) = session.api().await else { return };
    let mut dids = vec![(Did::new(BLUESKY_LABELER.to_owned()).expect("DID"), true)];
    dids.extend(
        labelers
            .into_iter()
            .filter(|d| d != BLUESKY_LABELER)
            .filter_map(|d| Did::new(d).ok())
            .map(|d| (d, false)),
    );
    api.configure_labelers_header(Some(dids));
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

/// Le post contient-il un mot masqué encore actif ? Un mot « mots-clics
/// seulement » ne se compare qu'aux mots-clics du post.
pub fn post_is_muted(post: &Post, muted: &[MutedWord], now: chrono::DateTime<chrono::Utc>) -> bool {
    let active = muted.iter().filter(|w| w.active(now));
    let (tags_only, anywhere): (Vec<&MutedWord>, Vec<&MutedWord>) =
        active.partition(|w| w.tags_only);
    let anywhere: Vec<String> = anywhere.iter().map(|w| w.value.clone()).collect();
    if !anywhere.is_empty() && contains_muted_word(&post.text, &anywhere) {
        return true;
    }
    let tags: Vec<String> = post
        .facets
        .iter()
        .filter_map(|f| match &f.feature {
            crate::richtext::Feature::Tag(tag) => Some(tag.to_lowercase()),
            _ => None,
        })
        .collect();
    tags_only
        .iter()
        .any(|w| tags.contains(&w.value.to_lowercase()))
}

/// Ce que demandent les étiquettes d'un contenu, avec l'étiquette qui
/// décide. « !hide » et « !warn » viennent de la modération de Bluesky et
/// ne se règlent pas ; le contenu pour adultes désactivé masque ses
/// étiquettes. Les étiquettes inconnues sont ignorées.
pub fn decide(preferences: &[Value], labels: &[String]) -> Option<(Visibility, String)> {
    DEFINITIONS.with_borrow(|definitions| decide_with(preferences, definitions, labels))
}

/// `decide`, avec les définitions des services tiers : une étiquette
/// « service#valeur » suit son réglage ; sans définition connue, ignorée.
pub fn decide_with(
    preferences: &[Value],
    definitions: &[LabelDefinition],
    labels: &[String],
) -> Option<(Visibility, String)> {
    let adult = adult_content(preferences);
    labels
        .iter()
        .filter_map(|label| {
            if let Some((labeler, value)) = label.split_once(LABELER_SEPARATOR) {
                let definition = definitions
                    .iter()
                    .find(|d| d.labeler == labeler && d.value == value)?;
                if definition.inform_only {
                    return None;
                }
                let visibility = if definition.adult_only && !adult {
                    Visibility::Hide
                } else {
                    labeler_label_visibility(preferences, definition)
                };
                return Some((visibility, label.clone()));
            }
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
    if let Some((labeler, value)) = label.split_once(LABELER_SEPARATOR) {
        return DEFINITIONS.with_borrow(|definitions| {
            definitions
                .iter()
                .find(|d| d.labeler == labeler && d.value == value)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| value.to_owned())
        });
    }
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
    static MUTED: RefCell<Vec<MutedWord>> = RefCell::default();
    /// Étiquettes des services tiers suivis.
    static DEFINITIONS: RefCell<Vec<LabelDefinition>> = RefCell::default();
    /// Services tiers suivis : DID et nom.
    static LABELER_NAMES: RefCell<Vec<(String, String)>> = RefCell::default();
    /// Langues des posts à afficher (vide : toutes).
    static CONTENT_LANGUAGES: RefCell<Vec<String>> = RefCell::default();
}

/// Préférences à appliquer aux fils ; vide au départ et sans compte.
pub fn set_preferences(preferences: Vec<Value>) {
    MUTED.with_borrow_mut(|muted| *muted = muted_word_items(&preferences));
    PREFERENCES.with_borrow_mut(|current| *current = preferences);
}

/// Post à cacher : il contient un mot masqué, ou une étiquette le masque.
pub fn is_hidden(post: &Post) -> bool {
    MUTED.with_borrow(|muted| post_is_muted(post, muted, chrono::Utc::now()))
        || CONTENT_LANGUAGES.with_borrow(|wanted| outside_languages(&post.langs, wanted))
        || matches!(media_warning_visibility(post), Some((Visibility::Hide, _)))
}

fn media_warning_visibility(post: &Post) -> Option<(Visibility, String)> {
    if post.labels.is_empty() {
        return None;
    }
    PREFERENCES.with_borrow(|preferences| decide(preferences, &post.labels))
}

/// Avertissement à mettre sur un profil étiqueté (bannière, avatar, bio),
/// ou `None`. Une étiquette qui masquerait un post avertit aussi.
pub fn profile_warning(labels: &[String]) -> Option<String> {
    if labels.is_empty() {
        return None;
    }
    PREFERENCES
        .with_borrow(|preferences| decide(preferences, labels))
        .map(|(_, label)| warning_title(&label))
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
    fn temporary_and_tag_words() {
        let mut p = Vec::new();
        let now = chrono::Utc::now();
        add_muted_word_with(
            &mut p,
            "#finale",
            true,
            Some(now + chrono::Duration::hours(24)),
        );
        add_muted_word_with(
            &mut p,
            "ancien",
            false,
            Some(now - chrono::Duration::hours(1)),
        );
        let words = muted_word_items(&p);
        assert!(words[0].tags_only && words[0].active(now));
        assert!(!words[1].active(now));
        let post = |text: &str| crate::data::Post {
            facets: crate::richtext::detect(text),
            ..crate::data::posts(crate::data::Feed::Following)[0].clone()
        };
        let with_text = |text: &str| crate::data::Post {
            text: text.into(),
            ..post(text)
        };
        // « finale » dans le texte : pas un mot-clic, pas masqué.
        assert!(!post_is_muted(&with_text("La finale ce soir"), &words, now));
        assert!(post_is_muted(&with_text("Ce soir #Finale"), &words, now));
        // Mot expiré : plus masqué.
        assert!(!post_is_muted(&with_text("Un ancien post"), &words, now));
    }

    #[test]
    fn third_party_labels_follow_their_settings() {
        let view = json!({
            "creator": {"did": "did:plc:veille", "handle": "veille.example"},
            "policies": {"labelValues": ["spoiler", "info"], "labelValueDefinitions": [
                {"identifier": "spoiler", "severity": "alert", "blurs": "content",
                 "defaultSetting": "warn", "adultOnly": false,
                 "locales": [{"lang": "en", "name": "Spoiler"}, {"lang": "fr", "name": "Divulgâcheur"}]},
                {"identifier": "info", "severity": "inform", "blurs": "none",
                 "defaultSetting": "warn", "locales": []},
            ]}
        });
        let definitions = definitions_from_view(&view, "fr");
        assert_eq!(definitions[0].name, "Divulgâcheur");
        assert!(definitions[1].inform_only);
        let mut p = Vec::new();
        let labels = vec![
            "did:plc:veille#spoiler".to_owned(),
            "did:plc:veille#info".to_owned(),
        ];
        assert_eq!(
            decide_with(&p, &definitions, &labels),
            Some((Visibility::Warn, "did:plc:veille#spoiler".to_owned()))
        );
        set_labeler_label_visibility(&mut p, "did:plc:veille", "spoiler", Visibility::Hide);
        assert_eq!(
            decide_with(&p, &definitions, &labels).map(|(v, _)| v),
            Some(Visibility::Hide)
        );
        // Service inconnu : ignoré.
        assert_eq!(decide_with(&p, &[], &labels), None);
    }

    #[test]
    fn labeled_profiles_are_warned() {
        set_preferences(Vec::new());
        assert_eq!(profile_warning(&[]), None);
        assert_eq!(
            profile_warning(&["graphic-media".to_owned()]).as_deref(),
            Some("Violence graphique")
        );
        // Étiquette inconnue (service non suivi) : rien.
        assert_eq!(profile_warning(&["did:plc:x#y".to_owned()]), None);
    }

    #[test]
    fn posts_are_filtered_by_language() {
        let langs = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!outside_languages(&langs(&["en"]), &[]));
        assert!(!outside_languages(&[], &langs(&["fr"])));
        assert!(!outside_languages(
            &langs(&["en-US"]),
            &langs(&["fr", "en"])
        ));
        assert!(outside_languages(&langs(&["ja"]), &langs(&["fr", "en"])));
    }

    #[test]
    fn labelers_are_subscribed_once() {
        let mut p = prefs();
        set_labeler(&mut p, "did:plc:autre", true);
        set_labeler(&mut p, "did:plc:autre", true);
        assert_eq!(labelers(&p), ["did:plc:veille", "did:plc:autre"]);
        set_labeler(&mut p, "did:plc:veille", false);
        assert_eq!(labelers(&p), ["did:plc:autre"]);
        let mut empty = Vec::new();
        set_labeler(&mut empty, "did:plc:x", true);
        assert_eq!(labelers(&empty), ["did:plc:x"]);
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
