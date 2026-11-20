//! Disposition des colonnes de cartes et du volet de discussion.
//!
//! `docs/design.md` : « Le nombre de colonnes suit la largeur de la fenêtre,
//! et chaque colonne reste entre 380 et 480 px. » Ce widget fait ce calcul :
//!
//! - volet visible : autant de colonnes que la place le permet, en laissant au
//!   volet au moins `PANE_MIN_WIDTH` ; le volet prend le reste ;
//! - volet masqué (paliers étroits) : une seule colonne, sur toute la largeur.
//!
//! Quand toutes les colonnes ouvertes ne tiennent pas, seules celles de
//! `offset` à `offset + visibles` sont affichées ; `reveal` choisit la
//! colonne qui doit rester visible.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

pub const COLUMN_MIN_WIDTH: i32 = 380;
pub const COLUMN_MAX_WIDTH: i32 = 480;
pub const PANE_MIN_WIDTH: i32 = 440;
const SPACING: i32 = 12;

/// Nombre de colonnes qui tiennent dans `available` pixels, entre 1 et `open`.
pub fn fitting_columns(available: i32, open: usize) -> usize {
    let fit = (available + SPACING) / (COLUMN_MIN_WIDTH + SPACING);
    (fit.max(1) as usize).min(open.max(1))
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ColumnsView {
        pub columns: RefCell<Vec<gtk::Widget>>,
        pub separator: RefCell<Option<gtk::Separator>>,
        pub pane: RefCell<Option<gtk::Widget>>,
        pub offset: Cell<usize>,
        /// Colonne à garder visible, même quand la largeur change.
        pub wanted: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ColumnsView {
        const NAME: &'static str = "TsubameColumnsView";
        type Type = super::ColumnsView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("columnsview");
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }
    }

    impl ObjectImpl for ColumnsView {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl ColumnsView {
        pub fn pane_visible(&self) -> Option<gtk::Widget> {
            self.pane.borrow().clone().filter(|pane| pane.get_visible())
        }
    }

    impl WidgetImpl for ColumnsView {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let mut minimum = 0;
            let mut natural = 0;
            let columns = self.columns.borrow();
            let pane = self.pane_visible();
            for child in columns.iter().chain(pane.iter()) {
                let (min, nat, _, _) = child.measure(orientation, -1);
                minimum = minimum.max(min);
                natural = natural.max(nat);
            }
            if orientation == gtk::Orientation::Vertical && pane.is_some() {
                minimum += 2 * SPACING;
                natural += 2 * SPACING;
            }
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let columns = self.columns.borrow();
            // Le volet n'a de place qu'à côté d'au moins une colonne de
            // largeur minimale. Pendant un redimensionnement, la largeur peut
            // passer sous ce seuil avant que le palier ne masque le volet :
            // on le met alors de côté pour cette allocation.
            let pane_fits = width >= COLUMN_MIN_WIDTH + PANE_MIN_WIDTH + 1 + 2 * SPACING;
            let pane = self.pane_visible().filter(|_| pane_fits);
            if let Some(pane) = self.pane.borrow().as_ref() {
                pane.set_child_visible(pane_fits);
            }
            let separator = self.separator.borrow().clone();

            // Avec le volet, les colonnes sont encadrées et espacées ; sans
            // lui, une seule colonne occupe toute la place.
            let (padding, count, column_width) = match &pane {
                Some(_) => {
                    let available = width - PANE_MIN_WIDTH - 1 - 2 * SPACING;
                    let count = fitting_columns(available, columns.len());
                    let gaps = SPACING * (count as i32 - 1);
                    let column_width = ((available - gaps) / count as i32).min(COLUMN_MAX_WIDTH);
                    (SPACING, count, column_width)
                }
                None => (0, 1, width),
            };
            let count = count.min(columns.len());
            let mut offset = self.offset.get();
            let wanted = self.wanted.get();
            if wanted < offset {
                offset = wanted;
            } else if wanted >= offset + count {
                offset = wanted + 1 - count;
            }
            let offset = offset.min(columns.len().saturating_sub(count));
            self.offset.set(offset);

            let mut x = padding;
            for (index, child) in columns.iter().enumerate() {
                let shown = (offset..offset + count).contains(&index);
                child.set_child_visible(shown);
                if shown {
                    child.measure(gtk::Orientation::Horizontal, -1);
                    child.allocate(
                        column_width,
                        height - 2 * padding,
                        -1,
                        Some(gsk_translate(x, padding)),
                    );
                    x += column_width + SPACING;
                }
            }

            if let (Some(separator), Some(pane)) = (separator, pane) {
                let columns_end = if count > 0 { x - SPACING + padding } else { 0 };
                separator.set_child_visible(true);
                separator.measure(gtk::Orientation::Horizontal, -1);
                separator.allocate(1, height, -1, Some(gsk_translate(columns_end, 0)));
                pane.measure(gtk::Orientation::Horizontal, -1);
                pane.allocate(
                    width - columns_end - 1,
                    height,
                    -1,
                    Some(gsk_translate(columns_end + 1, 0)),
                );
            } else if let Some(separator) = self.separator.borrow().as_ref() {
                separator.set_child_visible(false);
            }
        }
    }

    fn gsk_translate(x: i32, y: i32) -> gtk::gsk::Transform {
        gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, y as f32))
    }
}

