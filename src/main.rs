use adw::prelude::*;

mod card;
mod data;
mod i18n;
mod window;

const APP_ID: &str = "io.github.DocteurScience.Tsubame";

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("pas d'écran disponible"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn main() -> gtk::glib::ExitCode {
    i18n::init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| load_css());
    app.connect_activate(|app| window::build(app).present());
    app.run()
}
