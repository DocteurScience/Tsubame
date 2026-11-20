//! Fenêtre principale : barre latérale, colonnes de cartes et discussion.
//!
//! Disposition (voir `docs/design.md`, « Paliers d'adaptation ») :
//!
//! - `AdwOverlaySplitView` : barre latérale | contenu ;
//! - contenu : `AdwNavigationView`, dont la page racine porte les pages
//!   (`AdwViewStack`) et la barre du bas, et dans laquelle s'empile la
//!   discussion aux paliers étroits ;
//! - page Accueil : `ColumnsView`, les colonnes ouvertes puis le volet de
//!   discussion.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::auth::{self, Account};
use crate::column::{Column, ColumnKind};
use crate::columns::{ColumnsView, Layout};
use crate::data::Post;
use crate::login::LoginPage;
use crate::runtime::runtime;
use crate::thread::ThreadView;

/// Sous cette largeur, une seule colonne et la discussion en page empilée :
/// la barre latérale, une colonne de 380 px et le volet ne tiennent plus.
const NARROW_BREAKPOINT: &str = "max-width: 1120sp";
/// Sous cette largeur, plus de barre latérale : barre de navigation en bas.
const COMPACT_BREAKPOINT: &str = "max-width: 480sp";

/// Pages hors colonnes, dans la section « Ailleurs » de la barre latérale.
struct OtherPage {
    name: &'static str,
    icon: &'static str,
    title: String,
    badge: Option<u32>,
}

fn other_pages() -> Vec<OtherPage> {
    let page = |name, icon, title: String, badge| OtherPage {
        name,
        icon,
        title,
        badge,
    };
    vec![
        page(
            "explore",
            "system-search-symbolic",
            gettext("Explorer"),
            None,
        ),
        page(
            "messages",
            "mail-unread-symbolic",
            gettext("Messages"),
            Some(2),
        ),
        page(
            "bookmarks",
            "user-bookmarks-symbolic",
            gettext("Signets"),
            None,
        ),
        page(
            "lists",
            "view-list-bullet-symbolic",
            gettext("Listes"),
            None,
        ),
        page(
            "profile",
            "avatar-default-symbolic",
            gettext("Profil"),
            None,
        ),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarItem {
    Column(ColumnKind),
    AddColumn,
    Page(&'static str),
}

fn sidebar_row(icon: &str, title: &str, badge: Option<u32>) -> gtk::ListBoxRow {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let row = gtk::ListBoxRow::builder().child(&content).build();
    if let Some(count) = badge {
        content.append(
            &gtk::Label::builder()
                .label(count.to_string())
                .css_classes(["sidebar-badge"])
                .valign(gtk::Align::Center)
                .build(),
        );
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, {}",
            title,
            gettextrs::ngettext("{} nouveau", "{} nouveaux", count)
                .replace("{}", &count.to_string())
        ))]);
    }
    row
}

fn section_header(title: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .margin_top(12)
        .margin_bottom(4)
        .margin_start(12)
        .css_classes(["caption-heading", "dim-label"])
        .build()
}

fn placeholder_page(icon: &str, title: &str) -> adw::ToolbarView {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(
        &adw::StatusPage::builder()
            .icon_name(icon)
            .title(title)
            .description(gettext("Cette page n'est pas encore écrite."))
            .build(),
    ));
    view
}

fn add_column_button(menu: &gio::Menu, classes: &[&str]) -> gtk::MenuButton {
    gtk::MenuButton::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label(gettext("Ajouter une colonne"))
                .build(),
        )
        .menu_model(menu)
        .css_classes(classes.to_vec())
        .build()
}

struct State {
    stack: adw::ViewStack,
    outer_split: adw::OverlaySplitView,
    nav: adw::NavigationView,
    thread_page: adw::NavigationPage,
    sidebar_list: gtk::ListBox,
    sidebar_items: RefCell<Vec<SidebarItem>>,
    /// Vrai pendant que le code change lui-même la sélection de la barre
    /// latérale, pour ne pas la traiter comme un choix de l'utilisateur.
    syncing: Cell<bool>,
    columns_view: ColumnsView,
    columns: RefCell<Vec<Column>>,
    /// Indice de la dernière colonne montrée.
    current: Cell<usize>,
    no_columns: adw::ToolbarView,
    no_columns_header: adw::HeaderBar,
    columns_menu: gio::Menu,
    add_menu: gio::Menu,
    add_button: gtk::MenuButton,
    pages: Vec<OtherPage>,
    pane: ThreadView,
    stacked: ThreadView,
    /// Colonne de la page Notifications de la barre du bas (palier compact).
    compact_notifications: Column,
    /// Colonnes ouvertes et leur ordre, position du volet : retrouvés au
    /// lancement suivant.
    settings: gio::Settings,
    window: glib::WeakRef<adw::ApplicationWindow>,
    compact: adw::Breakpoint,
    /// Indices des colonnes affichées par `ColumnsView`.
    visible_columns: RefCell<std::ops::Range<usize>>,
    root_stack: gtk::Stack,
    login: Rc<LoginPage>,
    sidebar_title: adw::WindowTitle,
    /// Compte connecté ; `None` avec les posts de démonstration.
    account: RefCell<Option<Account>>,
    logout_action: gio::SimpleAction,
}

