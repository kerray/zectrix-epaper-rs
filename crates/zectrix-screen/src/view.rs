//! Drawing menus and simple screens with `embedded-graphics`.
//!
//! The layout is a title bar on top and fixed-height rows below it. The selected row is
//! inverted (white on black), which looked better on the NOTE4 than a frame around it.
//! [`draw_menu`] draws the whole menu every time: the panel side
//! ([`crate::present`]) compares the frame with what is on the panel and refreshes only
//! the part that changed, so the app never has to track rows.
//!
//! The text style is generic, so any font that implements `embedded-graphics`'
//! `TextRenderer` and `CharacterStyle` works: `MonoTextStyle` (ASCII only), or a font crate
//! with Latin-2 glyphs for Czech labels.

use core::fmt::{self, Write as _};

use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{
        Alignment, Baseline, Text, TextStyleBuilder,
        renderer::{CharacterStyle, TextRenderer},
    },
};

use crate::menu::{Kind, Nav, Row};

/// Sizes and the font. Colours are fixed: ink is [`BinaryColor::On`].
#[derive(Clone, Debug)]
pub struct Theme<S> {
    /// Text style for titles and rows; its text colour is replaced as needed.
    pub text: S,
    /// The part of the display to use, normally the whole 400×300 panel.
    pub area: Rectangle,
    pub title_h: u32,
    pub row_h: u32,
    /// Left and right text margin.
    pub pad: u32,
    /// Label of the Back row.
    pub back: &'static str,
    /// Drawn after the label of an item that opens a submenu.
    pub submenu_mark: &'static str,
}

impl<S> Theme<S> {
    /// A theme for the 400×300 panel: 40 px title bar and 6 rows of 40 px for a font about
    /// 20 px high.
    pub fn note4(text: S) -> Self {
        Self {
            text,
            area: Rectangle::new(Point::zero(), Size::new(400, 300)),
            title_h: 40,
            row_h: 40,
            pad: 10,
            back: "< Back",
            submenu_mark: ">",
        }
    }

    /// Rows that fit below the title bar.
    pub fn rows_per_page(&self) -> usize {
        let h = self.area.size.height.saturating_sub(self.title_h);
        (h / self.row_h.max(1)).max(1) as usize
    }

    /// The area below the title bar, for screens that draw their own content.
    pub fn body(&self) -> Rectangle {
        let top = self.area.top_left + Point::new(0, self.title_h as i32);
        Rectangle::new(
            top,
            Size::new(
                self.area.size.width,
                self.area.size.height.saturating_sub(self.title_h),
            ),
        )
    }

    fn row_rect(&self, slot: usize) -> Rectangle {
        let y = self.body().top_left.y + (slot as u32 * self.row_h) as i32;
        Rectangle::new(
            Point::new(self.area.top_left.x, y),
            Size::new(self.area.size.width, self.row_h),
        )
    }
}

/// Short text built on the stack, for row values and titles with numbers.
pub type Label = heapless::String<48>;

