//! Maps text that preprocessing and parsers rewrite back to the lines its author wrote.
//!
//! Before a diagram is parsed, front matter, directives, comment lines, and some blank lines are
//! removed. Each removal is a list of byte ranges, so a token offset in the rewritten text still
//! resolves to an authored line, which the `sourceLines` option writes to the SVG.

use std::ops::Range;

use crate::error::push_u32;

/// The authored line of each line of a rewritten text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lines {
    /// One-based authored line numbers, one for each line of the current text.
    authored: Vec<u32>,
}

impl Lines {
    /// The lines of authored `text`, whose line endings are already `\n`.
    pub(crate) fn new(text: &str) -> Self {
        let count = text.matches('\n').count() + 1;
        Self {
            authored: (1..=u32::try_from(count).unwrap_or(u32::MAX)).collect(),
        }
    }

    /// Follows the removal of `removed` from `text`, the current text. The ranges are in order
    /// and do not overlap. A line of the result takes the authored line of its first kept byte, so
    /// a line that joins two lines belongs to the first.
    pub(crate) fn remove(&mut self, text: &str, removed: &[Range<usize>]) {
        if removed.is_empty() {
            return;
        }

        let bytes = text.as_bytes();
        let mut authored = Vec::with_capacity(self.authored.len());
        let mut ranges = removed.iter().peekable();
        let mut at = 0;
        let mut line = 0;
        let mut starts = true;
        while let Some(&byte) = bytes.get(at) {
            if let Some(range) = ranges.next_if(|range| range.start <= at) {
                debug_assert!(range.start == at, "removed ranges are in order");
                let end = range.end.clamp(at, bytes.len());
                line += text.get(at..end).map_or(0, |cut| cut.matches('\n').count());
                at = end;
                continue;
            }

            if starts {
                authored.push(self.at(line));
                starts = false;
            }
            if byte == b'\n' {
                line += 1;
                starts = true;
            }
            at += 1;
        }
        if starts {
            authored.push(self.at(line));
        }

        self.authored = authored;
    }

    /// Finds the authored lines of byte offsets in `text`, the current text.
    pub(crate) fn locate(self, text: &str) -> Locator {
        let mut starts = Vec::with_capacity(self.authored.len());
        starts.push(0);
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Locator {
            lines: self,
            starts,
        }
    }

    /// The authored line of line `index`. Text past the last line belongs to the last line.
    fn at(&self, index: usize) -> u32 {
        self.authored
            .get(index)
            .or(self.authored.last())
            .copied()
            .unwrap_or(1)
    }
}

/// Finds authored lines for byte offsets of one rewritten text.
#[derive(Clone, Debug)]
pub(crate) struct Locator {
    /// The authored line of each line of the text.
    lines: Lines,
    /// The byte offset where each line of the text starts.
    starts: Vec<usize>,
}

impl Locator {
    /// The one-based authored line of the byte at `offset`.
    pub(crate) fn line(&self, offset: usize) -> u32 {
        let index = self
            .starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        self.lines.at(index)
    }
}

/// Adds `line` to `lines`, which stay ascending and hold each line once.
pub(crate) fn mark(lines: &mut Vec<u32>, line: u32) {
    if let Err(at) = lines.binary_search(&line) {
        lines.insert(at, line);
    }
}

/// Writes ` data-lines="…"`, the space-separated `lines`, or nothing when there are none. A space
/// list lets a selector such as `[data-lines~="5"]` find every element written on line 5.
pub(crate) fn push_attribute(out: &mut String, lines: &[u32]) {
    let Some((first, rest)) = lines.split_first() else {
        return;
    };

    out.push_str(" data-lines=\"");
    push_u32(out, *first);
    for &line in rest {
        out.push(' ');
        push_u32(out, line);
    }
    out.push('"');
}

/// `text` without the byte ranges `removed`, which are in order and do not overlap.
pub(crate) fn without(text: &str, removed: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in removed {
        out.push_str(text.get(last..range.start).unwrap_or(""));
        last = range.end;
    }
    out.push_str(text.get(last..).unwrap_or(""));
    out
}
