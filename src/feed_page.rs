//! Pages simples de posts et de comptes : Signets, fil d'une liste,
//! membres d'une liste, abonnés et abonnements, et la page Listes.

use std::rc::Rc;

use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;

use crate::api::{self, ApiError, Page, Source};
use crate::data::{self, Actor, List, Post};
use crate::explore::actor_row;
use crate::post_list::PostList;
use crate::profile::Host;
use crate::runtime::runtime;

/// Page d'un fil : en-tête avec « Actualiser », posts en cartes, suite au
/// défilement. Sans compte, `demo` est affiché.
pub struct FeedPage {
    pub root: adw::ToolbarView,
    host: Host,
    source: Source,
    demo: Vec<Post>,
    posts: PostList,
}

impl FeedPage {
    pub fn new(title: &str, source: Source, demo: Vec<Post>, host: Host) -> Rc<Self> {
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(gettext("Actualiser"))
            .build();
        refresh.update_property(&[gtk::accessible::Property::Label(&gettext("Actualiser"))]);
        let header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(title, ""))
            .build();
        header.pack_end(&refresh);
        let posts = PostList::new(title, "");
        let root = adw::ToolbarView::builder().content(&posts.root).build();
        root.add_top_bar(&header);
        let page = Rc::new(Self {
            root,
            host,
            source,
            demo,
            posts,
        });
        let weak = Rc::downgrade(&page);
        refresh.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.load(false);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_retry(move || {
            if let Some(page) = weak.upgrade() {
                page.load(false);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_end_reached(move || {
            if let Some(page) = weak.upgrade() {
                page.load(true);
            }
        });
        let weak = Rc::downgrade(&page);
        page.posts.connect_post_activated(move |post| {
            if let Some(page) = weak.upgrade() {
                (page.host.open_post)(post);
            }
        });
        page.load(false);
        page
    }

    pub fn load(self: &Rc<Self>, more: bool) {
        let Some(session) = self.host.session.clone() else {
            if !more {
                self.posts.show_posts(self.demo.clone());
            }
            return;
        };
        let Some(load) = self.posts.begin_load(more) else {
            return;
        };
        let task = runtime().spawn(api::load(session, self.source.clone(), load.cursor.clone()));
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

    /// Page à empiler dans la navigation.
    pub fn navigation_page(self: &Rc<Self>, title: &str) -> adw::NavigationPage {
        keep_alive(
            adw::NavigationPage::builder()
                .title(title)
                .child(&self.root)
                .build(),
            self.clone(),
        )
    }
}

/// La page garde `value` en vie tant qu'elle existe.
fn keep_alive<T: 'static>(page: adw::NavigationPage, value: T) -> adw::NavigationPage {
    page.connect_destroy(move |_| {
        let _ = &value;
    });
    page
}

fn boxed_list(label: &str) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .css_classes(["boxed-list"])
        .build();
    list.update_property(&[gtk::accessible::Property::Label(label)]);
    list
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                .margin_top(12)
                .margin_bottom(24)
                .margin_start(12)
                .margin_end(12)
                .child(child)
                .build(),
        )
        .build()
}

fn status(title: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name("view-list-bullet-symbolic")
        .title(title)
        .css_classes(["compact"])
        .build()
}