glib::wrapper! {
    pub struct ColumnsView(ObjectSubclass<imp::ColumnsView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ColumnsView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ColumnsView {
    /// Place le volet de discussion à droite des colonnes. Le masquer
    /// (`set_visible(false)`) fait passer en colonne unique.
    pub fn set_pane(&self, pane: &impl IsA<gtk::Widget>) {
        let imp = self.imp();
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        separator.set_parent(self);
        pane.set_parent(self);
        // Le volet change de visibilité avec les paliers : recalculer.
        let view = self.downgrade();
        pane.connect_visible_notify(move |_| {
            if let Some(view) = view.upgrade() {
                view.queue_resize();
            }
        });
        imp.separator.replace(Some(separator));
        imp.pane.replace(Some(pane.clone().upcast()));
    }

    pub fn append_column(&self, column: &impl IsA<gtk::Widget>) {
        // Les colonnes passent avant le volet dans l'ordre de tabulation.
        match self.imp().separator.borrow().as_ref() {
            Some(separator) => column.insert_before(self, Some(separator)),
            None => column.set_parent(self),
        }
        self.imp()
            .columns
            .borrow_mut()
            .push(column.clone().upcast());
        self.queue_resize();
    }

    pub fn remove_column(&self, column: &impl IsA<gtk::Widget>) {
        let column = column.as_ref();
        self.imp().columns.borrow_mut().retain(|c| c != column);
        column.unparent();
        self.queue_resize();
    }

    /// Déplace la colonne d'indice `from` à l'indice `to`.
    pub fn move_column(&self, from: usize, to: usize) {
        let imp = self.imp();
        let mut columns = imp.columns.borrow_mut();
        if from >= columns.len() || to >= columns.len() || from == to {
            return;
        }
        let column = columns.remove(from);
        columns.insert(to, column.clone());
        // L'ordre des widgets fixe l'ordre de tabulation : le suivre.
        let next = columns
            .get(to + 1)
            .cloned()
            .or_else(|| imp.separator.borrow().clone().map(|s| s.upcast()));
        column.insert_before(self, next.as_ref());
        drop(columns);
        self.queue_allocate();
    }

    /// Décale au besoin les colonnes affichées pour que celle d'indice
    /// `index` soit visible, et la garde visible quand la largeur change.
    pub fn reveal(&self, index: usize) {
        self.imp().wanted.set(index);
        self.queue_allocate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_follow_the_available_width() {
        // Palier moyen (1280 px) : une colonne.
        assert_eq!(fitting_columns(1040 - PANE_MIN_WIDTH - 25, 4), 1);
        // Palier large (1920 px) : deux à trois colonnes.
        assert_eq!(fitting_columns(1680 - PANE_MIN_WIDTH - 25, 4), 3);
        // Très large (2560 px) : quatre colonnes.
        assert_eq!(fitting_columns(2320 - PANE_MIN_WIDTH - 25, 6), 4);
    }

    #[test]
    fn columns_are_limited_by_open_columns() {
        assert_eq!(fitting_columns(3000, 2), 2);
        assert_eq!(fitting_columns(100, 3), 1);
        assert_eq!(fitting_columns(3000, 0), 1);
    }
}
