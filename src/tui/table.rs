//! Aligned picker columns, laid out once per frame by terminal width.

use super::state::Tone;
use crate::term::{text_width, truncate};

/// Styled text: pieces of one line, each with its own tone.
pub type Styled = Vec<(String, Tone)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub title: &'static str,
    pub align: Align,
    /// When a row does not fit, the column with the highest rank drops first.
    /// Column 0 (the item name) never drops.
    pub drop_rank: u8,
}

#[derive(Debug, Clone)]
pub struct PickTable {
    pub columns: Vec<Column>,
    /// One row per picker item, one cell per column. `None` means no data
    /// and draws a dim dash; `Some` of nothing draws blank.
    pub cells: Vec<Vec<Option<Styled>>>,
    /// Lines shown under the list for the highlighted item.
    pub details: Vec<Vec<Styled>>,
    pub legend: Styled,
    pub keys: &'static str,
}

pub const MISSING: &str = "—";
const GAP: usize = 2;
const MARKER: usize = 2;
/// The name column keeps at least this many columns before others drop.
pub const NAME_MIN: usize = 24;

fn width_of(styled: &Styled) -> usize {
    styled.iter().map(|(t, _)| text_width(t)).sum()
}

fn cell_width(cell: &Option<Styled>) -> usize {
    cell.as_ref().map_or(text_width(MISSING), width_of)
}

/// Width of each column at this terminal width, `None` for a dropped column.
pub fn layout(table: &PickTable, width: usize) -> Vec<Option<usize>> {
    let natural: Vec<usize> = table
        .columns
        .iter()
        .enumerate()
        .map(|(c, col)| {
            table
                .cells
                .iter()
                .map(|row| row.get(c).map_or(1, cell_width))
                .max()
                .unwrap_or(0)
                .max(text_width(col.title))
        })
        .collect();
    let mut kept: Vec<bool> = vec![true; natural.len()];
    let room = width.saturating_sub(MARKER);
    let name_room = |kept: &[bool]| {
        let others: usize = (1..natural.len())
            .filter(|&c| kept[c])
            .map(|c| natural[c] + GAP)
            .sum();
        room.saturating_sub(others)
    };
    let name_min = NAME_MIN.min(natural.first().copied().unwrap_or(0));
    while name_room(&kept) < name_min {
        let Some(drop) = (1..natural.len())
            .filter(|&c| kept[c])
            .max_by_key(|&c| (table.columns[c].drop_rank, c))
        else {
            break;
        };
        kept[drop] = false;
    }
    let name = natural.first().copied().unwrap_or(0).min(name_room(&kept));
    natural
        .iter()
        .enumerate()
        .map(|(c, w)| match c {
            0 => Some(name),
            _ if kept[c] => Some(*w),
            _ => None,
        })
        .collect()
}

fn pad(cell: Styled, width: usize, align: Align) -> Styled {
    let fill = " ".repeat(width.saturating_sub(width_of(&cell)));
    let mut out = Vec::with_capacity(cell.len() + 1);
    if align == Align::Right && !fill.is_empty() {
        out.push((fill.clone(), Tone::Normal));
    }
    out.extend(cell);
    if align == Align::Left && !fill.is_empty() {
        out.push((fill, Tone::Normal));
    }
    out
}

/// One row as styled pieces, without the selection marker. The first piece
/// is always the name cell.
pub fn row(table: &PickTable, item: usize, widths: &[Option<usize>]) -> Styled {
    let mut out = Styled::new();
    for (c, w) in widths.iter().enumerate() {
        let Some(w) = *w else { continue };
        let cell = match table.cells[item].get(c).cloned().flatten() {
            None => vec![(MISSING.to_string(), Tone::Muted)],
            Some(cell) if c == 0 && !cell.is_empty() => {
                vec![(truncate(&plain(&cell), w), cell[0].1)]
            }
            Some(cell) => cell,
        };
        if c > 0 {
            out.push((" ".repeat(GAP), Tone::Normal));
        }
        out.extend(pad(cell, w, table.columns[c].align));
    }
    out
}

