//! Hierarchical menus drawn on the device: a static tree of [`Menu`]s, and [`Nav`], which
//! walks it with the button [`Event`]s.
//!
//! * Every submenu starts with a **Back** row, so a menu can be left with the buttons alone
//!   (Up held, [`Event::Back`], also goes back). The root has no Back row.
//! * Up/Down move the selection and wrap around. A long list is shown a page at a time;
//!   the page follows the selection.
//! * OK on an item opens its submenu or hands its action to the app
//!   ([`Outcome::Activate`]). Actions are the app's own type, typically an enum such as
//!   `Open(Screen)` or `Run(Function)`.
//!
//! [`Outcome`] tells the app what changed: a cursor move only changes two rows, opening or
//! leaving a menu changes the page. The drawing is in [`crate::view`]; how the change gets
//! to the panel is in [`crate::present`].
//!
//! ```
//! use zectrix_screen::menu::{Item, Menu, Nav, Outcome};
//! use zectrix_screen::Event;
//!
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! enum Act { Info, Sleep }
//!
//! static SYSTEM: Menu<Act> = Menu::new("System", &[Item::action("Sleep now", Act::Sleep)]);
//! static ROOT: Menu<Act> = Menu::new("Menu", &[
//!     Item::action("Device info", Act::Info),
//!     Item::submenu("System", &SYSTEM),
//! ]);
//!
//! let mut nav = Nav::new(&ROOT, 6);
//! assert_eq!(nav.handle(Event::Down), Outcome::Moved);
//! assert_eq!(nav.handle(Event::Select), Outcome::Opened); // System, Back row on top
//! assert_eq!(nav.handle(Event::Select), Outcome::Activate(Act::Sleep));
//! assert_eq!(nav.handle(Event::Back), Outcome::Closed);
//! ```

use core::ops::Range;

use crate::Event;

/// Deepest nesting, root included.
pub const MAX_DEPTH: usize = 8;

/// A menu: a title and its items, usually a `static`.
#[derive(Debug)]
pub struct Menu<A: 'static> {
    pub title: &'static str,
    pub items: &'static [Item<A>],
}

impl<A> Menu<A> {
    pub const fn new(title: &'static str, items: &'static [Item<A>]) -> Self {
        Self { title, items }
    }
}

#[derive(Debug)]
pub struct Item<A: 'static> {
    pub label: &'static str,
    pub kind: Kind<A>,
}

impl<A> Item<A> {
    /// An item whose OK hands `action` to the app.
    pub const fn action(label: &'static str, action: A) -> Self {
        Self {
            label,
            kind: Kind::Action(action),
        }
    }

    /// An item whose OK opens `menu`.
    pub const fn submenu(label: &'static str, menu: &'static Menu<A>) -> Self {
        Self {
            label,
            kind: Kind::Submenu(menu),
        }
    }
}

#[derive(Debug)]
pub enum Kind<A: 'static> {
    Submenu(&'static Menu<A>),
    Action(A),
}

/// One row of the current menu.
#[derive(Debug)]
pub enum Row<'a, A: 'static> {
    /// The Back row on top of a submenu.
    Back,
    Item(&'a Item<A>),
}

/// Rows are equal if they are the same row (items compare by identity, so actions need
/// not be comparable).
impl<A> PartialEq for Row<'_, A> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Row::Back, Row::Back) => true,
            (Row::Item(a), Row::Item(b)) => core::ptr::eq(*a, *b),
            _ => false,
        }
    }
}

/// What an event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome<A> {
    /// Nothing changed (e.g. Up in an empty menu).
    None,
    /// The selection moved on the same page: two rows changed.
    Moved,
    /// The selection moved to another page of the same menu.
    Paged,
    /// A submenu was opened.
    Opened,
    /// The menu was left, back to its parent.
    Closed,
    /// OK on an action item.
    Activate(A),
    /// The menu does not use this event (talk, off, or Back on the root menu).
    Unhandled(Event),
}

impl<A> Outcome<A> {
    /// The whole page changed (as opposed to two rows).
    pub fn page_changed(&self) -> bool {
        matches!(self, Outcome::Paged | Outcome::Opened | Outcome::Closed)
    }

    /// The menu needs to be drawn again.
    pub fn redraw(&self) -> bool {
        matches!(self, Outcome::Moved) || self.page_changed()
    }
}

#[derive(Debug)]
struct Level<A: 'static> {
    menu: &'static Menu<A>,
    /// Selected row, Back row included.
    sel: usize,
}

impl<A> Clone for Level<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for Level<A> {}

/// Where the user is in the menu tree.
#[derive(Debug)]
pub struct Nav<A: 'static> {
    stack: [Option<Level<A>>; MAX_DEPTH],
    depth: usize,
    rows_per_page: usize,
}

