//! Page Messages (maquette `docs/mockups/source/Messages.dc.html`) :
//! conversations et demandes à gauche, conversation ouverte à droite.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::{gettext, ngettext};
use gtk::glib;

use crate::api::ApiError;
use crate::chat::{self, Convo, Message, MessagePage};
use crate::images;
use crate::profile::Host;
use crate::runtime::runtime;

type Unread = Box<dyn Fn(u32)>;

pub struct MessagesPage {
    pub root: adw::NavigationSplitView,
    host: Host,
    on_unread: Unread,
    list: gtk::ListBox,
    list_stack: gtk::Stack,
    list_error: adw::StatusPage,
    requests_tab: gtk::ToggleButton,
    showing_requests: Cell<bool>,
    accepted: RefCell<Vec<Convo>>,
    requests: RefCell<Vec<Convo>>,
    /// Conversations affichées dans la liste, dans l'ordre des rangées.
    shown: RefCell<Vec<Convo>>,
    title: adw::WindowTitle,
    convo_stack: gtk::Stack,
    bubbles: gtk::Box,
    scroll: gtk::ScrolledWindow,
    banner: adw::Banner,
    entry: gtk::Entry,
    send: gtk::Button,
    current: RefCell<Option<Convo>>,
    /// Messages affichés, du plus ancien au plus récent.
    messages: RefCell<Vec<Message>>,
    /// Curseur des messages plus anciens, s'il en reste.
    older: RefCell<Option<String>>,
    older_button: gtk::Button,
    /// La page elle-même, pour les rappels des bulles.
    this: RefCell<std::rc::Weak<Self>>,
    /// Numéro du prochain message envoyé (identifiant local).
    sent: Cell<u64>,
    /// Numéro du dernier chargement de messages, pour ignorer les réponses
    /// d'une conversation quittée entre-temps.
    generation: Cell<u64>,
}

fn spinner() -> gtk::Spinner {
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    spinner.update_property(&[gtk::accessible::Property::Label(&gettext("Chargement"))]);
    spinner
}

/// Rangée d'une conversation : avatar, nom, heure, dernier message,
/// pastille des messages non lus.
fn convo_row(convo: &Convo) -> gtk::ListBoxRow {
    let avatar = adw::Avatar::builder()
        .size(40)
        .text(&convo.title)
        .show_initials(true)
        .build();
    if let Some(url) = &convo.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    let title = gtk::Label::builder()
        .label(&convo.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    let time = gtk::Label::builder()
        .label(&convo.last_time)
        .css_classes(["dim-label", "caption"])
        .build();
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    top.append(&title);
    top.append(&time);
    let last = gtk::Label::builder()
        .label(&convo.last_text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    if convo.unread == 0 {
        last.add_css_class("dim-label");
    }
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bottom.append(&last);
    if convo.unread > 0 {
        bottom.append(
            &gtk::Label::builder()
                .label(convo.unread.to_string())
                .valign(gtk::Align::Center)
                .css_classes(["sidebar-badge"])
                .build(),
        );
    }
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    text.append(&top);
    text.append(&bottom);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    content.append(&avatar);
    content.append(&text);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    let mut description = format!("{}, {}", convo.title, convo.last_text);
    if convo.unread > 0 {
        description.push_str(", ");
        description.push_str(
            &ngettext("{} message non lu", "{} messages non lus", convo.unread)
                .replace("{}", &convo.unread.to_string()),
        );
    }
    row.update_property(&[gtk::accessible::Property::Label(&description)]);
    row
}

/// Bulle d'un message, à droite pour les miens, à gauche pour les autres.
/// Secondes entre deux actualisations, page Messages affichée.
const REFRESH_SECONDS: u32 = 15;

/// Réagir à un message : identifiant, émoji, ajouter (ou retirer).
type React = Rc<dyn Fn(&str, &str, bool)>;

/// Nom accessible d'une réaction : « 👍, 2 réactions, dont la vôtre ».
fn reaction_label(reaction: &chat::Reaction) -> String {
    let count = ngettext("{} réaction", "{} réactions", reaction.count)
        .replace("{}", &reaction.count.to_string());
    if reaction.mine {
        format!("{}, {count}, {}", reaction.value, gettext("dont la vôtre"))
    } else {
        format!("{}, {count}", reaction.value)
    }
}

/// Bouton « Réagir » : un menu des émojis proposés.
fn react_button(message_id: &str, react: &React) -> gtk::MenuButton {
    let choices = gtk::Box::builder().spacing(2).build();
    let popover = gtk::Popover::builder().child(&choices).build();
    for value in chat::REACTIONS {
        let choice = gtk::Button::builder()
            .label(value)
            .tooltip_text(value)
            .css_classes(["flat", "reaction-choice"])
            .build();
        let (react, id, popover) = (react.clone(), message_id.to_owned(), popover.downgrade());
        choice.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
            react(&id, value, true);
        });
        choices.append(&choice);
    }
    let button = gtk::MenuButton::builder()
        .icon_name("face-smile-symbolic")
        .tooltip_text(gettext("Réagir"))
        .valign(gtk::Align::Center)
        .popover(&popover)
        .css_classes(["flat", "circular", "dim-label"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&gettext("Réagir"))]);
    button
}

