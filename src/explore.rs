//! Page Explorer (maquette `docs/mockups/source/Explorer.dc.html`) :
//! recherche de posts et de personnes, sujets du moment, personnes à
//! suivre.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::{gettext, pgettext};
use gtk::glib;

use crate::api::{self, ApiError, Page, Source};
use crate::auth::Session;
use crate::data::{self, Actor, Post, Topic};
use crate::images;
use crate::post_list::PostList;
use crate::profile::Host;
use crate::runtime::runtime;
use crate::starter_packs::{self, StarterPack, StarterPackPage, pack_row};

/// Onglets des résultats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Results {
    Posts,
    People,
    Feeds,
    Packs,
}

pub struct ExplorePage {
    pub root: adw::ToolbarView,
    pub entry: gtk::SearchEntry,
    host: Host,
    stack: gtk::Stack,
    topics: gtk::ListBox,
    suggested: gtk::ListBox,
    results: gtk::Stack,
    posts: PostList,
    people: gtk::ListBox,
    people_status: adw::StatusPage,
    /// Starter packs suggérés (sans recherche) et trouvés.
    packs: gtk::ListBox,
    pack_results: gtk::ListBox,
    /// Flux trouvés (adresse et nom par rangée).
    feed_results: gtk::ListBox,
    known_packs: RefCell<Vec<StarterPack>>,
    query: RefCell<String>,
    /// Numéro de la dernière recherche de personnes : les réponses à une
    /// recherche dépassée sont ignorées.
    people_generation: Cell<u64>,
}

/// Étiquette d'onglet, raccourcie si la place manque (palier mobile).
fn tab_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build()
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .margin_top(12)
        .css_classes(["heading"])
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

fn clear(list: &gtk::ListBox) {
    list.remove_all();
}

/// Rangée d'une personne : avatar, nom, identifiant et bio ; « Suivre »
/// à droite. La rangée ouvre le profil.
pub fn actor_row(
    actor: &Actor,
    session: Option<Arc<Session>>,
    toasts: &adw::ToastOverlay,
) -> adw::ActionRow {
    let subtitle = if actor.bio.trim().is_empty() {
        format!("@{}", actor.handle)
    } else {
        format!("@{} · {}", actor.handle, actor.bio.replace('\n', " "))
    };
    let row = adw::ActionRow::builder()
        .title(&actor.name)
        .subtitle(&subtitle)
        .subtitle_lines(2)
        .use_markup(false)
        .activatable(true)
        .action_name("win.open-profile")
        .action_target(&actor.handle.to_variant())
        .build();
    let avatar = adw::Avatar::builder()
        .size(40)
        .text(&actor.name)
        .show_initials(true)
        .build();
    if let Some(url) = &actor.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    row.add_prefix(&avatar);

    // « Suivre » / « Abonné », comme dans la maquette.
    let following = Rc::new(RefCell::new(actor.following.clone()));
    let button = gtk::Button::builder()
        .valign(gtk::Align::Center)
        .css_classes(["pill", "small-pill"])
        .build();
    let name = actor.name.clone();
    let update = {
        let (button, following) = (button.clone(), following.clone());
        move || {
            let followed = following.borrow().is_some();
            button.set_label(&if followed {
                pgettext("bouton : vous suivez ce compte", "Abonné")
            } else {
                gettext("Suivre")
            });
            let accessible = if followed {
                gettext("Ne plus suivre {}")
            } else {
                gettext("Suivre {}")
            }
            .replace("{}", &name);
            button.set_tooltip_text(Some(&accessible));
            button.update_property(&[gtk::accessible::Property::Label(&accessible)]);
            if followed {
                button.remove_css_class("suggested-action");
            } else {
                button.add_css_class("suggested-action");
            }
        }
    };
    update();
    let did = actor.did.clone();
    let toasts = toasts.clone();
    button.connect_clicked(move |button| {
        let current = following.borrow().clone();
        let Some(session) = session.clone().filter(|_| !did.is_empty()) else {
            // Démonstration : bascule locale.
            following.replace(match current {
                Some(_) => None,
                None => Some(String::new()),
            });
            update();
            return;
        };
        button.set_sensitive(false);
        let did = did.clone();
        let task = runtime().spawn(async move {
            match current {
                Some(record) => api::delete(session, record).await.map(|()| None),
                None => api::follow(session, did).await.map(Some),
            }
        });
        let (button, following, update, toasts) = (
            button.clone(),
            following.clone(),
            update.clone(),
            toasts.clone(),
        );
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            button.set_sensitive(true);
            match result {
                Ok(record) => {
                    following.replace(record);
                    update();
                }
                Err(error) => {
                    glib::g_debug!("tsubame", "abonnement : {}", error.message);
                    toasts.add_toast(adw::Toast::new(&gettext("L'abonnement n'a pas changé.")));
                }
            }
        });
    });
    row.add_suffix(&button);
    row
}

