//! Text widths. By default every glyph advances by a fixed fraction of the font size, so widths
//! need no font tables, canvas, or DOM, and match in every host.
//!
//! A host may supply an advance table for a proportional font as render options. `render` installs
//! it with [`scope`] for the measurements of one render and removes it when the render returns.
//! The table is plain input data, so the same source and options still give the same widths in
//! every host. The table lives in a thread-local slot rather than a parameter, since layout reads
//! widths from dozens of helpers that take no render context.

use std::cell::RefCell;

/// Bold text, as a style bit.
pub(crate) const BOLD: u8 = 1;
/// Italic text, as a style bit.
pub(crate) const ITALIC: u8 = 2;
/// Monospace text, as a style bit. It does not change widths.
pub(crate) const MONO: u8 = 4;

/// Advance of an ordinary glyph, in thousandths of an em.
const ADVANCE: u32 = 600;
/// The number of printable ASCII characters, U+0020 through U+007E, that an advance table lists.
pub(crate) const ASCII: usize = 95;
/// Advance of a wide (East Asian or emoji) glyph, in thousandths of an em.
const WIDE: u32 = 1200;

/// Advances that a host supplies for its font, in thousandths of an em. Zero-width and control
/// characters stay zero, and wide characters keep their class.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Metrics {
    /// Advances of printable ASCII in code point order, with [`ASCII`] entries.
    pub(crate) advances: [f64; ASCII],
    /// Advances of printable ASCII in bold text, with [`ASCII`] entries. `advances` applies to bold
    /// text when absent.
    pub(crate) bold: Option<[f64; ASCII]>,
    /// Advance of every other character that is not wide or zero-width.
    pub(crate) other: f64,
    /// Advance of a wide character.
    pub(crate) wide: f64,
}

impl Metrics {
    /// The width of `text` in `style`, in thousandths of an em.
    fn units(&self, text: &str, style: u8) -> f64 {
        let table = match &self.bold {
            Some(bold) if style & BOLD != 0 => bold,
            _ => &self.advances,
        };

        let mut units = 0.0;
        for c in text.chars() {
            let c = u32::from(c);
            if c < 32 || is_zero_width(c) {
                continue;
            }

            units += if is_wide(c) {
                self.wide
            } else {
                usize::try_from(c - 32)
                    .ok()
                    .and_then(|at| table.get(at))
                    .copied()
                    .unwrap_or(self.other)
            };
        }
        units
    }
}

thread_local! {
    /// The advance table of the render in progress, or none for uniform advances.
    static METRICS: RefCell<Option<Metrics>> = const { RefCell::new(None) };
}

/// Removes the installed advance table when dropped.
pub(crate) struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        install(None);
    }
}

/// Installs `metrics` for every width measured until the returned guard drops. `None` measures
/// with uniform advances. Renders do not nest, so the guard restores uniform advances.
pub(crate) fn scope(metrics: Option<Box<Metrics>>) -> Scope {
    install(metrics.map(|metrics| *metrics));
    Scope
}

fn install(metrics: Option<Metrics>) {
    // A slot that cannot be reached, during thread teardown, measures with uniform advances.
    let _ = METRICS.try_with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            *slot = metrics;
        }
    });
}

/// The width of `text` at `size` pixels. Without an installed advance table, `style` is accepted
/// for parity with Pele's measurer and does not change monospace widths. With one, bold text reads
/// its bold advances.
pub(crate) fn width(text: &str, size: f64, style: u8) -> f64 {
    let custom = METRICS
        .try_with(|slot| match slot.try_borrow().as_deref() {
            Ok(Some(metrics)) => Some(metrics.units(text, style)),
            _ => None,
        })
        .unwrap_or(None);
    if let Some(units) = custom {
        return units * size / 1000.0;
    }

    let mut units: u32 = 0;
    for c in text.chars() {
        let c = u32::from(c);
        if c < 32 || is_zero_width(c) {
            continue;
        }
        units = units.saturating_add(if is_wide(c) { WIDE } else { ADVANCE });
    }
    f64::from(units) * size / 1000.0
}

// Pele tests UTF-16 units and counts a surrogate pair as one wide unit and one zero-width unit,
// so every supplementary character is wide here.
fn is_wide(c: u32) -> bool {
    (0x1100..=0x115f).contains(&c)
        || (0x2e80..=0xa4cf).contains(&c)
        || (0xac00..=0xd7a3).contains(&c)
        || (0xf900..=0xfaff).contains(&c)
        || (0xfe30..=0xfe4f).contains(&c)
        || (0xff00..=0xff60).contains(&c)
        || (0xffe0..=0xffe6).contains(&c)
        || c > 0xffff
}

fn is_zero_width(c: u32) -> bool {
    (0x300..=0x36f).contains(&c)
        || (0x200b..=0x200f).contains(&c)
        || (0xfe00..=0xfe0f).contains(&c)
        || c == 0xfeff
}
