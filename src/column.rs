//! Une colonne de cartes : en-tête (titre, fermeture) et liste de posts.

use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;

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

    /// Nombre d'éléments non lus, affiché en pastille.
    pub fn badge(self) -> Option<u32> {
        match self {
            ColumnKind::Notifications => Some(3),
            ColumnKind::Feed(_) => None,
        }
    }

    fn posts(self) -> Vec<Post> {
        match self {
            ColumnKind::Feed(feed) => data::posts(feed),
            ColumnKind::Notifications => data::notifications(),
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
    close: gtk::Button,
    options: gtk::MenuButton,
    moves: gio::Menu,
    posts: Rc<Vec<Post>>,
}

impl Column {
    /// `columns_menu` liste les colonnes ouvertes ; il sert au sélecteur de
    /// titre quand une seule colonne est affichée.
    pub fn new(kind: ColumnKind, columns_menu: &gio::Menu) -> Self {
        let posts = kind.posts();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["post-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&kind.title())]);
        for post in &posts {
            let row = gtk::ListBoxRow::builder()
                .child(&card::post_card(post))
                .build();
            let description = match post.context {
                Some(context) => format!("{} {} : {}", post.name, gettext(context), post.text),
                None => format!("{} : {}", post.name, post.text),
            };
            row.update_property(&[gtk::accessible::Property::Label(&description)]);
            list.append(&row);
        }
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

        let header = adw::HeaderBar::builder()
            .title_widget(&title_box)
            .show_end_title_buttons(false)
            .build();
        header.pack_end(&close);

        // Menu « Options de la colonne », comme dans la maquette Colonnes.
        let target = kind.id();
        let menu = gio::Menu::new();
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
            .content(&scroll)
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
            posts: Rc::new(posts),
        }
    }

    /// Appelle `f` avec le post sélectionné dans la colonne (ou `None`
    /// quand la sélection est vidée).
    pub fn connect_post_selected(&self, f: impl Fn(&gtk::ListBox, Option<Post>) + 'static) {
        let posts = self.posts.clone();
        self.list.connect_row_selected(move |list, row| {
            let post = row.and_then(|row| posts.get(row.index() as usize)).cloned();
            f(list, post);
        });
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

    /// Adapte l'en-tête selon qu'il est seul à l'écran (paliers étroits) ou
    /// à côté d'autres colonnes et du volet.
    pub fn set_alone(&self, alone: bool) {
        self.title.set_visible(!alone);
        self.switcher.set_visible(alone);
        // Seule, la colonne porte les boutons de la fenêtre ; sinon c'est
        // le volet de discussion, à droite.
        self.header.set_show_end_title_buttons(alone);
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
