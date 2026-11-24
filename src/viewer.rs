//! Visionneuse d'images : une image d'un post en grand, son texte
//! alternatif, et les autres images du post (flèches, ← et →). Zoom (+, -,
//! 0 pour ajuster) et enregistrement de l'image.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::data::Image;
use crate::images;

/// « 2 / 4 », ou rien pour une image seule.
pub fn position_label(index: usize, count: usize) -> String {
    if count < 2 {
        String::new()
    } else {
        gettext("{} / {}")
            .replacen("{}", &(index + 1).to_string(), 1)
            .replacen("{}", &count.to_string(), 1)
    }
}

/// Facteur entre deux crans de zoom.
const ZOOM_STEP: f64 = 1.25;
/// Zoom maximal, par rapport à la taille réelle de l'image.
const ZOOM_MAX: f64 = 8.0;

/// Zoom suivant (`direction` 1 : plus grand, -1 : plus petit) à partir de
/// `current`, borné ; `None` en dessous de `fit` (l'image tient alors dans
/// la fenêtre).
pub fn next_zoom(current: f64, fit: f64, direction: i32) -> Option<f64> {
    let next = if direction > 0 {
        current * ZOOM_STEP
    } else {
        current / ZOOM_STEP
    };
    if next <= fit {
        None
    } else {
        Some(next.min(ZOOM_MAX.max(fit)))
    }
}