impl State {
    fn is_open(&self, kind: ColumnKind) -> bool {
        self.columns.borrow().iter().any(|c| c.kind == kind)
    }

    fn column_index(&self, kind: ColumnKind) -> Option<usize> {
        self.columns.borrow().iter().position(|c| c.kind == kind)
    }

    /// Un palier étroit ou compact s'applique-t-il ?
    fn narrow(&self) -> bool {
        self.window
            .upgrade()
            .is_some_and(|w| w.current_breakpoint().is_some())
    }

    fn compact(&self) -> bool {
        self.window
            .upgrade()
            .and_then(|w| w.current_breakpoint())
            .is_some_and(|b| b == self.compact)
    }

    /// Disposition à appliquer : colonne unique aux paliers étroits, sinon
    /// la position du volet choisie par l'utilisateur.
    fn layout(&self) -> Layout {
        if self.narrow() {
            return Layout::Single;
        }
        match self.settings.string("pane-position").as_str() {
            "below" => Layout::Below,
            "hidden" => Layout::Hidden,
            _ => Layout::Right,
        }
    }

    fn pane_hidden(&self) -> bool {
        matches!(self.layout(), Layout::Single | Layout::Hidden)
    }

    /// Appelé quand un palier s'applique ou se retire, et quand la position
    /// du volet change.
    fn update_layout(&self) {
        let layout = self.layout();
        let narrow = layout == Layout::Single;
        self.columns_view.set_layout(layout);
        self.pane
            .root
            .set_visible(matches!(layout, Layout::Right | Layout::Below));
        self.pane.set_window_controls(layout == Layout::Right);
        self.pane.set_selector_visible(!narrow);
        self.stacked.set_selector_visible(!narrow);
        for column in self.columns.borrow().iter() {
            column.set_alone(narrow);
        }
        self.update_window_controls();
        // Cibles tactiles agrandies en palier compact (voir style.css).
        if self.compact() {
            self.root_stack.add_css_class("compact");
        } else {
            self.root_stack.remove_css_class("compact");
        }
        if !self.pane_hidden() {
            // Le volet reprend la discussion : retirer la page empilée.
            self.nav.pop_to_tag("main");
        }
    }

    /// Les boutons de la fenêtre vont dans l'en-tête le plus à droite, en
    /// haut : le volet s'il est à droite, sinon la dernière colonne affichée.
    fn update_window_controls(&self) {
        let layout = self.layout();
        let last = self.visible_columns.borrow().end.checked_sub(1);
        for (index, column) in self.columns.borrow().iter().enumerate() {
            column.set_window_controls(match layout {
                Layout::Single => true,
                Layout::Right => false,
                Layout::Below | Layout::Hidden => Some(index) == last,
            });
        }
        self.no_columns_header
            .set_show_end_title_buttons(layout != Layout::Right);
    }

    fn connect_column(self: &Rc<Self>, column: &Column) {
        let state = Rc::downgrade(self);
        column.connect_post_selected(move |list, post| {
            if let Some(state) = state.upgrade() {
                state.post_selected(list, post);
            }
        });
        let state = Rc::downgrade(self);
        column.list.connect_row_activated(move |_, _| {
            if let Some(state) = state.upgrade() {
                state.post_activated();
            }
        });
    }

    fn open_column(self: &Rc<Self>, kind: ColumnKind) {
        if !self.is_open(kind) {
            let column = Column::new(kind, &self.columns_menu);
            column.set_alone(self.layout() == Layout::Single);
            self.connect_column(&column);
            if self.columns.borrow().is_empty() {
                self.columns_view.remove_column(&self.no_columns);
            }
            self.columns_view.append_column(&column.root);
            self.columns.borrow_mut().push(column);
            self.columns_changed();
        }
        self.show_column(kind);
    }