/// Column titles laid out like the rows.
pub fn header(table: &PickTable, widths: &[Option<usize>]) -> String {
    let mut out = String::new();
    for (c, w) in widths.iter().enumerate() {
        let Some(w) = *w else { continue };
        if c > 0 {
            out.push_str(&" ".repeat(GAP));
        }
        let title = vec![(table.columns[c].title.to_string(), Tone::Muted)];
        out.push_str(&plain(&pad(title, w, table.columns[c].align)));
    }
    out.trim_end().to_string()
}

pub fn plain(styled: &Styled) -> String {
    styled.iter().map(|(t, _)| t.as_str()).collect()
}

/// Cut styled pieces to `width` display columns, ending in `…` when cut.
pub fn fit(styled: &Styled, width: usize) -> Styled {
    let total = width_of(styled);
    if total <= width {
        return styled.clone();
    }
    let mut out = Styled::new();
    let mut left = width;
    for (text, tone) in styled {
        let w = text_width(text);
        if w < left {
            out.push((text.clone(), *tone));
            left -= w;
        } else {
            out.push((truncate(text, left), *tone));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &str) -> Option<Styled> {
        Some(vec![(t.to_string(), Tone::Normal)])
    }

    fn table() -> PickTable {
        let col = |title, align, drop_rank| Column {
            title,
            align,
            drop_rank,
        };
        PickTable {
            columns: vec![
                col("model", Align::Left, 0),
                col("price", Align::Right, 1),
                col("ctx", Align::Right, 2),
                col("tags", Align::Left, 3),
            ],
            cells: vec![
                vec![s("a/short"), s("$1"), s("1M"), s("free new")],
                vec![
                    s("b/a-much-longer-model-id-here"),
                    s("$12.50"),
                    None,
                    Some(vec![]),
                ],
            ],
            details: vec![vec![], vec![]],
            legend: Styled::new(),
            keys: "",
        }
    }

    fn rows(t: &PickTable, width: usize) -> Vec<String> {
        let widths = layout(t, width);
        (0..t.cells.len())
            .map(|i| plain(&row(t, i, &widths)))
            .collect()
    }

    #[test]
    fn columns_start_at_the_same_offset_on_every_row() {
        // WHY: a price under the wrong column header reads as the wrong price.
        let t = table();
        let widths = layout(&t, 80);
        let head = header(&t, &widths);
        let lines = rows(&t, 80);
        assert_eq!(
            lines[0].find("$1").unwrap() + 2,
            lines[1].find("$12.50").unwrap() + 6
        );
        assert_eq!(
            head.find("price").unwrap() + 5,
            lines[1].find("$12.50").unwrap() + 6
        );
        assert_eq!(lines[0].find("1M"), head.find("ctx").map(|i| i + 1));
        assert_eq!(text_width(&lines[0]), text_width(&lines[1]));
    }

    #[test]
    fn missing_data_draws_a_dim_dash() {
        let t = table();
        let widths = layout(&t, 80);
        let r = row(&t, 1, &widths);
        let dashes: Vec<_> = r.iter().filter(|(text, _)| text == MISSING).collect();
        assert_eq!(dashes.len(), 1, "no tags is blank, not missing: {r:?}");
        assert!(dashes.iter().all(|(_, tone)| *tone == Tone::Muted), "{r:?}");
    }

    #[test]
    fn narrow_widths_drop_the_highest_rank_first_and_keep_the_name() {
        let t = table();
        assert!(layout(&t, 80).iter().all(Option::is_some));
        let w = layout(&t, 40);
        assert_eq!(w[3], None, "tags drop first: {w:?}");
        assert!(w[1].is_some(), "price outlives tags: {w:?}");
        let w = layout(&t, 30);
        assert_eq!(&w[1..], &[None, None, None], "{w:?}");
        for width in [30, 40, 60, 80, 120] {
            for line in rows(&t, width) {
                assert!(text_width(&line) + MARKER <= width, "{width}: {line:?}");
            }
        }
    }

    #[test]
    fn fit_cuts_styled_pieces_to_the_width() {
        let styled = vec![
            ("abc".to_string(), Tone::Muted),
            ("defgh".to_string(), Tone::Good),
        ];
        assert_eq!(plain(&fit(&styled, 20)), "abcdefgh");
        let cut = fit(&styled, 5);
        assert_eq!(plain(&cut), "abcd…");
        assert_eq!(cut[1].1, Tone::Good);
    }
}
