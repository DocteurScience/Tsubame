//! Page de profil (maquette `docs/mockups/source/Profil.dc.html`) :
//! bannière, avatar, nom, identifiant, bio, compteurs, « Suivre », onglets
//! et posts en cartes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};

use crate::api::{self, ApiError, AuthorFilter, Page, Source};
use crate::auth::Session;
use crate::data::{self, Post, Profile};
use crate::images;
use crate::post_list::PostList;
use crate::runtime::runtime;

/// Onglets du profil. « J'aime » n'existe que pour son propre profil :
/// l'API ne donne pas les « j'aime » des autres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Posts,
    Replies,
    Media,
    Likes,
}

impl Tab {
    fn title(self) -> String {
        match self {
            Tab::Posts => gettext("Posts"),
            Tab::Replies => gettext("Réponses"),
            Tab::Media => gettext("Médias"),
            Tab::Likes => gettext("J'aime"),
        }
    }

    fn source(self, actor: &str) -> Source {
        let author = |filter| Source::Author {
            actor: actor.to_owned(),
            filter,
        };
        match self {
            Tab::Posts => author(AuthorFilter::Posts),
            Tab::Replies => author(AuthorFilter::Replies),
            Tab::Media => author(AuthorFilter::Media),
            Tab::Likes => Source::Likes(actor.to_owned()),
        }
    }
}

/// Nombre avec séparateur des milliers à la française : « 1 284 ».
pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            // Espace fine insécable.
            out.push('\u{202f}');
        }
        out.push(digit);
    }
    out
}

/// Ce que la fenêtre fournit à une page de profil.
#[derive(Clone)]
pub struct Host {
    pub session: Option<Arc<Session>>,
    /// Mon DID, pour reconnaître mon propre profil.
    pub me: Option<String>,
    pub toasts: adw::ToastOverlay,
    /// Ouvrir la discussion d'un post.
    pub open_post: Rc<dyn Fn(Post)>,
    /// Session expirée : retour à l'écran de connexion.
    pub expired: Rc<dyn Fn(String)>,
    /// Empiler une page dans la navigation.
    pub push: Rc<dyn Fn(adw::NavigationPage)>,
}

pub struct ProfilePage {
    /// Contenu de la page, avec son en-tête.
    pub root: adw::ToolbarView,
    host: Host,
    actor: String,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    error: adw::StatusPage,
    banner: gtk::Picture,
    avatar: adw::Avatar,
    name: gtk::Label,
    handle: gtk::Label,
    bio: gtk::Label,
    /// Compteurs : abonnés et abonnements (boutons vers leur liste), posts.
    followers: gtk::Button,
    follows: gtk::Button,
    post_count: gtk::Label,
    follow: gtk::Button,
    /// « Modifier le profil », sur son propre profil.
    edit: gtk::Button,
    /// Menu ⋮ : lien, masquer, bloquer, signaler.
    more: gtk::MenuButton,
    likes_tab: gtk::ToggleButton,
    scroll: gtk::ScrolledWindow,
    posts: PostList,
    tab: Cell<Tab>,
    profile: RefCell<Option<Profile>>,
}