/// Membres d'une liste de modération (qui n'a pas de fil).
pub fn members_page(list: &List, host: &Host) -> adw::NavigationPage {
    let rows = boxed_list(&list.name);
    let stack = gtk::Stack::new();
    stack.add_named(&scrolled(&rows), Some("list"));
    stack.add_named(&status(&gettext("Chargement…")), Some("loading"));
    let error = status("");
    stack.add_named(&error, Some("error"));
    let root = adw::ToolbarView::builder().content(&stack).build();
    root.add_top_bar(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(&list.name, &gettext("Membres")))
            .build(),
    );
    let page = adw::NavigationPage::builder()
        .title(&list.name)
        .child(&root)
        .build();

    let fill = {
        let (rows, stack, error, session, toasts) = (
            rows.clone(),
            stack.clone(),
            error.clone(),
            host.session.clone(),
            host.toasts.clone(),
        );
        move |members: Result<Vec<Actor>, ApiError>| match members {
            Ok(members) if !members.is_empty() => {
                for actor in &members {
                    rows.append(&actor_row(actor, session.clone(), &toasts));
                }
                stack.set_visible_child_name("list");
            }
            Ok(_) => {
                error.set_title(&gettext("Cette liste est vide."));
                stack.set_visible_child_name("error");
            }
            Err(failure) => {
                error.set_title(&failure.message);
                stack.set_visible_child_name("error");
            }
        }
    };
    let Some(session) = host.session.clone() else {
        fill(Ok(data::demo_actors()));
        return page;
    };
    stack.set_visible_child_name("loading");
    let task = runtime().spawn(api::list_members(session, list.uri.clone()));
    glib::spawn_future_local(async move {
        fill(
            task.await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu.")))),
        );
    });
    page
}

/// Page « Listes » : mes listes. Une liste de curation ouvre son fil, une
/// liste de modération ses membres.
pub struct ListsPage {
    pub root: adw::ToolbarView,
    host: Host,
    rows: gtk::ListBox,
    stack: gtk::Stack,
    error: adw::StatusPage,
    lists: std::cell::RefCell<Vec<List>>,
}

fn list_row(list: &List) -> adw::ActionRow {
    let mut subtitle = ngettext(
        "{} membre",
        "{} membres",
        list.members.min(u32::MAX as u64) as u32,
    )
    .replace("{}", &crate::profile::format_count(list.members));
    if list.moderation {
        subtitle = format!("{} · {}", gettext("Liste de modération"), subtitle);
    }
    if !list.description.trim().is_empty() {
        subtitle = format!("{subtitle}\n{}", list.description.trim());
    }
    let row = adw::ActionRow::builder()
        .title(&list.name)
        .subtitle(&subtitle)
        .subtitle_lines(3)
        .use_markup(false)
        .activatable(true)
        .build();
    let avatar = adw::Avatar::builder()
        .size(40)
        .text(&list.name)
        .show_initials(true)
        .build();
    if let Some(url) = &list.avatar {
        let weak = avatar.downgrade();
        crate::images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    row.add_prefix(&avatar);
    row.add_suffix(&crate::card::decorative_icon("go-next-symbolic"));
    row
}

impl ListsPage {
    pub fn new(host: Host) -> Rc<Self> {
        let rows = boxed_list(&gettext("Mes listes"));
        let error = status("");
        let stack = gtk::Stack::new();
        stack.add_named(&scrolled(&rows), Some("list"));
        stack.add_named(&error, Some("error"));
        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(&gettext("Listes"), ""))
                .build(),
        );
        let page = Rc::new(Self {
            root,
            host,
            rows,
            stack,
            error,
            lists: Default::default(),
        });
        let weak = Rc::downgrade(&page);
        page.rows.connect_row_activated(move |_, row| {
            let Some(page) = weak.upgrade() else { return };
            let list = page.lists.borrow().get(row.index() as usize).cloned();
            if let Some(list) = list {
                page.open(&list);
            }
        });
        page.load();
        page
    }

    fn load(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            self.show(Ok(data::demo_lists()));
            return;
        };
        let me = session.account.did.clone();
        let task = runtime().spawn(api::lists(session, me));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Err(error) if error.expired => (page.host.expired)(error.message),
                result => page.show(result),
            }
        });
    }

    fn show(&self, result: Result<Vec<List>, ApiError>) {
        self.rows.remove_all();
        match result {
            Ok(lists) if !lists.is_empty() => {
                for list in &lists {
                    self.rows.append(&list_row(list));
                }
                self.lists.replace(lists);
                self.stack.set_visible_child_name("list");
            }
            Ok(_) => {
                self.error
                    .set_title(&gettext("Vous n'avez pas encore de liste."));
                self.stack.set_visible_child_name("error");
            }
            Err(error) => {
                self.error.set_title(&error.message);
                self.stack.set_visible_child_name("error");
            }
        }
    }

    fn open(&self, list: &List) {
        let page = if list.moderation {
            members_page(list, &self.host)
        } else {
            let feed = FeedPage::new(
                &list.name,
                Source::List(list.uri.clone()),
                data::posts(data::Feed::LinuxEtLibre),
                self.host.clone(),
            );
            feed.navigation_page(&list.name)
        };
        (self.host.push)(page);
    }
}

