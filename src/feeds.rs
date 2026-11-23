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
use gettextrs::{gettext, ngettext};
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
}

impl Change {
    pub fn apply(&self, preferences: &mut Vec<Value>) {
        let items = saved_pref(preferences);
        match self {
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
            Change::Move(uri, delta) => {
                // Échange avec le flux voisin ; les autres éléments (fil
                // Following, listes) gardent leur place.
                let feeds: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.get("type").and_then(Value::as_str) == Some("feed"))
                    .map(|(i, _)| i)
                    .collect();
                let Some(at) = feeds.iter().position(|&i| is_feed(&items[i], uri)) else {
                    return;
                };
                let target = at as i32 + delta;
                if target < 0 || target as usize >= feeds.len() {
                    return;
                }
                items.swap(feeds[at], feeds[target as usize]);
            }
        }
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
            let weak = Rc::downgrade(self);
            row.connect_activated(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open(&info);
                }
            });
            self.saved_list.append(&row);
        }
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
        let feed = FeedPage::new(
            &info.name,
            Source::Feed(info.uri.clone()),
            data::posts(data::Feed::Discover),
            self.host.clone(),
        );
        (self.host.push)(feed.navigation_page(&info.name));
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
