//! Page « Flux » : les flux enregistrés du compte (épingler, réordonner,
//! retirer, ouvrir) et la découverte de nouveaux flux (populaires ou par
//! recherche).
//!
//! Les flux enregistrés sont une préférence du compte, `savedFeedsPrefV2` :
//! une liste ordonnée d'éléments `{id, type, value, pinned}`. Les flux
//! épinglés deviennent des colonnes possibles de l'accueil.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::{gettext, ngettext, pgettext};
use gtk::glib;
use serde_json::{Value, json};

use crate::api::{ApiError, Source, request_failed};
use crate::auth::Session;
use crate::data;
use crate::feed_page::FeedPage;
use crate::images;
use crate::moderation;
use crate::profile::Host;
use crate::runtime::runtime;

const SAVED_FEEDS: &str = "app.bsky.actor.defs#savedFeedsPrefV2";

/// Un flux (générateur de fil) : son adresse et ce qu'on en montre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedInfo {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub creator: String,
    pub avatar: Option<String>,
    pub likes: u64,
}

/// Flux enregistré : son adresse et s'il est épinglé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFeed {
    pub uri: String,
    pub pinned: bool,
}

fn saved_items(preferences: &[Value]) -> Vec<Value> {
    preferences
        .iter()
        .find(|item| item.get("$type").and_then(Value::as_str) == Some(SAVED_FEEDS))
        .and_then(|item| item.get("items")?.as_array().cloned())
        .unwrap_or_default()
}

