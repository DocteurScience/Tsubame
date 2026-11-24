//! Réglages du compte qui passent par son serveur : identifiant, mots de
//! passe d'application.
//!
//! Bluesky peut refuser ces changements à une session OAuth (ils sont
//! réservés au compte complet) : le message renvoie alors au site.

use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::glib;

use crate::api::{ApiError, request_failed};
use crate::auth::Session;
use crate::runtime::runtime;

/// Mot de passe d'application : son nom et sa date de création.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPassword {
    pub name: String,
    pub created: String,
}

/// Erreur du serveur, avec le renvoi au site quand il refuse l'accès.
fn refused(error: impl std::fmt::Display) -> ApiError {
    let text = error.to_string();
    if [
        "scope",
        "Forbidden",
        "privileged",
        "Insufficient",
        "InvalidToken",
    ]
    .iter()
    .any(|word| text.contains(word))
    {
        return ApiError::new(gettext(
            "Bluesky n'autorise pas ce changement depuis une application. Faites-le sur le site de Bluesky.",
        ));
    }
    request_failed(text)
}

/// Un identifiant valide : un nom de domaine (« vous.bsky.social »,
/// « exemple.fr »), sans « @ ».
pub fn normalize_handle(text: &str) -> Option<String> {
    let handle = text.trim().trim_start_matches('@').to_lowercase();
    atrium_api::types::string::Handle::new(handle.clone())
        .ok()
        .map(|_| handle)
}

pub async fn change_handle(session: Arc<Session>, handle: String) -> Result<(), ApiError> {
    let handle = atrium_api::types::string::Handle::new(handle)
        .map_err(|_| ApiError::new(gettext("Cet identifiant n'est pas valide.")))?;
    let api = session.api().await?;
    api.api
        .com
        .atproto
        .identity
        .update_handle(
            atrium_api::com::atproto::identity::update_handle::InputData { handle }.into(),
        )
        .await
        .map_err(refused)
}

pub async fn app_passwords(session: Arc<Session>) -> Result<Vec<AppPassword>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .com
        .atproto
        .server
        .list_app_passwords()
        .await
        .map_err(refused)?;
    Ok(output
        .data
        .passwords
        .iter()
        .map(|p| AppPassword {
            name: p.name.clone(),
            created: crate::api::parse_datetime(p.created_at.as_str())
                .map(crate::api::full_date)
                .unwrap_or_default(),
        })
        .collect())
}

/// Crée un mot de passe d'application ; rend le mot de passe, montré une
/// seule fois.
pub async fn create_app_password(session: Arc<Session>, name: String) -> Result<String, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .com
        .atproto
        .server
        .create_app_password(
            atrium_api::com::atproto::server::create_app_password::InputData {
                name,
                privileged: None,
            }
            .into(),
        )
        .await
        .map_err(refused)?;
    Ok(output.data.password.clone())
}

pub async fn revoke_app_password(session: Arc<Session>, name: String) -> Result<(), ApiError> {
    let api = session.api().await?;
    api.api
        .com
        .atproto
        .server
        .revoke_app_password(
            atrium_api::com::atproto::server::revoke_app_password::InputData { name }.into(),
        )
        .await
        .map_err(refused)
}

/// Lance `task` ; `done` reçoit le résultat sur la boucle GTK.
fn run<T: Send + 'static>(
    task: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
    done: impl FnOnce(Result<T, ApiError>) + 'static,
) {
    let task = runtime().spawn(task);
    glib::spawn_future_local(async move {
        done(
            task.await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue.")))),
        );
    });
}

/// Boîte « Changer d'identifiant ».
pub fn change_handle_dialog(parent: &adw::PreferencesDialog, session: Arc<Session>) {
    let toasts = parent.downgrade();
    let dialog = adw::AlertDialog::new(
        Some(&gettext("Changer d'identifiant")),
        Some(&gettext(
            "Un identifiant en .bsky.social, ou un nom de domaine que vous possédez et que vous avez déjà relié à votre compte.",
        )),
    );
    let entry = gtk::Entry::builder()
        .placeholder_text(gettext("vous.bsky.social"))
        .activates_default(true)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Nouvel identifiant",
    ))]);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[
        ("cancel", &gettext("Annuler")),
        ("change", &gettext("Changer")),
    ]);
    dialog.set_response_appearance("change", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("change", false);
    dialog.set_default_response(Some("change"));
    dialog.set_close_response("cancel");
    {
        let dialog = dialog.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.set_response_enabled("change", normalize_handle(&entry.text()).is_some());
            }
        });
    }
    dialog.connect_response(Some("change"), move |_, _| {
        let Some(handle) = normalize_handle(&entry.text()) else {
            return;
        };
        let toasts = toasts.clone();
        run(change_handle(session.clone(), handle.clone()), move |result| {
            let Some(toasts) = toasts.upgrade() else { return };
            toasts.add_toast(adw::Toast::new(&match result {
                Ok(()) => gettext("Identifiant changé en @{}. Il s'affichera partout à la prochaine connexion.")
                    .replace("{}", &handle),
                Err(error) => error.message,
            }));
        });
    });
    dialog.present(Some(parent));
}

