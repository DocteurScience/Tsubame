//! Liste de posts en cartes, avec ses états (chargement, liste, liste vide,
//! erreur avec « Réessayer », pas encore disponible) et le chargement page
//! par page. Sert aux colonnes et aux profils.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;

use crate::api::{ApiError, Page};
use crate::card;
use crate::data::Post;

const LOADING: &str = "loading";
const LIST: &str = "list";
const EMPTY: &str = "empty";
const ERROR: &str = "error";
const UNAVAILABLE: &str = "unavailable";
const MESSAGE: &str = "message";

/// Chargement autorisé par `PostList::begin_load`.
pub struct Load {
    pub generation: u64,
    pub cursor: Option<String>,
    pub more: bool,
}

pub struct PostList {
    /// À placer dans l'interface.
    pub root: gtk::Stack,
    pub list: gtk::ListBox,
    retry: gtk::Button,
    scroll: gtk::ScrolledWindow,
    error: adw::StatusPage,
    posts: Rc<RefCell<Vec<Post>>>,
    /// Curseur de la page suivante ; `None` à la fin du fil.
    cursor: RefCell<Option<String>>,
    loading: Cell<bool>,
    /// Numéro du chargement en cours : un résultat d'un chargement dépassé
    /// (actualisation entre-temps) est ignoré.
    generation: Cell<u64>,
}

impl PostList {
    /// `label` nomme la liste pour les lecteurs d'écran ; `unavailable`
    /// explique pourquoi elle n'est pas encore reliée à Bluesky.
    pub fn new(label: &str, unavailable: &str) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["post-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(label)]);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH)
                    .tightening_threshold(crate::columns::COLUMN_MIN_WIDTH)
                    .child(&list)
                    .build(),
            )
            .build();

        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        spinner.update_property(&[gtk::accessible::Property::Label(&gettext("Chargement"))]);
        let retry = gtk::Button::builder()
            .label(gettext("Réessayer"))
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let error = adw::StatusPage::builder()
            .icon_name("network-error-symbolic")
            .title(gettext("Chargement impossible"))
            .child(&retry)
            .css_classes(["compact"])
            .build();
        let root = gtk::Stack::builder().vexpand(true).build();
        root.add_named(&spinner, Some(LOADING));
        root.add_named(&scroll, Some(LIST));
        root.add_named(
            &adw::StatusPage::builder()
                .icon_name("view-list-symbolic")
                .title(gettext("Rien à afficher pour l'instant"))
                .css_classes(["compact"])
                .build(),
            Some(EMPTY),
        );
        root.add_named(&error, Some(ERROR));
        root.add_named(
            &adw::StatusPage::builder()
                .icon_name("view-list-symbolic")
                .title(gettext("Pas encore disponible"))
                .description(unavailable)
                .css_classes(["compact"])
                .build(),
            Some(UNAVAILABLE),
        );
        let message = adw::StatusPage::builder()
            .icon_name("action-unavailable-symbolic")
            .css_classes(["compact"])
            .build();
        root.add_named(&message, Some(MESSAGE));
        root.set_visible_child_name(LOADING);