    fn close_column(self: &Rc<Self>, kind: ColumnKind) {
        let Some(index) = self.column_index(kind) else {
            return;
        };
        let column = self.columns.borrow_mut().remove(index);
        self.columns_view.remove_column(&column.root);
        if self.columns.borrow().is_empty() {
            self.columns_view.append_column(&self.no_columns);
        }
        let len = self.columns.borrow().len();
        self.current
            .set(self.current.get().min(len.saturating_sub(1)));
        self.columns_view.reveal(self.current.get());
        self.columns_changed();
    }

    fn show_column(&self, kind: ColumnKind) {
        let Some(index) = self.column_index(kind) else {
            return;
        };
        self.current.set(index);
        self.stack.set_visible_child_name("home");
        self.columns_view.reveal(index);
        self.sync_sidebar();
        if let Some(column) = self.columns.borrow().get(index) {
            column.focus();
        }
    }

    /// Déplace une colonne à l'indice `to`. Elle reste la colonne courante.
    fn move_column_to(self: &Rc<Self>, kind: ColumnKind, to: usize) {
        let Some(from) = self.column_index(kind) else {
            return;
        };
        let to = to.min(self.columns.borrow().len() - 1);
        if from == to {
            return;
        }
        {
            let mut columns = self.columns.borrow_mut();
            let column = columns.remove(from);
            columns.insert(to, column);
        }
        self.columns_view.move_column(from, to);
        self.columns_changed();
        self.show_column(kind);
    }

    /// Déplace une colonne d'un cran vers la gauche (-1) ou la droite (+1).
    fn move_column(self: &Rc<Self>, kind: ColumnKind, delta: isize) {
        if let Some(from) = self.column_index(kind) {
            let to = from as isize + delta;
            if to >= 0 {
                self.move_column_to(kind, to as usize);
            }
        }
    }

    fn current_kind(&self) -> Option<ColumnKind> {
        self.columns
            .borrow()
            .get(self.current.get())
            .map(|c| c.kind)
    }

    fn save_columns(&self) {
        let ids: Vec<&str> = self.columns.borrow().iter().map(|c| c.kind.id()).collect();
        if let Err(error) = self.settings.set_strv("columns", ids) {
            glib::g_warning!("tsubame", "Colonnes non enregistrées : {error}");
        }
    }

    /// Passe à l'application, avec le compte connecté ou sans (posts de
    /// démonstration).
    fn enter(&self, account: Option<Account>) {
        self.sidebar_title.set_subtitle(
            &account
                .as_ref()
                .map(|a| format!("@{}", a.handle))
                .unwrap_or_default(),
        );
        self.logout_action.set_enabled(account.is_some());
        self.account.replace(account);
        self.root_stack.set_visible_child_name("main");
        if let Some(column) = self.columns.borrow().get(self.current.get()) {
            column.focus();
        }
    }

    /// Revient à l'écran de connexion.
    fn leave(&self, message: Option<&str>) {
        self.account.replace(None);
        self.logout_action.set_enabled(false);
        self.sidebar_title.set_subtitle("");
        self.login.show_form(message);
        self.root_stack.set_visible_child_name("login");
    }

