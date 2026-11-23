//! Une colonne de cartes : en-tête (titre, fermeture) et liste de posts.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;

use crate::api::{ApiError, DISCOVER_FEED, Page, Source};
use crate::data::{self, Feed, Post};
use crate::post_list::{Load, PostList};

/// Fil personnalisé ouvert en colonne (fil épinglé sur Bluesky), repéré
/// par un numéro dans le registre des fils : `ColumnKind` reste ainsi
/// copiable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedId(usize);

#[derive(Debug, Clone)]
struct FeedInfo {
    uri: String,
    name: String,
}

thread_local! {
    static FEEDS: RefCell<Vec<FeedInfo>> = RefCell::default();
}

impl FeedId {
    /// Numéro du fil `uri`, enregistré au besoin. En attendant son vrai nom,
    /// le fil porte la fin de son adresse.
    pub fn intern(uri: &str) -> Self {
        FEEDS.with_borrow_mut(|feeds| {
            if let Some(index) = feeds.iter().position(|f| f.uri == uri) {
                return FeedId(index);
            }
            let name = uri.rsplit('/').next().unwrap_or(uri).to_owned();
            feeds.push(FeedInfo {
                uri: uri.to_owned(),
                name,
            });
            FeedId(feeds.len() - 1)
        })
    }

    pub fn set_name(self, name: &str) {
        FEEDS.with_borrow_mut(|feeds| feeds[self.0].name = name.to_owned());
    }

    pub fn uri(self) -> String {
        FEEDS.with_borrow(|feeds| feeds[self.0].uri.clone())
    }

    fn name(self) -> String {
        FEEDS.with_borrow(|feeds| feeds[self.0].name.clone())
    }
}

/// Ce qu'une colonne affiche. Chaque sorte s'ouvre au plus une fois.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Feed(Feed),
    Notifications,
    /// Fil épinglé sur Bluesky.
    Custom(FeedId),
}

const FEED_PREFIX: &str = "feed:";

impl ColumnKind {
    /// Colonnes toujours proposées. « Linux et libre », fil fictif des
    /// maquettes, ne l'est qu'en démonstration ; les fils épinglés
    /// s'ajoutent une fois connecté.
    pub const BUILTIN: [ColumnKind; 3] = [
        ColumnKind::Feed(Feed::Following),
        ColumnKind::Notifications,
        ColumnKind::Feed(Feed::Discover),
    ];
    pub const DEMO: ColumnKind = ColumnKind::Feed(Feed::LinuxEtLibre);

