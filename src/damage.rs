//! Owned terminal damage, shared with the live renderer and preview preparation.

use alacritty_terminal::term::{LineDamageBounds, TermDamage};

#[derive(Clone, Debug, Default)]
pub struct Damage {
    pub full: bool,
    pub lines: Vec<LineDamageBounds>,
}

impl Damage {
    pub fn full() -> Self {
        Self {
            full: true,
            lines: Vec::new(),
        }
    }

    pub fn collect(damage: TermDamage<'_>) -> Self {
        match damage {
            TermDamage::Full => Self::full(),
            TermDamage::Partial(lines) => Self {
                full: false,
                lines: lines.collect(),
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.full && self.lines.is_empty()
    }

    pub fn merge(&mut self, other: &Self) {
        if self.full {
            return;
        }
        if other.full {
            *self = Self::full();
            return;
        }
        for line in &other.lines {
            if let Some(old) = self.lines.iter_mut().find(|old| old.line == line.line) {
                old.left = old.left.min(line.left);
                old.right = old.right.max(line.right);
            } else {
                self.lines.push(*line);
            }
        }
        self.lines.sort_unstable_by_key(|line| line.line);
    }

    pub fn add_line(&mut self, line: usize) {
        self.merge(&Self {
            full: false,
            lines: vec![LineDamageBounds::new(line, 0, 0)],
        });
    }

    /// Shape complete runs on damaged rows, including neighboring rows for
    /// glyph overhang. Ligatures and wide characters can cross column bounds.
    pub fn paints_row(&self, row: u32) -> bool {
        self.full
            || self
                .lines
                .iter()
                .any(|line| line.line.abs_diff(row as usize) <= 1)
    }

    /// Include bordering runs whose glyphs can overhang into the paint bands.
    pub fn shapes_row(&self, row: u32) -> bool {
        self.full
            || self
                .lines
                .iter()
                .any(|line| line.line.abs_diff(row as usize) <= 2)
    }

    /// Disjoint, coalesced scissors. Full-width row bands preserve ligatures;
    /// outward rounding includes every pixel at fractional cell boundaries.
    pub fn scissors(
        &self,
        size: (u32, u32),
        cell_h: f32,
        strip_h: f32,
        rows: usize,
    ) -> Vec<[u32; 4]> {
        let (w, h) = size;
        if w == 0 || h == 0 {
            return Vec::new();
        }
        if self.full {
            return vec![[0, 0, w, h]];
        }
        let mut result: Vec<[u32; 4]> = Vec::new();
        for row in 0..rows {
            if !self.paints_row(row as u32) {
                continue;
            }
            let top = (strip_h + row as f32 * cell_h).floor().max(0.0) as u32;
            let bottom = if row + 1 == rows {
                h
            } else {
                ((strip_h + (row + 1) as f32 * cell_h).ceil().max(0.0) as u32).min(h)
            };
            if top >= bottom {
                continue;
            }
            if let Some(last) = result.last_mut().filter(|last| last[1] + last[3] >= top) {
                last[3] = bottom - last[1];
            } else {
                result.push([0, top, w, bottom - top]);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttled_consumer_keeps_all_damage_until_taken() {
        let mut pending = Damage::default();
        pending.add_line(2);
        pending.add_line(8);
        assert!(pending.paints_row(2) && pending.paints_row(8));
        let batch = std::mem::take(&mut pending);
        assert!(pending.is_empty());
        assert_eq!(
            batch.scissors((100, 200), 10.0, 20.0, 18),
            vec![[0, 30, 100, 30], [0, 90, 100, 30]]
        );
        pending.merge(&Damage::full());
        pending.add_line(5);
        assert_eq!(
            pending.scissors((100, 200), 10.0, 20.0, 18),
            vec![[0, 0, 100, 200]]
        );
    }

    #[test]
    fn neighboring_bands_merge_and_clip_to_target() {
        let mut damage = Damage::default();
        damage.add_line(0);
        damage.add_line(3);
        assert_eq!(
            damage.scissors((80, 48), 10.5, 5.5, 8),
            vec![[0, 5, 80, 43]]
        );
    }
}