fn topic_row(rank: usize, topic: &Topic) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&topic.name)
        .use_markup(false)
        .activatable(true)
        .build();
    row.add_prefix(
        &gtk::Label::builder()
            .label(rank.to_string())
            .width_chars(2)
            .css_classes(["dim-label", "heading", "numeric"])
            .build(),
    );
    row.add_suffix(&crate::card::decorative_icon("go-next-symbolic"));
    row
}

impl ExplorePage {
    pub fn new(host: Host) -> Rc<Self> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Rechercher des posts, des personnes, des flux…"))
            .search_delay(500)
            .hexpand(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&gettext("Rechercher"))]);
        let header = adw::HeaderBar::builder()
            .title_widget(
                &adw::Clamp::builder()
                    .maximum_size(480)
                    .child(&entry)
                    .build(),
            )
            .build();

        // Sans recherche : sujets du moment, personnes à suivre.
        let topics = boxed_list(&gettext("Sujets du moment"));
        let suggested = boxed_list(&gettext("Personnes à suivre"));
        let packs = boxed_list(&gettext("Starter packs"));
        let discover = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        discover.append(&heading(&gettext("Sujets du moment")));
        discover.append(&topics);
        discover.append(&heading(&gettext("Personnes à suivre")));
        discover.append(&suggested);
        discover.append(&heading(&gettext("Starter packs")));
        discover.append(&packs);
        let discover_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .child(&discover)
                    .build(),
            )
            .build();

        // Résultats : posts ou personnes.
        let tabs = gtk::Box::builder()
            .css_classes(["linked"])
            .halign(gtk::Align::Center)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        tabs.update_property(&[gtk::accessible::Property::Label(&gettext("Résultats"))]);
        let posts_tab = gtk::ToggleButton::builder()
            .child(&tab_label(&gettext("Posts")))
            .active(true)
            .build();
        let people_tab = gtk::ToggleButton::builder()
            .child(&tab_label(&gettext("Personnes")))
            .group(&posts_tab)
            .build();
        let feeds_tab = gtk::ToggleButton::builder()
            .child(&tab_label(&gettext("Flux")))
            .group(&posts_tab)
            .build();
        let packs_tab = gtk::ToggleButton::builder()
            .child(&tab_label(&gettext("Starter packs")))
            .group(&posts_tab)
            .build();
        tabs.append(&posts_tab);
        tabs.append(&people_tab);
        tabs.append(&feeds_tab);
        tabs.append(&packs_tab);
        let feed_results = boxed_list(&gettext("Flux trouvés"));
        let feeds_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .margin_top(12)
                    .margin_bottom(24)
                    .margin_start(12)
                    .margin_end(12)
                    .child(&feed_results)
                    .build(),
            )
            .build();
        let pack_results = boxed_list(&gettext("Starter packs trouvés"));
        let packs_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .margin_top(12)
                    .margin_bottom(24)
                    .margin_start(12)
                    .margin_end(12)
                    .child(&pack_results)
                    .build(),
            )
            .build();
        let posts = PostList::new(&gettext("Posts trouvés"), "");
        let people = boxed_list(&gettext("Personnes trouvées"));
        let people_status = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .css_classes(["compact"])
            .visible(false)
            .build();
        let people_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        people_box.append(&people);
        people_box.append(&people_status);
        let people_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .child(&people_box)
                    .build(),
            )
            .build();
        let results = gtk::Stack::new();
        results.add_named(&posts.root, Some("posts"));
        results.add_named(&people_scroll, Some("people"));
        results.add_named(&feeds_scroll, Some("feeds"));
        results.add_named(&packs_scroll, Some("packs"));
        let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        results_box.append(&tabs);
        results_box.append(&results);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&discover_scroll, Some("discover"));
        stack.add_named(&results_box, Some("results"));

        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(&header);

        let page = Rc::new(Self {
            root,
            entry,
            host,
            stack,
            topics,
            suggested,
            results,
            posts,
            people,
            people_status,
            packs,
            pack_results,
            feed_results,
            known_packs: RefCell::default(),
            query: RefCell::default(),
            people_generation: Cell::new(0),
        });

        let weak = Rc::downgrade(&page);
        page.entry.connect_search_changed(move |entry| {
            if let Some(page) = weak.upgrade() {
                page.search(entry.text().trim());
            }
        });
        let weak = Rc::downgrade(&page);
        page.entry.connect_activate(move |entry| {
            if let Some(page) = weak.upgrade() {
                page.search(entry.text().trim());
            }
        });
        for (tab, button) in [
            (Results::Posts, posts_tab),
            (Results::People, people_tab),
            (Results::Feeds, feeds_tab),
            (Results::Packs, packs_tab),
        ] {
            let weak = Rc::downgrade(&page);
            button.connect_toggled(move |button| {
                if button.is_active()
                    && let Some(page) = weak.upgrade()
                {
                    page.results.set_visible_child_name(match tab {
                        Results::Posts => "posts",
                        Results::People => "people",
                        Results::Feeds => "feeds",
                        Results::Packs => "packs",
                    });
                }
            });
        }
        let weak = Rc::downgrade(&page);
        page.posts.connect_post_activated(move |post| {
            if let Some(page) = weak.upgrade() {
                (page.host.open_post)(post);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_end_reached(move || {
            if let Some(page) = weak.upgrade() {
                page.load_posts(true);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_retry(move || {
            if let Some(page) = weak.upgrade() {
                page.load_posts(false);
            }
        });
        let weak = Rc::downgrade(&page);
        page.topics.connect_row_activated(move |_, row| {
            let Some(page) = weak.upgrade() else { return };
            let query = row
                .downcast_ref::<adw::ActionRow>()
                .and_then(|row| row.widget_name().strip_prefix("topic:").map(str::to_owned));
            if let Some(query) = query {
                page.entry.set_text(&query);
                page.search(&query);
            }
        });
        for list in [&page.packs, &page.pack_results] {
            let weak = Rc::downgrade(&page);
            list.connect_row_activated(move |_, row| {
                if let Some(page) = weak.upgrade() {
                    page.open_pack(&row.widget_name());
                }
            });
        }
        let weak = Rc::downgrade(&page);
        page.feed_results.connect_row_activated(move |_, row| {
            let Some(page) = weak.upgrade() else { return };
            let Some(row) = row.downcast_ref::<adw::ActionRow>() else {
                return;
            };
            let name = row.title().to_string();
            let feed = crate::feed_page::FeedPage::new(
                &name,
                Source::Feed(row.widget_name().to_string()),
                data::posts(data::Feed::Discover),
                page.host.clone(),
            );
            (page.host.push)(feed.navigation_page(&name));
        });

        page.load_discover();
        page
    }

    fn show_feeds(&self, feeds: Result<Vec<crate::feeds::FeedInfo>, ApiError>) {
        clear(&self.feed_results);
        match feeds {
            Ok(feeds) if !feeds.is_empty() => {
                for info in &feeds {
                    let row = crate::feeds::feed_row(info);
                    row.set_widget_name(&info.uri);
                    self.feed_results.append(&row);
                }
            }
            Ok(_) => self.feed_results.append(&message_row(&gettext(
                "Aucun flux ne correspond à cette recherche.",
            ))),
            Err(error) => self.feed_results.append(&message_row(&error.message)),
        }
    }

    fn load_feeds(self: &Rc<Self>) {
        let query = self.query.borrow().clone();
        let Some(session) = self.host.session.clone() else {
            let found = crate::feeds::demo_feeds()
                .into_iter()
                .filter(|f| matches(&f.name, &query) || matches(&f.description, &query))
                .collect();
            self.show_feeds(Ok(found));
            return;
        };
        let task = runtime().spawn(crate::feeds::discover(session, Some(query.clone())));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            if *page.query.borrow() != query {
                return;
            }
            page.show_feeds(result);
        });
    }

    fn open_pack(&self, uri: &str) {
        let pack = self
            .known_packs
            .borrow()
            .iter()
            .find(|p| p.uri == uri)
            .cloned();
        if let Some(pack) = pack {
            let page = StarterPackPage::new(&pack, self.host.clone());
            (self.host.push)(page.navigation_page());
        }
    }

    /// Remplit `list` de starter packs, ou d'un message.
    fn show_packs(&self, list: &gtk::ListBox, packs: Result<Vec<StarterPack>, ApiError>) {
        clear(list);
        match packs {
            Ok(packs) if !packs.is_empty() => {
                for pack in &packs {
                    let row = pack_row(pack);
                    row.set_widget_name(&pack.uri);
                    list.append(&row);
                }
                self.known_packs.borrow_mut().extend(packs);
            }
            Ok(_) => list.append(&message_row(&gettext("Aucun starter pack."))),
            Err(error) => list.append(&message_row(&error.message)),
        }
    }

    fn load_packs(self: &Rc<Self>) {
        let query = self.query.borrow().clone();
        let Some(session) = self.host.session.clone() else {
            let found = starter_packs::demo_packs()
                .into_iter()
                .filter(|p| matches(&p.name, &query) || matches(&p.creator, &query))
                .collect();
            self.show_packs(&self.pack_results, Ok(found));
            return;
        };
        let task = runtime().spawn(starter_packs::search(session, query.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            if *page.query.borrow() != query {
                return;
            }
            page.show_packs(&page.pack_results, result);
        });
    }

    /// Sujets du moment et personnes à suivre.
    fn load_discover(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            self.show_topics(Ok(data::demo_topics()));
            self.show_suggested(Ok(data::demo_actors()));
            self.show_packs(&self.packs, Ok(starter_packs::demo_packs()));
            return;
        };
        // Starter packs à part : leur échec n'empêche pas le reste.
        let packs = runtime().spawn(starter_packs::suggested(session.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = packs
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            if let Some(page) = weak.upgrade() {
                page.show_packs(&page.packs, result);
            }
        });
        let topics = runtime().spawn(api::trending(session.clone()));
        let people = runtime().spawn(api::suggestions(session));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let interrupted = || ApiError::new(gettext("Chargement interrompu."));
            let topics = topics.await.unwrap_or_else(|_| Err(interrupted()));
            let people = people.await.unwrap_or_else(|_| Err(interrupted()));
            let Some(page) = weak.upgrade() else { return };
            for error in [topics.as_ref().err(), people.as_ref().err()]
                .into_iter()
                .flatten()
            {
                if error.expired {
                    (page.host.expired)(error.message.clone());
                    return;
                }
            }
            page.show_topics(topics);
            page.show_suggested(people);
        });
    }

    fn show_topics(&self, topics: Result<Vec<Topic>, ApiError>) {
        clear(&self.topics);
        match topics {
            Ok(topics) if !topics.is_empty() => {
                for (i, topic) in topics.iter().enumerate() {
                    let row = topic_row(i + 1, topic);
                    // Recherche lancée par la rangée.
                    row.set_widget_name(&format!("topic:{}", topic.query));
                    self.topics.append(&row);
                }
            }
            Ok(_) => self
                .topics
                .append(&message_row(&gettext("Aucun sujet pour l'instant."))),
            Err(error) => self.topics.append(&message_row(&error.message)),
        }
    }

    fn show_suggested(&self, people: Result<Vec<Actor>, ApiError>) {
        clear(&self.suggested);
        match people {
            Ok(people) if !people.is_empty() => {
                for actor in &people {
                    self.suggested.append(&actor_row(
                        actor,
                        self.host.session.clone(),
                        &self.host.toasts,
                    ));
                }
            }
            Ok(_) => self
                .suggested
                .append(&message_row(&gettext("Aucune suggestion pour l'instant."))),
            Err(error) => self.suggested.append(&message_row(&error.message)),
        }
    }

    /// Lance une recherche ; texte vide : retour aux suggestions.
    pub fn search(self: &Rc<Self>, query: &str) {
        if query.is_empty() {
            self.query.replace(String::new());
            self.stack.set_visible_child_name("discover");
            return;
        }
        if *self.query.borrow() == query
            && self.stack.visible_child_name().as_deref() == Some("results")
        {
            return;
        }
        self.query.replace(query.to_owned());
        self.stack.set_visible_child_name("results");
        self.load_posts(false);
        self.load_people();
        self.load_feeds();
        self.load_packs();
    }

    fn load_posts(self: &Rc<Self>, more: bool) {
        let query = self.query.borrow().clone();
        let Some(session) = self.host.session.clone() else {
            if !more {
                self.posts.show_posts(demo_search_posts(&query));
            }
            return;
        };
        let Some(load) = self.posts.begin_load(more) else {
            return;
        };
        let task = runtime().spawn(api::load(
            session,
            Source::Search(query),
            load.cursor.clone(),
        ));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result: Result<Page, ApiError> = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            if let Err(error) = &result
                && error.expired
            {
                (page.host.expired)(error.message.clone());
                return;
            }
            page.posts.finish_load(&load, result);
        });
    }

    fn load_people(self: &Rc<Self>) {
        let query = self.query.borrow().clone();
        let generation = self.people_generation.get() + 1;
        self.people_generation.set(generation);
        let Some(session) = self.host.session.clone() else {
            self.show_people(Ok(demo_search_actors(&query)));
            return;
        };
        let task = runtime().spawn(api::search_actors(session, query));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            if page.people_generation.get() != generation {
                return;
            }
            if let Err(error) = &result
                && error.expired
            {
                (page.host.expired)(error.message.clone());
                return;
            }
            page.show_people(result);
        });
    }

    fn show_people(&self, people: Result<Vec<Actor>, ApiError>) {
        clear(&self.people);
        let message = match people {
            Ok(people) if !people.is_empty() => {
                for actor in &people {
                    self.people.append(&actor_row(
                        actor,
                        self.host.session.clone(),
                        &self.host.toasts,
                    ));
                }
                None
            }
            Ok(_) => Some(gettext("Personne ne correspond à cette recherche.")),
            Err(error) => Some(error.message),
        };
        self.people.set_visible(message.is_none());
        self.people_status.set_visible(message.is_some());
        self.people_status.set_title(&message.unwrap_or_default());
    }
}