    /// Identifiant stable, enregistré dans les réglages et utilisé comme
    /// cible des actions.
    pub fn id(self) -> String {
        match self {
            ColumnKind::Feed(Feed::Following) => "following".into(),
            ColumnKind::Feed(Feed::Discover) => "discover".into(),
            ColumnKind::Feed(Feed::LinuxEtLibre) => "linux-et-libre".into(),
            ColumnKind::Notifications => "notifications".into(),
            ColumnKind::Custom(feed) => format!("{FEED_PREFIX}{}", feed.uri()),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        if let Some(uri) = id.strip_prefix(FEED_PREFIX) {
            return uri
                .starts_with("at://")
                .then(|| ColumnKind::Custom(FeedId::intern(uri)));
        }
        Self::BUILTIN
            .into_iter()
            .chain([Self::DEMO])
            .find(|kind| kind.id() == id)
    }

    pub fn title(self) -> String {
        match self {
            // Noms propres des fils de Bluesky, non traduits.
            ColumnKind::Feed(Feed::Following) => "Following".to_owned(),
            ColumnKind::Feed(Feed::Discover) => "Discover".to_owned(),
            ColumnKind::Feed(Feed::LinuxEtLibre) => gettext("Linux et libre"),
            ColumnKind::Notifications => gettext("Notifications"),
            ColumnKind::Custom(feed) => feed.name(),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            ColumnKind::Feed(_) | ColumnKind::Custom(_) => "view-list-symbolic",
            ColumnKind::Notifications => "preferences-system-notifications-symbolic",
        }
    }

    /// Faux posts, sans compte (builds de développement).
    pub fn demo_posts(self) -> Vec<Post> {
        match self {
            ColumnKind::Feed(feed) => data::posts(feed),
            ColumnKind::Notifications => data::notifications(),
            ColumnKind::Custom(_) => Vec::new(),
        }
    }

    /// Source des posts sur Bluesky, ou `None` si la colonne n'y est pas
    /// reliée.
    pub fn source(self) -> Option<Source> {
        match self {
            ColumnKind::Feed(Feed::Following) => Some(Source::Timeline),
            ColumnKind::Feed(Feed::Discover) => Some(Source::Feed(DISCOVER_FEED.to_owned())),
            ColumnKind::Notifications => Some(Source::Notifications),
            ColumnKind::Custom(feed) => Some(Source::Feed(feed.uri())),
            // « Linux et libre » est le fil fictif des maquettes.
            ColumnKind::Feed(Feed::LinuxEtLibre) => None,
        }
    }
}

pub struct Column {
    pub kind: ColumnKind,
    pub root: adw::ToolbarView,
    pub list: gtk::ListBox,
    header: adw::HeaderBar,
    title: adw::WindowTitle,
    switcher: gtk::MenuButton,
    switcher_label: gtk::Label,
    close: gtk::Button,
    options: gtk::MenuButton,
    moves: gio::Menu,
    refresh: gtk::Button,
    /// Les cartes et leurs états.
    pub posts: PostList,
    /// Notifications : « Mentions » choisi plutôt que « Toutes ».
    mentions_only: Rc<Cell<bool>>,
    /// Boutons « Toutes » et « Mentions » (colonne Notifications).
    filters: Vec<gtk::ToggleButton>,
}

impl Column {
    /// `columns_menu` liste les colonnes ouvertes ; il sert au sélecteur de
    /// titre quand une seule colonne est affichée.
    pub fn new(kind: ColumnKind, columns_menu: &gio::Menu) -> Self {
        let posts = PostList::new(
            &kind.title(),
            &gettext("Cette colonne n'est pas encore reliée à Bluesky."),
        );
        let list = posts.list.clone();

        // Titre simple, ou menu des colonnes ouvertes quand une seule colonne
        // est visible (comme « Following ▾ » dans la maquette Mobile).
        let title = adw::WindowTitle::new(&kind.title(), "");
        // Le titre se raccourcit (« Follo… ») plutôt que d'élargir l'en-tête
        // au-delà de 360 px avec des polices plus larges.
        let switcher_label = gtk::Label::builder()
            .label(kind.title())
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let switcher = gtk::MenuButton::builder()
            .child(&switcher_label)
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
            .content(&posts.root)
            .css_classes(["column"])
            .build();
        root.add_top_bar(&header);

        // Notifications : « Toutes » ou « Mentions » (boutons liés, à
        // remplacer par AdwToggleGroup avec libadwaita 1.7).
        let mentions_only = Rc::new(Cell::new(false));
        let mut filters = Vec::new();
        if kind == ColumnKind::Notifications {
            let bar = gtk::Box::builder()
                .css_classes(["linked"])
                .halign(gtk::Align::Center)
                .margin_top(6)
                .margin_bottom(6)
                .accessible_role(gtk::AccessibleRole::Group)
                .build();
            bar.update_property(&[gtk::accessible::Property::Label(&gettext(
                "Notifications à afficher",
            ))]);
            for (label, mentions) in [(gettext("Toutes"), false), (gettext("Mentions"), true)] {
                let button = gtk::ToggleButton::builder()
                    .label(label)
                    .active(!mentions)
                    .build();
                if let Some(first) = filters.first() {
                    button.set_group(Some(first));
                }
                let mentions_only = mentions_only.clone();
                button.connect_toggled(move |button| {
                    if button.is_active() {
                        mentions_only.set(mentions);
                    }
                });
                bar.append(&button);
                filters.push(button);
            }
            root.add_top_bar(&bar);
        }

        Self {
            kind,
            root,
            list,
            header,
            title,
            switcher,
            switcher_label,
            close,
            options,
            moves,
            refresh,
            posts,
            mentions_only,
            filters,
        }
    }