    /// Demande confirmation, puis déconnecte le compte.
    fn confirm_logout(self: &Rc<Self>, window: &adw::ApplicationWindow) {
        let Some(account) = self.account.borrow().clone() else {
            return;
        };
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Se déconnecter ?")),
            Some(
                &gettext("La session de @{} sera retirée de cet ordinateur.")
                    .replace("{}", &account.handle),
            ),
        );
        dialog.add_responses(&[
            ("cancel", &gettext("Annuler")),
            ("logout", &gettext("Se déconnecter")),
        ]);
        dialog.set_response_appearance("logout", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(Some("logout"), move |_, _| {
            let Some(state) = weak.upgrade() else { return };
            let account = account.clone();
            let task = runtime().spawn(async move { auth::logout(&account).await });
            let weak = Rc::downgrade(&state);
            glib::spawn_future_local(async move {
                let result = task.await;
                let Some(state) = weak.upgrade() else { return };
                let message = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.message()),
                    Err(_) => Some(gettext("La déconnexion a échoué.")),
                };
                state.leave(message.as_deref());
            });
        });
        dialog.present(Some(window));
    }

    /// Au lancement : session gardée dans le trousseau, ou écran de
    /// connexion.
    fn restore_session(self: &Rc<Self>) {
        self.login.show_checking();
        self.root_stack.set_visible_child_name("login");
        let task = runtime().spawn(auth::stored_account());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task.await;
            let Some(state) = weak.upgrade() else { return };
            match result {
                Ok(Ok(Some(account))) => state.enter(Some(account)),
                Ok(Ok(None)) => state.leave(None),
                Ok(Err(error)) => {
                    // Sans trousseau, on peut se connecter, mais la session
                    // ne sera pas gardée : le signaler dans le journal.
                    glib::g_warning!("tsubame", "{}", error.message());
                    state.leave(None);
                }
                Err(_) => state.leave(None),
            }
        });
    }

    /// Montre la colonne voisine (Ctrl+Page suivante, Ctrl+Page précédente).
    fn step_column(&self, delta: isize) {
        let kind = {
            let columns = self.columns.borrow();
            let index = self.current.get() as isize + delta;
            let index = index.clamp(0, columns.len() as isize - 1);
            columns.get(index as usize).map(|c| c.kind)
        };
        if let Some(kind) = kind {
            self.show_column(kind);
        }
    }

    fn columns_changed(self: &Rc<Self>) {
        let count = self.columns.borrow().len();
        for (index, column) in self.columns.borrow().iter().enumerate() {
            column.set_position(index, count);
        }
        self.update_window_controls();
        self.columns_menu.remove_all();
        for column in self.columns.borrow().iter() {
            self.columns_menu.append(
                Some(&column.kind.title()),
                Some(&format!("win.show-column::{}", column.kind.id())),
            );
        }
        self.add_menu.remove_all();
        for kind in ColumnKind::ALL {
            if !self.is_open(kind) {
                self.add_menu.append(
                    Some(&kind.title()),
                    Some(&format!("win.add-column::{}", kind.id())),
                );
            }
        }
        let all_open = self.add_menu.n_items() == 0;
        self.add_button.set_sensitive(!all_open);
        self.add_button.set_tooltip_text(
            all_open
                .then(|| gettext("Toutes les colonnes sont ouvertes"))
                .as_deref(),
        );
        self.save_columns();
        self.rebuild_sidebar();
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        self.syncing.set(true);
        // Le bouton d'ajout est réutilisé : le détacher avant de vider.
        if let Some(row) = self.add_button.parent() {
            row.downcast::<gtk::ListBoxRow>()
                .expect("le bouton d'ajout est dans une rangée")
                .set_child(None::<&gtk::Widget>);
        }
        self.sidebar_list.remove_all();
        let mut items = Vec::new();
        for column in self.columns.borrow().iter() {
            let kind = column.kind;
            let row = sidebar_row(kind.icon(), &kind.title(), kind.badge());
            self.make_draggable(&row, kind);
            self.sidebar_list.append(&row);
            items.push(SidebarItem::Column(kind));
        }
        self.sidebar_list.append(
            &gtk::ListBoxRow::builder()
                .child(&self.add_button)
                .activatable(false)
                .selectable(false)
                .focusable(false)
                .build(),
        );
        items.push(SidebarItem::AddColumn);
        for page in &self.pages {
            self.sidebar_list
                .append(&sidebar_row(page.icon, &page.title, page.badge));
            items.push(SidebarItem::Page(page.name));
        }
        *self.sidebar_items.borrow_mut() = items;
        self.sidebar_list.invalidate_headers();
        self.syncing.set(false);
        self.sync_sidebar();
    }

    /// Les rangées des colonnes ouvertes se réordonnent par glisser-déposer.
    /// Pendant le survol, un trait marque l'endroit du dépôt : au-dessus de
    /// la rangée dans sa moitié haute, en dessous dans sa moitié basse.
    fn make_draggable(self: &Rc<Self>, row: &gtk::ListBoxRow, kind: ColumnKind) {
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        source.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&kind.id().to_value()))
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
        // Lire la colonne glissée dès le survol, pour ne rien marquer là où
        // le dépôt ne changerait rien.
        target.set_preload(true);

        let state = Rc::downgrade(self);
        target.connect_motion(move |target, _, y| {
            let Some(row) = target.widget() else {
                return gtk::gdk::DragAction::empty();
            };
            let destination = state
                .upgrade()
                .and_then(|state| state.drop_destination(target.value().as_ref(), kind, &row, y));
            show_drop_marker(&row, destination.map(|(_, _, after)| after));
            match destination {
                Some(_) => gtk::gdk::DragAction::MOVE,
                None => gtk::gdk::DragAction::empty(),
            }
        });
        target.connect_leave(|target| {
            if let Some(row) = target.widget() {
                show_drop_marker(&row, None);
            }
        });

        let state = Rc::downgrade(self);
        target.connect_drop(move |target, value, _, y| {
            let Some(row) = target.widget() else {
                return false;
            };
            show_drop_marker(&row, None);
            let Some(state) = state.upgrade() else {
                return false;
            };
            let Some((dropped, to, _)) = state.drop_destination(Some(value), kind, &row, y) else {
                return false;
            };
            // La barre latérale est reconstruite : attendre la fin du dépôt.
            glib::idle_add_local_once(move || state.move_column_to(dropped, to));
            true
        });
        row.add_controller(target);
    }

    /// Où irait la colonne glissée (`value`) si on la déposait à la hauteur
    /// `y` de la rangée de `kind` : la colonne, son nouvel indice et si le
    /// dépôt se fait après la rangée. `None` quand rien ne changerait.
    fn drop_destination(
        &self,
        value: Option<&glib::Value>,
        kind: ColumnKind,
        row: &gtk::Widget,
        y: f64,
    ) -> Option<(ColumnKind, usize, bool)> {
        let dragged = value
            .and_then(|v| v.get::<String>().ok())
            .and_then(|id| ColumnKind::from_id(&id))?;
        let from = self.column_index(dragged)?;
        let over = self.column_index(kind)?;
        let after = y > f64::from(row.height()) / 2.0;
        let to = drop_index(from, over, after)?;
        Some((dragged, to, after))
    }

    /// Met la sélection de la barre latérale en accord avec ce qui est
    /// affiché.
    fn sync_sidebar(&self) {
        let wanted = match self.stack.visible_child_name().as_deref() {
            Some("home") => self
                .columns
                .borrow()
                .get(self.current.get())
                .map(|c| SidebarItem::Column(c.kind)),
            Some(name) => self
                .pages
                .iter()
                .find(|p| p.name == name)
                .map(|p| SidebarItem::Page(p.name)),
            None => None,
        };
        let index = wanted.and_then(|w| self.sidebar_items.borrow().iter().position(|i| *i == w));
        let row = index.and_then(|i| self.sidebar_list.row_at_index(i as i32));
        if self.sidebar_list.selected_row() != row {
            self.syncing.set(true);
            self.sidebar_list.select_row(row.as_ref());
            self.syncing.set(false);
        }
    }

    fn sidebar_selected(&self, row: &gtk::ListBoxRow) {
        if self.syncing.get() {
            return;
        }
        // Quand le palier compact masque la barre latérale, GTK y déplace le
        // focus d'une rangée à l'autre, et la sélection suit. Ces changements
        // ne viennent pas de l'utilisateur.
        if self.outer_split.is_collapsed() && !self.outer_split.shows_sidebar() {
            self.sync_sidebar();
            return;
        }
        let item = self
            .sidebar_items
            .borrow()
            .get(row.index() as usize)
            .copied();
        match item {
            Some(SidebarItem::Column(kind)) => self.show_column(kind),
            Some(SidebarItem::Page(name)) => self.stack.set_visible_child_name(name),
            Some(SidebarItem::AddColumn) | None => {}
        }
    }

    fn lists(&self) -> Vec<gtk::ListBox> {
        let mut lists: Vec<_> = self
            .columns
            .borrow()
            .iter()
            .map(|c| c.list.clone())
            .collect();
        lists.push(self.compact_notifications.list.clone());
        lists
    }

    /// Un seul post est sélectionné à la fois, toutes colonnes confondues.
    fn post_selected(&self, list: &gtk::ListBox, post: Option<Post>) {
        let Some(post) = post else {
            return;
        };
        for other in self.lists() {
            if &other != list {
                other.unselect_all();
            }
        }
        self.pane.show(Some(&post));
        self.stacked.show(Some(&post));
    }

    /// Aux paliers étroits, ouvrir un post empile la page de discussion.
    fn post_activated(&self) {
        if self.pane_hidden() && self.nav.visible_page().as_ref() != Some(&self.thread_page) {
            self.nav.push(&self.thread_page);
        }
    }
}