/// Flux enregistrés (générateurs seulement), dans l'ordre.
pub fn saved_feeds(preferences: &[Value]) -> Vec<SavedFeed> {
    saved_items(preferences)
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("feed"))
        .filter_map(|item| {
            Some(SavedFeed {
                uri: item.get("value")?.as_str()?.to_owned(),
                pinned: item.get("pinned").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// Élément `savedFeedsPrefV2`, créé s'il manque, avec le fil Following
/// épinglé comme le fait Bluesky.
fn saved_pref(preferences: &mut Vec<Value>) -> &mut Vec<Value> {
    let index = match preferences
        .iter()
        .position(|item| item.get("$type").and_then(Value::as_str) == Some(SAVED_FEEDS))
    {
        Some(index) => index,
        None => {
            preferences.push(json!({
                "$type": SAVED_FEEDS,
                "items": [{"id": new_id(), "type": "timeline", "value": "following", "pinned": true}],
            }));
            preferences.len() - 1
        }
    };
    let item = &mut preferences[index];
    if !item.get("items").is_some_and(Value::is_array) {
        item["items"] = json!([]);
    }
    item.get_mut("items")
        .and_then(Value::as_array_mut)
        .expect("tableau créé plus haut")
}

/// Identifiant d'un élément : chaîne courte et unique, comme ceux de Bluesky.
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default();
    format!("{:x}{:x}", now, COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn is_feed(item: &Value, uri: &str) -> bool {
    item.get("type").and_then(Value::as_str) == Some("feed")
        && item.get("value").and_then(Value::as_str) == Some(uri)
}

/// Changement de la liste des flux enregistrés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Enregistrer un flux, épinglé ou non (sans effet s'il l'est déjà).
    Save(String, bool),
    Remove(String),
    Pin(String, bool),
    /// Déplacer d'un cran parmi les flux (-1 : vers le haut).
    Move(String, i32),
    /// Placer un flux à cet indice parmi les flux (glisser-déposer).
    MoveTo(String, usize),
    /// Épingler une liste (son fil devient une colonne), ou la retirer.
    PinList(String, bool),
}

/// La liste `uri` est-elle épinglée ?
pub fn list_pinned(preferences: &[Value], uri: &str) -> bool {
    saved_items(preferences).iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("list")
            && item.get("value").and_then(Value::as_str) == Some(uri)
            && item.get("pinned").and_then(Value::as_bool) == Some(true)
    })
}

impl Change {
    pub fn apply(&self, preferences: &mut Vec<Value>) {
        let items = saved_pref(preferences);
        match self {
            Change::PinList(uri, pinned) => {
                let is_list = |item: &Value| {
                    item.get("type").and_then(Value::as_str) == Some("list")
                        && item.get("value").and_then(Value::as_str) == Some(uri.as_str())
                };
                items.retain(|item| !is_list(item));
                if *pinned {
                    items.push(
                        json!({"id": new_id(), "type": "list", "value": uri, "pinned": true}),
                    );
                }
            }
            Change::Save(uri, pinned) => {
                if !items.iter().any(|item| is_feed(item, uri)) {
                    items.push(
                        json!({"id": new_id(), "type": "feed", "value": uri, "pinned": pinned}),
                    );
                }
            }
            Change::Remove(uri) => items.retain(|item| !is_feed(item, uri)),
            Change::Pin(uri, pinned) => {
                for item in items.iter_mut().filter(|item| is_feed(item, uri)) {
                    item["pinned"] = json!(pinned);
                }
            }
            Change::Move(uri, delta) => move_feed(items, uri, |at, count| {
                let target = at as i32 + delta;
                (target >= 0 && (target as usize) < count).then_some(target as usize)
            }),
            Change::MoveTo(uri, to) => {
                move_feed(items, uri, |_, count| (*to < count).then_some(*to))
            }
        }
    }
}

/// Déplace le flux `uri` parmi les flux, à l'indice que `target` calcule (depuis son indice et
/// le nombre de flux). Les autres éléments (fil Following, listes) gardent leur place.
fn move_feed(items: &mut [Value], uri: &str, target: impl FnOnce(usize, usize) -> Option<usize>) {
    let slots: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.get("type").and_then(Value::as_str) == Some("feed"))
        .map(|(i, _)| i)
        .collect();
    let Some(at) = slots.iter().position(|&i| is_feed(&items[i], uri)) else {
        return;
    };
    let Some(to) = target(at, slots.len()) else {
        return;
    };
    let mut feeds: Vec<Value> = slots.iter().map(|&i| items[i].clone()).collect();
    let moved = feeds.remove(at);
    feeds.insert(to, moved);
    for (slot, feed) in slots.into_iter().zip(feeds) {
        items[slot] = feed;
    }
}

pub(crate) fn info_from_view(view: &atrium_api::app::bsky::feed::defs::GeneratorView) -> FeedInfo {
    FeedInfo {
        uri: view.uri.clone(),
        name: view.display_name.clone(),
        description: view.description.clone().unwrap_or_default(),
        creator: view.creator.handle.as_str().to_owned(),
        avatar: view.avatar.clone(),
        likes: view.like_count.unwrap_or_default() as u64,
    }
}

/// Flux enregistrés du compte, avec leurs informations.
pub async fn load_saved(session: Arc<Session>) -> Result<Vec<(SavedFeed, FeedInfo)>, ApiError> {
    let preferences = Box::pin(moderation::preferences(session.clone())).await?;
    let saved = saved_feeds(&preferences);
    if saved.is_empty() {
        return Ok(Vec::new());
    }
    let api = session.api().await?;
    let output = Box::pin(
        api.api.app.bsky.feed.get_feed_generators(
            atrium_api::app::bsky::feed::get_feed_generators::ParametersData {
                feeds: saved.iter().map(|feed| feed.uri.clone()).collect(),
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    let infos: Vec<FeedInfo> = output.data.feeds.iter().map(info_from_view).collect();
    // Dans l'ordre des préférences ; un flux disparu depuis est ignoré.
    Ok(saved
        .into_iter()
        .filter_map(|feed| {
            let info = infos.iter().find(|info| info.uri == feed.uri)?.clone();
            Some((feed, info))
        })
        .collect())
}

/// Flux populaires, ou ceux qui correspondent à `query`.
pub async fn discover(
    session: Arc<Session>,
    query: Option<String>,
) -> Result<Vec<FeedInfo>, ApiError> {
    let api = session.api().await?;
    let output = Box::pin(
        api.api.app.bsky.unspecced.get_popular_feed_generators(
            atrium_api::app::bsky::unspecced::get_popular_feed_generators::ParametersData {
                cursor: None,
                limit: 30.try_into().ok(),
                query: query.filter(|q| !q.trim().is_empty()),
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    Ok(output.data.feeds.iter().map(info_from_view).collect())
}

/// Flux créés par un compte (`getActorFeeds`).
pub async fn by_actor(session: Arc<Session>, actor: String) -> Result<Vec<FeedInfo>, ApiError> {
    let api = session.api().await?;
    let output = Box::pin(
        api.api.app.bsky.feed.get_actor_feeds(
            atrium_api::app::bsky::feed::get_actor_feeds::ParametersData {
                actor: actor
                    .parse()
                    .map_err(|_| ApiError::new(gettext("Ce compte n'existe pas.")))?,
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    Ok(output.data.feeds.iter().map(info_from_view).collect())
}

/// Détails d'un flux : ses informations, son CID (pour l'aimer) et mon
/// « j'aime », s'il existe (`getFeedGenerator`).
#[derive(Debug, Clone, Default)]
pub struct FeedDetails {
    pub info: FeedInfo,
    pub cid: String,
    pub like: Option<String>,
}

pub async fn details(session: Arc<Session>, uri: String) -> Result<FeedDetails, ApiError> {
    let api = session.api().await?;
    let output = Box::pin(api.api.app.bsky.feed.get_feed_generator(
        atrium_api::app::bsky::feed::get_feed_generator::ParametersData { feed: uri }.into(),
    ))
    .await
    .map_err(request_failed)?;
    let view = &output.data.view;
    Ok(FeedDetails {
        info: info_from_view(view),
        cid: serde_json::to_value(&view.cid)
            .ok()
            .and_then(|cid| cid.as_str().map(str::to_owned))
            .unwrap_or_default(),
        like: view.viewer.as_ref().and_then(|v| v.like.clone()),
    })
}

/// Page d'un flux, avec « À propos du flux » dans l'en-tête.
pub fn feed_page(name: &str, uri: &str, host: &Host) -> adw::NavigationPage {
    let feed = FeedPage::new(
        name,
        Source::Feed(uri.to_owned()),
        data::posts(data::Feed::Discover),
        host.clone(),
    );
    let about = gtk::Button::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text(gettext("À propos du flux"))
        .build();
    about.update_property(&[gtk::accessible::Property::Label(&gettext(
        "À propos du flux",
    ))]);
    let (uri, host) = (uri.to_owned(), host.clone());
    about.connect_clicked(move |button| about_dialog(button, &uri, &host));
    feed.header.pack_end(&about);
    feed.navigation_page(name)
}

/// « À propos du flux » : avatar, nom, créateur, description, j'aime, et
/// « J'aime » / « Je n'aime plus ».
fn about_dialog(parent: &gtk::Button, uri: &str, host: &Host) {
    let avatar = adw::Avatar::builder()
        .size(64)
        .show_initials(true)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let name = gtk::Label::builder()
        .wrap(true)
        .css_classes(["title-2"])
        .build();
    let creator = gtk::Label::builder().css_classes(["dim-label"]).build();
    let description = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let likes = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .build();
    let like = gtk::Button::builder()
        .halign(gtk::Align::Center)
        .sensitive(false)
        .css_classes(["pill"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    for widget in [
        avatar.upcast_ref::<gtk::Widget>(),
        name.upcast_ref(),
        creator.upcast_ref(),
        description.upcast_ref(),
        likes.upcast_ref(),
        like.upcast_ref(),
    ] {
        content.append(widget);
    }
    let view = adw::ToolbarView::builder().content(&content).build();
    view.add_top_bar(&adw::HeaderBar::new());
    let dialog = adw::Dialog::builder()
        .title(gettext("À propos du flux"))
        .content_width(400)
        .child(&view)
        .build();

    let state: Rc<RefCell<FeedDetails>> = Rc::default();
    let show: Rc<dyn Fn()> = {
        let (state, avatar, name, creator, description, likes, like) = (
            state.clone(),
            avatar.clone(),
            name.clone(),
            creator.clone(),
            description.clone(),
            likes.clone(),
            like.clone(),
        );
        Rc::new(move || {
            let details = state.borrow();
            let info = &details.info;
            avatar.set_text(Some(&info.name));
            if let Some(url) = &info.avatar {
                let weak = avatar.downgrade();
                images::load(url, move |texture| {
                    if let Some(avatar) = weak.upgrade() {
                        avatar.set_custom_image(Some(texture));
                    }
                });
            }
            name.set_label(&info.name);
            creator.set_label(&gettext("par @{}").replace("{}", &info.creator));
            description.set_label(&info.description);
            description.set_visible(!info.description.trim().is_empty());
            likes.set_label(
                &ngettext(
                    "{} j'aime",
                    "{} j'aime",
                    info.likes.min(u32::MAX as u64) as u32,
                )
                .replace("{}", &crate::profile::format_count(info.likes)),
            );
            like.set_label(&if details.like.is_some() {
                gettext("Je n'aime plus")
            } else {
                pgettext("aimer un flux", "J'aime")
            });
        })
    };

    match host.session.clone() {
        None => {
            let info = demo_feeds()
                .into_iter()
                .find(|f| f.uri == uri)
                .unwrap_or_default();
            state.replace(FeedDetails {
                info,
                ..FeedDetails::default()
            });
            show();
        }
        Some(session) => {
            let task = runtime().spawn(details(session, uri.to_owned()));
            let (state, show, like, toasts) = (
                state.clone(),
                show.clone(),
                like.clone(),
                host.toasts.clone(),
            );
            glib::spawn_future_local(async move {
                match task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))))
                {
                    Ok(details) => {
                        state.replace(details);
                        show();
                        like.set_sensitive(true);
                    }
                    Err(error) => toasts.add_toast(adw::Toast::new(&error.message)),
                }
            });
        }
    }

    let (session, toasts) = (host.session.clone(), host.toasts.clone());
    like.connect_clicked(move |button| {
        let Some(session) = session.clone() else {
            return;
        };
        let current = state.borrow().clone();
        button.set_sensitive(false);
        let task = runtime().spawn(async move {
            match current.like {
                Some(record) => crate::api::delete(session, record).await.map(|()| None),
                None => crate::api::create(
                    session,
                    crate::api::Interaction::Like,
                    current.info.uri,
                    current.cid,
                )
                .await
                .map(Some),
            }
        });
        let (state, show, button, toasts) =
            (state.clone(), show.clone(), button.clone(), toasts.clone());
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            button.set_sensitive(true);
            match result {
                Ok(like) => {
                    {
                        let mut details = state.borrow_mut();
                        let liked = like.is_some();
                        details.info.likes = if liked {
                            details.info.likes + 1
                        } else {
                            details.info.likes.saturating_sub(1)
                        };
                        details.like = like;
                    }
                    show();
                }
                Err(error) => toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    });
    dialog.present(Some(parent));
}

/// Flux de démonstration.
pub fn demo_feeds() -> Vec<FeedInfo> {
    let feed = |uri: &str, name: &str, description: &str, creator: &str, likes| FeedInfo {
        uri: uri.into(),
        name: name.into(),
        description: description.into(),
        creator: creator.into(),
        avatar: None,
        likes,
    };
    vec![
        feed(
            "at://did:plc:demo/app.bsky.feed.generator/gnome",
            "GNOME",
            "Les posts sur le bureau GNOME et ses applications.",
            "lucie.example.social",
            1240,
        ),
        feed(
            "at://did:plc:demo/app.bsky.feed.generator/photo",
            "Photo de montagne",
            "Sommets, refuges et levers de soleil.",
            "aya.example.social",
            860,
        ),
        feed(
            "at://did:plc:demo/app.bsky.feed.generator/a11y",
            "Accessibilité",
            "Lecteurs d'écran, contrastes, navigation au clavier.",
            "pierre.example.social",
            415,
        ),
    ]
}

pub struct FeedsPage {
    pub root: adw::ToolbarView,
    host: Host,
    saved_list: gtk::ListBox,
    found_list: gtk::ListBox,
    found_title: adw::PreferencesGroup,
    search: gtk::SearchEntry,
    /// Flux enregistrés affichés (et, en démonstration, la liste elle-même).
    saved: RefCell<Vec<(SavedFeed, FeedInfo)>>,
    /// Flux affichés dans la découverte, avec leur bouton « Ajouter ».
    found: RefCell<Vec<(FeedInfo, gtk::Button)>>,
    /// Les flux épinglés ont changé : les colonnes possibles aussi.
    on_changed: Rc<dyn Fn()>,
}

fn boxed_list(label: &str) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.update_property(&[gtk::accessible::Property::Label(label)]);
    list
}

fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(label)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Boîte « Ajouter un flux » : mes flux enregistrés, ou ceux d'une recherche. `exclude` : les
/// flux déjà choisis, qui ne sont pas proposés. `on_pick` reçoit le flux choisi.
pub fn pick_feed(
    parent: &impl IsA<gtk::Widget>,
    session: Option<Arc<Session>>,
    exclude: Vec<String>,
    on_pick: impl Fn(FeedInfo) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(gettext("Ajouter un flux"))
        .content_width(420)
        .content_height(480)
        .build();
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(gettext("Rechercher des flux"))
        .search_delay(300)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Rechercher des flux",
    ))]);
    let title = gtk::Label::builder()
        .label(gettext("Mes flux"))
        .xalign(0.0)
        .css_classes(["heading"])
        .accessible_role(gtk::AccessibleRole::Heading)
        .build();
    let results = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .css_classes(["boxed-list"])
        .build();
    results.update_property(&[gtk::accessible::Property::Label(&gettext("Flux"))]);
    let empty = gtk::Label::builder()
        .label(gettext("Aucun flux."))
        .wrap(true)
        .visible(false)
        .css_classes(["dim-label"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&entry);
    content.append(&title);
    content.append(&results);
    content.append(&empty);
    let view = adw::ToolbarView::builder()
        .content(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&content)
                .build(),
        )
        .build();
    view.add_top_bar(&adw::HeaderBar::new());
    dialog.set_child(Some(&view));

    let found: Rc<RefCell<Vec<FeedInfo>>> = Rc::default();
    let show = {
        let (results, empty, found) = (results.clone(), empty.clone(), found.clone());
        Rc::new(move |feeds: Vec<FeedInfo>| {
            results.remove_all();
            let feeds: Vec<FeedInfo> = feeds
                .into_iter()
                .filter(|feed| !exclude.contains(&feed.uri))
                .collect();
            for feed in &feeds {
                results.append(&feed_row(feed));
            }
            results.set_visible(!feeds.is_empty());
            empty.set_visible(feeds.is_empty());
            found.replace(feeds);
        })
    };
    // Sans recherche : mes flux enregistrés ; avec : les flux trouvés.
    let fill = {
        let (show, title, empty) = (show.clone(), title.clone(), empty.clone());
        Rc::new(move |query: String| {
            title.set_label(&if query.is_empty() {
                gettext("Mes flux")
            } else {
                gettext("Résultats")
            });
            let Some(session) = session.clone() else {
                let query = query.to_lowercase();
                show(
                    demo_feeds()
                        .into_iter()
                        .filter(|feed| feed.name.to_lowercase().contains(&query))
                        .collect(),
                );
                return;
            };
            let task = runtime().spawn(async move {
                if query.is_empty() {
                    Box::pin(load_saved(session))
                        .await
                        .map(|feeds| feeds.into_iter().map(|(_, info)| info).collect())
                } else {
                    Box::pin(discover(session, Some(query))).await
                }
            });
            let (show, empty) = (show.clone(), empty.clone());
            glib::spawn_future_local(async move {
                match task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))))
                {
                    Ok(feeds) => show(feeds),
                    Err(error) => {
                        show(Vec::new());
                        empty.set_label(&error.message);
                    }
                }
            });
        })
    };
    fill(String::new());
    entry.connect_search_changed(move |entry| {
        fill(entry.text().trim().to_owned());
    });
    let weak = dialog.downgrade();
    results.connect_row_activated(move |_, row| {
        let feed = usize::try_from(row.index())
            .ok()
            .and_then(|i| found.borrow().get(i).cloned());
        if let Some(feed) = feed {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
            on_pick(feed);
        }
    });
    dialog.present(Some(parent));
}