        Self {
            root,
            list,
            retry,
            scroll,
            error,
            posts: Rc::default(),
            cursor: RefCell::default(),
            loading: Cell::new(false),
            generation: Cell::new(0),
        }
    }

    /// La liste défile elle-même (colonnes), ou laisse défiler la page qui
    /// la contient (profil).
    pub fn set_scrolls(&self, scrolls: bool) {
        self.scroll.set_vscrollbar_policy(if scrolls {
            gtk::PolicyType::Automatic
        } else {
            gtk::PolicyType::Never
        });
        self.scroll.set_propagate_natural_height(!scrolls);
        self.root.set_vhomogeneous(scrolls);
    }

    /// Appelle `f` avec le post sélectionné dans la liste (ou `None`
    /// quand la sélection est vidée).
    pub fn connect_post_selected(&self, f: impl Fn(&gtk::ListBox, Option<Post>) + 'static) {
        let posts = self.posts.clone();
        self.list.connect_row_selected(move |list, row| {
            let post = row.and_then(|row| posts.borrow().get(row.index() as usize).cloned());
            f(list, post);
        });
    }

    /// Appelle `f` avec le post activé (clic, Entrée).
    pub fn connect_post_activated(&self, f: impl Fn(Post) + 'static) {
        let posts = self.posts.clone();
        self.list.connect_row_activated(move |_, row| {
            let post = posts.borrow().get(row.index() as usize).cloned();
            if let Some(post) = post {
                f(post);
            }
        });
    }

    /// « Réessayer » appelle `f`.
    pub fn connect_retry(&self, f: impl Fn() + 'static) {
        self.retry.connect_clicked(move |_| f());
    }

    /// Appelle `f` quand on arrive en bas de la liste, pour charger la suite.
    pub fn connect_end_reached(&self, f: impl Fn() + 'static) {
        self.scroll.connect_edge_reached(move |_, edge| {
            if edge == gtk::PositionType::Bottom {
                f();
            }
        });
    }

    /// Commence un chargement : depuis le début (`more` faux, actualisation)
    /// ou la page suivante. `None` si rien n'est à charger : suite déjà en
    /// cours, ou fin du fil.
    pub fn begin_load(&self, more: bool) -> Option<Load> {
        if more {
            if self.loading.get() || self.posts.borrow().is_empty() {
                return None;
            }
            let cursor = self.cursor.borrow().clone()?;
            self.loading.set(true);
            return Some(Load {
                generation: self.generation.get(),
                cursor: Some(cursor),
                more,
            });
        }
        self.generation.set(self.generation.get() + 1);
        self.loading.set(true);
        // Une liste déjà remplie reste visible pendant l'actualisation.
        if self.posts.borrow().is_empty() {
            self.root.set_visible_child_name(LOADING);
        }
        Some(Load {
            generation: self.generation.get(),
            cursor: None,
            more,
        })
    }

    /// Termine un chargement commencé par `begin_load`.
    pub fn finish_load(&self, load: &Load, result: Result<Page, ApiError>) {
        if load.generation != self.generation.get() {
            return;
        }
        self.loading.set(false);
        match result {
            Ok(page) => {
                if !load.more {
                    self.list.remove_all();
                    self.posts.borrow_mut().clear();
                }
                // Les posts qui contiennent un mot masqué ne sont pas
                // affichés ; les indices des rangées suivent ceux des posts.
                let posts: Vec<Post> = page
                    .posts
                    .into_iter()
                    .filter(|post| !crate::moderation::is_hidden(post))
                    .collect();
                for post in &posts {
                    self.list.append(&post_row(post));
                }
                self.posts.borrow_mut().extend(posts);
                self.cursor.replace(page.cursor);
                let empty = self.posts.borrow().is_empty();
                self.root
                    .set_visible_child_name(if empty { EMPTY } else { LIST });
            }
            // La suite n'a pas pu être chargée : on garde ce qui est
            // affiché, et le curseur pour réessayer au prochain défilement.
            Err(_) if load.more => {}
            Err(ApiError { message, .. }) => {
                self.error.set_description(Some(&message));
                self.root.set_visible_child_name(ERROR);
            }
        }
    }

    /// Notifications pas encore vues parmi les posts chargés.
    pub fn unread_count(&self) -> u32 {
        self.posts
            .borrow()
            .iter()
            .filter(|post| post.unread)
            .count() as u32
    }

    /// Marque tous les posts comme vus, à l'écran.
    pub fn mark_all_read(&self) {
        for post in self.posts.borrow_mut().iter_mut() {
            post.unread = false;
        }
        let mut row = self.list.first_child();
        while let Some(widget) = row {
            if let Some(card) = widget
                .downcast_ref::<gtk::ListBoxRow>()
                .and_then(|row| row.child())
            {
                card.remove_css_class("unread");
            }
            row = widget.next_sibling();
        }
    }

    /// Affiche ces posts, sans suite (faux posts, sans compte).
    pub fn show_posts(&self, posts: Vec<Post>) {
        let load = self
            .begin_load(false)
            .expect("un chargement complet est toujours permis");
        self.finish_load(
            &load,
            Ok(Page {
                posts,
                cursor: None,
            }),
        );
    }

    /// Le chargement en cours, s'il y en a un.
    pub fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// Liste pas encore reliée à Bluesky.
    pub fn show_unavailable(&self) {
        self.generation.set(self.generation.get() + 1);
        self.loading.set(false);
        self.list.remove_all();
        self.posts.borrow_mut().clear();
        self.cursor.replace(None);
        self.root.set_visible_child_name(UNAVAILABLE);
    }

    /// Pas de posts, avec un message (compte bloqué…).
    pub fn show_message(&self, title: &str, description: &str) {
        self.show_unavailable();
        if let Some(page) = self
            .root
            .child_by_name(MESSAGE)
            .and_downcast::<adw::StatusPage>()
        {
            page.set_title(title);
            page.set_description(Some(description).filter(|d| !d.is_empty()));
        }
        self.root.set_visible_child_name(MESSAGE);
    }

    /// Donne le focus à la carte sélectionnée, ou à la première.
    pub fn focus(&self) {
        let row = self
            .list
            .selected_row()
            .or_else(|| self.list.row_at_index(0));
        if let Some(row) = row {
            row.grab_focus();
        }
    }
}

/// Rangée d'une carte, avec une description pour les lecteurs d'écran.
fn post_row(post: &Post) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::builder()
        .child(&card::post_card(post))
        .build();
    let description = match &post.context {
        Some(context) => format!("{context}. {} : {}", post.name, post.text),
        None => format!("{} : {}", post.name, post.text),
    };
    row.update_property(&[gtk::accessible::Property::Label(&description)]);
    row
}
