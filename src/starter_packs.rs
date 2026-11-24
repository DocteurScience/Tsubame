//! Starter packs : sélections de comptes (et de flux) à suivre d'un coup,
//! proposées dans Explorer (maquette `Explorer.dc.html`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};

use crate::api::{self, ApiError};
use crate::auth::Session;
use crate::data::{self, Actor};
use crate::explore::actor_row;
use crate::feeds::{FeedInfo, feed_row, info_from_view};
use crate::profile::Host;
use crate::runtime::runtime;

/// Starter pack, tel qu'affiché dans une liste ou sur sa page.
#[derive(Debug, Clone, Default)]
pub struct StarterPack {
    pub uri: String,
    pub name: String,
    pub description: String,
    /// Nom de la personne qui l'a créé.
    pub creator: String,
    /// Son DID.
    pub creator_did: String,
    /// Nombre de comptes, s'il est connu.
    pub members: Option<usize>,
    /// Liste des comptes (`app.bsky.graph.list`).
    pub list: Option<String>,
    pub feeds: Vec<FeedInfo>,
}

fn record_field(record: &serde_json::Value, field: &str) -> String {
    record
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn creator_name(creator: &atrium_api::app::bsky::actor::defs::ProfileViewBasic) -> String {
    creator
        .display_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| creator.handle.as_str().to_owned())
}

fn from_view(view: &atrium_api::app::bsky::graph::defs::StarterPackView) -> StarterPack {
    let record = serde_json::to_value(&view.record).unwrap_or_default();
    StarterPack {
        uri: view.uri.clone(),
        name: record_field(&record, "name"),
        description: record_field(&record, "description"),
        creator: creator_name(&view.creator),
        creator_did: view.creator.did.as_str().to_owned(),
        members: view.list.as_ref().and_then(|l| l.list_item_count),
        list: view.list.as_ref().map(|l| l.uri.clone()),
        feeds: view.feeds.iter().flatten().map(info_from_view).collect(),
    }
}

fn from_basic(view: &atrium_api::app::bsky::graph::defs::StarterPackViewBasic) -> StarterPack {
    let record = serde_json::to_value(&view.record).unwrap_or_default();
    StarterPack {
        uri: view.uri.clone(),
        name: record_field(&record, "name"),
        description: record_field(&record, "description"),
        creator: creator_name(&view.creator),
        creator_did: view.creator.did.as_str().to_owned(),
        members: view.list_item_count,
        list: None,
        feeds: Vec::new(),
    }
}

/// Starter packs suggérés (`getSuggestedStarterPacks`).
pub async fn suggested(session: Arc<Session>) -> Result<Vec<StarterPack>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .unspecced
        .get_suggested_starter_packs(
            atrium_api::app::bsky::unspecced::get_suggested_starter_packs::ParametersData {
                limit: 5.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(api::request_failed)?;
    Ok(output.data.starter_packs.iter().map(from_view).collect())
}

/// Recherche de starter packs (`searchStarterPacks`).
pub async fn search(session: Arc<Session>, query: String) -> Result<Vec<StarterPack>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .search_starter_packs(
            atrium_api::app::bsky::graph::search_starter_packs::ParametersData {
                cursor: None,
                limit: 25.try_into().ok(),
                q: query,
            }
            .into(),
        )
        .await
        .map_err(api::request_failed)?;
    Ok(output.data.starter_packs.iter().map(from_basic).collect())
}

/// Starter packs créés par un compte (`getActorStarterPacks`).
pub async fn by_actor(session: Arc<Session>, actor: String) -> Result<Vec<StarterPack>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_actor_starter_packs(
            atrium_api::app::bsky::graph::get_actor_starter_packs::ParametersData {
                actor: actor.parse().map_err(|_| api::invalid_actor())?,
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(api::request_failed)?;
    Ok(output.data.starter_packs.iter().map(from_basic).collect())
}

/// Compte d'un starter pack, avec l'élément qui l'y place (pour le
/// retirer).
pub type Member = (Actor, Option<String>);

/// Un starter pack et ses comptes (`getStarterPack`, puis `getList`).
pub async fn load(
    session: Arc<Session>,
    uri: String,
) -> Result<(StarterPack, Vec<Member>), ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_starter_pack(
            atrium_api::app::bsky::graph::get_starter_pack::ParametersData { starter_pack: uri }
                .into(),
        )
        .await
        .map_err(api::request_failed)?;
    let pack = from_view(&output.data.starter_pack);
    let members = match &pack.list {
        Some(list) => crate::lists::items(session.clone(), list.clone())
            .await?
            .into_iter()
            .map(|(actor, item)| (actor, Some(item)))
            .collect(),
        None => Vec::new(),
    };
    Ok((pack, members))
}

