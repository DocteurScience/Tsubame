//! Écran de connexion : identifiant Bluesky, puis confirmation dans le
//! navigateur (OAuth). Voir `auth` pour le déroulé côté protocole.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::auth::{self, LoginError, Session};
use crate::runtime::runtime;

type LoggedIn = Box<dyn Fn(Option<Arc<Session>>)>;

pub struct LoginPage {
    pub root: adw::ToolbarView,
    stack: gtk::Stack,
    entry: adw::EntryRow,
    connect: gtk::Button,
    error: gtk::Label,
    waiting_label: gtk::Label,
    reopen: gtk::Button,
    url: RefCell<Option<String>>,
    task: RefCell<Option<tokio::task::AbortHandle>>,
    on_logged_in: RefCell<Option<LoggedIn>>,
}

fn centered(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(400)
                .valign(gtk::Align::Center)
                .margin_top(24)
                .margin_bottom(24)
                .margin_start(12)
                .margin_end(12)
                .child(child)
                .build(),
        )
        .build()
}

fn spinner_box(label: &gtk::Label) -> gtk::Box {
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&spinner);
    content.append(label);
    content
}

impl LoginPage {
    pub fn new() -> Rc<Self> {
        // Formulaire.
        let icon = gtk::Image::builder()
            .icon_name(crate::APP_ID)
            .pixel_size(128)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let title = gtk::Label::builder()
            .label(gettext("Bienvenue dans Tsubame"))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["title-1"])
            .build();
        let intro = gtk::Label::builder()
            .label(gettext("Connectez-vous avec votre compte Bluesky."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let entry = adw::EntryRow::builder()
            .title(gettext("Identifiant"))
            .input_purpose(gtk::InputPurpose::Url)
            .build();
        let entries = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        entries.append(&entry);
        let hint = gtk::Label::builder()
            .label(gettext(
                "Par exemple : vous.bsky.social. Votre navigateur s'ouvrira pour confirmer la connexion ; Tsubame ne voit jamais votre mot de passe.",
            ))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let error = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .visible(false)
            .accessible_role(gtk::AccessibleRole::Alert)
            .css_classes(["error"])
            .build();
        let connect = gtk::Button::builder()
            .label(gettext("Se connecter"))
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();

        let form = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .build();
        form.append(&icon);
        form.append(&title);
        form.append(&intro);
        let fields = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .build();
        fields.append(&entries);
        fields.append(&hint);
        fields.append(&error);
        form.append(&fields);
        connect.set_margin_top(12);
        form.append(&connect);
        // Les builds de développement peuvent continuer sans compte : le
        // réseau n'est pas encore branché sur les fils.
        let demo = gtk::Button::builder()
            .label(gettext("Continuer avec les posts de démonstration"))
            .halign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(cfg!(debug_assertions))
            .build();
        form.append(&demo);

        // Attente du navigateur.
        let waiting_label = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let reopen = gtk::Button::builder()
            .label(gettext("Rouvrir le navigateur"))
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let cancel = gtk::Button::builder()
            .label(gettext("Annuler"))
            .halign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let waiting = spinner_box(&waiting_label);
        waiting.append(&reopen);
        waiting.append(&cancel);

        // Vérification du trousseau au lancement.
        let checking = spinner_box(&gtk::Label::new(Some(&gettext("Ouverture de la session…"))));

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&centered(&checking), Some("checking"));
        stack.add_named(&centered(&form), Some("form"));
        stack.add_named(&centered(&waiting), Some("waiting"));

        let menu = gio::Menu::new();
        menu.append(Some(&gettext("À propos de Tsubame")), Some("app.about"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text(gettext("Menu principal"))
            .build();
        menu_button
            .update_property(&[gtk::accessible::Property::Label(&gettext("Menu principal"))]);
        let header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new("", ""))
            .css_classes(["flat"])
            .build();
        header.pack_end(&menu_button);
        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(&header);

        let page = Rc::new(Self {
            root,
            stack,
            entry,
            connect,
            error,
            waiting_label,
            reopen,
            url: RefCell::default(),
            task: RefCell::default(),
            on_logged_in: RefCell::default(),
        });

        let weak = Rc::downgrade(&page);
        page.connect
            .connect_clicked(move |_| with(&weak, |p| p.start()));
        let weak = Rc::downgrade(&page);
        page.entry
            .connect_entry_activated(move |_| with(&weak, |p| p.start()));
        let weak = Rc::downgrade(&page);
        page.entry
            .connect_changed(move |_| with(&weak, |p| p.update_sensitivity()));
        let weak = Rc::downgrade(&page);
        page.reopen
            .connect_clicked(move |_| with(&weak, |p| p.open_browser()));
        let weak = Rc::downgrade(&page);
        cancel.connect_clicked(move |_| with(&weak, |p| p.cancel()));
        let weak = Rc::downgrade(&page);
        demo.connect_clicked(move |_| with(&weak, |p| p.logged_in(None)));
        page.update_sensitivity();
        page
    }

    /// Appelle `f` une fois connecté, avec la session, ou `None` pour les
    /// posts de démonstration.
    pub fn connect_logged_in(&self, f: impl Fn(Option<Arc<Session>>) + 'static) {
        self.on_logged_in.replace(Some(Box::new(f)));
    }

    /// Pré-remplit l'identifiant, par exemple après une session expirée.
    pub fn set_identifier(&self, identifier: &str) {
        self.entry.set_text(identifier);
    }

    /// Montre l'attente pendant la lecture du trousseau au lancement.
    pub fn show_checking(&self) {
        self.stack.set_visible_child_name("checking");
    }

    /// Montre le formulaire, avec un message éventuel (session expirée…).
    pub fn show_form(&self, message: Option<&str>) {
        self.cancel_task();
        self.error.set_label(message.unwrap_or_default());
        self.error.set_visible(message.is_some());
        self.stack.set_visible_child_name("form");
        self.entry.grab_focus();
    }

    fn update_sensitivity(&self) {
        self.connect
            .set_sensitive(!self.entry.text().trim().is_empty());
    }

    fn cancel_task(&self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    fn cancel(&self) {
        self.show_form(None);
    }

    fn show_error(&self, error: &LoginError) {
        self.show_form(Some(&error.message()));
    }

    fn logged_in(&self, session: Option<Arc<Session>>) {
        self.cancel_task();
        if let Some(f) = self.on_logged_in.borrow().as_ref() {
            f(session);
        }
    }

    fn start(self: &Rc<Self>) {
        let identifier = self.entry.text().to_string();
        if identifier.trim().is_empty() {
            return;
        }
        // Erreur de saisie : inutile d'aller sur le réseau.
        if let Err(error) = auth::normalize_identifier(&identifier) {
            self.show_error(&error);
            return;
        }
        self.cancel_task();
        self.url.replace(None);
        self.reopen.set_visible(false);
        self.waiting_label
            .set_label(&gettext("Recherche de votre compte…"));
        self.stack.set_visible_child_name("waiting");

        let handle = runtime().spawn(auth::start(identifier));
        self.task.replace(Some(handle.abort_handle()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = handle.await;
            let Some(page) = weak.upgrade() else { return };
            match flatten(result) {
                Some(Ok(pending)) => page.wait_for_browser(pending),
                Some(Err(error)) => page.show_error(&error),
                None => {} // Annulé.
            }
        });
    }

    fn wait_for_browser(self: &Rc<Self>, pending: auth::PendingLogin) {
        self.url.replace(Some(pending.url.clone()));
        self.waiting_label
            .set_label(&gettext("Confirmez la connexion dans votre navigateur."));
        self.reopen.set_visible(true);
        self.open_browser();

        let handle = runtime().spawn(pending.finish());
        self.task.replace(Some(handle.abort_handle()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = handle.await;
            let Some(page) = weak.upgrade() else { return };
            match flatten(result) {
                Some(Ok(session)) => page.logged_in(Some(session)),
                Some(Err(error)) => page.show_error(&error),
                None => {}
            }
        });
    }

    fn open_browser(&self) {
        let Some(url) = self.url.borrow().clone() else {
            return;
        };
        let window = self.root.root().and_downcast::<gtk::Window>();
        let weak = self.waiting_label.downgrade();
        gtk::UriLauncher::new(&url).launch(
            window.as_ref(),
            gio::Cancellable::NONE,
            move |result| {
                if let (Err(error), Some(label)) = (result, weak.upgrade()) {
                    label.set_label(
                        &gettext("Impossible d'ouvrir le navigateur : {}")
                            .replace("{}", error.message()),
                    );
                }
            },
        );
    }
}

fn with(page: &Weak<LoginPage>, f: impl FnOnce(&Rc<LoginPage>)) {
    if let Some(page) = page.upgrade() {
        f(&page);
    }
}

/// Résultat d'une tâche tokio : `None` si elle a été annulée. Une panique
/// dans `atrium-oauth` (il en reste, par exemple sur un échange de code
/// refusé) devient une erreur de connexion.
fn flatten<T>(
    result: Result<Result<T, LoginError>, tokio::task::JoinError>,
) -> Option<Result<T, LoginError>> {
    match result {
        Ok(result) => Some(result),
        Err(error) if error.is_cancelled() => None,
        Err(_) => Some(Err(LoginError::Failed(gettext("erreur inattendue")))),
    }
}
