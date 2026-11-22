//! Préférences (maquette `docs/mockups/source/Reglages.dc.html`), page
//! « Modération » : comptes masqués et bloqués, mots masqués, contenu
//! sensible, services d'étiquetage.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use serde_json::Value;

use crate::api::{self, ApiError};
use crate::auth::Session;
use crate::data::Actor;
use crate::moderation::{self, LABELS, Visibility};
use crate::runtime::runtime;

/// Changement de préférence, appliqué à la copie locale puis envoyé.
#[derive(Debug, Clone)]
enum Change {
    Adult(bool),
    Label(&'static str, Visibility),
    AddWord(String),
    RemoveWord(String),
}

impl Change {
    fn apply(&self, preferences: &mut Vec<Value>) {
        match self {
            Change::Adult(enabled) => moderation::set_adult_content(preferences, *enabled),
            Change::Label(label, visibility) => {
                moderation::set_label_visibility(preferences, label, *visibility)
            }
            Change::AddWord(word) => moderation::add_muted_word(preferences, word),
            Change::RemoveWord(word) => moderation::remove_muted_word(preferences, word),
        }
    }
}

struct Dialog {
    dialog: adw::PreferencesDialog,
    session: Option<Arc<Session>>,
    preferences: RefCell<Vec<Value>>,
    /// Préférences changées : les fils doivent être refiltrés.
    on_changed: Rc<dyn Fn()>,
    muted_count: gtk::Label,
    blocked_count: gtk::Label,
    words_count: gtk::Label,
    adult: adw::SwitchRow,
    labels: Vec<(&'static str, adw::ComboRow)>,
    labelers: adw::PreferencesGroup,
    rows: Vec<adw::ActionRow>,
    /// Le dialogue ne réagit pas à ses propres mises à jour.
    filling: std::cell::Cell<bool>,
}

fn count_label() -> gtk::Label {
    gtk::Label::builder()
        .css_classes(["dim-label", "numeric"])
        .valign(gtk::Align::Center)
        .build()
}

fn link_row(title: &str, count: &gtk::Label) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .sensitive(false)
        .build();
    row.add_suffix(count);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

fn subpage(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let view = adw::ToolbarView::builder().content(content).build();
    view.add_top_bar(&adw::HeaderBar::new());
    adw::NavigationPage::builder()
        .title(title)
        .child(&view)
        .build()
}

pub fn present(
    parent: &impl IsA<gtk::Widget>,
    session: Option<Arc<Session>>,
    on_changed: impl Fn() + 'static,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title(gettext("Préférences"))
        .search_enabled(true)
        .build();
    let page = adw::PreferencesPage::builder()
        .title(gettext("Modération"))
        .icon_name("security-medium-symbolic")
        .build();

    // Listes et comptes.
    let accounts = adw::PreferencesGroup::builder()
        .title(gettext("Listes et comptes"))
        .build();
    let (muted_count, blocked_count, words_count) = (count_label(), count_label(), count_label());
    let muted_row = link_row(&gettext("Comptes masqués"), &muted_count);
    let blocked_row = link_row(&gettext("Comptes bloqués"), &blocked_count);
    let words_row = link_row(&gettext("Mots et mots-clics masqués"), &words_count);
    accounts.add(&muted_row);
    accounts.add(&blocked_row);
    accounts.add(&words_row);
    page.add(&accounts);

    // Contenu sensible.
    let sensitive = adw::PreferencesGroup::builder()
        .title(gettext("Contenu sensible"))
        .description(gettext(
            "Chaque étiquette peut être affichée, floutée avec un avertissement, ou masquée.",
        ))
        .build();
    let adult = adw::SwitchRow::builder()
        .title(gettext("Activer le contenu pour adultes"))
        .sensitive(false)
        .build();
    sensitive.add(&adult);
    let choices = gtk::StringList::new(&[]);
    for visibility in Visibility::ALL {
        choices.append(&visibility.title());
    }
    let mut labels = Vec::new();
    for (label, _) in LABELS {
        let row = adw::ComboRow::builder()
            .title(moderation::label_title(label))
            .model(&choices)
            .sensitive(false)
            .build();
        sensitive.add(&row);
        labels.push((label, row));
    }
    page.add(&sensitive);

    // Services d'étiquetage.
    let labelers = adw::PreferencesGroup::builder()
        .title(gettext("Services d'étiquetage"))
        .description(gettext(
            "Des services tiers ajoutent leurs propres étiquettes à vos fils et profils.",
        ))
        .build();
    let bluesky = adw::ActionRow::builder()
        .title(gettext("Modération Bluesky"))
        .subtitle(gettext("Service par défaut · activé"))
        .build();
    labelers.add(&bluesky);
    page.add(&labelers);
    dialog.add(&page);

    let this = Rc::new(Dialog {
        dialog: dialog.clone(),
        session,
        preferences: RefCell::default(),
        on_changed: Rc::new(on_changed),
        muted_count,
        blocked_count,
        words_count,
        adult,
        labels,
        labelers,
        rows: vec![muted_row.clone(), blocked_row.clone(), words_row.clone()],
        filling: std::cell::Cell::new(false),
    });

    let weak = Rc::downgrade(&this);
    this.adult.connect_active_notify(move |row| {
        if let Some(this) = weak.upgrade() {
            this.update_adult_rows();
            this.change(Change::Adult(row.is_active()));
        }
    });
    for (label, row) in &this.labels {
        let (label, weak) = (*label, Rc::downgrade(&this));
        row.connect_selected_notify(move |row| {
            if let Some(this) = weak.upgrade()
                && let Some(visibility) = Visibility::ALL.get(row.selected() as usize)
            {
                this.change(Change::Label(label, *visibility));
            }
        });
    }
    let weak = Rc::downgrade(&this);
    muted_row.connect_activated(move |_| {
        if let Some(this) = weak.upgrade() {
            this.show_accounts(false);
        }
    });
    let weak = Rc::downgrade(&this);
    blocked_row.connect_activated(move |_| {
        if let Some(this) = weak.upgrade() {
            this.show_accounts(true);
        }
    });
    let weak = Rc::downgrade(&this);
    words_row.connect_activated(move |_| {
        if let Some(this) = weak.upgrade() {
            this.show_words();
        }
    });
    // Le dialogue garde son état en vie.
    let keep = this.clone();
    dialog.connect_closed(move |_| {
        let _ = &keep;
    });

    this.load();
    dialog.present(Some(parent));
}

impl Dialog {
    fn load(self: &Rc<Self>) {
        let Some(session) = self.session.clone() else {
            self.fill(Vec::new(), 0, 0, Vec::new());
            return;
        };
        let task = runtime().spawn(async move {
            let preferences = moderation::preferences(session.clone()).await?;
            let mutes = moderation::mutes(session.clone()).await?.len();
            let blocks = moderation::blocks(session.clone()).await?.len();
            let names = moderation::labeler_names(session, moderation::labelers(&preferences))
                .await
                .unwrap_or_default();
            Ok::<_, ApiError>((preferences, mutes, blocks, names))
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok((preferences, mutes, blocks, names)) => {
                    this.fill(preferences, mutes, blocks, names)
                }
                Err(error) => this.dialog.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    fn fill(
        &self,
        preferences: Vec<Value>,
        mutes: usize,
        blocks: usize,
        labelers: Vec<(String, String)>,
    ) {
        self.filling.set(true);
        self.adult
            .set_active(moderation::adult_content(&preferences));
        for (label, row) in &self.labels {
            let visibility = moderation::label_visibility(&preferences, label);
            let index = Visibility::ALL
                .iter()
                .position(|v| *v == visibility)
                .unwrap_or(1);
            row.set_selected(index as u32);
        }
        self.muted_count.set_label(&mutes.to_string());
        self.blocked_count.set_label(&blocks.to_string());
        self.words_count
            .set_label(&moderation::muted_words(&preferences).len().to_string());
        for (_, name) in labelers {
            self.labelers.add(
                &adw::ActionRow::builder()
                    .title(&name)
                    .subtitle(gettext("Abonnement actif"))
                    .use_markup(false)
                    .build(),
            );
        }
        self.preferences.replace(preferences);
        self.adult.set_sensitive(true);
        for row in &self.rows {
            row.set_sensitive(true);
        }
        self.update_adult_rows();
        self.filling.set(false);
    }

    /// Les étiquettes pour adultes ne se règlent qu'avec le contenu pour
    /// adultes activé.
    fn update_adult_rows(&self) {
        for (label, row) in &self.labels {
            row.set_sensitive(!moderation::is_adult_label(label) || self.adult.is_active());
        }
    }

    /// Applique un changement à la copie locale, puis l'envoie à Bluesky
    /// (relecture, modification, écriture).
    fn change(self: &Rc<Self>, change: Change) {
        if self.filling.get() {
            return;
        }
        change.apply(&mut self.preferences.borrow_mut());
        let words = moderation::muted_words(&self.preferences.borrow());
        self.words_count.set_label(&words.len().to_string());
        moderation::set_preferences(self.preferences.borrow().clone());
        (self.on_changed)();
        let Some(session) = self.session.clone() else {
            return;
        };
        let task = runtime().spawn(async move {
            moderation::update_preferences(session, move |preferences| change.apply(preferences))
                .await
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(preferences) => {
                    this.preferences.replace(preferences);
                }
                Err(error) => {
                    glib::g_debug!("tsubame", "préférences : {}", error.message);
                    this.dialog.add_toast(adw::Toast::new(&gettext(
                        "La préférence n'a pas été enregistrée.",
                    )));
                }
            }
        });
    }

    /// Comptes masqués (`blocked` faux) ou bloqués, avec de quoi annuler.
    fn show_accounts(self: &Rc<Self>, blocked: bool) {
        let group = adw::PreferencesGroup::new();
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let title = if blocked {
            gettext("Comptes bloqués")
        } else {
            gettext("Comptes masqués")
        };
        self.dialog.push_subpage(&subpage(&title, &page));

        let Some(session) = self.session.clone() else {
            group.add(&empty_row());
            return;
        };
        let task = runtime().spawn({
            let session = session.clone();
            async move {
                if blocked {
                    moderation::blocks(session).await
                } else {
                    moderation::mutes(session)
                        .await
                        .map(|actors| actors.into_iter().map(|a| (a, None)).collect())
                }
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(accounts) if accounts.is_empty() => group.add(&empty_row()),
                Ok(accounts) => {
                    for (actor, block) in accounts {
                        group.add(&this.account_row(&actor, block, &group));
                    }
                }
                Err(error) => this.dialog.add_toast(adw::Toast::new(&error.message)),
            }
        });
    }

    fn account_row(
        self: &Rc<Self>,
        actor: &Actor,
        block: Option<String>,
        group: &adw::PreferencesGroup,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&actor.name)
            .subtitle(format!("@{}", actor.handle))
            .use_markup(false)
            .build();
        let blocked = block.is_some();
        let button = gtk::Button::builder()
            .label(if blocked {
                gettext("Débloquer")
            } else {
                gettext("Ne plus masquer")
            })
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&button);
        let (weak, group, row_clone, handle) = (
            Rc::downgrade(self),
            group.clone(),
            row.clone(),
            actor.handle.clone(),
        );
        button.connect_clicked(move |button| {
            let Some(this) = weak.upgrade() else { return };
            let Some(session) = this.session.clone() else {
                return;
            };
            button.set_sensitive(false);
            let block = block.clone();
            let handle = handle.clone();
            let task = runtime().spawn(async move {
                match block {
                    Some(record) => api::delete(session, record).await,
                    None => moderation::mute(session, handle, false).await,
                }
            });
            let (weak, group, row, button) = (
                Rc::downgrade(&this),
                group.clone(),
                row_clone.clone(),
                button.clone(),
            );
            glib::spawn_future_local(async move {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                let Some(this) = weak.upgrade() else { return };
                match result {
                    Ok(()) => {
                        group.remove(&row);
                        let count = if blocked {
                            &this.blocked_count
                        } else {
                            &this.muted_count
                        };
                        let n: u32 = count.label().parse().unwrap_or(1);
                        count.set_label(&n.saturating_sub(1).to_string());
                    }
                    Err(error) => {
                        button.set_sensitive(true);
                        this.dialog.add_toast(adw::Toast::new(&error.message));
                    }
                }
            });
        });
        row
    }

