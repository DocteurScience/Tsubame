use adw::prelude::*;

const APP_ID: &str = "io.github.DocteurScience.Tsubame";

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Tsubame")
            .default_width(1280)
            .default_height(800)
            .build();
        window.present();
    });
    app.run()
}