    /// Source à charger, d'après le genre de colonne et le filtre choisi.
    pub fn source(&self) -> Option<Source> {
        match self.kind {
            ColumnKind::Notifications if self.mentions_only.get() => Some(Source::Mentions),
            kind => kind.source(),
        }
    }

    pub fn connect_post_selected(&self, f: impl Fn(&gtk::ListBox, Option<Post>) + 'static) {
        self.posts.connect_post_selected(f);
    }

    /// « Actualiser » et « Réessayer » appellent `f`.
    pub fn connect_refresh(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        let g = f.clone();
        for filter in &self.filters {
            let f = f.clone();
            filter.connect_toggled(move |button| {
                if button.is_active() {
                    f();
                }
            });
        }
        self.refresh.connect_clicked(move |_| f());
        self.posts.connect_retry(move || g());
    }

    pub fn connect_end_reached(&self, f: impl Fn() + 'static) {
        self.posts.connect_end_reached(f);
    }

    /// Voir `PostList::begin_load` ; « Actualiser » est grisé pendant le
    /// chargement.
    pub fn begin_load(&self, more: bool) -> Option<Load> {
        let load = self.posts.begin_load(more);
        if load.as_ref().is_some_and(|load| !load.more) {
            self.refresh.set_sensitive(false);
        }
        load
    }

    pub fn finish_load(&self, load: &Load, result: Result<Page, ApiError>) {
        self.posts.finish_load(load, result);
        self.refresh.set_sensitive(!self.posts.is_loading());
    }

    pub fn unread_count(&self) -> u32 {
        self.posts.unread_count()
    }

    pub fn mark_all_read(&self) {
        self.posts.mark_all_read();
    }

    /// Faux posts, sans compte.
    pub fn show_demo(&self) {
        if self.mentions_only.get() {
            self.posts.show_posts(data::demo_mentions());
            return;
        }
        self.posts.show_posts(self.kind.demo_posts());
    }

    pub fn show_unavailable(&self) {
        self.posts.show_unavailable();
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

    /// Met à jour le titre, une fois le nom du fil connu.
    pub fn refresh_title(&self) {
        let title = self.kind.title();
        self.title.set_title(&title);
        self.switcher_label.set_label(&title);
        self.list
            .update_property(&[gtk::accessible::Property::Label(&title)]);
    }

    /// Donne le focus à la carte sélectionnée, ou à la première.
    pub fn focus(&self) {
        self.posts.focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for kind in ColumnKind::BUILTIN.into_iter().chain([ColumnKind::DEMO]) {
            assert_eq!(ColumnKind::from_id(&kind.id()), Some(kind));
        }
        assert_eq!(ColumnKind::from_id("inconnue"), None);
        assert_eq!(ColumnKind::from_id("feed:pas-une-adresse"), None);
    }

    #[test]
    fn pinned_feeds_get_a_stable_id() {
        let uri = "at://did:plc:a/app.bsky.feed.generator/linux";
        let kind = ColumnKind::from_id(&format!("feed:{uri}")).unwrap();
        assert_eq!(kind, ColumnKind::Custom(FeedId::intern(uri)));
        assert_eq!(kind.id(), format!("feed:{uri}"));
        // Nom provisoire, puis le vrai nom.
        assert_eq!(kind.title(), "linux");
        FeedId::intern(uri).set_name("Linux et logiciels libres");
        assert_eq!(kind.title(), "Linux et logiciels libres");
        assert_eq!(kind.source(), Some(Source::Feed(uri.to_owned())));
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
