//! Disposition des colonnes de cartes et du volet de discussion.
//!
//! `docs/design.md` : « Le nombre de colonnes suit la largeur de la fenêtre,
//! et chaque colonne reste entre 380 et 480 px. » Ce widget fait ce calcul,
//! selon la disposition choisie (`Layout`) :
//!
//! - `Right` : autant de colonnes que la place le permet, en laissant au
//!   volet au moins `PANE_MIN_WIDTH` ; le volet prend le reste, à droite ;
//! - `Below` : les colonnes sur toute la largeur, le volet en dessous ;
//! - `Hidden` : les colonnes sur toute la largeur, sans volet ;
//!
//! Dans ces deux derniers cas, une colonne peut dépasser 480 px quand peu de
//! colonnes sont ouvertes ; les cartes, elles, restent limitées à 480 px.
//! - `Single` (paliers étroits) : une seule colonne, sans cadre.
//!
//! Quand toutes les colonnes ouvertes ne tiennent pas, seules celles de
//! `offset` à `offset + visibles` sont affichées ; `reveal` choisit la
//! colonne qui doit rester visible.

use std::cell::{Cell, RefCell};
use std::ops::Range;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

pub const COLUMN_MIN_WIDTH: i32 = 380;
pub const COLUMN_MAX_WIDTH: i32 = 480;
pub const PANE_MIN_WIDTH: i32 = 440;
/// Hauteur minimale des colonnes et du volet quand il est en dessous.
const BELOW_MIN_HEIGHT: i32 = 240;
const SPACING: i32 = 12;

/// Disposition des colonnes et du volet de discussion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Une seule colonne, sans cadre ; la discussion s'empile.
    Single,
    #[default]
    Right,
    Below,
    /// Colonnes sans volet ; la discussion s'empile.
    Hidden,
}

/// Nombre de colonnes qui tiennent dans `available` pixels, entre 1 et `open`.
pub fn fitting_columns(available: i32, open: usize) -> usize {
    let fit = (available + SPACING) / (COLUMN_MIN_WIDTH + SPACING);
    (fit.max(1) as usize).min(open.max(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        }
    }
}

/// Résultat du calcul de disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Colonnes affichées (indices) et leur place, dans l'ordre.
    pub visible: Range<usize>,
    pub columns: Vec<Rect>,
    /// Le volet et le séparateur qui le borde, s'il est affiché.
    pub pane: Option<(Rect, Rect)>,
    /// Le volet est-il sous les colonnes (séparateur horizontal) ?
    pub below: bool,
}

/// Calcule la place des colonnes et du volet. `offset` est la première
/// colonne affichée jusque-là, `wanted` celle qui doit rester visible.
pub fn plan(
    layout: Layout,
    width: i32,
    height: i32,
    open: usize,
    offset: usize,
    wanted: usize,
) -> Plan {
    // Sans la place nécessaire, on se replie sur une disposition plus simple.
    let layout = match layout {
        Layout::Right if width < COLUMN_MIN_WIDTH + PANE_MIN_WIDTH + 1 + 2 * SPACING => {
            Layout::Single
        }
        Layout::Below if height < 2 * BELOW_MIN_HEIGHT + 1 => Layout::Hidden,
        layout => layout,
    };

    let (padding, count, column_width, columns_height, pane) = match layout {
        Layout::Single => (0, 1, width, height, None),
        Layout::Right => {
            let available = width - PANE_MIN_WIDTH - 1 - 2 * SPACING;
            let count = fitting_columns(available, open);
            let gaps = SPACING * (count as i32 - 1);
            let column_width = ((available - gaps) / count as i32).min(COLUMN_MAX_WIDTH);
            let columns_end = 2 * SPACING + count as i32 * column_width + gaps;
            let separator = Rect::new(columns_end, 0, 1, height);
            let pane = Rect::new(columns_end + 1, 0, width - columns_end - 1, height);
            (
                SPACING,
                count,
                column_width,
                height,
                Some((pane, separator)),
            )
        }
        Layout::Below | Layout::Hidden => {
            // Les colonnes occupent toute la largeur, pour que la dernière
            // touche le bord de la fenêtre et porte ses boutons. Les cartes
            // restent limitées à `COLUMN_MAX_WIDTH` dans chaque colonne.
            let available = width - 2 * SPACING;
            let count = fitting_columns(available, open);
            let gaps = SPACING * (count as i32 - 1);
            let column_width = (available - gaps) / count as i32;
            if layout == Layout::Below {
                let pane_height =
                    (height * 2 / 5).clamp(BELOW_MIN_HEIGHT, height - BELOW_MIN_HEIGHT - 1);
                let columns_height = height - pane_height - 1;
                let separator = Rect::new(0, columns_height, width, 1);
                let pane = Rect::new(0, columns_height + 1, width, pane_height);
                (
                    SPACING,
                    count,
                    column_width,
                    columns_height,
                    Some((pane, separator)),
                )
            } else {
                (SPACING, count, column_width, height, None)
            }
        }
    };

    let count = count.min(open);
    let mut offset = offset;
    if wanted < offset {
        offset = wanted;
    } else if wanted >= offset + count {
        offset = wanted + 1 - count;
    }
    let offset = offset.min(open.saturating_sub(count));

    let columns = (0..count)
        .map(|i| {
            Rect::new(
                padding + i as i32 * (column_width + SPACING),
                padding,
                column_width,
                columns_height - 2 * padding,
            )
        })
        .collect();

    Plan {
        visible: offset..offset + count,
        columns,
        pane,
        below: layout == Layout::Below,
    }
}