/// Liste de comptes (abonnés, abonnements, j'aime, reposts), avec la
/// suite au défilement. `subtitle` : le compte ou le post concerné.
pub fn people_page(people: api::People, subtitle: &str, host: &Host) -> adw::NavigationPage {
    let title = people.title();
    let rows = boxed_list(&title);
    let scroll = scrolled(&rows);
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&status(&gettext("Chargement…")), Some("loading"));
    let error = status("");
    stack.add_named(&error, Some("error"));
    let root = adw::ToolbarView::builder().content(&stack).build();
    root.add_top_bar(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(&title, subtitle))
            .build(),
    );
    let page = adw::NavigationPage::builder()
        .title(&title)
        .child(&root)
        .build();

    let Some(session) = host.session.clone() else {
        let toasts = host.toasts.clone();
        for actor in data::demo_actors() {
            rows.append(&actor_row(&actor, None, &toasts));
        }
        return page;
    };
    stack.set_visible_child_name("loading");

    // Page suivante : `None` avant la première, `Some(None)` à la fin.
    struct Loader {
        session: std::sync::Arc<crate::auth::Session>,
        people: api::People,
        rows: gtk::ListBox,
        stack: gtk::Stack,
        error: adw::StatusPage,
        toasts: adw::ToastOverlay,
        expired: Rc<dyn Fn(String)>,
        cursor: std::cell::RefCell<Option<Option<String>>>,
        loading: std::cell::Cell<bool>,
    }
    impl Loader {
        fn load(self: &Rc<Self>) {
            let cursor = match self.cursor.borrow().clone() {
                Some(None) => return,
                Some(Some(cursor)) => Some(cursor),
                None => None,
            };
            if self.loading.replace(true) {
                return;
            }
            let task = runtime().spawn(api::people(
                self.session.clone(),
                self.people.clone(),
                cursor,
            ));
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
                let Some(this) = weak.upgrade() else { return };
                this.loading.set(false);
                let first = this.cursor.borrow().is_none();
                match result {
                    Ok((actors, cursor)) => {
                        for actor in &actors {
                            this.rows.append(&actor_row(
                                actor,
                                Some(this.session.clone()),
                                &this.toasts,
                            ));
                        }
                        this.cursor
                            .replace(Some(cursor.filter(|_| !actors.is_empty())));
                        if first && actors.is_empty() {
                            this.error.set_title(&this.people.empty());
                            this.stack.set_visible_child_name("error");
                        } else {
                            this.stack.set_visible_child_name("list");
                        }
                    }
                    Err(error) if error.expired => (this.expired)(error.message),
                    Err(error) if first => {
                        this.error.set_title(&error.message);
                        this.stack.set_visible_child_name("error");
                    }
                    Err(error) => this.toasts.add_toast(adw::Toast::new(&error.message)),
                }
            });
        }
    }
    let loader = Rc::new(Loader {
        session,
        people,
        rows,
        stack,
        error,
        toasts: host.toasts.clone(),
        expired: host.expired.clone(),
        cursor: Default::default(),
        loading: Default::default(),
    });
    let weak = Rc::downgrade(&loader);
    scroll.connect_edge_reached(move |_, edge| {
        if edge == gtk::PositionType::Bottom
            && let Some(loader) = weak.upgrade()
        {
            loader.load();
        }
    });
    loader.load();
    keep_alive(page, loader)
}
