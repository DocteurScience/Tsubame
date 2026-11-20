//! Fenêtre principale : barre latérale, fil de cartes et volet de discussion.
//!
//! Disposition (voir `docs/design.md`, « Paliers d'adaptation ») :
//!
//! - `AdwOverlaySplitView` : barre latérale | pages ;
//! - `AdwViewStack` : une page par destination (Accueil, Explorer…) ;
//! - page Accueil : `AdwNavigationSplitView` fil | discussion ;
//! - `AdwViewSwitcherBar` en bas, montrée seulement en palier compact.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;

use crate::card;
use crate::data::{self, Feed, Post};

/// Largeur sous laquelle la fenêtre passe en colonne unique.
const COMPACT_BREAKPOINT: &str = "max-width: 480sp";
/// Largeur sous laquelle le fil et la discussion ne tiennent plus côte à côte.
const NARROW_BREAKPOINT: &str = "max-width: 860sp";

/// Largeur d'une colonne de cartes (voir `docs/design.md`).
const COLUMN_MIN_WIDTH: i32 = 380;
const COLUMN_MAX_WIDTH: i32 = 480;

#[derive(Clone, Copy)]
enum Destination {
    Feed(Feed),
    Page(&'static str),
}

struct SidebarEntry {
    icon: &'static str,
    title: String,
    badge: Option<u32>,
    destination: Destination,
}

fn feed_title(feed: Feed) -> String {
    match feed {
        // Noms propres des fils de Bluesky, non traduits.
        Feed::Following => "Following".to_owned(),
        Feed::Discover => "Discover".to_owned(),
        Feed::LinuxEtLibre => gettext("Linux et libre"),
    }
}

fn sidebar_entries() -> Vec<SidebarEntry> {
    let feed = |feed| SidebarEntry {
        icon: "view-list-symbolic",
        title: feed_title(feed),
        badge: None,
        destination: Destination::Feed(feed),
    };
    let page = |icon, title: String, badge, name| SidebarEntry {
        icon,
        title,
        badge,
        destination: Destination::Page(name),
    };
    vec![
        feed(Feed::Following),
        feed(Feed::Discover),
        feed(Feed::LinuxEtLibre),
        page(
            "system-search-symbolic",
            gettext("Explorer"),
            None,
            "explore",
        ),
        page(
            "preferences-system-notifications-symbolic",
            gettext("Notifications"),
            Some(3),
            "notifications",
        ),
        page(
            "mail-unread-symbolic",
            gettext("Messages"),
            Some(2),
            "messages",
        ),
        page(
            "user-bookmarks-symbolic",
            gettext("Signets"),
            None,
            "bookmarks",
        ),
        page(
            "view-list-bullet-symbolic",
            gettext("Listes"),
            None,
            "lists",
        ),
        page(
            "avatar-default-symbolic",
            gettext("Profil"),
            None,
            "profile",
        ),
    ]
}

/// Index des premières rangées de chaque section de la barre latérale.
const SECTION_FEEDS: i32 = 0;
const SECTION_BROWSE: i32 = 3;

fn sidebar_row(entry: &SidebarEntry) -> gtk::ListBoxRow {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(entry.icon));
    content.append(
        &gtk::Label::builder()
            .label(&entry.title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let row = gtk::ListBoxRow::builder().child(&content).build();
    if let Some(count) = entry.badge {
        content.append(
            &gtk::Label::builder()
                .label(count.to_string())
                .css_classes(["sidebar-badge"])
                .valign(gtk::Align::Center)
                .build(),
        );
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, {}",
            entry.title,
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

/// Widgets dont les gestionnaires d'événements ont besoin.
struct State {
    stack: adw::ViewStack,
    sidebar_list: gtk::ListBox,
    feed_page: adw::NavigationPage,
    feed_list: gtk::ListBox,
    home_split: adw::NavigationSplitView,
    thread_stack: gtk::Stack,
    thread_box: gtk::Box,
    entries: Vec<SidebarEntry>,
    posts: RefCell<Vec<Post>>,
}

impl State {
    fn show_feed(&self, feed: Feed) {
        self.feed_page.set_title(&feed_title(feed));
        self.feed_list.remove_all();
        let posts = data::posts(feed);
        for post in &posts {
            let row = gtk::ListBoxRow::builder()
                .child(&card::post_card(post))
                .build();
            row.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} : {}",
                post.name, post.text
            ))]);
            self.feed_list.append(&row);
        }
        *self.posts.borrow_mut() = posts;
        self.show_thread(None);
    }

    fn show_thread(&self, index: Option<usize>) {
        let posts = self.posts.borrow();
        let Some(post) = index.and_then(|i| posts.get(i)) else {
            self.thread_stack.set_visible_child_name("empty");
            self.home_split.set_show_content(false);
            return;
        };

        while let Some(child) = self.thread_box.first_child() {
            self.thread_box.remove(&child);
        }
        self.thread_box.append(&card::post_detail(post));
        if !post.thread.is_empty() {
            let count = post.thread.len() as u32;
            self.thread_box.append(
                &gtk::Label::builder()
                    .label(
                        gettextrs::ngettext("{} réponse", "{} réponses", count)
                            .replace("{}", &count.to_string()),
                    )
                    .xalign(0.0)
                    .margin_top(12)
                    .css_classes(["heading"])
                    .build(),
            );
            for reply in &post.thread {
                self.thread_box.append(&card::reply_card(reply));
            }
        }
        self.thread_stack.set_visible_child_name("thread");
    }

    /// Met la rangée de la barre latérale en accord avec la page affichée,
    /// par exemple après un clic dans la barre du bas.
    fn sync_sidebar(&self) {
        let Some(name) = self.stack.visible_child_name() else {
            return;
        };
        let current = self
            .sidebar_list
            .selected_row()
            .and_then(|row| self.entries.get(row.index() as usize))
            .map(|entry| page_name(entry.destination));
        if current == Some(name.as_str()) {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| page_name(entry.destination) == name.as_str())
        {
            self.sidebar_list
                .select_row(self.sidebar_list.row_at_index(index as i32).as_ref());
        }
    }
}