impl<A: Copy> Nav<A> {
    /// Start at `root`, showing `rows_per_page` rows at a time (at least 1; see
    /// [`crate::view::Theme::rows_per_page`]).
    pub fn new(root: &'static Menu<A>, rows_per_page: usize) -> Self {
        let mut stack = [None; MAX_DEPTH];
        stack[0] = Some(Level { menu: root, sel: 0 });
        Self {
            stack,
            depth: 1,
            rows_per_page: rows_per_page.max(1),
        }
    }

    /// Back to the root menu, first item selected.
    pub fn reset(&mut self) {
        let root = self.level_at(0).menu;
        *self = Self::new(root, self.rows_per_page);
    }

    fn level_at(&self, i: usize) -> Level<A> {
        self.stack[i].expect("levels below depth are set")
    }

    fn level(&self) -> Level<A> {
        self.level_at(self.depth - 1)
    }

    fn level_mut(&mut self) -> &mut Level<A> {
        self.stack[self.depth - 1]
            .as_mut()
            .expect("levels below depth are set")
    }

    /// The menu on screen.
    pub fn menu(&self) -> &'static Menu<A> {
        self.level().menu
    }

    /// 1 on the root menu.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Titles from the root to the current menu.
    pub fn path(&self) -> impl Iterator<Item = &'static str> + '_ {
        (0..self.depth).map(|i| self.level_at(i).menu.title)
    }

    fn has_back(&self) -> bool {
        self.depth > 1
    }

    /// Rows of the current menu, Back row included.
    pub fn len(&self) -> usize {
        self.menu().items.len() + usize::from(self.has_back())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn row(&self, i: usize) -> Option<Row<'static, A>> {
        let items = self.menu().items;
        if self.has_back() {
            match i {
                0 => Some(Row::Back),
                _ => items.get(i - 1).map(Row::Item),
            }
        } else {
            items.get(i).map(Row::Item)
        }
    }

    pub fn selected(&self) -> usize {
        self.level().sel
    }

    pub fn rows_per_page(&self) -> usize {
        self.rows_per_page
    }

    /// Page of the selection, from 0.
    pub fn page(&self) -> usize {
        self.selected() / self.rows_per_page
    }

    pub fn pages(&self) -> usize {
        self.len().div_ceil(self.rows_per_page).max(1)
    }

    /// The rows on the current page.
    pub fn visible(&self) -> Range<usize> {
        let first = self.page() * self.rows_per_page;
        first..(first + self.rows_per_page).min(self.len())
    }

    fn select(&mut self, sel: usize) -> Outcome<A> {
        let page = self.page();
        self.level_mut().sel = sel;
        if self.page() == page {
            Outcome::Moved
        } else {
            Outcome::Paged
        }
    }

    pub fn up(&mut self) -> Outcome<A> {
        let n = self.len();
        if n < 2 {
            return Outcome::None;
        }
        self.select((self.selected() + n - 1) % n)
    }

    pub fn down(&mut self) -> Outcome<A> {
        let n = self.len();
        if n < 2 {
            return Outcome::None;
        }
        self.select((self.selected() + 1) % n)
    }

    /// OK on the selected row.
    pub fn activate(&mut self) -> Outcome<A> {
        match self.row(self.selected()) {
            None => Outcome::None,
            Some(Row::Back) => self.back(),
            Some(Row::Item(item)) => match item.kind {
                Kind::Action(a) => Outcome::Activate(a),
                Kind::Submenu(menu) => self.open(menu),
            },
        }
    }

    /// Open `menu` as a submenu of the current one, its first item selected (not Back).
    /// Returns [`Outcome::None`] at [`MAX_DEPTH`].
    pub fn open(&mut self, menu: &'static Menu<A>) -> Outcome<A> {
        if self.depth == MAX_DEPTH {
            return Outcome::None;
        }
        let sel = usize::from(!menu.items.is_empty());
        self.stack[self.depth] = Some(Level { menu, sel });
        self.depth += 1;
        Outcome::Opened
    }

    /// Leave the current submenu; the parent keeps its selection. On the root menu this is
    /// [`Outcome::Unhandled`]`(`[`Event::Back`]`)`, so the app can leave the menu itself.
    pub fn back(&mut self) -> Outcome<A> {
        if self.depth == 1 {
            return Outcome::Unhandled(Event::Back);
        }
        self.depth -= 1;
        self.stack[self.depth] = None;
        Outcome::Closed
    }

    pub fn handle(&mut self, event: Event) -> Outcome<A> {
        match event {
            Event::Up => self.up(),
            Event::Down => self.down(),
            Event::Select => self.activate(),
            Event::Back => self.back(),
            other => Outcome::Unhandled(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum A {
        One,
        Two,
        Deep,
        N(u8),
    }

    static DEEP: Menu<A> = Menu::new("Deep", &[Item::action("deep", A::Deep)]);
    static SUB: Menu<A> = Menu::new(
        "Sub",
        &[Item::action("two", A::Two), Item::submenu("deeper", &DEEP)],
    );
    static EMPTY: Menu<A> = Menu::new("Empty", &[]);
    static LONG: Menu<A> = Menu::new(
        "Long",
        &[
            Item::action("0", A::N(0)),
            Item::action("1", A::N(1)),
            Item::action("2", A::N(2)),
            Item::action("3", A::N(3)),
            Item::action("4", A::N(4)),
        ],
    );
    static ROOT: Menu<A> = Menu::new(
        "Root",
        &[
            Item::action("one", A::One),
            Item::submenu("sub", &SUB),
            Item::submenu("empty", &EMPTY),
            Item::submenu("long", &LONG),
        ],
    );

    #[test]
    fn root_has_no_back_row_and_wraps() {
        let mut nav = Nav::new(&ROOT, 6);
        assert_eq!(nav.len(), 4);
        assert!(matches!(nav.row(0), Some(Row::Item(i)) if i.label == "one"));
        assert_eq!(nav.up(), Outcome::Moved);
        assert_eq!(nav.selected(), 3);
        assert_eq!(nav.down(), Outcome::Moved);
        assert_eq!(nav.selected(), 0);
        assert_eq!(nav.activate(), Outcome::Activate(A::One));
        assert_eq!(nav.back(), Outcome::Unhandled(Event::Back));
    }

    #[test]
    fn submenu_back_row_on_top_first_item_selected() {
        let mut nav = Nav::new(&ROOT, 6);
        nav.down();
        assert_eq!(nav.activate(), Outcome::Opened);
        assert_eq!(nav.depth(), 2);
        assert_eq!(nav.row(0), Some(Row::Back));
        assert_eq!(nav.selected(), 1);
        assert_eq!(nav.len(), 3);
        assert_eq!(nav.activate(), Outcome::Activate(A::Two));
        assert!(nav.path().eq(["Root", "Sub"]));

        // OK on Back closes; the parent keeps its selection.
        nav.up();
        assert_eq!(nav.selected(), 0);
        assert_eq!(nav.activate(), Outcome::Closed);
        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.selected(), 1);
    }

    #[test]
    fn nesting_and_back_event() {
        let mut nav = Nav::new(&ROOT, 6);
        nav.down();
        nav.activate();
        nav.down();
        assert_eq!(nav.activate(), Outcome::Opened);
        assert!(nav.path().eq(["Root", "Sub", "Deep"]));
        assert_eq!(nav.handle(Event::Select), Outcome::Activate(A::Deep));
        assert_eq!(nav.handle(Event::Back), Outcome::Closed);
        assert_eq!(nav.handle(Event::Back), Outcome::Closed);
        assert_eq!(nav.menu().title, "Root");
        assert_eq!(
            nav.handle(Event::TalkStart),
            Outcome::Unhandled(Event::TalkStart)
        );
    }

    #[test]
    fn empty_submenu_is_just_back() {
        let mut nav = Nav::new(&ROOT, 6);
        nav.down();
        nav.down();
        nav.activate();
        assert_eq!(nav.len(), 1);
        assert_eq!(nav.selected(), 0);
        assert_eq!(nav.down(), Outcome::None);
        assert_eq!(nav.activate(), Outcome::Closed);
    }

    #[test]
    fn pages_follow_the_selection() {
        let mut nav = Nav::new(&ROOT, 6);
        nav.up();
        nav.activate(); // Long: Back + 5 items, 4 per page
        let mut nav = Nav {
            rows_per_page: 4,
            ..nav
        };
        assert_eq!(nav.pages(), 2);
        assert_eq!(nav.visible(), 0..4);
        assert_eq!(nav.down(), Outcome::Moved); // row 2
        assert_eq!(nav.down(), Outcome::Moved); // row 3
        assert_eq!(nav.down(), Outcome::Paged); // row 4, page 2
        assert_eq!(nav.visible(), 4..6);
        assert_eq!(nav.down(), Outcome::Moved);
        assert_eq!(nav.down(), Outcome::Paged); // wraps to Back
        assert_eq!(nav.page(), 0);
        assert!(Outcome::<A>::Paged.page_changed());
        assert!(!Outcome::<A>::Moved.page_changed());
    }

    #[test]
    fn depth_limit_and_reset() {
        static SELF: Menu<A> = Menu::new("Self", &[Item::submenu("again", &SELF)]);
        let mut nav = Nav::new(&SELF, 6);
        for _ in 1..MAX_DEPTH {
            assert_eq!(nav.activate(), Outcome::Opened);
        }
        assert_eq!(nav.depth(), MAX_DEPTH);
        assert_eq!(nav.activate(), Outcome::None);
        nav.reset();
        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.selected(), 0);
    }
}