/// Sous-page « Mots de passe d'application » : la liste, « Révoquer », et
/// la création d'un nouveau (montré une seule fois).
pub fn app_passwords_page(
    session: Arc<Session>,
    dialog: &adw::PreferencesDialog,
) -> adw::NavigationPage {
    let add = adw::EntryRow::builder()
        .title(gettext("Nom d'un nouveau mot de passe"))
        .show_apply_button(true)
        .build();
    let adding = adw::PreferencesGroup::builder()
        .description(gettext(
            "Pour vous connecter à une application qui ne gère pas la connexion par navigateur. Chaque mot de passe se révoque à part.",
        ))
        .build();
    adding.add(&add);
    let list = adw::PreferencesGroup::new();
    let page = adw::PreferencesPage::new();
    page.add(&adding);
    page.add(&list);
    let view = adw::ToolbarView::builder().content(&page).build();
    view.add_top_bar(&adw::HeaderBar::new());
    let navigation = adw::NavigationPage::builder()
        .title(gettext("Mots de passe d'application"))
        .child(&view)
        .build();

    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Rc::default();
    let refill: Rc<dyn Fn()> = {
        let (session, list, rows, dialog) = (
            session.clone(),
            list.clone(),
            rows.clone(),
            dialog.downgrade(),
        );
        Rc::new(move || {
            let (session, list, rows, dialog) =
                (session.clone(), list.clone(), rows.clone(), dialog.clone());
            run(app_passwords(session.clone()), move |result| {
                for row in rows.borrow_mut().drain(..) {
                    list.remove(&row);
                }
                let passwords = match result {
                    Ok(passwords) => passwords,
                    Err(error) => {
                        if let Some(dialog) = dialog.upgrade() {
                            dialog.add_toast(adw::Toast::new(&error.message));
                        }
                        return;
                    }
                };
                for password in passwords {
                    let row = adw::ActionRow::builder()
                        .title(&password.name)
                        .subtitle(gettext("Créé le {}").replace("{}", &password.created))
                        .use_markup(false)
                        .build();
                    let revoke = gtk::Button::builder()
                        .label(gettext("Révoquer"))
                        .valign(gtk::Align::Center)
                        .css_classes(["destructive-action"])
                        .build();
                    revoke.update_property(&[gtk::accessible::Property::Label(
                        &gettext("Révoquer {}").replace("{}", &password.name),
                    )]);
                    let (session, dialog, name) =
                        (session.clone(), dialog.clone(), password.name.clone());
                    revoke.connect_clicked(move |button| {
                        let row = button.ancestor(adw::ActionRow::static_type());
                        let dialog = dialog.clone();
                        run(
                            revoke_app_password(session.clone(), name.clone()),
                            move |result| match result {
                                Ok(()) => {
                                    if let Some(row) = row {
                                        row.set_visible(false);
                                    }
                                }
                                Err(error) => {
                                    if let Some(dialog) = dialog.upgrade() {
                                        dialog.add_toast(adw::Toast::new(&error.message));
                                    }
                                }
                            },
                        );
                    });
                    row.add_suffix(&revoke);
                    list.add(&row);
                    rows.borrow_mut().push(row);
                }
            });
        })
    };
    refill();
    let dialog = dialog.downgrade();
    add.connect_apply(move |entry| {
        let name = entry.text().trim().to_owned();
        if name.is_empty() {
            return;
        }
        entry.set_text("");
        let (refill, dialog) = (refill.clone(), dialog.clone());
        run(
            create_app_password(session.clone(), name.clone()),
            move |result| {
                let Some(dialog) = dialog.upgrade() else {
                    return;
                };
                match result {
                    Ok(password) => {
                        show_password(&dialog, &name, &password);
                        refill();
                    }
                    Err(error) => dialog.add_toast(adw::Toast::new(&error.message)),
                }
            },
        );
    });
    navigation
}

/// Le nouveau mot de passe, à copier : il ne sera plus montré.
fn show_password(parent: &adw::PreferencesDialog, name: &str, password: &str) {
    let alert = adw::AlertDialog::new(
        Some(&gettext("Mot de passe « {} »").replace("{}", name)),
        Some(&gettext(
            "Notez-le maintenant : il ne sera plus montré. Il donne accès à votre compte, gardez-le pour vous.",
        )),
    );
    let label = gtk::Label::builder()
        .label(password)
        .selectable(true)
        .css_classes(["title-2", "monospace"])
        .build();
    alert.set_extra_child(Some(&label));
    alert.add_responses(&[("copy", &gettext("Copier")), ("close", &gettext("Fermer"))]);
    alert.set_default_response(Some("copy"));
    alert.set_close_response("close");
    let password = password.to_owned();
    alert.connect_response(Some("copy"), move |alert, _| {
        alert.clipboard().set_text(&password);
    });
    alert.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_normalized() {
        assert_eq!(
            normalize_handle(" @Lucie.bsky.social ").as_deref(),
            Some("lucie.bsky.social")
        );
        assert_eq!(
            normalize_handle("exemple.fr").as_deref(),
            Some("exemple.fr")
        );
        assert_eq!(normalize_handle("sans-point"), None);
        assert_eq!(normalize_handle(""), None);
    }
}