fn message_row(text: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(text)
        .use_markup(false)
        .css_classes(["dim-label"])
        .build()
}

fn matches(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(&query.to_lowercase())
}

/// Recherche dans les faux posts : texte ou auteur.
pub fn demo_search_posts(query: &str) -> Vec<Post> {
    let mut seen = std::collections::HashSet::new();
    [
        data::Feed::Following,
        data::Feed::Discover,
        data::Feed::LinuxEtLibre,
    ]
    .into_iter()
    .flat_map(data::posts)
    .filter(|post| matches(&post.text, query) || matches(&post.name, query))
    .filter(|post| seen.insert(post.date.clone()))
    .collect()
}

fn demo_search_actors(query: &str) -> Vec<Actor> {
    data::demo_actors()
        .into_iter()
        .filter(|actor| matches(&actor.name, query) || matches(&actor.handle, query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_search_looks_at_text_and_authors() {
        let posts = demo_search_posts("gnome 49");
        assert!(!posts.is_empty());
        assert!(
            posts
                .iter()
                .all(|post| post.text.to_lowercase().contains("gnome 49"))
        );
        // Un même post présent dans deux fils n'apparaît qu'une fois.
        let lucie = demo_search_posts("Lucie");
        let mut dates: Vec<_> = lucie.iter().map(|p| p.date.clone()).collect();
        dates.dedup();
        assert_eq!(dates.len(), lucie.len());
        assert_eq!(demo_search_actors("NADIA").len(), 1);
        assert!(demo_search_actors("personne").is_empty());
    }
}
