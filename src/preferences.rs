//! Préférences (maquette `docs/mockups/source/Reglages.dc.html`), page
//! « Modération » : comptes masqués et bloqués, mots masqués, contenu
//! sensible, services d'étiquetage.

use chrono::Datelike;
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
    /// Mot, mots-clics seulement, durée en heures (`None` : toujours).
    AddWord(String, bool, Option<i64>),
    RemoveWord(String),
    /// Se désabonner d'un service d'étiquetage.
    DropLabeler(String),
    /// Réglage d'une étiquette d'un service tiers : service, étiquette.
    LabelerLabel(String, String, Visibility),
}

impl Change {
    fn apply(&self, preferences: &mut Vec<Value>) {
        match self {
            Change::Adult(enabled) => moderation::set_adult_content(preferences, *enabled),
            Change::Label(label, visibility) => {
                moderation::set_label_visibility(preferences, label, *visibility)
            }
            Change::AddWord(word, tags_only, hours) => moderation::add_muted_word_with(
                preferences,
                word,
                *tags_only,
                hours.map(|h| chrono::Utc::now() + chrono::Duration::hours(h)),
            ),
            Change::RemoveWord(word) => moderation::remove_muted_word(preferences, word),
            Change::DropLabeler(did) => moderation::set_labeler(preferences, did, false),
            Change::LabelerLabel(did, label, visibility) => {
                moderation::set_labeler_label_visibility(preferences, did, label, *visibility)
            }
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
    row.add_suffix(&crate::card::decorative_icon("go-next-symbolic"));
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
    let lists_row = link_row(
        &gettext("Listes de modération suivies"),
        &gtk::Label::new(None),
    );
    accounts.add(&muted_row);
    accounts.add(&blocked_row);
    accounts.add(&words_row);
    accounts.add(&lists_row);
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

    // Langues, réglages de cet ordinateur (GSettings).
    dialog.add(&languages_page());

    // Notifications du bureau, réglage de cet ordinateur (GSettings).
    let notifications = adw::PreferencesPage::builder()
        .title(gettext("Notifications"))
        .icon_name("preferences-system-notifications-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(gettext(
            "Tsubame vérifie les notifications toutes les minutes tant qu'il est ouvert.",
        ))
        .build();
    let desktop = adw::SwitchRow::builder()
        .title(gettext("Notifications du bureau"))
        .subtitle(gettext(
            "Prévenir des nouvelles notifications quand la fenêtre n'est pas au premier plan",
        ))
        .build();
    crate::settings::settings()
        .bind("desktop-notifications", &desktop, "active")
        .build();
    group.add(&desktop);
    notifications.add(&group);
    dialog.add(&notifications);

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
        rows: vec![
            muted_row.clone(),
            blocked_row.clone(),
            words_row.clone(),
            lists_row.clone(),
        ],
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
    lists_row.connect_activated(move |_| {
        if let Some(this) = weak.upgrade() {
            this.show_lists();
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
            let names = moderation::labeler_details(
                session,
                moderation::labelers(&preferences),
                moderation::interface_language(),
            )
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
        self: &Rc<Self>,
        preferences: Vec<Value>,
        mutes: usize,
        blocks: usize,
        labelers: Vec<(String, String, Vec<moderation::LabelDefinition>)>,
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
        let choices = gtk::StringList::new(&[]);
        for visibility in Visibility::ALL {
            choices.append(&visibility.title());
        }
        for (did, name, definitions) in labelers {
            // Une rangée par service, ouverte sur le réglage de chacune de
            // ses étiquettes (les informatives n'en ont pas).
            let row = adw::ExpanderRow::builder()
                .title(&name)
                .subtitle(gettext("Abonnement actif"))
                .use_markup(false)
                .build();
            for definition in definitions.iter().filter(|d| !d.inform_only) {
                let combo = adw::ComboRow::builder()
                    .title(&definition.name)
                    .use_markup(false)
                    .model(&choices)
                    .build();
                let visibility = moderation::labeler_label_visibility(&preferences, definition);
                let index = Visibility::ALL
                    .iter()
                    .position(|v| *v == visibility)
                    .unwrap_or(1);
                combo.set_selected(index as u32);
                let weak = Rc::downgrade(self);
                let (did, value) = (did.clone(), definition.value.clone());
                combo.connect_selected_notify(move |combo| {
                    if let (Some(this), Some(visibility)) = (
                        weak.upgrade(),
                        Visibility::ALL.get(combo.selected() as usize),
                    ) {
                        this.change(Change::LabelerLabel(
                            did.clone(),
                            value.clone(),
                            *visibility,
                        ));
                    }
                });
                row.add_row(&combo);
            }
            let drop = gtk::Button::builder()
                .label(gettext("Se désabonner"))
                .valign(gtk::Align::Center)
                .build();
            drop.update_property(&[gtk::accessible::Property::Label(
                &gettext("Se désabonner de {}").replace("{}", &name),
            )]);
            let weak = Rc::downgrade(self);
            drop.connect_clicked(move |button| {
                if let Some(this) = weak.upgrade() {
                    this.change(Change::DropLabeler(did.clone()));
                    if let Some(row) = button.ancestor(adw::ActionRow::static_type()) {
                        row.set_visible(false);
                    }
                }
            });
            row.add_suffix(&drop);
            self.labelers.add(&row);
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
            let preferences = moderation::update_preferences(session.clone(), move |preferences| {
                change.apply(preferences)
            })
            .await?;
            // Les étiquettes suivent les services encore choisis.
            moderation::configure_labelers(session, moderation::labelers(&preferences)).await;
            Ok::<_, ApiError>(preferences)
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

    /// Listes de modération suivies, avec « Ne plus masquer » ou « Ne plus
    /// bloquer ».
    fn show_lists(self: &Rc<Self>) {
        let group = adw::PreferencesGroup::builder()
            .description(gettext(
                "Les comptes de ces listes sont masqués ou bloqués pour vous.",
            ))
            .build();
        let page = adw::PreferencesPage::new();
        page.add(&group);
        self.dialog
            .push_subpage(&subpage(&gettext("Listes de modération suivies"), &page));
        let Some(session) = self.session.clone() else {
            group.add(&empty_list_row());
            return;
        };
        let task = runtime().spawn(crate::lists::subscribed(session.clone()));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
            let Some(this) = weak.upgrade() else { return };
            let lists = match result {
                Ok(lists) => lists,
                Err(error) => {
                    this.dialog.add_toast(adw::Toast::new(&error.message));
                    return;
                }
            };
            if lists.is_empty() {
                group.add(&empty_list_row());
            }
            for list in lists {
                let mut state = Vec::new();
                if list.muted {
                    state.push(gettext("Comptes masqués"));
                }
                if list.blocked.is_some() {
                    state.push(gettext("Comptes bloqués"));
                }
                let row = adw::ActionRow::builder()
                    .title(&list.name)
                    .subtitle(state.join(" · "))
                    .use_markup(false)
                    .build();
                let undo = gtk::Button::builder()
                    .label(if list.blocked.is_some() {
                        gettext("Ne plus bloquer")
                    } else {
                        gettext("Ne plus masquer")
                    })
                    .valign(gtk::Align::Center)
                    .build();
                undo.update_property(&[gtk::accessible::Property::Label(
                    &gettext("Ne plus suivre la liste {}").replace("{}", &list.name),
                )]);
                let (session, weak) = (session.clone(), Rc::downgrade(&this));
                undo.connect_clicked(move |button| {
                    let (session, list) = (session.clone(), list.clone());
                    // Ne plus masquer, ni bloquer : la liste n'agit plus.
                    let task = runtime().spawn(async move {
                        if list.muted {
                            crate::lists::set_muted(session.clone(), list.uri.clone(), false)
                                .await?;
                        }
                        if let Some(record) = list.blocked {
                            api::delete(session, record).await?;
                        }
                        Ok::<_, ApiError>(())
                    });
                    let (weak, row) =
                        (weak.clone(), button.ancestor(adw::ActionRow::static_type()));
                    glib::spawn_future_local(async move {
                        let result = task.await.unwrap_or_else(|_| {
                            Err(ApiError::new(gettext("Requête interrompue.")))
                        });
                        let Some(this) = weak.upgrade() else { return };
                        match result {
                            Ok(()) => {
                                if let Some(row) = row {
                                    row.set_visible(false);
                                }
                            }
                            Err(error) => this.dialog.add_toast(adw::Toast::new(&error.message)),
                        }
                    });
                });
                row.add_suffix(&undo);
                group.add(&row);
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
        // Durées proposées, en heures (0 : toujours).
        const DURATIONS: [i64; 4] = [0, 24, 24 * 7, 24 * 30];
        let durations = gtk::StringList::new(&[]);
        for hours in DURATIONS {
            durations.append(&match hours {
                0 => gettext("Toujours"),
                24 => gettext("24 heures"),
                168 => gettext("7 jours"),
                _ => gettext("30 jours"),
            });
        }
        let duration = adw::ComboRow::builder()
            .title(gettext("Durée"))
            .model(&durations)
            .build();
        let tags_only = adw::SwitchRow::builder()
            .title(gettext("Mots-clics seulement"))
            .subtitle(gettext("Ne pas chercher le mot dans le texte des posts."))
            .build();
        adding.add(&duration);
        adding.add(&tags_only);
        let words = adw::PreferencesGroup::new();
        let page = adw::PreferencesPage::new();
        page.add(&adding);
        page.add(&words);
        self.dialog
            .push_subpage(&subpage(&gettext("Mots et mots-clics masqués"), &page));

        let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::default();
        let refill: Rc<dyn Fn()> =
            {
                let (weak, words, rows) = (Rc::downgrade(self), words.clone(), rows.clone());
                Rc::new(move || {
                    let Some(this) = weak.upgrade() else { return };
                    for row in rows.borrow_mut().drain(..) {
                        words.remove(&row);
                    }
                    let now = chrono::Utc::now();
                    for item in moderation::muted_word_items(&this.preferences.borrow()) {
                        let word = item.value.clone();
                        let mut details = Vec::new();
                        if item.tags_only {
                            details.push(gettext("Mots-clics seulement"));
                        }
                        match item.expires {
                            Some(end) if end <= now => details.push(gettext("Expiré")),
                            Some(end) => {
                                let local = end.with_timezone(&chrono::Local);
                                details.push(gettext("Jusqu'au {}").replace(
                                    "{}",
                                    &crate::api::day_month(local.day(), local.month()),
                                ));
                            }
                            None => {}
                        }
                        let row = adw::ActionRow::builder()
                            .title(&word)
                            .subtitle(details.join(" · "))
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
        let (duration, tags_only) = (duration.clone(), tags_only.clone());
        add.connect_apply(move |entry| {
            let word = entry.text().trim().to_owned();
            if let Some(this) = weak.upgrade()
                && !word.is_empty()
            {
                let hours = DURATIONS
                    .get(duration.selected() as usize)
                    .copied()
                    .filter(|h| *h > 0);
                this.change(Change::AddWord(word, tags_only.is_active(), hours));
                entry.set_text("");
                refill();
            }
        });
    }
}

/// Page « Langues » : langue proposée pour les posts, langues des posts
/// affichés dans les fils.
fn languages_page() -> adw::PreferencesPage {
    use crate::compose::LANGUAGES;
    let settings = crate::settings::settings();
    let page = adw::PreferencesPage::builder()
        .title(gettext("Langues"))
        .icon_name("preferences-desktop-locale-symbolic")
        .build();

    let mine = adw::PreferencesGroup::builder()
        .title(gettext("Vos posts"))
        .build();
    let names = gtk::StringList::new(&[]);
    names.append(&gettext("Celle du système"));
    for (_, name) in LANGUAGES {
        names.append(name);
    }
    let post_language = adw::ComboRow::builder()
        .title(gettext("Langue proposée"))
        .subtitle(gettext("Elle se change aussi dans chaque post."))
        .model(&names)
        .build();
    let current = settings.string("post-language");
    let index = LANGUAGES
        .iter()
        .position(|(code, _)| *code == current.as_str())
        .map_or(0, |i| i + 1);
    post_language.set_selected(index as u32);
    {
        let settings = settings.clone();
        post_language.connect_selected_notify(move |row| {
            let code = match row.selected() {
                0 => "",
                i => LANGUAGES.get(i as usize - 1).map_or("", |(code, _)| *code),
            };
            let _ = settings.set_string("post-language", code);
        });
    }
    mine.add(&post_language);
    page.add(&mine);

    let shown = adw::PreferencesGroup::builder()
        .title(gettext("Posts affichés"))
        .description(gettext(
            "Seuls les posts dans ces langues s'affichent dans vos fils. Aucune langue cochée : toutes.",
        ))
        .build();
    let wanted: Vec<String> = settings
        .strv("content-languages")
        .iter()
        .map(|l| l.to_string())
        .collect();
    for (code, name) in LANGUAGES {
        let row = adw::SwitchRow::builder()
            .title(name)
            .active(wanted.iter().any(|w| w == code))
            .build();
        let settings = settings.clone();
        row.connect_active_notify(move |row| {
            let mut wanted: Vec<String> = settings
                .strv("content-languages")
                .iter()
                .map(|l| l.to_string())
                .filter(|l| l != code)
                .collect();
            if row.is_active() {
                wanted.push(code.to_owned());
            }
            let refs: Vec<&str> = wanted.iter().map(String::as_str).collect();
            let _ = settings.set_strv("content-languages", refs.as_slice());
        });
        shown.add(&row);
    }
    page.add(&shown);
    page
}

fn empty_list_row() -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(gettext("Aucune liste"))
        .css_classes(["dim-label"])
        .build()
}

fn empty_row() -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(gettext("Aucun compte"))
        .css_classes(["dim-label"])
        .build()
}