    /// Mots masqués : ajout et retrait.
    fn show_words(self: &Rc<Self>) {
        let add = adw::EntryRow::builder()
            .title(gettext("Ajouter un mot ou un mot-clic"))
            .show_apply_button(true)
            .build();
        let adding = adw::PreferencesGroup::builder()
            .description(gettext(
                "Les posts qui contiennent ces mots sont cachés dans vos fils, de tous les comptes.",
            ))
            .build();
        adding.add(&add);
        let words = adw::PreferencesGroup::new();
        let page = adw::PreferencesPage::new();
        page.add(&adding);
        page.add(&words);
        self.dialog
            .push_subpage(&subpage(&gettext("Mots et mots-clics masqués"), &page));

        let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::default();
        let refill: Rc<dyn Fn()> = {
            let (weak, words, rows) = (Rc::downgrade(self), words.clone(), rows.clone());
            Rc::new(move || {
                let Some(this) = weak.upgrade() else { return };
                for row in rows.borrow_mut().drain(..) {
                    words.remove(&row);
                }
                for word in moderation::muted_words(&this.preferences.borrow()) {
                    let row = adw::ActionRow::builder()
                        .title(&word)
                        .use_markup(false)
                        .build();
                    let remove = gtk::Button::builder()
                        .icon_name("user-trash-symbolic")
                        .valign(gtk::Align::Center)
                        .tooltip_text(gettext("Retirer « {} »").replace("{}", &word))
                        .css_classes(["flat"])
                        .build();
                    remove.update_property(&[gtk::accessible::Property::Label(
                        &gettext("Retirer « {} »").replace("{}", &word),
                    )]);
                    row.add_suffix(&remove);
                    let weak = Rc::downgrade(&this);
                    let word = word.clone();
                    remove.connect_clicked(move |button| {
                        if let Some(this) = weak.upgrade() {
                            this.change(Change::RemoveWord(word.clone()));
                            if let Some(row) = button.ancestor(adw::ActionRow::static_type()) {
                                row.set_visible(false);
                            }
                        }
                    });
                    words.add(&row);
                    rows.borrow_mut().push(row);
                }
            })
        };
        refill();
        let weak = Rc::downgrade(self);
        add.connect_apply(move |entry| {
            let word = entry.text().trim().to_owned();
            if let Some(this) = weak.upgrade()
                && !word.is_empty()
            {
                this.change(Change::AddWord(word));
                entry.set_text("");
                refill();
            }
        });
    }
}

fn empty_row() -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(gettext("Aucun compte"))
        .css_classes(["dim-label"])
        .build()
}