/// Suit les comptes qui ne le sont pas encore, sauf soi-même. Rend le
/// nombre de nouveaux abonnements, et la première erreur s'il y en a eu.
pub async fn follow_all(
    session: Arc<Session>,
    members: Vec<Actor>,
) -> (Vec<(String, String)>, Option<ApiError>) {
    let mut done = Vec::new();
    for actor in members {
        if actor.following.is_some() || actor.did == session.account.did {
            continue;
        }
        match api::follow(session.clone(), actor.did.clone()).await {
            Ok(record) => done.push((actor.did, record)),
            Err(error) => return (done, Some(error)),
        }
    }
    (done, None)
}

pub fn demo_packs() -> Vec<StarterPack> {
    vec![
        StarterPack {
            uri: "demo:pack-gnome".into(),
            name: "Communauté GNOME francophone".into(),
            description: "Contributrices et contributeurs de GNOME qui publient en français."
                .into(),
            creator: "Lucie Morel".into(),
            creator_did: String::new(),
            members: Some(24),
            list: None,
            feeds: crate::feeds::demo_feeds().into_iter().take(1).collect(),
        },
        StarterPack {
            uri: "demo:pack-nature".into(),
            name: "Photographes de nature".into(),
            description: "Montagne, forêts et lumière du matin.".into(),
            creator: "Aya Koné".into(),
            creator_did: String::new(),
            members: Some(40),
            list: None,
            feeds: Vec::new(),
        },
    ]
}

/// Rangée d'un starter pack : nom, auteur et nombre de comptes.
pub fn pack_row(pack: &StarterPack) -> adw::ActionRow {
    let mut subtitle = gettext("Par {}").replace("{}", &pack.creator);
    if let Some(members) = pack.members {
        subtitle = format!(
            "{subtitle} · {}",
            ngettext(
                "{} compte",
                "{} comptes",
                members.min(u32::MAX as usize) as u32
            )
            .replace("{}", &members.to_string())
        );
    }
    if !pack.feeds.is_empty() {
        subtitle = format!(
            "{subtitle} · {}",
            ngettext("{} flux", "{} flux", pack.feeds.len() as u32)
                .replace("{}", &pack.feeds.len().to_string())
        );
    }
    let row = adw::ActionRow::builder()
        .title(&pack.name)
        .subtitle(&subtitle)
        .use_markup(false)
        .activatable(true)
        .build();
    row.add_prefix(
        &adw::Avatar::builder()
            .size(40)
            .text(&pack.name)
            .show_initials(true)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build(),
    );
    row.add_suffix(&crate::card::decorative_icon("go-next-symbolic"));
    row
}

/// Page d'un starter pack : description, « Tout suivre », ses flux et ses
/// comptes.
pub struct StarterPackPage {
    root: adw::ToolbarView,
    host: Host,
    uri: String,
    title: adw::WindowTitle,
    description: gtk::Label,
    follow_all: gtk::Button,
    feeds_title: gtk::Label,
    feeds: gtk::ListBox,
    people: gtk::ListBox,
    members: RefCell<Vec<Member>>,
    /// Le starter pack chargé (sa liste, son auteur).
    pack: RefCell<StarterPack>,
    /// Menu de l'auteur : ajouter des comptes ou des flux, modifier, supprimer.
    menu: gtk::MenuButton,
    /// « Ajouter un flux… », possible tant qu'il y a moins de trois flux.
    add_feed_action: gio::SimpleAction,
    weak: RefCell<std::rc::Weak<Self>>,
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["heading"])
        .accessible_role(gtk::AccessibleRole::Heading)
        .build()
}

fn boxed_list(label: &str) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.update_property(&[gtk::accessible::Property::Label(label)]);
    list
}