fn page_name(destination: Destination) -> &'static str {
    match destination {
        Destination::Feed(_) => "home",
        Destination::Page(name) => name,
    }
}

fn build_home(
    state_slot: &Rc<RefCell<Option<Rc<State>>>>,
) -> (
    adw::NavigationSplitView,
    adw::NavigationPage,
    gtk::ListBox,
    gtk::Stack,
    gtk::Box,
) {
    // Fil de cartes.
    let feed_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .css_classes(["post-list"])
        .build();
    feed_list.update_property(&[gtk::accessible::Property::Label(&gettext("Posts"))]);
    let feed_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(COLUMN_MAX_WIDTH)
                .tightening_threshold(COLUMN_MIN_WIDTH)
                .child(&feed_list)
                .build(),
        )
        .build();
    let feed_view = adw::ToolbarView::new();
    feed_view.add_top_bar(&adw::HeaderBar::new());
    feed_view.set_content(Some(&feed_scroll));
    let feed_page = adw::NavigationPage::builder()
        .tag("feed")
        .title("Following")
        .child(&feed_view)
        .build();

    // Volet de discussion.
    let thread_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let thread_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(720)
                .child(&thread_box)
                .build(),
        )
        .build();
    let thread_stack = gtk::Stack::new();
    thread_stack.add_named(
        &adw::StatusPage::builder()
            .icon_name(format!("{}-symbolic", crate::APP_ID))
            .title(gettext("Aucun post ouvert"))
            .description(gettext("Choisissez un post pour afficher la discussion."))
            .build(),
        Some("empty"),
    );
    thread_stack.add_named(&thread_scroll, Some("thread"));
    let thread_view = adw::ToolbarView::new();
    thread_view.add_top_bar(&adw::HeaderBar::new());
    thread_view.set_content(Some(&thread_stack));
    let thread_page = adw::NavigationPage::builder()
        .tag("thread")
        .title(gettext("Discussion"))
        .child(&thread_view)
        .build();

    let home_split = adw::NavigationSplitView::builder()
        .sidebar(&feed_page)
        .content(&thread_page)
        .min_sidebar_width(COLUMN_MIN_WIDTH as f64)
        .max_sidebar_width(COLUMN_MAX_WIDTH as f64)
        .sidebar_width_fraction(0.45)
        .build();

    // La sélection (clic ou flèches) remplit le volet ; l'activation (clic ou
    // Entrée) l'ouvre aussi quand il est empilé, en palier compact.
    let slot = state_slot.clone();
    feed_list.connect_row_selected(move |_, row| {
        if let Some(state) = slot.borrow().as_ref() {
            state.show_thread(row.map(|r| r.index() as usize));
        }
    });
    let slot = state_slot.clone();
    feed_list.connect_row_activated(move |_, _| {
        if let Some(state) = slot.borrow().as_ref() {
            state.home_split.set_show_content(true);
        }
    });

    (home_split, feed_page, feed_list, thread_stack, thread_box)
}

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let state_slot: Rc<RefCell<Option<Rc<State>>>> = Rc::default();

    // Pages.
    let stack = adw::ViewStack::new();
    let (home_split, feed_page, feed_list, thread_stack, thread_box) = build_home(&state_slot);
    stack.add_titled_with_icon(
        &home_split,
        Some("home"),
        &gettext("Accueil"),
        "go-home-symbolic",
    );
    stack.add_titled_with_icon(
        &placeholder_page("system-search-symbolic", &gettext("Explorer")),
        Some("explore"),
        &gettext("Explorer"),
        "system-search-symbolic",
    );
    let notifications = stack.add_titled_with_icon(
        &placeholder_page(
            "preferences-system-notifications-symbolic",
            &gettext("Notifications"),
        ),
        Some("notifications"),
        &gettext("Notifications"),
        "preferences-system-notifications-symbolic",
    );
    notifications.set_badge_number(3);
    notifications.set_needs_attention(true);
    let messages = stack.add_titled_with_icon(
        &placeholder_page("mail-unread-symbolic", &gettext("Messages")),
        Some("messages"),
        &gettext("Messages"),
        "mail-unread-symbolic",
    );
    messages.set_badge_number(2);
    messages.set_needs_attention(true);
    let bookmarks = stack.add_titled_with_icon(
        &placeholder_page("user-bookmarks-symbolic", &gettext("Signets")),
        Some("bookmarks"),
        &gettext("Signets"),
        "user-bookmarks-symbolic",
    );
    let lists = stack.add_titled_with_icon(
        &placeholder_page("view-list-bullet-symbolic", &gettext("Listes")),
        Some("lists"),
        &gettext("Listes"),
        "view-list-bullet-symbolic",
    );
    let profile = stack.add_titled_with_icon(
        &placeholder_page("avatar-default-symbolic", &gettext("Profil")),
        Some("profile"),
        &gettext("Profil"),
        "avatar-default-symbolic",
    );

    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let content = adw::ToolbarView::builder().content(&stack).build();
    content.add_bottom_bar(&switcher_bar);

    // Barre latérale.
    let entries = sidebar_entries();
    let sidebar_list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    sidebar_list.update_property(&[gtk::accessible::Property::Label(&gettext("Navigation"))]);
    for entry in &entries {
        sidebar_list.append(&sidebar_row(entry));
    }
    sidebar_list.set_header_func(|row, _| match row.index() {
        SECTION_FEEDS => row.set_header(Some(&section_header(&gettext("Accueil")))),
        SECTION_BROWSE => row.set_header(Some(&section_header(&gettext("Naviguer")))),
        _ => row.set_header(None::<&gtk::Widget>),
    });
    let sidebar_view = adw::ToolbarView::new();
    let menu = gio::Menu::new();
    menu.append(Some(&gettext("À propos de Tsubame")), Some("app.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text(gettext("Menu principal"))
        .build();
    menu_button.update_property(&[gtk::accessible::Property::Label(&gettext("Menu principal"))]);
    let sidebar_header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new("Tsubame", ""))
        .build();
    sidebar_header.pack_end(&menu_button);
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&sidebar_list)
            .build(),
    ));

    let outer_split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar_view)
        .content(&content)
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
        .content(&outer_split)
        .build();

    // Paliers d'adaptation. Un seul point de rupture s'applique à la fois :
    // le dernier ajouté parmi ceux qui correspondent, d'où la répétition.
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse(NARROW_BREAKPOINT).unwrap());
    narrow.add_setter(&home_split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(narrow);

    let compact =
        adw::Breakpoint::new(adw::BreakpointCondition::parse(COMPACT_BREAKPOINT).unwrap());
    compact.add_setter(&home_split, "collapsed", Some(&true.to_value()));
    compact.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
    compact.add_setter(&outer_split, "show-sidebar", Some(&false.to_value()));
    compact.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    // La barre du bas ne garde que les quatre destinations principales, comme
    // dans la maquette Mobile. Les autres pages resteront accessibles depuis
    // le profil.
    for page in [&bookmarks, &lists, &profile] {
        compact.add_setter(page, "visible", Some(&false.to_value()));
    }
    window.add_breakpoint(compact);

    let state = Rc::new(State {
        stack: stack.clone(),
        sidebar_list: sidebar_list.clone(),
        feed_page,
        feed_list,
        home_split,
        thread_stack,
        thread_box,
        entries,
        posts: RefCell::default(),
    });
    *state_slot.borrow_mut() = Some(state.clone());

    let slot = state_slot.clone();
    let split = outer_split.clone();
    sidebar_list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Some(state) = slot.borrow().clone() else {
            return;
        };
        // Quand le palier compact masque la barre latérale, GTK y déplace le
        // focus d'une rangée à l'autre, et la sélection suit. Ces changements
        // ne viennent pas de l'utilisateur : on remet la rangée de la page
        // affichée.
        if split.is_collapsed() && !split.shows_sidebar() {
            state.sync_sidebar();
            return;
        }
        let Some(entry) = state.entries.get(row.index() as usize) else {
            return;
        };
        if let Destination::Feed(feed) = entry.destination {
            state.show_feed(feed);
        }
        state
            .stack
            .set_visible_child_name(page_name(entry.destination));
    });
    let slot = state_slot;
    stack.connect_visible_child_name_notify(move |_| {
        if let Some(state) = slot.borrow().clone() {
            state.sync_sidebar();
        }
    });

    sidebar_list.select_row(sidebar_list.row_at_index(0).as_ref());
    window
}