/// Nouvel indice d'une colonne glissée depuis `from` et déposée avant ou
/// après la colonne d'indice `over`, ou `None` si elle ne bouge pas.
fn drop_index(from: usize, over: usize, after: bool) -> Option<usize> {
    let insert_at = over + usize::from(after);
    // Retirer la colonne décale celles qui la suivent.
    let to = if from < insert_at {
        insert_at - 1
    } else {
        insert_at
    };
    (to != from).then_some(to)
}

/// Trait de dépôt au-dessus (`Some(false)`) ou en dessous (`Some(true)`)
/// d'une rangée, ou aucun.
fn show_drop_marker(row: &gtk::Widget, after: Option<bool>) {
    row.set_css_classes(
        &row.css_classes()
            .iter()
            .map(|c| c.as_str())
            .filter(|c| *c != "drop-before" && *c != "drop-after")
            .chain(after.map(|after| if after { "drop-after" } else { "drop-before" }))
            .collect::<Vec<_>>(),
    );
}

fn install_actions(window: &adw::ApplicationWindow, state: &Rc<State>) {
    let weak = Rc::downgrade(state);
    let win = window.downgrade();
    state.logout_action.connect_activate(move |_, _| {
        if let (Some(state), Some(window)) = (weak.upgrade(), win.upgrade()) {
            state.confirm_logout(&window);
        }
    });
    state.logout_action.set_enabled(false);
    window.add_action(&state.logout_action);

    fn kind_action(
        name: &str,
        state: &Rc<State>,
        f: impl Fn(&Rc<State>, ColumnKind) + 'static,
    ) -> gio::SimpleAction {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let state: Weak<State> = Rc::downgrade(state);
        action.connect_activate(move |_, param| {
            let kind = param
                .and_then(|p| p.get::<String>())
                .and_then(|id| ColumnKind::from_id(&id));
            if let (Some(state), Some(kind)) = (state.upgrade(), kind) {
                f(&state, kind);
            }
        });
        action
    }
    window.add_action(&kind_action("add-column", state, |s, k| s.open_column(k)));
    window.add_action(&kind_action("close-column", state, |s, k| {
        s.close_column(k)
    }));
    window.add_action(&kind_action("show-column", state, |s, k| s.show_column(k)));
    window.add_action(&kind_action("move-column-left", state, |s, k| {
        s.move_column(k, -1)
    }));
    window.add_action(&kind_action("move-column-right", state, |s, k| {
        s.move_column(k, 1)
    }));
    for (name, delta) in [("move-current-left", -1), ("move-current-right", 1)] {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(state);
        action.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade()
                && let Some(kind) = state.current_kind()
            {
                state.move_column(kind, delta);
            }
        });
        window.add_action(&action);
    }

    for (name, delta) in [("next-column", 1), ("previous-column", -1)] {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(state);
        action.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.step_column(delta);
            }
        });
        window.add_action(&action);
    }
}

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let columns_menu = gio::Menu::new();
    let add_menu = gio::Menu::new();

    // Discussion : volet à droite des colonnes, ou page empilée.
    let pane = ThreadView::new();
    let stacked = ThreadView::new();
    let thread_page = adw::NavigationPage::builder()
        .tag("thread")
        .title(gettext("Discussion"))
        .child(&stacked.root)
        .build();

    // Colonnes, et ce qui les remplace quand aucune n'est ouverte.
    let columns_view = ColumnsView::default();
    columns_view.set_pane(&pane.root);
    let no_columns = adw::ToolbarView::new();
    let no_columns_header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new("", ""))
        .show_end_title_buttons(false)
        .build();
    no_columns.add_top_bar(&no_columns_header);
    let add_from_empty = add_column_button(&add_menu, &["pill", "suggested-action"]);
    add_from_empty.set_halign(gtk::Align::Center);
    no_columns.set_content(Some(
        &adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title(gettext("Aucune colonne ouverte"))
            .child(&add_from_empty)
            .build(),
    ));

    // Pages.
    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &columns_view,
        Some("home"),
        &gettext("Accueil"),
        "go-home-symbolic",
    );
    let pages = other_pages();
    let explore = &pages[0];
    stack.add_titled_with_icon(
        &placeholder_page(explore.icon, &explore.title),
        Some(explore.name),
        &explore.title,
        explore.icon,
    );
    // En palier compact, la barre du bas a son propre onglet Notifications,
    // comme dans la maquette Mobile. Aux autres paliers, les notifications
    // sont une colonne.
    let compact_notifications = Column::new(ColumnKind::Notifications, &columns_menu);
    compact_notifications.set_standalone();
    let notifications = stack.add_titled_with_icon(
        &compact_notifications.root,
        Some("notifications"),
        &gettext("Notifications"),
        ColumnKind::Notifications.icon(),
    );
    notifications.set_badge_number(3);
    notifications.set_needs_attention(true);
    notifications.set_visible(false);
    let mut hidden_when_compact = Vec::new();
    for page in &pages[1..] {
        let stack_page = stack.add_titled_with_icon(
            &placeholder_page(page.icon, &page.title),
            Some(page.name),
            &page.title,
            page.icon,
        );
        if let Some(badge) = page.badge {
            stack_page.set_badge_number(badge);
            stack_page.set_needs_attention(true);
        }
        if page.name != "messages" {
            hidden_when_compact.push(stack_page);
        }
    }

    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let main_view = adw::ToolbarView::builder().content(&stack).build();
    main_view.add_bottom_bar(&switcher_bar);
    let nav = adw::NavigationView::new();
    nav.add(
        &adw::NavigationPage::builder()
            .tag("main")
            .title("Tsubame")
            .child(&main_view)
            .build(),
    );

    // Barre latérale.
    let sidebar_list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    sidebar_list.update_property(&[gtk::accessible::Property::Label(&gettext("Navigation"))]);
    let menu = gio::Menu::new();
    let account_section = gio::Menu::new();
    account_section.append(Some(&gettext("Se déconnecter…")), Some("win.logout"));
    menu.append_section(None, &account_section);
    let about_section = gio::Menu::new();
    about_section.append(Some(&gettext("À propos de Tsubame")), Some("app.about"));
    menu.append_section(None, &about_section);
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text(gettext("Menu principal"))
        .build();
    menu_button.update_property(&[gtk::accessible::Property::Label(&gettext("Menu principal"))]);
    // Le sous-titre affiche le compte connecté.
    let sidebar_title = adw::WindowTitle::new("Tsubame", "");
    let sidebar_header = adw::HeaderBar::builder()
        .title_widget(&sidebar_title)
        .build();
    sidebar_header.pack_end(&menu_button);
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&sidebar_list)
            .build(),
    ));

    let outer_split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar_view)
        .content(&nav)
        .min_sidebar_width(200.0)
        .max_sidebar_width(260.0)
        .build();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tsubame")
        .default_width(1280)
        .default_height(800)
        .width_request(360)
        .height_request(294)
        .build();

    // À la racine : l'écran de connexion ou l'application.
    let login = LoginPage::new();
    let root_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    root_stack.add_named(&login.root, Some("login"));
    root_stack.add_named(&outer_split, Some("main"));
    window.set_content(Some(&root_stack));

    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse(NARROW_BREAKPOINT).unwrap());
    let compact =
        adw::Breakpoint::new(adw::BreakpointCondition::parse(COMPACT_BREAKPOINT).unwrap());

    let state = Rc::new(State {
        stack: stack.clone(),
        outer_split: outer_split.clone(),
        nav,
        thread_page,
        sidebar_list: sidebar_list.clone(),
        sidebar_items: RefCell::default(),
        syncing: Cell::new(false),
        columns_view: columns_view.clone(),
        columns: RefCell::default(),
        current: Cell::new(0),
        no_columns,
        no_columns_header,
        columns_menu,
        add_menu: add_menu.clone(),
        add_button: add_column_button(&add_menu, &["flat", "add-column"]),
        pages,
        pane,
        stacked,
        compact_notifications,
        settings: crate::settings::settings(),
        window: window.downgrade(),
        compact: compact.clone(),
        visible_columns: RefCell::default(),
        root_stack: root_stack.clone(),
        login: login.clone(),
        sidebar_title,
        account: RefCell::default(),
        logout_action: gio::SimpleAction::new("logout", None),
    });
    state.connect_column(&state.compact_notifications);
    install_actions(&window, &state);

    // Position du volet : action à état liée au réglage, que les boutons du
    // sélecteur reflètent d'eux-mêmes.
    window.add_action(&state.settings.create_action("pane-position"));
    let weak = Rc::downgrade(&state);
    state
        .settings
        .connect_changed(Some("pane-position"), move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.update_layout();
            }
        });
    let weak = Rc::downgrade(&state);
    columns_view.connect_visible_changed(move |visible| {
        if let Some(state) = weak.upgrade() {
            *state.visible_columns.borrow_mut() = visible;
            state.update_window_controls();
        }
    });

    // En-têtes de section : « Colonnes ouvertes » puis « Ailleurs ».
    let weak = Rc::downgrade(&state);
    sidebar_list.set_header_func(move |row, _| {
        let Some(state) = weak.upgrade() else { return };
        let item = state
            .sidebar_items
            .borrow()
            .get(row.index() as usize)
            .copied();
        let title = match item {
            _ if row.index() == 0 => Some(gettext("Colonnes ouvertes")),
            Some(SidebarItem::Page(name)) if name == state.pages[0].name => {
                Some(gettext("Ailleurs"))
            }
            _ => None,
        };
        row.set_header(title.map(|t| section_header(&t)).as_ref());
    });
    let weak = Rc::downgrade(&state);
    sidebar_list.connect_row_selected(move |_, row| {
        if let (Some(state), Some(row)) = (weak.upgrade(), row) {
            state.sidebar_selected(row);
        }
    });
    let weak = Rc::downgrade(&state);
    stack.connect_visible_child_name_notify(move |_| {
        if let Some(state) = weak.upgrade() {
            state.sync_sidebar();
        }
    });

    // Paliers d'adaptation. Un seul point de rupture s'applique à la fois :
    // le dernier ajouté parmi ceux qui correspondent, d'où la répétition.
    compact.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
    compact.add_setter(&outer_split, "show-sidebar", Some(&false.to_value()));
    compact.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    compact.add_setter(&notifications, "visible", Some(&true.to_value()));
    // La barre du bas ne garde que les quatre destinations principales.
    for page in &hidden_when_compact {
        compact.add_setter(page, "visible", Some(&false.to_value()));
    }
    for breakpoint in [narrow, compact] {
        // Les setters ne sont appliqués ou rétablis qu'après ces signaux :
        // attendre avant de lire la nouvelle disposition.
        let weak = Rc::downgrade(&state);
        let changed = move || {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.update_layout();
                }
            });
        };
        let on_unapply = changed.clone();
        breakpoint.connect_apply(move |_| changed());
        breakpoint.connect_unapply(move |_| on_unapply());
        window.add_breakpoint(breakpoint);
    }

    // Colonnes enregistrées (Following et Notifications au premier
    // lancement), la première au premier plan.
    columns_view.append_column(&state.no_columns);
    let saved: Vec<ColumnKind> = state
        .settings
        .strv("columns")
        .iter()
        .filter_map(|id| ColumnKind::from_id(id))
        .collect();
    for kind in &saved {
        state.open_column(*kind);
    }
    state.columns_changed();
    if let Some(first) = saved.first() {
        state.show_column(*first);
    }

    state.update_layout();

    let weak = Rc::downgrade(&state);
    login.connect_logged_in(move |account| {
        if let Some(state) = weak.upgrade() {
            state.enter(account);
        }
    });
    state.restore_session();

    // Les gestionnaires ne gardent qu'une référence faible à l'état ; la
    // fenêtre garde la seule référence forte, libérée avec elle.
    window.connect_destroy(move |_| {
        let _ = &state;
    });

    let app = app.clone();
    app.set_accels_for_action("win.next-column", &["<Control>Page_Down"]);
    app.set_accels_for_action("win.previous-column", &["<Control>Page_Up"]);
    app.set_accels_for_action("win.move-current-right", &["<Control><Shift>Page_Down"]);
    app.set_accels_for_action("win.move-current-left", &["<Control><Shift>Page_Up"]);
    window
}

#[cfg(test)]
mod tests {
    use super::drop_index;

    #[test]
    fn dropping_moves_to_the_marked_place() {
        // Colonnes A B C D ; on glisse A (0).
        assert_eq!(drop_index(0, 2, false), Some(1)); // avant C : B A C D
        assert_eq!(drop_index(0, 2, true), Some(2)); // après C : B C A D
        assert_eq!(drop_index(0, 3, true), Some(3)); // après D : B C D A
        // On glisse D (3).
        assert_eq!(drop_index(3, 0, false), Some(0)); // avant A : D A B C
        assert_eq!(drop_index(3, 1, true), Some(2)); // après B : A B D C
    }

    #[test]
    fn dropping_next_to_itself_changes_nothing() {
        assert_eq!(drop_index(1, 1, false), None);
        assert_eq!(drop_index(1, 1, true), None);
        assert_eq!(drop_index(1, 0, true), None); // après la précédente
        assert_eq!(drop_index(1, 2, false), None); // avant la suivante
    }
}