impl StarterPackPage {
    pub fn new(pack: &StarterPack, host: Host) -> Rc<Self> {
        let title = adw::WindowTitle::new(&pack.name, &gettext("Starter pack"));
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let description = gtk::Label::builder().wrap(true).xalign(0.0).build();
        let follow_all = gtk::Button::builder()
            .label(gettext("Tout suivre"))
            .halign(gtk::Align::Start)
            .css_classes(["suggested-action", "pill"])
            .build();
        let feeds_title = heading(&gettext("Flux"));
        let feeds = boxed_list(&gettext("Flux du starter pack"));
        let people = boxed_list(&gettext("Comptes du starter pack"));
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&description);
        content.append(&follow_all);
        content.append(&feeds_title);
        content.append(&feeds);
        content.append(&heading(&gettext("Comptes")));
        content.append(&people);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .child(&content)
                    .build(),
            )
            .build();
        let menu_model = gio::Menu::new();
        menu_model.append(Some(&gettext("Ajouter un compte…")), Some("pack.add"));
        menu_model.append(Some(&gettext("Ajouter un flux…")), Some("pack.add-feed"));
        menu_model.append(Some(&gettext("Modifier…")), Some("pack.edit"));
        menu_model.append(
            Some(&gettext("Supprimer le starter pack…")),
            Some("pack.delete"),
        );
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(gettext("Options du starter pack"))
            .menu_model(&menu_model)
            .visible(false)
            .build();
        menu.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Options du starter pack",
        ))]);
        header.pack_end(&menu);
        let root = adw::ToolbarView::builder().content(&scroll).build();
        root.add_top_bar(&header);
        let page = Rc::new(Self {
            root,
            host,
            uri: pack.uri.clone(),
            title,
            description,
            follow_all,
            feeds_title,
            feeds,
            people,
            members: RefCell::default(),
            pack: RefCell::new(pack.clone()),
            menu,
            add_feed_action: gio::SimpleAction::new("add-feed", None),
            weak: RefCell::default(),
        });
        page.weak.replace(Rc::downgrade(&page));
        let actions = gio::SimpleActionGroup::new();
        for (name, handler) in [
            ("add", Self::add_member as fn(&Rc<Self>)),
            ("edit", Self::edit),
            ("delete", Self::confirm_delete),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(&page);
            action.connect_activate(move |_, _| {
                if let Some(page) = weak.upgrade() {
                    handler(&page);
                }
            });
            actions.add_action(&action);
        }
        {
            let weak = Rc::downgrade(&page);
            page.add_feed_action.connect_activate(move |_, _| {
                if let Some(page) = weak.upgrade() {
                    page.add_feed();
                }
            });
            actions.add_action(&page.add_feed_action);
        }
        page.root.insert_action_group("pack", Some(&actions));
        page.show(pack, None);
        let weak = Rc::downgrade(&page);
        page.follow_all.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.follow_everyone();
            }
        });
        let weak = Rc::downgrade(&page);
        page.feeds.connect_row_activated(move |_, row| {
            let Some(page) = weak.upgrade() else { return };
            let uri = row.widget_name();
            let name = row
                .downcast_ref::<adw::ActionRow>()
                .map(|r| r.title().to_string())
                .unwrap_or_default();
            (page.host.push)(crate::feeds::feed_page(&name, &uri, &page.host));
        });
        page.load();
        page
    }

    /// Page à empiler dans la navigation.
    pub fn navigation_page(self: &Rc<Self>) -> adw::NavigationPage {
        let page = adw::NavigationPage::builder()
            .title(self.title.title())
            .child(&self.root)
            .build();
        let keep = self.clone();
        page.connect_destroy(move |_| {
            let _ = &keep;
        });
        page
    }

    /// Mon starter pack : je peux le modifier.
    fn mine(&self) -> bool {
        let pack = self.pack.borrow();
        !pack.creator_did.is_empty() && self.host.me.as_deref() == Some(pack.creator_did.as_str())
    }

    fn show(&self, pack: &StarterPack, members: Option<Vec<Member>>) {
        self.pack.replace(pack.clone());
        self.menu.set_visible(self.mine());
        self.title.set_title(&pack.name);
        self.description.set_label(&pack.description);
        self.description.set_visible(!pack.description.is_empty());
        while let Some(row) = self.feeds.first_child() {
            self.feeds.remove(&row);
        }
        let mine = self.mine();
        for info in &pack.feeds {
            let row = feed_row(info);
            row.set_widget_name(&info.uri);
            if mine {
                let remove = gtk::Button::builder()
                    .icon_name("list-remove-symbolic")
                    .tooltip_text(gettext("Retirer du starter pack"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                remove.update_property(&[gtk::accessible::Property::Label(
                    &gettext("Retirer {} du starter pack").replace("{}", &info.name),
                )]);
                let (uri, weak) = (info.uri.clone(), self.this());
                remove.connect_clicked(move |_| {
                    if let Some(page) = weak.upgrade() {
                        let feeds = page
                            .feed_uris()
                            .into_iter()
                            .filter(|feed| *feed != uri)
                            .collect();
                        page.set_feeds(feeds, gettext("Flux retiré du starter pack."));
                    }
                });
                row.add_suffix(&remove);
            }
            self.feeds.append(&row);
        }
        self.add_feed_action
            .set_enabled(pack.feeds.len() < crate::lists::MAX_PACK_FEEDS);
        self.feeds.set_visible(!pack.feeds.is_empty());
        self.feeds_title.set_visible(!pack.feeds.is_empty());
        if let Some(members) = members {
            self.show_members(members);
        }
    }

    fn load(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            let pack = demo_packs()
                .into_iter()
                .find(|p| p.uri == self.uri)
                .unwrap_or_default();
            self.show(
                &pack,
                Some(data::demo_actors().into_iter().map(|a| (a, None)).collect()),
            );
            return;
        };
        let task = runtime().spawn(load(session, self.uri.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok((pack, members)) => page.show(&pack, Some(members)),
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => {
                    page.follow_all.set_sensitive(false);
                    page.host.toasts.add_toast(adw::Toast::new(&error.message));
                }
            }
        });
    }

    fn follow_everyone(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            self.host.toasts.add_toast(adw::Toast::new(&gettext(
                "Mode démonstration : rien n'est publié.",
            )));
            return;
        };
        self.follow_all.set_sensitive(false);
        let members = self
            .members
            .borrow()
            .iter()
            .map(|(a, _)| a.clone())
            .collect();
        let task = runtime().spawn(follow_all(session, members));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok((done, failure)) = task.await else {
                return;
            };
            let Some(page) = weak.upgrade() else { return };
            let count = done.len();
            let message = match failure {
                Some(error) => {
                    glib::g_debug!("tsubame", "tout suivre : {}", error.message);
                    ngettext(
                        "{} compte suivi, puis une erreur s'est produite.",
                        "{} comptes suivis, puis une erreur s'est produite.",
                        count as u32,
                    )
                }
                None => ngettext("{} compte suivi.", "{} comptes suivis.", count as u32),
            }
            .replace("{}", &count.to_string());
            page.host.toasts.add_toast(adw::Toast::new(&message));
            // Les rangées affichent le nouvel état.
            let mut members = page.members.borrow().clone();
            for (actor, _) in &mut members {
                if let Some((_, record)) = done.iter().find(|(did, _)| *did == actor.did) {
                    actor.following = Some(record.clone());
                }
            }
            page.show_members(members);
        });
    }

    /// Les comptes, et « Tout suivre » s'il en reste à suivre. Sur mon
    /// starter pack, chaque compte a « Retirer du starter pack ».
    fn show_members(&self, members: Vec<Member>) {
        while let Some(row) = self.people.first_child() {
            self.people.remove(&row);
        }
        let mine = self.mine();
        for (actor, item) in &members {
            let row = actor_row(actor, self.host.session.clone(), &self.host.toasts);
            if let (true, Some(item), Some(session)) = (mine, item, self.host.session.clone()) {
                let remove = gtk::Button::builder()
                    .icon_name("list-remove-symbolic")
                    .tooltip_text(gettext("Retirer du starter pack"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                remove.update_property(&[gtk::accessible::Property::Label(
                    &gettext("Retirer {} du starter pack").replace("{}", &actor.name),
                )]);
                let (item, weak) = (item.clone(), self.this());
                remove.connect_clicked(move |_| {
                    let task = runtime().spawn(api::delete(session.clone(), item.clone()));
                    let weak = weak.clone();
                    glib::spawn_future_local(async move {
                        let result = task.await.unwrap_or_else(|_| {
                            Err(ApiError::new(gettext("Requête interrompue.")))
                        });
                        let Some(page) = weak.upgrade() else { return };
                        match result {
                            Ok(()) => page.load(),
                            Err(error) => {
                                page.host.toasts.add_toast(adw::Toast::new(&error.message))
                            }
                        }
                    });
                });
                row.add_suffix(&remove);
            }
            self.people.append(&row);
        }
        self.follow_all.set_sensitive(
            members
                .iter()
                .any(|(a, _)| a.following.is_none() && Some(&a.did) != self.host.me.as_ref()),
        );
        self.members.replace(members);
    }

    /// Référence faible à la page, pour les rappels des rangées.
    fn this(&self) -> std::rc::Weak<Self> {
        self.weak.borrow().clone()
    }

    fn feed_uris(&self) -> Vec<String> {
        self.pack
            .borrow()
            .feeds
            .iter()
            .map(|feed| feed.uri.clone())
            .collect()
    }

    /// Remplace les flux du starter pack, puis le recharge.
    fn set_feeds(self: &Rc<Self>, feeds: Vec<String>, success: String) {
        let Some(session) = self.host.session.clone() else {
            self.host.toasts.add_toast(adw::Toast::new(&gettext(
                "Mode démonstration : rien n'est publié.",
            )));
            return;
        };
        let task = runtime().spawn(Box::pin(crate::lists::update_pack_feeds(
            session,
            self.uri.clone(),
            feeds,
        )));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    page.host.toasts.add_toast(adw::Toast::new(&success));
                    page.load();
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    fn add_feed(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        crate::feeds::pick_feed(
            &self.root,
            self.host.session.clone(),
            self.feed_uris(),
            move |info| {
                let Some(page) = weak.upgrade() else { return };
                let mut feeds = page.feed_uris();
                feeds.push(info.uri.clone());
                page.set_feeds(
                    feeds,
                    gettext("« {} » ajouté au starter pack.").replace("{}", &info.name),
                );
            },
        );
    }

    fn add_member(self: &Rc<Self>) {
        let Some(list) = self.pack.borrow().list.clone() else {
            return;
        };
        let (session, weak) = (self.host.session.clone(), Rc::downgrade(self));
        crate::lists::pick_account(
            &self.root,
            &gettext("Ajouter un compte"),
            session.clone(),
            move |actor| {
                let (Some(session), Some(page)) = (session.clone(), weak.upgrade()) else {
                    return;
                };
                let task = runtime().spawn(crate::lists::add_member(
                    session,
                    list.clone(),
                    actor.did.clone(),
                ));
                let weak = Rc::downgrade(&page);
                glib::spawn_future_local(async move {
                    let result = task
                        .await
                        .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                    let Some(page) = weak.upgrade() else { return };
                    match result {
                        Ok(_) => {
                            page.host.toasts.add_toast(adw::Toast::new(
                                &gettext("@{} ajouté au starter pack.")
                                    .replace("{}", &actor.handle),
                            ));
                            page.load();
                        }
                        Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
                    }
                });
            },
        );
    }

    fn edit(self: &Rc<Self>) {
        let pack = self.pack.borrow().clone();
        let weak = Rc::downgrade(self);
        crate::lists::starter_pack_dialog(
            &self.root,
            Some((&pack.name, &pack.description)),
            move |name, description| {
                let Some(page) = weak.upgrade() else { return };
                let Some(session) = page.host.session.clone() else {
                    return;
                };
                let (uri, list) = (pack.uri.clone(), pack.list.clone());
                let task = runtime().spawn(async move {
                    Box::pin(crate::lists::update_list(
                        session.clone(),
                        uri,
                        name.clone(),
                        description,
                        crate::edit_profile::ImageChange::Keep,
                    ))
                    .await?;
                    // La liste porte le même nom.
                    if let Some(list) = list {
                        Box::pin(crate::lists::update_list(
                            session,
                            list,
                            name,
                            String::new(),
                            crate::edit_profile::ImageChange::Keep,
                        ))
                        .await?;
                    }
                    Ok::<_, ApiError>(())
                });
                let weak = Rc::downgrade(&page);
                glib::spawn_future_local(async move {
                    let result = task
                        .await
                        .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                    let Some(page) = weak.upgrade() else { return };
                    match result {
                        Ok(()) => page.load(),
                        Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
                    }
                });
            },
        );
    }

    fn confirm_delete(self: &Rc<Self>) {
        let confirm = adw::AlertDialog::new(
            Some(&gettext("Supprimer ce starter pack ?")),
            Some(&gettext(
                "Il disparaîtra pour tout le monde. Ses comptes ne sont pas prévenus.",
            )),
        );
        confirm.add_responses(&[
            ("cancel", &gettext("Annuler")),
            ("delete", &gettext("Supprimer")),
        ]);
        confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        confirm.set_default_response(Some("cancel"));
        confirm.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        confirm.connect_response(Some("delete"), move |_, _| {
            let Some(page) = weak.upgrade() else { return };
            let Some(session) = page.host.session.clone() else {
                return;
            };
            let pack = page.pack.borrow().clone();
            let task = runtime().spawn(crate::lists::delete_starter_pack(
                session, pack.uri, pack.list,
            ));
            let weak = Rc::downgrade(&page);
            glib::spawn_future_local(async move {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(()) => {
                        page.host
                            .toasts
                            .add_toast(adw::Toast::new(&gettext("Starter pack supprimé.")));
                        let _ = page.root.activate_action("navigation.pop", None);
                    }
                    Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
                }
            });
        });
        confirm.present(Some(&self.root));
    }
}
