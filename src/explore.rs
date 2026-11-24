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
    /// « Pour vous » : les premiers sujets, personnes et starter packs.
    topics: gtk::ListBox,
    suggested: gtk::ListBox,
    /// Onglets complets : tendances, flux populaires, starter packs, personnes.
    trends: gtk::ListBox,
    popular_feeds: gtk::ListBox,
    all_packs: gtk::ListBox,
    all_people: gtk::ListBox,
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

/// Colonne de sections d'un onglet.
fn section_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build()
}

/// Onglet défilant, à la largeur d'une colonne.
fn scroll_page(child: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                .child(child)
                .build(),
        )
        .build()
}

/// Remplit `list` de flux (une rangée ouvre le flux), ou d'un message.
fn fill_feeds(
    list: &gtk::ListBox,
    feeds: Result<Vec<crate::feeds::FeedInfo>, ApiError>,
    empty: &str,
) {
    clear(list);
    match feeds {
        Ok(feeds) if !feeds.is_empty() => {
            for info in &feeds {
                let row = crate::feeds::feed_row(info);
                row.set_widget_name(&info.uri);
                list.append(&row);
            }
        }
        Ok(_) => list.append(&message_row(empty)),
        Err(error) => list.append(&message_row(&error.message)),
    }
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

/// Nombre d'éléments repris dans « Pour vous ».
const FOR_YOU: usize = 3;

/// Nom d'une catégorie de tendance de Bluesky ; une catégorie inconnue s'affiche telle quelle.
pub fn category_label(category: &str) -> Option<String> {
    Some(match category {
        "other" | "" => return None,
        "pop-culture" => pgettext("catégorie de tendance", "Culture"),
        "sports" => pgettext("catégorie de tendance", "Sport"),
        "politics" => pgettext("catégorie de tendance", "Politique"),
        "video-games" => pgettext("catégorie de tendance", "Jeux vidéo"),
        "news" => pgettext("catégorie de tendance", "Actualité"),
        other => {
            let mut chars = other.chars();
            let first = chars.next()?;
            first
                .to_uppercase()
                .chain(chars)
                .collect::<String>()
                .replace('-', " ")
        }
    })
}

/// « Catégorie · n posts », ce qui est connu.
fn topic_subtitle(topic: &Topic) -> String {
    let mut parts = Vec::new();
    if let Some(category) = topic.category.as_deref().and_then(category_label) {
        parts.push(category);
    }
    if topic.posts > 0 {
        parts.push(
            gettextrs::ngettext(
                "{} post",
                "{} posts",
                topic.posts.min(u32::MAX as u64) as u32,
            )
            .replace("{}", &crate::profile::format_count(topic.posts)),
        );
    }
    parts.join(" · ")
}

fn topic_row(rank: usize, topic: &Topic) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&topic.name)
        .subtitle(topic_subtitle(topic))
        .use_markup(false)
        .activatable(true)
        .build();
    if topic.hot {
        let hot = gtk::Label::builder()
            .label(pgettext("tendance", "En hausse"))
            .valign(gtk::Align::Center)
            .css_classes(["trend-hot", "caption-heading"])
            .build();
        row.add_suffix(&hot);
    }
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

        // Sans recherche : onglets « Pour vous », « Tendances », « Flux », « Starter packs »,
        // « Personnes » (maquette).
        let topics = boxed_list(&gettext("Sujets du moment"));
        let suggested = boxed_list(&gettext("Personnes à suivre"));
        let packs = boxed_list(&gettext("Starter packs"));
        let for_you = section_box();
        for_you.append(&heading(&gettext("Sujets du moment")));
        for_you.append(&topics);
        for_you.append(&heading(&gettext("Personnes à suivre")));
        for_you.append(&suggested);
        for_you.append(&heading(&gettext("Starter packs")));
        for_you.append(&packs);
        let trends = boxed_list(&gettext("Tendances"));
        let trends_box = section_box();
        trends_box.append(&trends);
        let popular_feeds = boxed_list(&gettext("Flux populaires"));
        let feeds_box = section_box();
        feeds_box.append(&popular_feeds);
        let all_packs = boxed_list(&gettext("Starter packs"));
        let packs_box = section_box();
        packs_box.append(&all_packs);
        let create_pack = gtk::Button::builder()
            .label(gettext("Créer un starter pack…"))
            .halign(gtk::Align::Start)
            .visible(host.session.is_some())
            .css_classes(["pill"])
            .build();
        packs_box.append(&create_pack);
        let all_people = boxed_list(&gettext("Personnes à suivre"));
        let people_tab_box = section_box();
        people_tab_box.append(&all_people);
        let discover_stack = gtk::Stack::new();
        let discover_tabs = gtk::Box::builder()
            .css_classes(["linked"])
            .halign(gtk::Align::Center)
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        discover_tabs.update_property(&[gtk::accessible::Property::Label(&gettext("Explorer"))]);
        let mut first: Option<gtk::ToggleButton> = None;
        for (name, label, child) in [
            ("for-you", gettext("Pour vous"), &for_you),
            ("trends", gettext("Tendances"), &trends_box),
            ("feeds", gettext("Flux"), &feeds_box),
            ("packs", gettext("Starter packs"), &packs_box),
            ("people", gettext("Personnes"), &people_tab_box),
        ] {
            discover_stack.add_named(&scroll_page(child), Some(name));
            let button = gtk::ToggleButton::builder()
                .label(&label)
                .active(first.is_none())
                .build();
            button.set_group(first.as_ref());
            let stack = discover_stack.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    stack.set_visible_child_name(name);
                }
            });
            discover_tabs.append(&button);
            first.get_or_insert(button);
        }
        // Les onglets défilent à l'horizontale s'ils ne tiennent pas (palier mobile).
        let tabs_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .child(&discover_tabs)
            .build();
        let discover_scroll = gtk::Box::new(gtk::Orientation::Vertical, 0);
        discover_scroll.append(&tabs_scroll);
        discover_scroll.append(&discover_stack);

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
            trends,
            popular_feeds,
            all_packs,
            all_people,
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
        for list in [&page.topics, &page.trends] {
            let weak = Rc::downgrade(&page);
            list.connect_row_activated(move |_, row| {
                let Some(page) = weak.upgrade() else { return };
                let query = row
                    .downcast_ref::<adw::ActionRow>()
                    .and_then(|row| row.widget_name().strip_prefix("topic:").map(str::to_owned));
                if let Some(query) = query {
                    page.entry.set_text(&query);
                    page.search(&query);
                }
            });
        }
        let weak = Rc::downgrade(&page);
        create_pack.connect_clicked(move |button| {
            if let Some(page) = weak.upgrade() {
                page.create_pack(button);
            }
        });
        for list in [&page.packs, &page.all_packs, &page.pack_results] {
            let weak = Rc::downgrade(&page);
            list.connect_row_activated(move |_, row| {
                if let Some(page) = weak.upgrade() {
                    page.open_pack(&row.widget_name());
                }
            });
        }
        for list in [&page.feed_results, &page.popular_feeds] {
            let weak = Rc::downgrade(&page);
            list.connect_row_activated(move |_, row| {
                let Some(page) = weak.upgrade() else { return };
                let Some(row) = row.downcast_ref::<adw::ActionRow>() else {
                    return;
                };
                let name = row.title().to_string();
                (page.host.push)(crate::feeds::feed_page(
                    &name,
                    &row.widget_name(),
                    &page.host,
                ));
            });
        }

        page.load_discover();
        page
    }

    fn show_feeds(&self, feeds: Result<Vec<crate::feeds::FeedInfo>, ApiError>) {
        fill_feeds(
            &self.feed_results,
            feeds,
            &gettext("Aucun flux ne correspond à cette recherche."),
        );
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

    /// Nouveau starter pack : nom et description, puis sa page, pour y
    /// ajouter des comptes.
    fn create_pack(self: &Rc<Self>, button: &gtk::Button) {
        let weak = Rc::downgrade(self);
        crate::lists::starter_pack_dialog(button, None, move |name, description| {
            let Some(page) = weak.upgrade() else { return };
            let Some(session) = page.host.session.clone() else {
                return;
            };
            let task = runtime().spawn(Box::pin(crate::lists::create_starter_pack(
                session,
                name.clone(),
                description.clone(),
            )));
            let weak = Rc::downgrade(&page);
            glib::spawn_future_local(async move {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(uri) => {
                        let pack = StarterPack {
                            uri,
                            name,
                            description,
                            ..StarterPack::default()
                        };
                        let pack_page = StarterPackPage::new(&pack, page.host.clone());
                        (page.host.push)(pack_page.navigation_page());
                    }
                    Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
                }
            });
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
            self.show_suggested_packs(Ok(starter_packs::demo_packs()));
            fill_feeds(
                &self.popular_feeds,
                Ok(crate::feeds::demo_feeds()),
                &gettext("Aucun flux."),
            );
            return;
        };
        // Starter packs et flux à part : leur échec n'empêche pas le reste.
        let packs = runtime().spawn(starter_packs::suggested(session.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = packs
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            if let Some(page) = weak.upgrade() {
                page.show_suggested_packs(result);
            }
        });
        let feeds = runtime().spawn(crate::feeds::discover(session.clone(), None));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = feeds
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            if let Some(page) = weak.upgrade() {
                fill_feeds(&page.popular_feeds, result, &gettext("Aucun flux."));
            }
        });
        let topics = runtime().spawn(api::trends(session.clone()));
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

    /// Les premiers sujets dans « Pour vous », tous dans « Tendances ».
    fn show_topics(&self, topics: Result<Vec<Topic>, ApiError>) {
        for (list, limit) in [(&self.topics, FOR_YOU), (&self.trends, usize::MAX)] {
            clear(list);
            match &topics {
                Ok(topics) if !topics.is_empty() => {
                    for (i, topic) in topics.iter().take(limit).enumerate() {
                        let row = topic_row(i + 1, topic);
                        // Recherche lancée par la rangée.
                        row.set_widget_name(&format!("topic:{}", topic.query));
                        list.append(&row);
                    }
                }
                Ok(_) => list.append(&message_row(&gettext("Aucun sujet pour l'instant."))),
                Err(error) => list.append(&message_row(&error.message)),
            }
        }
    }

    /// Les premières personnes dans « Pour vous », toutes dans « Personnes ».
    fn show_suggested(&self, people: Result<Vec<Actor>, ApiError>) {
        for (list, limit) in [(&self.suggested, FOR_YOU), (&self.all_people, usize::MAX)] {
            clear(list);
            match &people {
                Ok(people) if !people.is_empty() => {
                    for actor in people.iter().take(limit) {
                        list.append(&actor_row(
                            actor,
                            self.host.session.clone(),
                            &self.host.toasts,
                        ));
                    }
                }
                Ok(_) => list.append(&message_row(&gettext("Aucune suggestion pour l'instant."))),
                Err(error) => list.append(&message_row(&error.message)),
            }
        }
    }

    /// Les premiers starter packs dans « Pour vous », tous dans « Starter packs ».
    fn show_suggested_packs(&self, packs: Result<Vec<StarterPack>, ApiError>) {
        let first = packs
            .as_ref()
            .map(|packs| packs.iter().take(FOR_YOU).cloned().collect())
            .map_err(Clone::clone);
        self.show_packs(&self.packs, first);
        self.show_packs(&self.all_packs, packs);
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

    #[test]
    fn trend_categories_are_named() {
        assert_eq!(category_label("sports").as_deref(), Some("Sport"));
        assert_eq!(category_label("other"), None);
        assert_eq!(category_label("tech-news").as_deref(), Some("Tech news"));
        let topic = Topic {
            name: "Flatpak".into(),
            query: "Flatpak".into(),
            category: Some("news".into()),
            posts: 1240,
            hot: false,
        };
        assert_eq!(topic_subtitle(&topic), "Actualité · 1\u{202f}240 posts");
    }
}
