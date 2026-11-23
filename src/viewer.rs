//! Visionneuse d'images : une image d'un post en grand, son texte
//! alternatif, et les autres images du post (flèches, ← et →).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, glib};

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
    let stage = gtk::Overlay::builder().child(&picture).build();
    stage.add_overlay(&previous);
    stage.add_overlay(&next);

    let title = adw::WindowTitle::new(&gettext("Image"), "");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
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

    let show: Rc<dyn Fn()> = {
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
        _ => glib::Propagation::Proceed,
    });
    dialog.add_controller(keys);
    show();
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_is_shown_for_several_images() {
        assert_eq!(position_label(0, 1), "");
        assert_eq!(position_label(1, 4), "2 / 4");
    }
}