/// Ouvre la visionneuse sur l'image `start` de `images`.
pub fn present(parent: &impl IsA<gtk::Widget>, images: Vec<Image>, start: usize) {
    if images.is_empty() {
        return;
    }
    let count = images.len();
    let index = Rc::new(Cell::new(start.min(count - 1)));

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .vexpand(true)
        .hexpand(true)
        .build();
    let alt = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let arrow = |icon: &str, label: String, halign: gtk::Align| {
        let button = gtk::Button::builder()
            .icon_name(icon)
            .tooltip_text(&label)
            .halign(halign)
            .valign(gtk::Align::Center)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["circular", "osd", "viewer-arrow"])
            .visible(count > 1)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        button
    };
    let previous = arrow(
        "go-previous-symbolic",
        gettext("Image précédente"),
        gtk::Align::Start,
    );
    let next = arrow(
        "go-next-symbolic",
        gettext("Image suivante"),
        gtk::Align::End,
    );
    // L'image défile quand elle est agrandie.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&picture)
        .vexpand(true)
        .hexpand(true)
        .build();
    let stage = gtk::Overlay::builder().child(&scroller).build();
    stage.add_overlay(&previous);
    stage.add_overlay(&next);

    let title = adw::WindowTitle::new(&gettext("Image"), "");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let tool = |icon: &str, label: String| {
        let button = gtk::Button::builder()
            .icon_name(icon)
            .tooltip_text(&label)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        button
    };
    let save = tool("document-save-symbolic", gettext("Enregistrer l'image"));
    let zoom_out = tool("zoom-out-symbolic", gettext("Zoom arrière"));
    let zoom_fit = tool("zoom-fit-best-symbolic", gettext("Ajuster à la fenêtre"));
    let zoom_in = tool("zoom-in-symbolic", gettext("Zoom avant"));
    header.pack_start(&save);
    let zooms = gtk::Box::builder().css_classes(["linked"]).build();
    zooms.append(&zoom_out);
    zooms.append(&zoom_fit);
    zooms.append(&zoom_in);
    header.pack_end(&zooms);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&stage);
    content.append(&alt);
    let toolbar = adw::ToolbarView::builder().content(&content).build();
    toolbar.add_top_bar(&header);
    let dialog = adw::Dialog::builder()
        .title(gettext("Image"))
        .content_width(960)
        .content_height(720)
        .child(&toolbar)
        .build();

    // Zoom : `None`, l'image tient dans la fenêtre ; sinon, facteur par
    // rapport à sa taille réelle.
    let zoom: Rc<Cell<Option<f64>>> = Rc::default();
    let apply_zoom: Rc<dyn Fn(Option<f64>)> = {
        let (picture, zoom, zoom_out, zoom_fit) = (
            picture.clone(),
            zoom.clone(),
            zoom_out.clone(),
            zoom_fit.clone(),
        );
        Rc::new(move |value: Option<f64>| {
            zoom.set(value);
            let size = picture
                .paintable()
                .map(|p| (p.intrinsic_width(), p.intrinsic_height()))
                .filter(|(w, h)| *w > 0 && *h > 0);
            match (value, size) {
                (Some(z), Some((w, h))) => {
                    picture.set_can_shrink(false);
                    picture.set_content_fit(gtk::ContentFit::Fill);
                    picture.set_size_request((w as f64 * z) as i32, (h as f64 * z) as i32);
                }
                _ => {
                    picture.set_size_request(-1, -1);
                    picture.set_can_shrink(true);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                }
            }
            zoom_out.set_sensitive(value.is_some());
            zoom_fit.set_sensitive(value.is_some());
        })
    };
    // Facteur auquel l'image tient dans la fenêtre.
    let fit_scale = {
        let (picture, scroller) = (picture.clone(), scroller.clone());
        move || -> f64 {
            let Some((w, h)) = picture
                .paintable()
                .map(|p| (p.intrinsic_width(), p.intrinsic_height()))
                .filter(|(w, h)| *w > 0 && *h > 0)
            else {
                return 1.0;
            };
            let (sw, sh) = (scroller.width().max(1), scroller.height().max(1));
            (sw as f64 / w as f64).min(sh as f64 / h as f64)
        }
    };
    let zoom_by: Rc<dyn Fn(i32)> = {
        let (zoom, apply_zoom) = (zoom.clone(), apply_zoom.clone());
        Rc::new(move |direction: i32| {
            let fit = fit_scale();
            let current = zoom.get().unwrap_or(fit);
            apply_zoom(next_zoom(current, fit, direction));
        })
    };
    {
        let zoom_by = zoom_by.clone();
        zoom_in.connect_clicked(move |_| zoom_by(1));
    }
    {
        let zoom_by = zoom_by.clone();
        zoom_out.connect_clicked(move |_| zoom_by(-1));
    }
    {
        let apply_zoom = apply_zoom.clone();
        zoom_fit.connect_clicked(move |_| apply_zoom(None));
    }
    {
        let picture = picture.clone();
        save.connect_clicked(move |button| save_image(button, &picture));
    }

    let show: Rc<dyn Fn()> = {
        let apply_zoom = apply_zoom.clone();
        let (picture, alt, title, previous, next, index) = (
            picture.clone(),
            alt.clone(),
            title.clone(),
            previous.clone(),
            next.clone(),
            index.clone(),
        );
        Rc::new(move || {
            let i = index.get();
            let image = &images[i];
            title.set_subtitle(&position_label(i, count));
            let text = if image.alt.trim().is_empty() {
                gettext("Pas de texte alternatif.")
            } else {
                image.alt.clone()
            };
            alt.set_label(&text);
            if image.alt.trim().is_empty() {
                alt.add_css_class("dim-label");
            } else {
                alt.remove_css_class("dim-label");
            }
            picture.set_alternative_text(Some(&image.alt));
            picture.update_property(&[gtk::accessible::Property::Label(&text)]);
            picture.set_paintable(None::<&gdk::Paintable>);
            // La miniature d'abord, puis l'image en grand si elle arrive
            // pendant qu'on la regarde encore.
            for url in [&image.thumb, &image.fullsize] {
                let (weak, index, wanted) = (picture.downgrade(), index.clone(), i);
                images::load(url, move |texture| {
                    if let Some(picture) = weak.upgrade()
                        && index.get() == wanted
                    {
                        picture.set_paintable(Some(texture));
                    }
                });
            }
            previous.set_sensitive(i > 0);
            next.set_sensitive(i + 1 < count);
            apply_zoom(None);
        })
    };
    let step: Rc<dyn Fn(isize)> = {
        let (index, show) = (index.clone(), show.clone());
        Rc::new(move |delta: isize| {
            let target = index.get() as isize + delta;
            if target >= 0 && (target as usize) < count {
                index.set(target as usize);
                show();
            }
        })
    };
    {
        let step = step.clone();
        previous.connect_clicked(move |_| step(-1));
    }
    {
        let step = step.clone();
        next.connect_clicked(move |_| step(1));
    }
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| match key {
        gdk::Key::Left => {
            step(-1);
            glib::Propagation::Stop
        }
        gdk::Key::Right => {
            step(1);
            glib::Propagation::Stop
        }
        gdk::Key::plus | gdk::Key::KP_Add | gdk::Key::equal => {
            zoom_by(1);
            glib::Propagation::Stop
        }
        gdk::Key::minus | gdk::Key::KP_Subtract => {
            zoom_by(-1);
            glib::Propagation::Stop
        }
        gdk::Key::_0 | gdk::Key::KP_0 => {
            apply_zoom(None);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    dialog.add_controller(keys);
    show();
    dialog.present(Some(parent));
}

/// Enregistre l'image affichée (PNG), là où la personne le choisit.
fn save_image(button: &gtk::Button, picture: &gtk::Picture) {
    let Some(texture) = picture.paintable().and_downcast::<gdk::Texture>() else {
        return;
    };
    let chooser = gtk::FileDialog::builder()
        .title(gettext("Enregistrer l'image"))
        .initial_name(format!("{}.png", gettext("image")))
        .modal(true)
        .build();
    let window = button.root().and_downcast::<gtk::Window>();
    chooser.save(window.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let bytes = texture.save_to_png_bytes();
        glib::spawn_future_local(async move {
            if let Err(error) = file
                .replace_contents_future(bytes, None, false, gio::FileCreateFlags::NONE)
                .await
            {
                glib::g_warning!("tsubame", "image non enregistrée : {}", error.1);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_stay_in_bounds() {
        // Sous le facteur « ajusté », retour à l'ajustement.
        assert_eq!(next_zoom(0.5, 0.5, -1), None);
        assert_eq!(next_zoom(0.5, 0.5, 1), Some(0.625));
        assert_eq!(next_zoom(7.0, 0.5, 1), Some(ZOOM_MAX));
    }

    #[test]
    fn position_is_shown_for_several_images() {
        assert_eq!(position_label(0, 1), "");
        assert_eq!(position_label(1, 4), "2 / 4");
    }
}
