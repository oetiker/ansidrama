//! Screen-content predicates: a regex, optionally scoped to one row.

use anyhow::{Context, Result};
use regex_lite::Regex;

use crate::grid::Cell;

/// The grid as text: one line per row, trailing blanks trimmed.
pub fn screen_text(grid: &[Vec<Cell>]) -> String {
    let mut out = String::new();
    for (i, row) in grid.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line: String = row.iter().map(|c| c.ch).collect();
        out.push_str(line.trim_end());
    }
    out
}

/// A compiled screen predicate.
#[derive(Debug)]
pub struct Pattern {
    re: Regex,
    row: Option<i32>,
    source: String,
}

impl Pattern {
    pub fn new(find: &str, row: Option<i32>) -> Result<Pattern> {
        let re = Regex::new(find)
            .with_context(|| format!("compile await pattern {find:?}"))?;
        Ok(Pattern { re, row, source: find.to_string() })
    }

    pub fn matches(&self, grid: &[Vec<Cell>]) -> bool {
        match self.row {
            None => self.re.is_match(&screen_text(grid)),
            Some(r) => {
                let Some(idx) = resolve_row(r, grid.len()) else {
                    return false;
                };
                let line: String = grid[idx].iter().map(|c| c.ch).collect();
                self.re.is_match(line.trim_end())
            }
        }
    }

    pub fn row(&self) -> Option<i32> {
        self.row
    }

    /// The pattern as written, for error messages.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Resolve a possibly-negative row index against a screen of `rows` rows.
/// `-1` is the last row. Out of range yields `None`.
fn resolve_row(row: i32, rows: usize) -> Option<usize> {
    let rows = rows as i32;
    let idx = if row < 0 { rows + row } else { row };
    (idx >= 0 && idx < rows).then_some(idx as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char) -> Cell {
        Cell { ch, fg: (0, 0, 0), bg: (0, 0, 0), bold: false }
    }

    /// Build a grid from lines, padded to `cols`.
    fn grid(lines: &[&str], cols: usize) -> Vec<Vec<Cell>> {
        lines
            .iter()
            .map(|l| {
                let mut row: Vec<Cell> = l.chars().map(cell).collect();
                row.resize(cols, cell(' '));
                row
            })
            .collect()
    }

    #[test]
    fn matches_anywhere_on_screen() {
        let g = grid(&["hello world", "theme: light"], 20);
        assert!(Pattern::new("theme: light", None).unwrap().matches(&g));
        assert!(!Pattern::new("theme: dark", None).unwrap().matches(&g));
    }

    #[test]
    fn row_scoping_restricts_the_match() {
        let g = grid(&["theme: light", "nothing here"], 20);
        // present, but on row 0 — a row-1 scoped pattern must not see it
        assert!(Pattern::new("theme: light", Some(0)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(1)).unwrap().matches(&g));
    }

    #[test]
    fn negative_row_counts_from_the_bottom() {
        let g = grid(&["a", "b", "theme: light"], 20);
        assert!(Pattern::new("theme: light", Some(-1)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(-2)).unwrap().matches(&g));
    }

    #[test]
    fn out_of_range_row_never_matches() {
        let g = grid(&["theme: light"], 20);
        assert!(!Pattern::new("theme: light", Some(9)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(-9)).unwrap().matches(&g));
    }

    #[test]
    fn a_pattern_does_not_match_across_a_row_boundary() {
        // `.` must not cross the newline that separates rows.
        let g = grid(&["abc", "def"], 3);
        assert!(!Pattern::new("abc.def", None).unwrap().matches(&g));
    }

    #[test]
    fn trailing_blanks_are_trimmed_so_end_anchors_work() {
        let g = grid(&["done"], 40);
        assert!(Pattern::new("done$", Some(0)).unwrap().matches(&g));
    }

    #[test]
    fn a_bad_regex_is_an_error_not_a_panic() {
        assert!(Pattern::new("unclosed(", None).is_err());
    }
}
