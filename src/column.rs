//! Une colonne de cartes : en-tête (titre, fermeture) et liste de posts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;

use crate::api::{ApiError, DISCOVER_FEED, Page, Source};
use crate::card;
use crate::data::{self, Feed, Post};

/// Ce qu'une colonne affiche. Chaque sorte s'ouvre au plus une fois.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Feed(Feed),
    Notifications,
}

impl ColumnKind {
    pub const ALL: [ColumnKind; 4] = [
        ColumnKind::Feed(Feed::Following),
        ColumnKind::Notifications,
        ColumnKind::Feed(Feed::Discover),
        ColumnKind::Feed(Feed::LinuxEtLibre),
    ];

    /// Identifiant stable, utilisé comme cible des actions.
    pub fn id(self) -> &'static str {
        match self {
            ColumnKind::Feed(Feed::Following) => "following",
            ColumnKind::Feed(Feed::Discover) => "discover",
            ColumnKind::Feed(Feed::LinuxEtLibre) => "linux-et-libre",
            ColumnKind::Notifications => "notifications",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn title(self) -> String {
        match self {
            // Noms propres des fils de Bluesky, non traduits.
            ColumnKind::Feed(Feed::Following) => "Following".to_owned(),
            ColumnKind::Feed(Feed::Discover) => "Discover".to_owned(),
            ColumnKind::Feed(Feed::LinuxEtLibre) => gettext("Linux et libre"),
            ColumnKind::Notifications => gettext("Notifications"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            ColumnKind::Feed(_) => "view-list-symbolic",
            ColumnKind::Notifications => "preferences-system-notifications-symbolic",
        }
    }

    /// Faux posts, sans compte (builds de développement).
    pub fn demo_posts(self) -> Vec<Post> {
        match self {
            ColumnKind::Feed(feed) => data::posts(feed),
            ColumnKind::Notifications => data::notifications(),
        }
    }

    /// Source des posts sur Bluesky, ou `None` si la colonne n'y est pas
    /// encore reliée.
    pub fn source(self) -> Option<Source> {
        match self {
            ColumnKind::Feed(Feed::Following) => Some(Source::Timeline),
            ColumnKind::Feed(Feed::Discover) => Some(Source::Feed(DISCOVER_FEED.to_owned())),
            ColumnKind::Notifications => Some(Source::Notifications),
            // « Linux et libre » est le fil fictif des maquettes.
            ColumnKind::Feed(Feed::LinuxEtLibre) => None,
        }
    }
}

/// Ce qu'une colonne affiche : chargement, liste de cartes, liste vide,
/// erreur, ou colonne pas encore reliée à Bluesky.
const LOADING: &str = "loading";
const LIST: &str = "list";
const EMPTY: &str = "empty";
const ERROR: &str = "error";
const UNAVAILABLE: &str = "unavailable";

pub struct Column {
    pub kind: ColumnKind,
    pub root: adw::ToolbarView,
    pub list: gtk::ListBox,
    header: adw::HeaderBar,
    title: adw::WindowTitle,
    switcher: gtk::MenuButton,
    close: gtk::Button,
    options: gtk::MenuButton,
    moves: gio::Menu,
    refresh: gtk::Button,
    retry: gtk::Button,
    scroll: gtk::ScrolledWindow,
    stack: gtk::Stack,
    error: adw::StatusPage,
    posts: Rc<RefCell<Vec<Post>>>,
    /// Curseur de la page suivante ; `None` à la fin du fil.
    cursor: RefCell<Option<String>>,
    loading: Cell<bool>,
    /// Numéro du chargement en cours : un résultat d'un chargement dépassé
    /// (actualisation entre-temps) est ignoré.
    generation: Cell<u64>,
}

/// Chargement autorisé par `Column::begin_load`.
pub struct Load {
    pub generation: u64,
    pub cursor: Option<String>,
    pub more: bool,
}

impl Column {
    /// `columns_menu` liste les colonnes ouvertes ; il sert au sélecteur de
    /// titre quand une seule colonne est affichée.
    pub fn new(kind: ColumnKind, columns_menu: &gio::Menu) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["post-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&kind.title())]);
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
        let stack = gtk::Stack::new();
        stack.add_named(&spinner, Some(LOADING));
        stack.add_named(&scroll, Some(LIST));
        stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("view-list-symbolic")
                .title(gettext("Rien à afficher pour l'instant"))
                .css_classes(["compact"])
                .build(),
            Some(EMPTY),
        );
        stack.add_named(&error, Some(ERROR));
        stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("view-list-symbolic")
                .title(gettext("Pas encore disponible"))
                .description(gettext("Cette colonne n'est pas encore reliée à Bluesky."))
                .css_classes(["compact"])
                .build(),
            Some(UNAVAILABLE),
        );
        stack.set_visible_child_name(LOADING);

        // Titre simple, ou menu des colonnes ouvertes quand une seule colonne
        // est visible (comme « Following ▾ » dans la maquette Mobile).
        let title = adw::WindowTitle::new(&kind.title(), "");
        let switcher = gtk::MenuButton::builder()
            .label(kind.title())
            .menu_model(columns_menu)
            .always_show_arrow(true)
            .tooltip_text(gettext("Changer de colonne"))
            .css_classes(["flat"])
            .visible(false)
            .build();
        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_box.append(&title);
        title_box.append(&switcher);

        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .action_name("win.close-column")
            .action_target(&kind.id().to_variant())
            .tooltip_text(gettext("Fermer la colonne"))
            .build();
        close.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Fermer la colonne",
        ))]);

        // « Actualiser », à gauche du titre comme dans la maquette Colonnes.
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(gettext("Actualiser"))
            .build();
        refresh.update_property(&[gtk::accessible::Property::Label(&gettext("Actualiser"))]);

        let header = adw::HeaderBar::builder()
            .title_widget(&title_box)
            .show_end_title_buttons(false)
            .build();
        header.pack_start(&refresh);
        header.pack_end(&close);

        // Menu « Options de la colonne », comme dans la maquette Colonnes.
        let target = kind.id();
        let menu = gio::Menu::new();
        let refreshing = gio::Menu::new();
        refreshing.append(
            Some(&gettext("Actualiser")),
            Some(&format!("win.refresh-column::{target}")),
        );
        menu.append_section(None, &refreshing);
        let moves = gio::Menu::new();
        menu.append_section(None, &moves);
        let closing = gio::Menu::new();
        closing.append(
            Some(&gettext("Fermer la colonne")),
            Some(&format!("win.close-column::{target}")),
        );
        menu.append_section(None, &closing);
        let options = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .tooltip_text(gettext("Options de la colonne"))
            .build();
        options.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Options de la colonne",
        ))]);
        header.pack_end(&options);

        let root = adw::ToolbarView::builder()
            .content(&stack)
            .css_classes(["column"])
            .build();
        root.add_top_bar(&header);

        Self {
            kind,
            root,
            list,
            header,
            title,
            switcher,
            close,
            options,
            moves,
            refresh,
            retry,
            scroll,
            stack,
            error,
            posts: Rc::default(),
            cursor: RefCell::default(),
            loading: Cell::new(false),
            generation: Cell::new(0),
        }
    }

    /// Appelle `f` avec le post sélectionné dans la colonne (ou `None`
    /// quand la sélection est vidée).
    pub fn connect_post_selected(&self, f: impl Fn(&gtk::ListBox, Option<Post>) + 'static) {
        let posts = self.posts.clone();
        self.list.connect_row_selected(move |list, row| {
            let post = row.and_then(|row| posts.borrow().get(row.index() as usize).cloned());
            f(list, post);
        });
    }

    /// « Actualiser » et « Réessayer » appellent `f`.
    pub fn connect_refresh(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        let g = f.clone();
        self.refresh.connect_clicked(move |_| f());
        self.retry.connect_clicked(move |_| g());
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
        self.refresh.set_sensitive(false);
        // Une liste déjà remplie reste visible pendant l'actualisation.
        if self.posts.borrow().is_empty() {
            self.stack.set_visible_child_name(LOADING);
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
        self.refresh.set_sensitive(true);
        match result {
            Ok(page) => {
                if !load.more {
                    self.list.remove_all();
                    self.posts.borrow_mut().clear();
                }
                for post in &page.posts {
                    self.list.append(&post_row(post));
                }
                self.posts.borrow_mut().extend(page.posts);
                self.cursor.replace(page.cursor);
                let empty = self.posts.borrow().is_empty();
                self.stack
                    .set_visible_child_name(if empty { EMPTY } else { LIST });
            }
            // La suite n'a pas pu être chargée : on garde ce qui est
            // affiché, et le curseur pour réessayer au prochain défilement.
            Err(_) if load.more => {}
            Err(ApiError { message, .. }) => {
                self.error.set_description(Some(&message));
                self.stack.set_visible_child_name(ERROR);
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

    /// Faux posts, sans compte.
    pub fn show_demo(&self) {
        let load = self
            .begin_load(false)
            .expect("un chargement complet est toujours permis");
        let page = Page {
            posts: self.kind.demo_posts(),
            cursor: None,
        };
        self.finish_load(&load, Ok(page));
    }

    /// Colonne pas encore reliée à Bluesky.
    pub fn show_unavailable(&self) {
        self.generation.set(self.generation.get() + 1);
        self.loading.set(false);
        self.list.remove_all();
        self.posts.borrow_mut().clear();
        self.cursor.replace(None);
        self.stack.set_visible_child_name(UNAVAILABLE);
    }

    /// Ne propose dans le menu que les déplacements possibles depuis la
    /// position `index` parmi `count` colonnes.
    pub fn set_position(&self, index: usize, count: usize) {
        let target = self.kind.id();
        self.moves.remove_all();
        if index > 0 {
            self.moves.append(
                Some(&gettext("Déplacer vers la gauche")),
                Some(&format!("win.move-column-left::{target}")),
            );
        }
        if index + 1 < count {
            self.moves.append(
                Some(&gettext("Déplacer vers la droite")),
                Some(&format!("win.move-column-right::{target}")),
            );
        }
    }

    /// Colonne affichée seule dans sa propre page, sans bouton de
    /// fermeture (onglet Notifications du palier compact).
    pub fn set_standalone(&self) {
        self.close.set_visible(false);
        self.options.set_visible(false);
        self.header.set_show_end_title_buttons(true);
    }

    /// Aux paliers étroits, une seule colonne est visible : son titre
    /// devient un menu pour passer aux autres.
    /// Le bouton de fermeture laisse alors la place au titre : « Fermer la
    /// colonne » reste dans le menu d'options.
    pub fn set_alone(&self, alone: bool) {
        self.title.set_visible(!alone);
        self.switcher.set_visible(alone);
        self.close.set_visible(!alone && self.options.get_visible());
        // Seule (palier compact), la colonne n'a pas la place pour
        // « Actualiser » : il reste dans le menu d'options, et F5.
        self.refresh
            .set_visible(!alone || !self.options.get_visible());
    }

    /// Montre les boutons de la fenêtre dans l'en-tête, quand la colonne est
    /// la plus à droite de la fenêtre.
    pub fn set_window_controls(&self, visible: bool) {
        self.header.set_show_end_title_buttons(visible);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for kind in ColumnKind::ALL {
            assert_eq!(ColumnKind::from_id(kind.id()), Some(kind));
        }
        assert_eq!(ColumnKind::from_id("inconnue"), None);
    }

    #[test]
    fn schema_default_columns_exist() {
        let schema = include_str!("../data/io.github.DocteurScience.Tsubame.gschema.xml");
        let default = schema
            .split("<default>")
            .nth(1)
            .and_then(|rest| rest.split("</default>").next())
            .expect("valeur par défaut de « columns »");
        let ids: Vec<&str> = default
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|id| id.trim().trim_matches('\''))
            .collect();
        assert_eq!(ids, ["following", "notifications"]);
        assert!(ids.iter().all(|id| ColumnKind::from_id(id).is_some()));
    }
}