type RangeCallback = Box<dyn Fn(Range<usize>)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ColumnsView {
        pub columns: RefCell<Vec<gtk::Widget>>,
        pub vertical_separator: RefCell<Option<gtk::Separator>>,
        pub horizontal_separator: RefCell<Option<gtk::Separator>>,
        pub pane: RefCell<Option<gtk::Widget>>,
        pub layout: Cell<Layout>,
        pub offset: Cell<usize>,
        /// Colonne à garder visible, même quand la largeur change.
        pub wanted: Cell<usize>,
        pub visible: RefCell<Range<usize>>,
        pub on_visible_changed: RefCell<Option<RangeCallback>>,
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

    fn place(widget: &impl IsA<gtk::Widget>, rect: Rect) {
        widget.set_child_visible(true);
        widget.measure(gtk::Orientation::Horizontal, -1);
        widget.allocate(
            rect.width,
            rect.height,
            -1,
            Some(
                gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(rect.x as f32, rect.y as f32)),
            ),
        );
    }

    impl WidgetImpl for ColumnsView {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let mut minimum = 0;
            let mut natural = 0;
            for child in self.columns.borrow().iter() {
                let (min, nat, _, _) = child.measure(orientation, -1);
                minimum = minimum.max(min);
                natural = natural.max(nat);
            }
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let columns = self.columns.borrow();
            let plan = plan(
                self.layout.get(),
                width,
                height,
                columns.len(),
                self.offset.get(),
                self.wanted.get(),
            );
            self.offset.set(plan.visible.start);

            for (index, child) in columns.iter().enumerate() {
                let rect = index
                    .checked_sub(plan.visible.start)
                    .and_then(|i| plan.columns.get(i))
                    .filter(|_| plan.visible.contains(&index));
                match rect {
                    Some(rect) => place(child, *rect),
                    None => child.set_child_visible(false),
                }
            }

            let pane = self.pane.borrow().clone();
            let vertical = self.vertical_separator.borrow().clone();
            let horizontal = self.horizontal_separator.borrow().clone();
            if let (Some(pane), Some(vertical), Some(horizontal)) = (pane, vertical, horizontal) {
                match plan.pane {
                    Some((pane_rect, separator_rect)) if pane.get_visible() => {
                        let (shown, hidden) = if plan.below {
                            (&horizontal, &vertical)
                        } else {
                            (&vertical, &horizontal)
                        };
                        place(shown, separator_rect);
                        hidden.set_child_visible(false);
                        place(&pane, pane_rect);
                    }
                    _ => {
                        pane.set_child_visible(false);
                        vertical.set_child_visible(false);
                        horizontal.set_child_visible(false);
                    }
                }
            }

            // Prévenir quand les colonnes affichées changent, après
            // l'allocation (on ne modifie pas de widgets pendant celle-ci).
            if *self.visible.borrow() != plan.visible {
                *self.visible.borrow_mut() = plan.visible.clone();
                let view = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(view) = view.upgrade() {
                        let visible = view.imp().visible.borrow().clone();
                        if let Some(callback) = view.imp().on_visible_changed.borrow().as_ref() {
                            callback(visible);
                        }
                    }
                });
            }
        }
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
    /// Place le volet de discussion à côté des colonnes. Il n'est affiché
    /// qu'avec les dispositions `Right` et `Below`.
    pub fn set_pane(&self, pane: &impl IsA<gtk::Widget>) {
        let imp = self.imp();
        let vertical = gtk::Separator::new(gtk::Orientation::Vertical);
        let horizontal = gtk::Separator::new(gtk::Orientation::Horizontal);
        vertical.set_parent(self);
        horizontal.set_parent(self);
        pane.set_parent(self);
        imp.vertical_separator.replace(Some(vertical));
        imp.horizontal_separator.replace(Some(horizontal));
        imp.pane.replace(Some(pane.clone().upcast()));
    }

    pub fn set_layout(&self, layout: Layout) {
        if self.imp().layout.replace(layout) != layout {
            self.queue_resize();
        }
    }

    /// Appelle `f` avec les indices des colonnes affichées quand ils changent.
    pub fn connect_visible_changed(&self, f: impl Fn(Range<usize>) + 'static) {
        self.imp().on_visible_changed.replace(Some(Box::new(f)));
    }

    fn first_non_column(&self) -> Option<gtk::Widget> {
        self.imp()
            .vertical_separator
            .borrow()
            .clone()
            .map(|s| s.upcast())
    }

    pub fn append_column(&self, column: &impl IsA<gtk::Widget>) {
        // Les colonnes passent avant le volet dans l'ordre de tabulation.
        match self.first_non_column() {
            Some(separator) => column.insert_before(self, Some(&separator)),
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
            .or_else(|| self.first_non_column());
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

    #[test]
    fn pane_on_the_right_takes_the_remaining_width() {
        let p = plan(Layout::Right, 2320, 1000, 4, 0, 0);
        assert_eq!(p.visible, 0..4);
        let (pane, separator) = p.pane.unwrap();
        assert!(!p.below);
        assert!(pane.width >= PANE_MIN_WIDTH);
        assert_eq!(pane.x + pane.width, 2320);
        assert_eq!(separator.x + 1, pane.x);
        for column in &p.columns {
            assert!((COLUMN_MIN_WIDTH..=COLUMN_MAX_WIDTH).contains(&column.width));
        }
    }

    #[test]
    fn pane_below_spans_the_width_under_the_columns() {
        let p = plan(Layout::Below, 2320, 1000, 4, 0, 0);
        let (pane, separator) = p.pane.unwrap();
        assert!(p.below);
        assert_eq!((pane.x, pane.width), (0, 2320));
        assert_eq!(pane.y + pane.height, 1000);
        assert_eq!(separator.y + 1, pane.y);
        // Les colonnes, plus nombreuses sans volet à droite, s'arrêtent
        // au-dessus du séparateur.
        assert_eq!(p.visible, 0..4);
        assert!(p.columns.iter().all(|c| c.y + c.height < separator.y));
    }

    #[test]
    fn without_pane_on_the_right_columns_fill_the_width() {
        for layout in [Layout::Hidden, Layout::Below] {
            let p = plan(layout, 2320, 1000, 2, 0, 0);
            let first = p.columns[0];
            let last = p.columns[1];
            assert_eq!(first.x, SPACING);
            // À un pixel près (division entière).
            assert!((2320 - SPACING - (last.x + last.width)).abs() <= 1);
        }
        assert!(plan(Layout::Hidden, 2320, 1000, 2, 0, 0).pane.is_none());
    }

    #[test]
    fn layouts_fall_back_when_space_is_missing() {
        // Trop étroit pour une colonne et le volet à droite.
        let p = plan(Layout::Right, 700, 1000, 3, 0, 2);
        assert!(p.pane.is_none());
        assert_eq!(p.visible, 2..3);
        assert_eq!(p.columns[0].width, 700);
        // Trop bas pour le volet en dessous.
        let p = plan(Layout::Below, 2000, 400, 3, 0, 0);
        assert!(p.pane.is_none());
    }

    #[test]
    fn wanted_column_stays_visible() {
        let p = plan(Layout::Right, 1680, 1000, 4, 0, 3);
        assert_eq!(p.visible, 1..4);
        let p = plan(Layout::Right, 1680, 1000, 4, 1, 0);
        assert_eq!(p.visible, 0..3);
    }
}
