use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

mod api;
mod auth;
mod card;
mod column;
mod columns;
mod data;
mod i18n;
mod images;
mod interactions;
mod login;
mod runtime;
mod settings;
mod thread;
mod window;

const APP_ID: &str = "io.github.DocteurScience.Tsubame";

fn startup(app: &adw::Application) {
    gtk::Window::set_default_icon_name(APP_ID);

    let about = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| show_about(app))
        .build();
    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| app.quit())
        .build();
    app.add_action_entries([about, quit]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
}

fn show_about(app: &adw::Application) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Tsubame")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(gettext("Client Bluesky pour GNOME"))
        .website("https://github.com/DocteurScience/Tsubame")
        .issue_url("https://github.com/DocteurScience/Tsubame/issues")
        .license_type(gtk::License::Gpl30)
        .build();
    // Traducteurs : remplacez « translator-credits » par vos noms, un par ligne.
    let credits = gettext("translator-credits");
    if credits != "translator-credits" {
        dialog.set_translator_credits(&credits);
    }
    dialog.present(app.active_window().as_ref());
}

fn main() -> glib::ExitCode {
    i18n::init();
    gio::resources_register_include!("tsubame.gresource").expect("ressources introuvables");
    glib::set_application_name("Tsubame");

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(startup);
    app.connect_activate(|app| window::build(app).present());
    app.run()
}