pub(crate) fn feed_row(info: &FeedInfo) -> adw::ActionRow {
    let mut subtitle = gettext("par @{}").replace("{}", &info.creator);
    if info.likes > 0 {
        subtitle = format!(
            "{subtitle} · {}",
            ngettext(
                "{} j'aime",
                "{} j'aime",
                info.likes.min(u32::MAX as u64) as u32
            )
            .replace("{}", &crate::profile::format_count(info.likes))
        );
    }
    if !info.description.trim().is_empty() {
        subtitle = format!("{subtitle}\n{}", info.description.trim());
    }
    let row = adw::ActionRow::builder()
        .title(&info.name)
        .subtitle(&subtitle)
        .subtitle_lines(3)
        .use_markup(false)
        .activatable(true)
        .build();
    let avatar = adw::Avatar::builder()
        .size(40)
        .text(&info.name)
        .show_initials(true)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    if let Some(url) = &info.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    row.add_prefix(&avatar);
    row
}

impl FeedsPage {
    pub fn new(host: Host, on_changed: impl Fn() + 'static) -> Rc<Self> {
        let search = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Rechercher des flux"))
            .hexpand(true)
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Rechercher des flux",
        ))]);
        let saved_list = boxed_list(&gettext("Mes flux"));
        saved_list.add_css_class("saved-feeds");
        let found_list = boxed_list(&gettext("Flux à découvrir"));
        let saved_group = adw::PreferencesGroup::builder()
            .title(gettext("Mes flux"))
            .description(gettext(
                "Les flux épinglés peuvent s'ouvrir en colonne sur l'accueil.",
            ))
            .build();
        saved_group.add(&saved_list);
        let found_title = adw::PreferencesGroup::builder()
            .title(gettext("Flux populaires"))
            .build();
        found_title.add(&found_list);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&search);
        content.append(&saved_group);
        content.append(&found_title);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 120)
                    .child(&content)
                    .build(),
            )
            .build();
        let root = adw::ToolbarView::builder().content(&scroll).build();
        root.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(&gettext("Flux"), ""))
                .build(),
        );
        let page = Rc::new(Self {
            root,
            host,
            saved_list,
            found_list,
            found_title,
            search,
            saved: RefCell::default(),
            found: RefCell::default(),
            on_changed: Rc::new(on_changed),
        });
        let weak = Rc::downgrade(&page);
        page.search.connect_search_changed(move |_| {
            if let Some(page) = weak.upgrade() {
                page.load_found();
            }
        });
        page.load_saved();
        page.load_found();
        page
    }

    fn load_saved(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            if self.saved.borrow().is_empty() {
                let demo = demo_feeds()
                    .into_iter()
                    .take(2)
                    .enumerate()
                    .map(|(i, info)| {
                        (
                            SavedFeed {
                                uri: info.uri.clone(),
                                pinned: i == 0,
                            },
                            info,
                        )
                    })
                    .collect();
                self.saved.replace(demo);
            }
            self.show_saved();
            return;
        };
        let task = runtime().spawn(load_saved(session));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(saved) => {
                    page.saved.replace(saved);
                    page.show_saved();
                    page.show_found_states();
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    fn show_saved(self: &Rc<Self>) {
        self.saved_list.remove_all();
        let saved = self.saved.borrow().clone();
        if saved.is_empty() {
            self.saved_list.append(
                &adw::ActionRow::builder()
                    .title(gettext("Aucun flux enregistré."))
                    .css_classes(["dim-label"])
                    .build(),
            );
            return;
        }
        let count = saved.len();
        for (index, (feed, info)) in saved.into_iter().enumerate() {
            let row = feed_row(&info);
            let pin = gtk::ToggleButton::builder()
                .icon_name("view-pin-symbolic")
                .active(feed.pinned)
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let pin_label = if feed.pinned {
                gettext("Désépingler {}")
            } else {
                gettext("Épingler {}")
            }
            .replace("{}", &info.name);
            pin.set_tooltip_text(Some(&pin_label));
            pin.update_property(&[gtk::accessible::Property::Label(&pin_label)]);
            let up = icon_button(
                "go-up-symbolic",
                &gettext("Monter {}").replace("{}", &info.name),
            );
            up.set_sensitive(index > 0);
            let down = icon_button(
                "go-down-symbolic",
                &gettext("Descendre {}").replace("{}", &info.name),
            );
            down.set_sensitive(index + 1 < count);
            let remove = icon_button(
                "user-trash-symbolic",
                &gettext("Retirer {}").replace("{}", &info.name),
            );
            for widget in [
                pin.upcast_ref::<gtk::Widget>(),
                up.upcast_ref(),
                down.upcast_ref(),
                remove.upcast_ref(),
            ] {
                row.add_suffix(widget);
            }
            let uri = feed.uri.clone();
            let weak = Rc::downgrade(self);
            pin.connect_toggled(move |button| {
                if let Some(page) = weak.upgrade() {
                    page.change(Change::Pin(uri.clone(), button.is_active()));
                }
            });
            for (button, delta) in [(&up, -1), (&down, 1)] {
                let (uri, weak) = (feed.uri.clone(), Rc::downgrade(self));
                button.connect_clicked(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.change(Change::Move(uri.clone(), delta));
                    }
                });
            }
            let (uri, weak) = (feed.uri.clone(), Rc::downgrade(self));
            remove.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.change(Change::Remove(uri.clone()));
                }
            });
            self.make_draggable(&row, &feed.uri, index);
            let weak = Rc::downgrade(self);
            row.connect_activated(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open(&info);
                }
            });
            self.saved_list.append(&row);
        }
    }

    /// Les flux enregistrés se réordonnent aussi par glisser-déposer (les boutons « Monter » et
    /// « Descendre » restent pour le clavier). Un trait marque l'endroit du dépôt.
    fn make_draggable(self: &Rc<Self>, row: &adw::ActionRow, uri: &str, index: usize) {
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        let dragged = uri.to_owned();
        source.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&dragged.to_value()))
        });
        source.connect_drag_begin(|source, _| {
            if let Some(row) = source.widget() {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&row))), 0, 0);
                row.add_css_class("dragged");
            }
        });
        source.connect_drag_end(|source, _, _| {
            if let Some(row) = source.widget() {
                row.remove_css_class("dragged");
            }
        });
        row.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gtk::gdk::DragAction::MOVE);
        target.set_preload(true);
        let page = Rc::downgrade(self);
        target.connect_motion(move |target, _, y| {
            let Some(row) = target.widget() else {
                return gtk::gdk::DragAction::empty();
            };
            let destination = page
                .upgrade()
                .and_then(|page| page.drop_destination(target.value().as_ref(), index, &row, y));
            crate::window::show_drop_marker(&row, destination.as_ref().map(|(_, _, after)| *after));
            match destination {
                Some(_) => gtk::gdk::DragAction::MOVE,
                None => gtk::gdk::DragAction::empty(),
            }
        });
        target.connect_leave(|target| {
            if let Some(row) = target.widget() {
                crate::window::show_drop_marker(&row, None);
            }
        });
        let page = Rc::downgrade(self);
        target.connect_drop(move |target, value, _, y| {
            let Some(row) = target.widget() else {
                return false;
            };
            crate::window::show_drop_marker(&row, None);
            let Some(page) = page.upgrade() else {
                return false;
            };
            let Some((uri, to, _)) = page.drop_destination(Some(value), index, &row, y) else {
                return false;
            };
            page.change(Change::MoveTo(uri, to));
            true
        });
        row.add_controller(target);
    }

    /// Où irait le flux glissé (`value`) si on le déposait à la hauteur `y` de la rangée
    /// d'indice `over` : son adresse, son nouvel indice et si le dépôt se fait après la rangée.
    /// `None` quand rien ne changerait.
    fn drop_destination(
        &self,
        value: Option<&glib::Value>,
        over: usize,
        row: &gtk::Widget,
        y: f64,
    ) -> Option<(String, usize, bool)> {
        let uri = value.and_then(|v| v.get::<String>().ok())?;
        let from = self.saved.borrow().iter().position(|(f, _)| f.uri == uri)?;
        let after = y > f64::from(row.height()) / 2.0;
        let to = crate::window::drop_index(from, over, after)?;
        Some((uri, to, after))
    }

    fn load_found(self: &Rc<Self>) {
        let query = self.search.text().to_string();
        self.found_title.set_title(&if query.trim().is_empty() {
            gettext("Flux populaires")
        } else {
            gettext("Résultats")
        });
        let Some(session) = self.host.session.clone() else {
            let needle = query.trim().to_lowercase();
            let found = demo_feeds()
                .into_iter()
                .filter(|feed| {
                    needle.is_empty()
                        || feed.name.to_lowercase().contains(&needle)
                        || feed.description.to_lowercase().contains(&needle)
                })
                .collect();
            self.show_found(Ok(found));
            return;
        };
        let task = runtime().spawn(discover(session, Some(query.clone())));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            // Une recherche plus récente a remplacé celle-ci.
            if page.search.text() != query {
                return;
            }
            match result {
                Err(error) if error.expired => (page.host.expired)(error.message),
                result => page.show_found(result),
            }
        });
    }

    fn show_found(self: &Rc<Self>, result: Result<Vec<FeedInfo>, ApiError>) {
        self.found_list.remove_all();
        self.found.borrow_mut().clear();
        let feeds = match result {
            Ok(feeds) if !feeds.is_empty() => feeds,
            Ok(_) => {
                self.found_list.append(
                    &adw::ActionRow::builder()
                        .title(gettext("Aucun flux trouvé."))
                        .css_classes(["dim-label"])
                        .build(),
                );
                return;
            }
            Err(error) => {
                self.found_list.append(
                    &adw::ActionRow::builder()
                        .title(&error.message)
                        .use_markup(false)
                        .css_classes(["dim-label"])
                        .build(),
                );
                return;
            }
        };
        for info in feeds {
            let row = feed_row(&info);
            let add = gtk::Button::builder()
                .valign(gtk::Align::Center)
                .css_classes(["pill", "small-pill"])
                .build();
            row.add_suffix(&add);
            let (uri, weak) = (info.uri.clone(), Rc::downgrade(self));
            add.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.change(Change::Save(uri.clone(), true));
                }
            });
            let weak = Rc::downgrade(self);
            let opened = info.clone();
            row.connect_activated(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open(&opened);
                }
            });
            // L'état du bouton suit la liste des flux enregistrés.
            self.found.borrow_mut().push((info, add));
            self.found_list.append(&row);
        }
        self.show_found_states();
    }

    /// « Ajouter » ou « Ajouté », selon les flux enregistrés.
    fn show_found_states(&self) {
        let saved: Vec<String> = self
            .saved
            .borrow()
            .iter()
            .map(|(f, _)| f.uri.clone())
            .collect();
        for (info, button) in self.found.borrow().iter() {
            let is_saved = saved.contains(&info.uri);
            button.set_label(&if is_saved {
                gettext("Ajouté")
            } else {
                gettext("Ajouter")
            });
            button.set_sensitive(!is_saved);
            let name = if is_saved {
                gettext("{} est dans vos flux")
            } else {
                gettext("Ajouter {} à vos flux")
            }
            .replace("{}", &info.name);
            button.update_property(&[gtk::accessible::Property::Label(&name)]);
            if is_saved {
                button.remove_css_class("suggested-action");
            } else {
                button.add_css_class("suggested-action");
            }
        }
    }

    fn open(&self, info: &FeedInfo) {
        (self.host.push)(feed_page(&info.name, &info.uri, &self.host));
    }

    /// Applique le changement à l'écran, puis aux préférences du compte.
    fn change(self: &Rc<Self>, change: Change) {
        // À l'écran d'abord (et seulement, en démonstration).
        {
            let mut saved = self.saved.borrow_mut();
            match &change {
                Change::Save(uri, pinned) => {
                    if !saved.iter().any(|(f, _)| &f.uri == uri) {
                        let info = self.known(uri).unwrap_or_else(|| FeedInfo {
                            uri: uri.clone(),
                            ..Default::default()
                        });
                        saved.push((
                            SavedFeed {
                                uri: uri.clone(),
                                pinned: *pinned,
                            },
                            info,
                        ));
                    }
                }
                Change::Remove(uri) => saved.retain(|(f, _)| &f.uri != uri),
                // Les listes ne sont pas affichées sur cette page.
                Change::PinList(..) => {}
                Change::Pin(uri, pinned) => {
                    for (feed, _) in saved.iter_mut().filter(|(f, _)| &f.uri == uri) {
                        feed.pinned = *pinned;
                    }
                }
                Change::Move(uri, delta) => {
                    if let Some(at) = saved.iter().position(|(f, _)| &f.uri == uri) {
                        let target = at as i32 + delta;
                        if target >= 0 && (target as usize) < saved.len() {
                            saved.swap(at, target as usize);
                        }
                    }
                }
                Change::MoveTo(uri, to) => {
                    if let Some(at) = saved.iter().position(|(f, _)| &f.uri == uri)
                        && *to < saved.len()
                    {
                        let moved = saved.remove(at);
                        saved.insert(*to, moved);
                    }
                }
            }
        }
        // Reconstruire la liste après la fin du signal en cours (le bouton
        // qui l'a émis en fait partie).
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(page) = weak.upgrade() {
                page.show_saved();
                page.show_found_states();
            }
        });
        let Some(session) = self.host.session.clone() else {
            return;
        };
        let task = runtime().spawn(async move {
            moderation::update_preferences(session, move |preferences| change.apply(preferences))
                .await
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(_) => (page.on_changed)(),
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => {
                    glib::g_debug!("tsubame", "flux : {}", error.message);
                    page.host.toasts.add_toast(adw::Toast::new(&gettext(
                        "La liste des flux n'a pas été enregistrée.",
                    )));
                    page.load_saved();
                }
            }
        });
    }

    /// Informations d'un flux affiché dans la découverte.
    fn known(&self, uri: &str) -> Option<FeedInfo> {
        self.found
            .borrow()
            .iter()
            .map(|(info, _)| info.clone())
            .chain(demo_feeds())
            .find(|feed| feed.uri == uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> Vec<Value> {
        vec![
            json!({"$type": "app.bsky.actor.defs#adultContentPref", "enabled": false}),
            json!({"$type": SAVED_FEEDS, "items": [
                {"id": "1", "type": "timeline", "value": "following", "pinned": true},
                {"id": "2", "type": "feed", "value": "at://a/app.bsky.feed.generator/x", "pinned": true},
                {"id": "3", "type": "list", "value": "at://a/app.bsky.graph.list/l", "pinned": false},
                {"id": "4", "type": "feed", "value": "at://a/app.bsky.feed.generator/y", "pinned": false},
            ]}),
        ]
    }

    #[test]
    fn saved_feeds_are_read_in_order() {
        let feeds = saved_feeds(&prefs());
        assert_eq!(
            feeds,
            vec![
                SavedFeed {
                    uri: "at://a/app.bsky.feed.generator/x".into(),
                    pinned: true
                },
                SavedFeed {
                    uri: "at://a/app.bsky.feed.generator/y".into(),
                    pinned: false
                },
            ]
        );
    }

    #[test]
    fn changes_keep_other_items() {
        let x = "at://a/app.bsky.feed.generator/x".to_owned();
        let y = "at://a/app.bsky.feed.generator/y".to_owned();
        let z = "at://a/app.bsky.feed.generator/z".to_owned();
        let mut p = prefs();
        Change::Move(y.clone(), -1).apply(&mut p);
        assert_eq!(saved_feeds(&p)[0].uri, y);
        // Le fil Following et la liste n'ont pas bougé.
        let items = saved_items(&p);
        assert_eq!(items[0]["value"], "following");
        assert_eq!(items[2]["type"], "list");
        Change::Move(y.clone(), -1).apply(&mut p);
        assert_eq!(saved_feeds(&p)[0].uri, y, "déjà en tête");
        Change::MoveTo(y.clone(), 1).apply(&mut p);
        assert_eq!(saved_feeds(&p)[1].uri, y);
        assert_eq!(
            saved_items(&p)[2]["type"],
            "list",
            "la liste reste à sa place"
        );
        Change::MoveTo(y.clone(), 0).apply(&mut p);
        assert_eq!(saved_feeds(&p)[0].uri, y);
        Change::MoveTo(y.clone(), 9).apply(&mut p);
        assert_eq!(saved_feeds(&p)[0].uri, y, "hors limites : rien ne change");
        Change::Pin(x.clone(), false).apply(&mut p);
        assert!(!saved_feeds(&p)[1].pinned);
        Change::Save(z.clone(), true).apply(&mut p);
        Change::Save(z.clone(), true).apply(&mut p);
        assert_eq!(saved_feeds(&p).len(), 3);
        Change::Remove(x).apply(&mut p);
        assert_eq!(saved_feeds(&p).len(), 2);
        assert_eq!(p[0]["$type"], "app.bsky.actor.defs#adultContentPref");
    }

    #[test]
    fn lists_can_be_pinned() {
        let mut p = Vec::new();
        let uri = "at://did:plc:a/app.bsky.graph.list/l";
        Change::PinList(uri.into(), true).apply(&mut p);
        Change::PinList(uri.into(), true).apply(&mut p);
        assert!(list_pinned(&p, uri));
        assert_eq!(saved_items(&p).len(), 2);
        let preferences = Value::Array(p.clone());
        assert_eq!(crate::api::pinned_feed_uris(&preferences), [uri]);
        Change::PinList(uri.into(), false).apply(&mut p);
        assert!(!list_pinned(&p, uri));
    }

    #[test]
    fn saving_creates_the_preference() {
        let mut p = Vec::new();
        Change::Save("at://a/app.bsky.feed.generator/x".into(), true).apply(&mut p);
        let items = saved_items(&p);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["value"], "following");
        assert_eq!(saved_feeds(&p).len(), 1);
        // Préférences valides pour putPreferences.
        let _: atrium_api::app::bsky::actor::defs::Preferences =
            serde_json::from_value(Value::Array(p)).unwrap();
    }
}