fn bubble(message: &Message, react: Option<&React>) -> gtk::Box {
    let label = gtk::Label::builder()
        .label(&message.text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .max_width_chars(48)
        .selectable(true)
        .tooltip_text(&message.time)
        .css_classes(["bubble", if message.mine { "mine" } else { "theirs" }])
        .build();
    if message.failed {
        label.add_css_class("failed");
        label.set_tooltip_text(Some(&gettext("Message non envoyé")));
    }
    let who = if message.mine {
        gettext("Vous, {} : ")
    } else {
        gettext("{} : ")
    }
    .replace("{}", &message.time);
    label.update_property(&[gtk::accessible::Property::Label(&format!(
        "{who}{}",
        message.text
    ))]);
    let align = if message.mine {
        gtk::Align::End
    } else {
        gtk::Align::Start
    };
    let line = gtk::Box::builder().spacing(4).halign(align).build();
    let react = react.filter(|_| !message.failed && !message.id.starts_with(chat::LOCAL_ID));
    let button = react.map(|react| react_button(&message.id, react));
    if let (true, Some(button)) = (message.mine, &button) {
        line.append(button);
    }
    line.append(&label);
    if let (false, Some(button)) = (message.mine, &button) {
        line.append(button);
    }
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .halign(align)
        .build();
    column.append(&line);
    if !message.reactions.is_empty() {
        let pills = gtk::Box::builder().spacing(4).halign(align).build();
        for reaction in &message.reactions {
            let pill = gtk::Button::builder()
                .label(format!("{} {}", reaction.value, reaction.count))
                .tooltip_text(reaction_label(reaction))
                .css_classes(["reaction"])
                .sensitive(react.is_some())
                .build();
            if reaction.mine {
                pill.add_css_class("mine");
            }
            pill.update_property(&[gtk::accessible::Property::Label(&reaction_label(reaction))]);
            if let Some(react) = react {
                // Cliquer sa réaction la retire ; celle des autres s'y ajoute.
                let (react, id, value, mine) = (
                    react.clone(),
                    message.id.clone(),
                    reaction.value.clone(),
                    reaction.mine,
                );
                pill.connect_clicked(move |_| react(&id, &value, !mine));
            }
            pills.append(&pill);
        }
        column.append(&pills);
    }
    column
}

fn day_separator(day: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(day)
        .margin_top(6)
        .css_classes(["dim-label", "caption-heading"])
        .build()
}

impl MessagesPage {
    pub fn new(host: Host, on_unread: impl Fn(u32) + 'static) -> Rc<Self> {
        // Liste des conversations.
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(gettext("Actualiser"))
            .build();
        refresh.update_property(&[gtk::accessible::Property::Label(&gettext("Actualiser"))]);
        let list_header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(&gettext("Messages"), ""))
            .build();
        list_header.pack_start(&refresh);
        let new_message = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(gettext("Nouveau message"))
            .build();
        new_message.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Nouveau message",
        ))]);
        list_header.pack_end(&new_message);
        let accepted_tab = gtk::ToggleButton::builder()
            .label(gettext("Conversations"))
            .active(true)
            .hexpand(true)
            .build();
        let requests_tab = gtk::ToggleButton::builder()
            .label(gettext("Demandes"))
            .group(&accepted_tab)
            .hexpand(true)
            .build();
        let tabs = gtk::Box::builder()
            .css_classes(["linked"])
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        tabs.update_property(&[gtk::accessible::Property::Label(&gettext("Conversations"))]);
        tabs.append(&accepted_tab);
        tabs.append(&requests_tab);
        let list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&gettext("Conversations"))]);
        let retry = gtk::Button::builder()
            .label(gettext("Réessayer"))
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let list_error = adw::StatusPage::builder()
            .icon_name("network-error-symbolic")
            .title(gettext("Chargement impossible"))
            .child(&retry)
            .css_classes(["compact"])
            .build();
        let list_stack = gtk::Stack::builder().vexpand(true).build();
        list_stack.add_named(&spinner(), Some("loading"));
        list_stack.add_named(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list)
                .build(),
            Some("list"),
        );
        list_stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title(gettext("Aucune conversation"))
                .css_classes(["compact"])
                .build(),
            Some("empty"),
        );
        list_stack.add_named(&list_error, Some("error"));
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_box.append(&tabs);
        list_box.append(&list_stack);
        let list_view = adw::ToolbarView::builder().content(&list_box).build();
        list_view.add_top_bar(&list_header);
        let sidebar = adw::NavigationPage::builder()
            .title(gettext("Messages"))
            .child(&list_view)
            .build();

        // Conversation ouverte.
        let title = adw::WindowTitle::new("", "");
        let bubbles = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .valign(gtk::Align::End)
            .build();
        let older_button = gtk::Button::builder()
            .label(gettext("Messages plus anciens"))
            .halign(gtk::Align::Center)
            .margin_top(12)
            .visible(false)
            .css_classes(["pill"])
            .build();
        let thread = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::End)
            .build();
        thread.append(&older_button);
        thread.append(&bubbles);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(720)
                    .child(&thread)
                    .build(),
            )
            .build();
        let banner = adw::Banner::builder()
            .title(gettext(
                "Demande de message : cette personne ne fait pas partie de vos contacts.",
            ))
            .button_label(gettext("Accepter"))
            .build();
        let entry = gtk::Entry::builder()
            .placeholder_text(gettext("Écrire un message…"))
            .hexpand(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&gettext("Message"))]);
        let send = gtk::Button::builder()
            .icon_name("mail-send-symbolic")
            .tooltip_text(gettext("Envoyer"))
            .css_classes(["suggested-action", "circular"])
            .sensitive(false)
            .build();
        send.update_property(&[gtk::accessible::Property::Label(&gettext("Envoyer"))]);
        let composer = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        composer.append(&entry);
        composer.append(&send);
        let convo_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        convo_box.append(&banner);
        convo_box.append(&scroll);
        convo_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        convo_box.append(&composer);
        let convo_stack = gtk::Stack::new();
        convo_stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title(gettext("Aucune conversation ouverte"))
                .description(gettext(
                    "Choisissez une conversation pour lire ses messages.",
                ))
                .build(),
            Some("empty"),
        );
        convo_stack.add_named(&spinner(), Some("loading"));
        convo_stack.add_named(&convo_box, Some("convo"));
        let convo_view = adw::ToolbarView::builder().content(&convo_stack).build();
        convo_view.add_top_bar(&adw::HeaderBar::builder().title_widget(&title).build());
        let content = adw::NavigationPage::builder()
            .title(gettext("Conversation"))
            .child(&convo_view)
            .build();

        let root = adw::NavigationSplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(260.0)
            .max_sidebar_width(360.0)
            .build();

        let page = Rc::new(Self {
            root,
            host,
            on_unread: Box::new(on_unread),
            list,
            list_stack,
            list_error,
            requests_tab: requests_tab.clone(),
            showing_requests: Cell::new(false),
            accepted: RefCell::default(),
            requests: RefCell::default(),
            shown: RefCell::default(),
            title,
            convo_stack,
            bubbles,
            scroll,
            banner,
            entry,
            send,
            current: RefCell::default(),
            messages: RefCell::default(),
            older: RefCell::default(),
            older_button,
            sent: Cell::new(0),
            this: RefCell::default(),
            generation: Cell::new(0),
        });
        page.this.replace(Rc::downgrade(&page));
        let weak = Rc::downgrade(&page);
        new_message.connect_clicked(move |button| {
            if let Some(page) = weak.upgrade() {
                page.new_message_dialog(button);
            }
        });
        let weak = Rc::downgrade(&page);
        page.older_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.load_older();
            }
        });
        // Actualisation régulière, seulement quand la page est affichée.
        let weak = Rc::downgrade(&page);
        glib::timeout_add_seconds_local(REFRESH_SECONDS, move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if page.root.is_mapped() && page.host.session.is_some() {
                page.load_convos();
                page.refresh_messages();
            }
            glib::ControlFlow::Continue
        });

        for (requests, button) in [(false, accepted_tab), (true, requests_tab)] {
            let weak = Rc::downgrade(&page);
            button.connect_toggled(move |button| {
                if button.is_active()
                    && let Some(page) = weak.upgrade()
                {
                    page.showing_requests.set(requests);
                    page.fill_list();
                }
            });
        }
        for button in [refresh, retry] {
            let weak = Rc::downgrade(&page);
            button.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.load_convos();
                }
            });
        }
        let weak = Rc::downgrade(&page);
        page.list.connect_row_activated(move |_, row| {
            if let Some(page) = weak.upgrade() {
                let convo = page.shown.borrow().get(row.index() as usize).cloned();
                if let Some(convo) = convo {
                    page.open(convo);
                }
            }
        });
        let weak = Rc::downgrade(&page);
        page.entry.connect_changed(move |entry| {
            if let Some(page) = weak.upgrade() {
                page.send.set_sensitive(!entry.text().trim().is_empty());
            }
        });
        let weak = Rc::downgrade(&page);
        page.entry.connect_activate(move |_| {
            if let Some(page) = weak.upgrade() {
                page.send_message();
            }
        });
        let weak = Rc::downgrade(&page);
        page.send.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.send_message();
            }
        });
        let weak = Rc::downgrade(&page);
        page.banner.connect_button_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.accept();
            }
        });

        page.load_convos();
        page
    }

    fn load_convos(self: &Rc<Self>) {
        let Some(session) = self.host.session.clone() else {
            self.show_convos(Ok(chat::demo_convos()));
            return;
        };
        if self.accepted.borrow().is_empty() && self.requests.borrow().is_empty() {
            self.list_stack.set_visible_child_name("loading");
        }
        let task = runtime().spawn(chat::convos(session));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Err(error) if error.expired => (page.host.expired)(error.message),
                result => page.show_convos(result),
            }
        });
    }

    fn show_convos(&self, result: Result<(Vec<Convo>, Vec<Convo>), ApiError>) {
        match result {
            Ok((accepted, requests)) => {
                // Rien de neuf : la liste reste telle quelle (focus compris).
                if *self.accepted.borrow() == accepted
                    && *self.requests.borrow() == requests
                    && self.list_stack.visible_child_name().as_deref() == Some("list")
                {
                    return;
                }
                self.accepted.replace(accepted);
                self.requests.replace(requests);
                self.fill_list();
                self.report_unread();
            }
            Err(error) => {
                self.list_error.set_description(Some(&error.message));
                self.list_stack.set_visible_child_name("error");
            }
        }
    }

    fn fill_list(&self) {
        let convos = if self.showing_requests.get() {
            self.requests.borrow().clone()
        } else {
            self.accepted.borrow().clone()
        };
        let requests = self.requests.borrow().len();
        self.requests_tab.set_label(&if requests > 0 {
            gettext("Demandes · {}").replace("{}", &requests.to_string())
        } else {
            gettext("Demandes")
        });
        self.list.remove_all();
        for convo in &convos {
            self.list.append(&convo_row(convo));
        }
        self.list_stack
            .set_visible_child_name(if convos.is_empty() { "empty" } else { "list" });
        // Garder la conversation ouverte sélectionnée.
        let current = self.current.borrow().as_ref().map(|c| c.id.clone());
        if let Some(index) = current.and_then(|id| convos.iter().position(|c| c.id == id)) {
            self.list
                .select_row(self.list.row_at_index(index as i32).as_ref());
        }
        self.shown.replace(convos);
    }

    fn report_unread(&self) {
        let total: u32 = self
            .accepted
            .borrow()
            .iter()
            .chain(self.requests.borrow().iter())
            .map(|c| c.unread)
            .sum();
        (self.on_unread)(total);
    }

    /// Ouvre une conversation : messages, puis lecture marquée.
    fn open(self: &Rc<Self>, convo: Convo) {
        self.title.set_title(&convo.title);
        self.title.set_subtitle(
            &convo
                .handle
                .as_ref()
                .map(|h| format!("@{h}"))
                .unwrap_or_default(),
        );
        self.banner.set_revealed(convo.request);
        self.current.replace(Some(convo.clone()));
        self.root.set_show_content(true);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.mark_read(&convo);

        let Some(session) = self.host.session.clone() else {
            self.show_messages(Ok((chat::demo_messages(&convo.id), None)));
            return;
        };
        self.convo_stack.set_visible_child_name("loading");
        let task = runtime().spawn(chat::messages(session, convo.id.clone(), None));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            if page.generation.get() != generation {
                return;
            }
            match result {
                Err(error) if error.expired => (page.host.expired)(error.message),
                result => page.show_messages(result),
            }
        });
    }

    fn show_messages(&self, result: Result<MessagePage, ApiError>) {
        match result {
            Ok((messages, cursor)) => {
                self.messages.replace(messages);
                self.older.replace(cursor);
                self.render();
            }
            Err(error) => {
                self.messages.replace(Vec::new());
                self.older.replace(None);
                self.render();
                self.bubbles.append(&day_separator(&error.message));
            }
        }
        self.convo_stack.set_visible_child_name("convo");
        self.scroll_to_end();
        self.entry.grab_focus();
    }

    /// Bulles des messages affichés, séparées par jour.
    fn render(&self) {
        while let Some(child) = self.bubbles.first_child() {
            self.bubbles.remove(&child);
        }
        let mut day = String::new();
        for message in self.messages.borrow().iter() {
            if message.day != day {
                day = message.day.clone();
                self.bubbles.append(&day_separator(&day));
            }
            self.bubbles
                .append(&bubble(message, Some(&self.react_handler())));
        }
        self.older_button.set_visible(self.older.borrow().is_some());
    }

    fn react_handler(&self) -> React {
        let weak = self.this.borrow().clone();
        Rc::new(move |id: &str, value: &str, add: bool| {
            if let Some(page) = weak.upgrade() {
                page.react(id, value, add);
            }
        })
    }

    /// Ajoute ou retire ma réaction : affichée tout de suite, puis
    /// remplacée par l'état du service.
    fn react(self: &Rc<Self>, id: &str, value: &str, add: bool) {
        {
            let mut messages = self.messages.borrow_mut();
            let Some(message) = messages.iter_mut().find(|m| m.id == id) else {
                return;
            };
            let reactions = &mut message.reactions;
            match reactions.iter_mut().position(|r| r.value == value) {
                Some(i) if add && !reactions[i].mine => {
                    reactions[i].count += 1;
                    reactions[i].mine = true;
                }
                Some(_) if add => return,
                Some(i) if reactions[i].mine => {
                    reactions[i].count -= 1;
                    reactions[i].mine = false;
                    if reactions[i].count == 0 {
                        reactions.remove(i);
                    }
                }
                Some(_) => return,
                None if add => reactions.push(chat::Reaction {
                    value: value.to_owned(),
                    count: 1,
                    mine: true,
                }),
                None => return,
            }
        }
        self.render();
        // Les bulles sont refaites : le focus revient au champ de saisie.
        self.entry.grab_focus();
        let (Some(session), Some(convo)) =
            (self.host.session.clone(), self.current.borrow().clone())
        else {
            return;
        };
        let task = runtime().spawn(chat::react(
            session,
            convo.id,
            id.to_owned(),
            value.to_owned(),
            add,
        ));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(updated) => {
                    if let Some(m) = page
                        .messages
                        .borrow_mut()
                        .iter_mut()
                        .find(|m| m.id == updated.id)
                    {
                        *m = updated;
                    }
                    page.render();
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => {
                    page.host.toasts.add_toast(adw::Toast::new(&error.message));
                    page.refresh_messages();
                }
            }
        });
    }

    /// Messages d'avant ceux affichés, sans bouger ce qu'on lit.
    fn load_older(self: &Rc<Self>) {
        let (Some(session), Some(convo), Some(cursor)) = (
            self.host.session.clone(),
            self.current.borrow().clone(),
            self.older.borrow().clone(),
        ) else {
            return;
        };
        self.older_button.set_sensitive(false);
        let generation = self.generation.get();
        let task = runtime().spawn(chat::messages(session, convo.id, Some(cursor)));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            page.older_button.set_sensitive(true);
            if page.generation.get() != generation {
                return;
            }
            match result {
                Ok((older, cursor)) => {
                    let adjustment = page.scroll.vadjustment();
                    let from_end = adjustment.upper() - adjustment.value();
                    {
                        let mut messages = page.messages.borrow_mut();
                        let shown = std::mem::take(&mut *messages);
                        *messages = older.into_iter().chain(shown).collect();
                    }
                    page.older.replace(cursor);
                    page.render();
                    // Même distance à la fin qu'avant : le message lu reste en place.
                    glib::idle_add_local_once(move || {
                        adjustment.set_value(adjustment.upper() - from_end);
                    });
                }
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    /// Nouveaux messages de la conversation ouverte (actualisation).
    fn refresh_messages(self: &Rc<Self>) {
        let (Some(session), Some(convo)) =
            (self.host.session.clone(), self.current.borrow().clone())
        else {
            return;
        };
        if self.convo_stack.visible_child_name().as_deref() != Some("convo") {
            return;
        }
        let generation = self.generation.get();
        let task = runtime().spawn(chat::messages(session, convo.id.clone(), None));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(Ok((latest, _))) = task.await else {
                return;
            };
            let Some(page) = weak.upgrade() else { return };
            if page.generation.get() != generation {
                return;
            }
            let merged = chat::merge(&page.messages.borrow(), latest);
            if *page.messages.borrow() == merged {
                return;
            }
            let adjustment = page.scroll.vadjustment();
            let at_end = adjustment.value() + adjustment.page_size() >= adjustment.upper() - 24.0;
            page.messages.replace(merged);
            page.render();
            if at_end {
                page.scroll_to_end();
            }
        });
    }

    fn scroll_to_end(&self) {
        let scroll = self.scroll.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(scroll) = scroll.upgrade() {
                let adjustment = scroll.vadjustment();
                adjustment.set_value(adjustment.upper());
            }
        });
    }

    /// Conversation ouverte : plus de non-lus, ici et sur Bluesky.
    fn mark_read(&self, convo: &Convo) {
        if convo.unread == 0 {
            return;
        }
        for list in [&self.accepted, &self.requests] {
            for c in list.borrow_mut().iter_mut().filter(|c| c.id == convo.id) {
                c.unread = 0;
            }
        }
        self.fill_list();
        self.report_unread();
        if let Some(session) = self.host.session.clone() {
            let id = convo.id.clone();
            runtime().spawn(async move {
                let _ = chat::mark_read(session, id).await;
            });
        }
    }

    fn send_message(self: &Rc<Self>) {
        let text = self.entry.text().trim().to_owned();
        let Some(convo) = self.current.borrow().clone() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.entry.set_text("");
        // Affiché tout de suite ; marqué si l'envoi échoue.
        let now = chrono::Local::now();
        self.sent.set(self.sent.get() + 1);
        let local_id = format!("{}{}", chat::LOCAL_ID, self.sent.get());
        let pending = Message {
            id: local_id.clone(),
            text: text.clone(),
            mine: true,
            sender: String::new(),
            time: now.format("%H:%M").to_string(),
            day: gettext("Aujourd'hui"),
            failed: false,
            reactions: Vec::new(),
        };
        self.messages.borrow_mut().push(pending.clone());
        self.render();
        self.scroll_to_end();
        // La conversation remonte en tête, avec ce dernier message.
        {
            let mut accepted = self.accepted.borrow_mut();
            if let Some(index) = accepted.iter().position(|c| c.id == convo.id) {
                let mut updated = accepted.remove(index);
                updated.last_text = text.replace('\n', " ");
                updated.last_time = pending.time.clone();
                accepted.insert(0, updated);
            }
        }
        if !self.showing_requests.get() {
            self.fill_list();
        }
        let Some(session) = self.host.session.clone() else {
            return;
        };
        let task = runtime().spawn(chat::send(session, convo.id, text));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Envoi interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(sent) => {
                    // Le message local devient celui du service (sauf si
                    // une actualisation l'a déjà apporté).
                    let mut messages = page.messages.borrow_mut();
                    let known = messages.iter().any(|m| m.id == sent.id);
                    messages.retain(|m| m.id != local_id || !known);
                    if let Some(m) = messages.iter_mut().find(|m| m.id == local_id) {
                        *m = sent;
                    }
                }
                Err(error) => {
                    if error.expired {
                        (page.host.expired)(error.message);
                        return;
                    }
                    if let Some(m) = page
                        .messages
                        .borrow_mut()
                        .iter_mut()
                        .find(|m| m.id == local_id)
                    {
                        m.failed = true;
                    }
                    page.render();
                    glib::g_debug!("tsubame", "message : {}", error.message);
                    page.host
                        .toasts
                        .add_toast(adw::Toast::new(&gettext("Message non envoyé.")));
                }
            }
        });
    }

    /// Ouvre la conversation avec un compte (DID), créée au besoin ;
    /// `name` sert au titre en démonstration.
    pub fn start_with(self: &Rc<Self>, did: &str, name: &str) {
        let Some(session) = self.host.session.clone() else {
            let convo = Convo {
                id: format!("demo:{did}"),
                title: name.to_owned(),
                handle: None,
                avatar: None,
                last_text: String::new(),
                last_time: String::new(),
                unread: 0,
                request: false,
            };
            self.add_and_open(convo);
            return;
        };
        let task = runtime().spawn(chat::convo_for(session, did.to_owned()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(page) = weak.upgrade() else { return };
            match result {
                Ok(convo) => page.add_and_open(convo),
                Err(error) if error.expired => (page.host.expired)(error.message),
                Err(error) => page.host.toasts.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    fn add_and_open(self: &Rc<Self>, convo: Convo) {
        let known = self
            .accepted
            .borrow()
            .iter()
            .chain(self.requests.borrow().iter())
            .any(|c| c.id == convo.id);
        if !known {
            self.accepted.borrow_mut().insert(0, convo.clone());
        }
        if convo.request {
            self.requests_tab.set_active(true);
        }
        self.open(convo);
        // Après `open` : la rangée de la conversation ouverte est choisie.
        self.fill_list();
    }

    /// Boîte « Nouveau message » : chercher un compte, puis ouvrir la
    /// conversation avec lui.
    fn new_message_dialog(self: &Rc<Self>, parent: &gtk::Button) {
        let dialog = adw::Dialog::builder()
            .title(gettext("Nouveau message"))
            .content_width(420)
            .content_height(480)
            .build();
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Rechercher un compte"))
            .search_delay(300)
            .hexpand(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Rechercher un compte",
        ))]);
        let results = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .valign(gtk::Align::Start)
            .css_classes(["boxed-list"])
            .visible(false)
            .build();
        results.update_property(&[gtk::accessible::Property::Label(&gettext("Comptes"))]);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&entry);
        content.append(&results);
        let view = adw::ToolbarView::builder()
            .content(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .child(&content)
                    .build(),
            )
            .build();
        view.add_top_bar(&adw::HeaderBar::new());
        dialog.set_child(Some(&view));

        let found: Rc<RefCell<Vec<crate::data::Actor>>> = Rc::default();
        let show = {
            let (results, found) = (results.clone(), found.clone());
            move |actors: Vec<crate::data::Actor>| {
                results.remove_all();
                for actor in &actors {
                    let row = adw::ActionRow::builder()
                        .title(&actor.name)
                        .subtitle(format!("@{}", actor.handle))
                        .use_markup(false)
                        .activatable(true)
                        .build();
                    let avatar = adw::Avatar::builder()
                        .size(32)
                        .text(&actor.name)
                        .show_initials(true)
                        .accessible_role(gtk::AccessibleRole::Presentation)
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
                    results.append(&row);
                }
                results.set_visible(!actors.is_empty());
                found.replace(actors);
            }
        };
        let show = Rc::new(show);
        let session = self.host.session.clone();
        entry.connect_search_changed(move |entry| {
            let query = entry.text().trim().to_owned();
            if query.is_empty() {
                show(Vec::new());
                return;
            }
            let Some(session) = session.clone() else {
                let lower = query.to_lowercase();
                show(
                    crate::data::demo_actors()
                        .into_iter()
                        .filter(|a| {
                            a.name.to_lowercase().contains(&lower) || a.handle.contains(&lower)
                        })
                        .collect(),
                );
                return;
            };
            let task = runtime().spawn(crate::api::typeahead(session, query.clone()));
            let (show, entry) = (show.clone(), entry.downgrade());
            glib::spawn_future_local(async move {
                let Ok(Ok(actors)) = task.await else { return };
                // Une frappe plus récente l'emporte.
                if entry.upgrade().map(|e| e.text().trim() == query) == Some(true) {
                    show(actors);
                }
            });
        });
        let weak = Rc::downgrade(self);
        let closing = dialog.downgrade();
        results.connect_row_activated(move |_, row| {
            let actor = found.borrow().get(row.index() as usize).cloned();
            let (Some(page), Some(actor)) = (weak.upgrade(), actor) else {
                return;
            };
            if let Some(dialog) = closing.upgrade() {
                dialog.close();
            }
            page.start_with(&actor.did, &actor.name);
        });
        dialog.present(Some(parent));
        entry.grab_focus();
    }

    /// Accepter une demande : elle rejoint les conversations.
    fn accept(self: &Rc<Self>) {
        let Some(convo) = self.current.borrow().clone() else {
            return;
        };
        self.banner.set_revealed(false);
        let moved = {
            let mut requests = self.requests.borrow_mut();
            let index = requests.iter().position(|c| c.id == convo.id);
            index.map(|i| requests.remove(i))
        };
        if let Some(mut moved) = moved {
            moved.request = false;
            self.accepted.borrow_mut().insert(0, moved);
        }
        self.fill_list();
        if let Some(session) = self.host.session.clone() {
            runtime().spawn(async move {
                let _ = chat::accept(session, convo.id).await;
            });
        }
    }
}