/// Clear the area and draw a title bar: `title` on the left, `right` (e.g. a page counter
/// or the time) on the right, and a rule below.
pub fn draw_title<D, S>(
    target: &mut D,
    theme: &Theme<S>,
    title: &str,
    right: Option<&str>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
    S: TextRenderer<Color = BinaryColor> + CharacterStyle<Color = BinaryColor> + Clone,
{
    theme
        .area
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
        .draw(target)?;
    let bar = Rectangle::new(
        theme.area.top_left,
        Size::new(theme.area.size.width, theme.title_h),
    );
    let mid = bar.top_left.y + bar.size.height as i32 / 2;
    let ink = with_color(&theme.text, BinaryColor::On);
    left_text(target, title, bar.top_left.x + theme.pad as i32, mid, &ink)?;
    if let Some(right) = right {
        let x = bar.top_left.x + bar.size.width as i32 - theme.pad as i32;
        right_text(target, right, x, mid, &ink)?;
    }
    Rectangle::new(
        Point::new(bar.top_left.x, bar.top_left.y + bar.size.height as i32 - 3),
        Size::new(bar.size.width, 2),
    )
    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
    .draw(target)
}

/// Draw the current menu of `nav`: title bar (with "page/pages" if there is more than one
/// page) and the rows of the current page.
///
/// `value` may write a short text for an action item (a light's state, a setting); it is
/// drawn right-aligned in the row. Pass `|_, _| Ok(())` for none.
pub fn draw_menu<A, D, S>(
    target: &mut D,
    theme: &Theme<S>,
    nav: &Nav<A>,
    mut value: impl FnMut(&A, &mut Label) -> fmt::Result,
) -> Result<(), D::Error>
where
    A: Copy,
    D: DrawTarget<Color = BinaryColor>,
    S: TextRenderer<Color = BinaryColor> + CharacterStyle<Color = BinaryColor> + Clone,
{
    let mut pages = Label::new();
    if nav.pages() > 1 {
        let _ = write!(pages, "{}/{}", nav.page() + 1, nav.pages());
    }
    draw_title(
        target,
        theme,
        nav.menu().title,
        (!pages.is_empty()).then_some(pages.as_str()),
    )?;

    let ink = with_color(&theme.text, BinaryColor::On);
    let paper = with_color(&theme.text, BinaryColor::Off);
    for (slot, i) in nav.visible().enumerate() {
        let Some(row) = nav.row(i) else { break };
        let rect = theme.row_rect(slot);
        let selected = i == nav.selected();
        let style = if selected {
            rect.into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(target)?;
            &paper
        } else {
            &ink
        };
        let mid = rect.top_left.y + rect.size.height as i32 / 2;
        let left = rect.top_left.x + theme.pad as i32;
        let right = rect.top_left.x + rect.size.width as i32 - theme.pad as i32;
        match row {
            Row::Back => {
                left_text(target, theme.back, left, mid, style)?;
            }
            Row::Item(item) => {
                left_text(target, item.label, left, mid, style)?;
                match &item.kind {
                    Kind::Submenu(_) => {
                        right_text(target, theme.submenu_mark, right, mid, style)?;
                    }
                    Kind::Action(a) => {
                        let mut v = Label::new();
                        if value(a, &mut v).is_ok() && !v.is_empty() {
                            right_text(target, &v, right, mid, style)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Draw `lines` below the title bar, one per row, for simple information screens.
pub fn draw_lines<'a, D, S>(
    target: &mut D,
    theme: &Theme<S>,
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
    S: TextRenderer<Color = BinaryColor> + CharacterStyle<Color = BinaryColor> + Clone,
{
    let ink = with_color(&theme.text, BinaryColor::On);
    let rows = theme.rows_per_page();
    for (slot, line) in lines.into_iter().take(rows).enumerate() {
        let rect = theme.row_rect(slot);
        let mid = rect.top_left.y + rect.size.height as i32 / 2;
        left_text(target, line, rect.top_left.x + theme.pad as i32, mid, &ink)?;
    }
    Ok(())
}

fn with_color<S: CharacterStyle<Color = BinaryColor> + Clone>(s: &S, c: BinaryColor) -> S {
    let mut s = s.clone();
    s.set_text_color(Some(c));
    s.set_background_color(None);
    s
}

fn left_text<D, S>(target: &mut D, s: &str, x: i32, mid: i32, style: &S) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
    S: TextRenderer<Color = BinaryColor> + Clone,
{
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Left)
        .baseline(Baseline::Middle)
        .build();
    Text::with_text_style(s, Point::new(x, mid), style.clone(), layout).draw(target)?;
    Ok(())
}

fn right_text<D, S>(target: &mut D, s: &str, x: i32, mid: i32, style: &S) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
    S: TextRenderer<Color = BinaryColor> + Clone,
{
    let layout = TextStyleBuilder::new()
        .alignment(Alignment::Right)
        .baseline(Baseline::Middle)
        .build();
    Text::with_text_style(s, Point::new(x, mid), style.clone(), layout).draw(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use embedded_graphics::mono_font::{MonoTextStyle, ascii::FONT_10X20};
    use ssd2683::MonoFrame;

    use super::*;
    use crate::menu::{Item, Menu};

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum A {
        Light,
        Info,
    }

    static SUB: Menu<A> = Menu::new("Sub", &[Item::action("Info", A::Info)]);
    static ROOT: Menu<A> = Menu::new(
        "Menu",
        &[Item::action("Light", A::Light), Item::submenu("Sub", &SUB)],
    );

    fn theme() -> Theme<MonoTextStyle<'static, BinaryColor>> {
        Theme::note4(MonoTextStyle::new(&FONT_10X20, BinaryColor::On))
    }

    fn render(nav: &Nav<A>) -> MonoFrame {
        let mut f = MonoFrame::new();
        draw_menu(&mut f, &theme(), nav, |a, out| match a {
            A::Light => out.write_str("on"),
            A::Info => Ok(()),
        })
        .unwrap();
        f
    }

    #[test]
    fn layout_fits_six_rows() {
        let t = theme();
        assert_eq!(t.rows_per_page(), 6);
        assert_eq!(
            t.body(),
            Rectangle::new(Point::new(0, 40), Size::new(400, 260))
        );
    }

    #[test]
    fn selected_row_is_inverted() {
        let nav = Nav::new(&ROOT, 6);
        let f = render(&nav);
        // Row 0 (y 40..80) is selected: its left margin is ink; row 1's is paper.
        assert_eq!(f.get(2, 45), Some(BinaryColor::On));
        assert_eq!(f.get(2, 85), Some(BinaryColor::Off));
        // The title rule.
        assert_eq!(f.get(200, 38), Some(BinaryColor::On));
    }

    #[test]
    fn a_move_changes_only_the_two_rows() {
        let mut nav = Nav::new(&ROOT, 6);
        let before = render(&nav);
        nav.down();
        let after = render(&nav);
        let dirty = after.dirty_rect(&before).unwrap();
        assert_eq!((dirty.y, dirty.h), (40, 80));
    }

    #[test]
    fn opening_a_submenu_redraws_the_page_with_back_on_top() {
        let mut nav = Nav::new(&ROOT, 6);
        nav.down();
        let before = render(&nav);
        nav.activate();
        let after = render(&nav);
        let dirty = after.dirty_rect(&before).unwrap();
        assert!(dirty.y < 40, "title changed");
        // Row 0 is Back (not selected), row 1 is Info (selected).
        assert_eq!(after.get(2, 45), Some(BinaryColor::Off));
        assert_eq!(after.get(2, 85), Some(BinaryColor::On));
    }
}