/// Texte de l'en-tête. Non sélectionnable : une étiquette sélectionnable
/// prendrait le focus à l'ouverture de la page, tout son texte surligné.
fn label(classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

impl ProfilePage {
    /// Page du compte `actor` (DID ou identifiant).
    pub fn new(actor: &str, host: Host) -> Rc<Self> {
        let title = adw::WindowTitle::new("", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let more = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(gettext("Plus d'actions"))
            .visible(false)
            .build();
        more.update_property(&[gtk::accessible::Property::Label(&gettext("Plus d'actions"))]);
        header.pack_end(&more);

        // En-tête du profil.
        let banner = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .height_request(150)
            .css_classes(["profile-banner"])
            .build();
        banner.update_property(&[gtk::accessible::Property::Label(&gettext("Bannière"))]);
        let avatar = adw::Avatar::builder()
            .size(88)
            .show_initials(true)
            .valign(gtk::Align::End)
            .build();
        let follow = gtk::Button::builder()
            .valign(gtk::Align::Center)
            .halign(gtk::Align::End)
            .hexpand(true)
            .visible(false)
            .css_classes(["pill"])
            .build();
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        top.append(&avatar);
        let edit = gtk::Button::builder()
            .label(gettext("Modifier le profil"))
            .valign(gtk::Align::Center)
            .halign(gtk::Align::End)
            .hexpand(true)
            .visible(false)
            .css_classes(["pill"])
            .build();
        top.append(&follow);
        top.append(&edit);
        let name = label(&["title-1"]);
        let handle = label(&["dim-label"]);
        let bio = label(&[]);
        let counter = || {
            let label = gtk::Label::builder().use_markup(true).build();
            gtk::Button::builder()
                .child(&label)
                .css_classes(["flat", "profile-count"])
                .build()
        };
        let (followers, follows) = (counter(), counter());
        let post_count = gtk::Label::builder()
            .use_markup(true)
            .margin_start(8)
            .build();
        let stats = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        stats.append(&followers);
        stats.append(&follows);
        stats.append(&post_count);

        // Onglets : boutons liés, à remplacer par AdwToggleGroup avec
        // libadwaita 1.7.
        let tabs = gtk::Box::builder()
            .css_classes(["linked"])
            .halign(gtk::Align::Start)
            .accessible_role(gtk::AccessibleRole::Group)
            .margin_top(6)
            .build();
        tabs.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Contenu du profil",
        ))]);
        let mut buttons = Vec::new();
        for tab in [Tab::Posts, Tab::Replies, Tab::Media, Tab::Likes] {
            let button = gtk::ToggleButton::builder()
                .label(tab.title())
                .active(tab == Tab::Posts)
                .build();
            if let Some((_, first)) = buttons.first() {
                button.set_group(Some(first));
            }
            tabs.append(&button);
            buttons.push((tab, button));
        }
        let likes_tab = buttons[3].1.clone();
        likes_tab.set_visible(false);

        let posts = PostList::new(
            &gettext("Posts du profil"),
            &gettext("Connectez-vous pour voir ces posts."),
        );
        posts.set_scrolls(false);

        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        details.append(&top);
        details.append(&name);
        details.append(&handle);
        details.append(&bio);
        details.append(&stats);
        details.append(&tabs);
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .build();
        let banner_frame = gtk::Box::builder()
            .overflow(gtk::Overflow::Hidden)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["profile-banner-frame"])
            .build();
        banner_frame.append(&banner);
        banner.set_hexpand(true);
        column.append(&banner_frame);
        column.append(&details);
        column.append(&posts.root);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            // Même largeur que les cartes (et leurs marges), pour aligner
            // l'en-tête sur les posts.
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .child(&column)
                    .build(),
            )
            .build();

        // États de la page : chargement, erreur, profil.
        let retry = gtk::Button::builder()
            .label(gettext("Réessayer"))
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let error = adw::StatusPage::builder()
            .icon_name("avatar-default-symbolic")
            .title(gettext("Profil introuvable"))
            .child(&retry)
            .build();
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        spinner.update_property(&[gtk::accessible::Property::Label(&gettext("Chargement"))]);
        let stack = gtk::Stack::new();
        stack.add_named(&spinner, Some("loading"));
        stack.add_named(&error, Some("error"));
        stack.add_named(&scroll, Some("profile"));

        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(&header);

        let page = Rc::new(Self {
            root,
            host,
            actor: actor.to_owned(),
            title,
            stack,
            error,
            banner,
            avatar,
            name,
            handle,
            bio,
            followers,
            follows,
            post_count,
            follow,
            edit,
            more,
            likes_tab,
            scroll,
            posts,
            tab: Cell::new(Tab::Posts),
            profile: RefCell::default(),
        });

        for (tab, button) in buttons {
            let weak = Rc::downgrade(&page);
            button.connect_toggled(move |button| {
                if button.is_active()
                    && let Some(page) = weak.upgrade()
                {
                    page.tab.set(tab);
                    page.load_posts(false);
                }
            });
        }
        let weak = Rc::downgrade(&page);
        retry.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.load();
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_retry(move || {
            if let Some(page) = weak.upgrade() {
                page.load_posts(false);
            }
        });
        let weak = Rc::downgrade(&page);
        page.scroll.connect_edge_reached(move |_, edge| {
            if edge == gtk::PositionType::Bottom
                && let Some(page) = weak.upgrade()
            {
                page.load_posts(true);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_post_activated(move |post| {
            if let Some(page) = weak.upgrade() {
                (page.host.open_post)(post);
            }
        });
        let weak = Rc::downgrade(&page);
        page.edit.connect_clicked(move |_| {
            let Some(page) = weak.upgrade() else { return };
            let Some(profile) = page.profile.borrow().clone() else {
                return;
            };
            let weak = Rc::downgrade(&page);
            crate::edit_profile::present(
                &page.root,
                page.host.session.clone(),
                &profile,
                move || {
                    if let Some(page) = weak.upgrade() {
                        page.host.toasts.add_toast(adw::Toast::new(
                            &if page.host.session.is_some() {
                                gettext("Profil enregistré.")
                            } else {
                                gettext("Mode démonstration : rien n'est enregistré.")
                            },
                        ));
                        page.load();
                    }
                },
            );
        });
        let weak = Rc::downgrade(&page);
        page.follow.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.toggle_follow();
            }
        });

        for (button, followers) in [(&page.followers, true), (&page.follows, false)] {
            let weak = Rc::downgrade(&page);
            button.connect_clicked(move |_| {
                let Some(page) = weak.upgrade() else { return };
                let Some(profile) = page.profile.borrow().clone() else {
                    return;
                };
                let actor = if profile.did.is_empty() {
                    profile.handle.clone()
                } else {
                    profile.did.clone()
                };
                let people = if followers {
                    api::People::Followers(actor)
                } else {
                    api::People::Follows(actor)
                };
                (page.host.push)(crate::feed_page::people_page(
                    people,
                    &profile.name,
                    &page.host,
                ));
            });
        }

        let actions = gio::SimpleActionGroup::new();
        for (name, handler) in [
            ("copy-link", Self::copy_link as fn(&Rc<Self>)),
            ("mute", Self::toggle_mute),
            ("block", Self::toggle_block),
            ("report", Self::report),
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
        page.root.insert_action_group("profile", Some(&actions));

        page.load();
        page
    }

    /// Page à empiler dans la navigation.
    pub fn navigation_page(self: &Rc<Self>) -> adw::NavigationPage {
        let page = adw::NavigationPage::builder()
            .title(gettext("Profil"))
            .child(&self.root)
            .build();
        // La page garde le profil en vie tant qu'elle est affichée.
        let this = self.clone();
        page.connect_destroy(move |_| {
            let _ = &this;
        });
        let weak = page.downgrade();
        self.title.connect_title_notify(move |title| {
            if let Some(page) = weak.upgrade() {
                page.set_title(&title.title());
            }
        });
        page
    }

    fn load(self: &Rc<Self>) {
        self.stack.set_visible_child_name("loading");
        let Some(session) = self.host.session.clone() else {
            self.show(demo_profile(&self.actor));
            return;
        };
        let task = runtime().spawn(api::profile(session, self.actor.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(profile) => page.show(profile),
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => {
                    page.error.set_description(Some(&error.message));
                    page.stack.set_visible_child_name("error");
                }
            }
        });
    }

    fn show(self: &Rc<Self>, profile: Profile) {
        let is_me = self.host.me.as_deref() == Some(profile.did.as_str());
        self.title.set_title(&profile.name);
        self.title.set_subtitle(&format!("@{}", profile.handle));
        self.avatar.set_text(Some(&profile.name));
        self.name.set_label(&profile.name);
        self.handle.set_label(&if profile.follows_me {
            gettext("@{} · vous suit").replace("{}", &profile.handle)
        } else {
            format!("@{}", profile.handle)
        });
        self.bio.set_label(&profile.bio);
        self.bio.set_visible(!profile.bio.trim().is_empty());
        self.likes_tab.set_visible(is_me);
        self.edit.set_visible(is_me);
        self.follow
            .set_visible(!is_me && self.host.session.is_some() && !profile.did.is_empty());
        self.more.set_visible(!is_me && !profile.handle.is_empty());
        if let Some(url) = &profile.avatar {
            let weak = self.avatar.downgrade();
            images::load(url, move |texture| {
                if let Some(avatar) = weak.upgrade() {
                    avatar.set_custom_image(Some(texture));
                }
            });
        }
        self.banner.set_paintable(None::<&gtk::gdk::Paintable>);
        if let Some(url) = &profile.banner {
            let weak = self.banner.downgrade();
            images::load(url, move |texture| {
                if let Some(banner) = weak.upgrade() {
                    banner.set_paintable(Some(texture));
                }
            });
        }
        self.profile.replace(Some(profile));
        self.update_counts();
        self.stack.set_visible_child_name("profile");
        self.load_posts(false);
    }

    /// Compteurs et bouton « Suivre », d'après le profil.
    fn update_counts(&self) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        // Les chaînes restent littérales pour que xgettext les trouve ; le
        // nombre est mis en gras là où la traduction le place.
        let n = |count: u64| count.min(u64::from(u32::MAX)) as u32;
        let part = |template: String, count: u64| {
            glib::markup_escape_text(&template)
                .replace("{}", &format!("<b>{}</b>", format_count(count)))
        };
        for (button, text, tooltip) in [
            (
                &self.followers,
                part(
                    ngettext("{} abonné", "{} abonnés", n(profile.followers)),
                    profile.followers,
                ),
                gettext("Voir les abonnés"),
            ),
            (
                &self.follows,
                part(
                    ngettext("{} abonnement", "{} abonnements", n(profile.follows)),
                    profile.follows,
                ),
                gettext("Voir les abonnements"),
            ),
        ] {
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_label(&text);
            }
            button.set_tooltip_text(Some(&tooltip));
        }
        self.post_count.set_label(&part(
            ngettext("{} post", "{} posts", n(profile.posts)),
            profile.posts,
        ));
        self.more.set_menu_model(Some(&profile_menu(&profile)));
        self.follow.set_sensitive(profile.blocking.is_none());
        if profile.following.is_some() {
            self.follow.set_label(&gettext("Ne plus suivre"));
            self.follow.remove_css_class("suggested-action");
        } else {
            self.follow.set_label(&gettext("Suivre"));
            self.follow.add_css_class("suggested-action");
        }
    }

    fn load_posts(self: &Rc<Self>, more: bool) {
        let tab = self.tab.get();
        if self
            .profile
            .borrow()
            .as_ref()
            .is_some_and(|p| p.blocking.is_some())
        {
            if !more {
                self.posts.show_message(
                    &gettext("Vous avez bloqué ce compte"),
                    &gettext("Débloquez-le depuis le menu pour voir ses posts."),
                );
            }
            return;
        }
        let Some(session) = self.host.session.clone() else {
            if !more {
                let posts = demo_posts(&self.actor, tab);
                self.posts.show_posts(posts);
            }
            return;
        };
        let actor = self
            .profile
            .borrow()
            .as_ref()
            .map(|p| p.did.clone())
            .unwrap_or_else(|| self.actor.clone());
        let Some(load) = self.posts.begin_load(more) else {
            return;
        };
        let task = runtime().spawn(api::load(session, tab.source(&actor), load.cursor.clone()));
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
            // Un autre onglet a été choisi entre-temps : ignoré par le
            // numéro de chargement de la liste.
            page.posts.finish_load(&load, result);
        });
    }

    /// Suivre ou ne plus suivre : le bouton est grisé pendant la requête ;
    /// le compteur d'abonnés suit.
    fn toggle_follow(self: &Rc<Self>) {
        let (Some(session), Some(profile)) =
            (self.host.session.clone(), self.profile.borrow().clone())
        else {
            return;
        };
        self.follow.set_sensitive(false);
        let following = profile.following.clone();
        let task = runtime().spawn(async move {
            match following {
                Some(record) => api::delete(session, record).await.map(|()| None),
                None => api::follow(session, profile.did.clone()).await.map(Some),
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(page) = weak.upgrade() else { return };
            page.follow.set_sensitive(true);
            match result {
                Ok(record) => {
                    if let Some(profile) = page.profile.borrow_mut().as_mut() {
                        profile.followers = match record {
                            Some(_) => profile.followers + 1,
                            None => profile.followers.saturating_sub(1),
                        };
                        profile.following = record;
                    }
                    page.update_counts();
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => {
                    glib::g_debug!("tsubame", "abonnement : {}", error.message);
                    page.host
                        .toasts
                        .add_toast(adw::Toast::new(&gettext("L'abonnement n'a pas changé.")));
                }
            }
        });
    }
}

/// Menu ⋮ d'un profil, d'après son état (masqué, bloqué).
fn profile_menu(profile: &Profile) -> gio::Menu {
    let menu = gio::Menu::new();
    let sharing = gio::Menu::new();
    sharing.append(Some(&gettext("Copier le lien")), Some("profile.copy-link"));
    menu.append_section(None, &sharing);
    let moderation = gio::Menu::new();
    let mute = if profile.muted {
        gettext("Ne plus masquer")
    } else {
        gettext("Masquer")
    };
    moderation.append(Some(&mute), Some("profile.mute"));
    let block = if profile.blocking.is_some() {
        gettext("Débloquer")
    } else {
        gettext("Bloquer…")
    };
    moderation.append(Some(&block), Some("profile.block"));
    moderation.append(Some(&gettext("Signaler…")), Some("profile.report"));
    menu.append_section(None, &moderation);
    menu
}

impl ProfilePage {
    fn copy_link(self: &Rc<Self>) {
        let Some(handle) = self.profile.borrow().as_ref().map(|p| p.handle.clone()) else {
            return;
        };
        self.root
            .clipboard()
            .set_text(&format!("https://bsky.app/profile/{handle}"));
        self.host
            .toasts
            .add_toast(adw::Toast::new(&gettext("Lien copié.")));
    }

    fn report(self: &Rc<Self>) {
        if let Some(handle) = self.profile.borrow().as_ref().map(|p| p.handle.clone()) {
            let _ = self
                .root
                .activate_action("win.report-author", Some(&handle.to_variant()));
        }
    }

    /// Requête de modération sur ce compte ; `apply` met le profil à jour
    /// si elle réussit, puis l'en-tête et les posts suivent.
    fn moderate<T: Send + 'static>(
        self: &Rc<Self>,
        request: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
        done: String,
        apply: impl FnOnce(&mut Profile, T) + 'static,
    ) {
        let task = runtime().spawn(request);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(value) => {
                    if let Some(profile) = page.profile.borrow_mut().as_mut() {
                        apply(profile, value);
                    }
                    page.update_counts();
                    page.load_posts(false);
                    page.host.toasts.add_toast(adw::Toast::new(&done));
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    /// Sans compte, rien n'est envoyé : on le dit.
    fn session_or_demo(&self) -> Option<Arc<Session>> {
        let session = self.host.session.clone();
        if session.is_none() {
            self.host.toasts.add_toast(adw::Toast::new(&gettext(
                "Mode démonstration : rien n'est enregistré.",
            )));
        }
        session
    }

    fn toggle_mute(self: &Rc<Self>) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        let Some(session) = self.session_or_demo() else {
            return;
        };
        let mute = !profile.muted;
        let done = if mute {
            gettext("@{} est masqué.")
        } else {
            gettext("@{} n'est plus masqué.")
        }
        .replace("{}", &profile.handle);
        self.moderate(
            crate::moderation::mute(session, profile.did.clone(), mute),
            done,
            move |profile, ()| profile.muted = mute,
        );
    }

    fn toggle_block(self: &Rc<Self>) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        if let Some(record) = profile.blocking.clone() {
            let Some(session) = self.session_or_demo() else {
                return;
            };
            self.moderate(
                api::delete(session, record),
                gettext("@{} est débloqué.").replace("{}", &profile.handle),
                |profile, ()| profile.blocking = None,
            );
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Bloquer @{} ?").replace("{}", &profile.handle)),
            Some(&gettext(
                "Ses posts et son profil vous seront cachés, et ce compte ne pourra plus interagir avec vous. Le blocage est public.",
            )),
        );
        dialog.add_responses(&[
            ("cancel", &gettext("Annuler")),
            ("block", &gettext("Bloquer")),
        ]);
        dialog.set_response_appearance("block", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(Some("block"), move |_, _| {
            let Some(page) = weak.upgrade() else { return };
            let Some(session) = page.session_or_demo() else {
                return;
            };
            page.moderate(
                crate::moderation::block(session, profile.did.clone()),
                gettext("@{} est bloqué.").replace("{}", &profile.handle),
                |profile, record| profile.blocking = Some(record),
            );
        });
        dialog.present(Some(&self.root));
    }
}

/// Posts de démonstration d'un auteur, d'après son identifiant.
fn demo_posts(handle: &str, tab: Tab) -> Vec<Post> {
    let mut posts: Vec<Post> = [
        data::Feed::Following,
        data::Feed::Discover,
        data::Feed::LinuxEtLibre,
    ]
    .into_iter()
    .flat_map(data::posts)
    .filter(|post| post.handle == handle)
    .collect();
    // Un même post peut figurer dans plusieurs fils.
    let mut seen = std::collections::HashSet::new();
    posts.retain(|post| seen.insert(post.date.clone()));
    match tab {
        Tab::Media => posts.retain(|post| post.media.is_some()),
        Tab::Likes => posts.clear(),
        Tab::Posts | Tab::Replies => {}
    }
    posts
}

/// Profil de démonstration, d'après les faux posts de l'auteur.
fn demo_profile(handle: &str) -> Profile {
    let posts = demo_posts(handle, Tab::Posts);
    let name = posts
        .first()
        .map(|post| post.name.clone())
        .unwrap_or_else(|| handle.to_owned());
    Profile {
        did: String::new(),
        handle: handle.to_owned(),
        name,
        bio: gettext("Profil de démonstration : les noms et les posts sont fictifs."),
        avatar: None,
        banner: None,
        followers: 1284,
        follows: 312,
        posts: posts.len() as u64,
        following: None,
        follows_me: false,
        muted: false,
        blocking: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_use_french_grouping() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1284), "1\u{202f}284");
        assert_eq!(format_count(1_234_567), "1\u{202f}234\u{202f}567");
    }

    #[test]
    fn demo_profiles_gather_the_author_posts() {
        let profile = demo_profile("lucie.example.social");
        assert_eq!(profile.name, "Lucie Morel");
        assert!(profile.posts >= 1);
        assert!(
            demo_posts("aya.example.social", Tab::Media)
                .iter()
                .all(|post| post.media.is_some())
        );
    }
}
