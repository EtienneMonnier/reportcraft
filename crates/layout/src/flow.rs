//! Boxes laid side by side in a band (`Element::flow`, an imported grid row): for each row of
//! data, a box whose elements are all hidden gives its width up. The boxes after it in its row
//! move left, and those that grow share the room it leaves in proportion to their widths, as
//! Bootstrap's `col`s share a row's width among the columns a template shows.

use reportcraft_model::{Band, Content, Element, Flow};

/// Two edges this close (millimetres) are the same.
const SAME: f64 = 1e-6;

/// A box of a row, as designed, and whether any of its elements is shown.
struct Slot {
    row: u32,
    x: f64,
    w: f64,
    grows: bool,
    shown: bool,
}

/// The box an element flows with: subreports keep their place, and a box without a width (which
/// validation refuses) is no box.
fn flow_of(el: &Element) -> Option<&Flow> {
    if matches!(el.content, Content::Subreport { .. }) {
        return None;
    }
    el.flow.as_ref().filter(|f| f.x_mm.is_finite() && f.width_mm.is_finite() && f.width_mm > SAME)
}

/// Each element's left edge and width for one row of data (`shown[i]`: whether element `i` is
/// drawn): moved and widened with its box when boxes of its row are hidden. `None` when nothing
/// moves (every box shows, or the band has none).
pub(crate) fn across(band: &Band, shown: &[bool]) -> Option<Vec<(f64, f64)>> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut slot_of: Vec<Option<usize>> = Vec::with_capacity(band.elements.len());
    for (i, el) in band.elements.iter().enumerate() {
        let Some(f) = flow_of(el) else {
            slot_of.push(None);
            continue;
        };
        let same = |s: &Slot| s.row == f.row && (s.x - f.x_mm).abs() < SAME && (s.w - f.width_mm).abs() < SAME;
        let k = match slots.iter().position(same) {
            Some(k) => k,
            None => {
                slots.push(Slot { row: f.row, x: f.x_mm, w: f.width_mm, grows: f.grows, shown: false });
                slots.len() - 1
            }
        };
        if shown.get(i).copied().unwrap_or(true)
            && let Some(s) = slots.get_mut(k)
        {
            s.shown = true;
        }
        slot_of.push(Some(k));
    }
    if slots.iter().all(|s| s.shown) {
        return None;
    }
    let placed = place(&slots);
    let at = band
        .elements
        .iter()
        .zip(&slot_of)
        .map(|(el, k)| match k.and_then(|k| Some((slots.get(k)?, placed.get(k)?))) {
            Some((s, &(x, w))) => {
                let scale = w / s.w;
                (x + (el.x_mm - s.x) * scale, el.width_mm * scale)
            }
            None => (el.x_mm, el.width_mm),
        })
        .collect();
    Some(at)
}

/// Where each box goes: in each row with a hidden box, the shown ones from the row's left edge,
/// each after the room that came before it, those that grow widened to fill the row. A hidden box
/// keeps its place (nothing of it is drawn).
fn place(slots: &[Slot]) -> Vec<(f64, f64)> {
    let mut placed: Vec<(f64, f64)> = slots.iter().map(|s| (s.x, s.w)).collect();
    let mut rows: Vec<u32> = slots.iter().filter(|s| !s.shown).map(|s| s.row).collect();
    rows.sort_unstable();
    rows.dedup();
    for row in rows {
        let mut members: Vec<(usize, &Slot)> = slots.iter().enumerate().filter(|(_, s)| s.row == row).collect();
        members.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        let left = members.iter().map(|(_, s)| s.x).fold(f64::INFINITY, f64::min);
        let right = members.iter().map(|(_, s)| s.x + s.w).fold(f64::NEG_INFINITY, f64::max);
        // The room before each box: from the right edge of the boxes before it.
        let mut edge = left;
        let gaps: Vec<f64> = members
            .iter()
            .map(|(_, s)| {
                let gap = (s.x - edge).max(0.0);
                edge = edge.max(s.x + s.w);
                gap
            })
            .collect();
        let shown = || members.iter().zip(&gaps).filter(|((_, s), _)| s.shown);
        let fixed: f64 = shown().map(|((_, s), gap)| gap + if s.grows { 0.0 } else { s.w }).sum();
        let growing: f64 = shown().filter(|((_, s), _)| s.grows).map(|((_, s), _)| s.w).sum();
        // Growing boxes never get narrower than designed (overlapping boxes could ask for it).
        let scale = if growing > SAME { ((right - left - fixed) / growing).max(1.0) } else { 1.0 };
        let mut x = left;
        for ((k, s), gap) in shown() {
            let w = if s.grows { s.w * scale } else { s.w };
            x += gap;
            if let Some(p) = placed.get_mut(*k) {
                *p = (x, w);
            }
            x += w;
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use reportcraft_model::TextStyle;

    fn el(x: f64, w: f64, flow: Option<Flow>) -> Element {
        Element {
            x_mm: x,
            y_mm: 0.0,
            width_mm: w,
            height_mm: 5.0,
            style: TextStyle::default(),
            content: Content::Text { text: "t".into() },
            format: None,
            visible_if: None,
            color_formula: None,
            background_formula: None,
            can_grow: false,
            flow,
        }
    }

    fn boxed(row: u32, x: f64, w: f64, grows: bool) -> Option<Flow> {
        Some(Flow { row, x_mm: x, width_mm: w, grows })
    }

    #[test]
    fn hidden_boxes_give_their_width_to_the_growing_ones_and_the_rest_moves_left() {
        // Four growing boxes of 45 mm (two elements in the second), then a fixed one of 20 mm
        // after 10 mm of room, and an element of no box.
        let band = Band {
            height_mm: 10.0,
            elements: vec![
                el(0.0, 45.0, boxed(1, 0.0, 45.0, true)),
                el(45.0, 45.0, boxed(1, 45.0, 45.0, true)),
                el(50.0, 20.0, boxed(1, 45.0, 45.0, true)),
                el(90.0, 45.0, boxed(1, 90.0, 45.0, true)),
                el(135.0, 15.0, boxed(1, 135.0, 15.0, true)),
                el(160.0, 20.0, boxed(1, 160.0, 20.0, false)),
                el(0.0, 30.0, None),
            ],
            ..Band::default()
        };
        assert_eq!(across(&band, &[true; 7]), None, "nothing hidden, nothing moves");
        // The second box hidden: 45 mm shared by the three growing boxes left (150 → 140 + 45).
        let at = across(&band, &[true, false, false, true, true, true, true]).unwrap_or_default();
        let scale = (180.0 - 10.0 - 20.0) / 105.0;
        let expected =
            [(0.0, 45.0 * scale), (45.0, 45.0), (50.0, 20.0), (45.0 * scale, 45.0 * scale), (90.0 * scale, 15.0 * scale), (160.0, 20.0), (0.0, 30.0)];
        for (got, want) in at.iter().zip(expected) {
            assert!((got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9, "{at:?}");
        }
        // A box shows when one of its elements does: only one of the second box's two hidden.
        assert_eq!(across(&band, &[true, false, true, true, true, true, true]), None);
        // Only the fixed box and the last growing one: the growing one fills what is left, the
        // fixed one keeps its width and its room before it.
        let at = across(&band, &[false, false, false, false, true, true, false]).unwrap_or_default();
        assert_eq!((at[4], at[5], at[6]), ((0.0, 150.0), (160.0, 20.0), (0.0, 30.0)));
    }

    #[test]
    fn fixed_boxes_move_left_and_rows_are_apart() {
        let band = Band {
            height_mm: 10.0,
            elements: vec![
                el(0.0, 60.0, boxed(1, 0.0, 60.0, false)),
                el(60.0, 60.0, boxed(1, 60.0, 60.0, false)),
                el(120.0, 60.0, boxed(1, 120.0, 60.0, false)),
                el(0.0, 90.0, boxed(2, 0.0, 90.0, true)),
                el(90.0, 90.0, boxed(2, 90.0, 90.0, true)),
            ],
            ..Band::default()
        };
        let at = across(&band, &[false, true, true, true, true]).unwrap_or_default();
        assert_eq!(at, vec![(0.0, 60.0), (0.0, 60.0), (60.0, 60.0), (0.0, 90.0), (90.0, 90.0)]);
        // Everything of a row hidden: nothing to place, nothing breaks.
        let at = across(&band, &[false, false, false, false, true]).unwrap_or_default();
        assert_eq!(at[4], (0.0, 180.0));
    }

    #[test]
    fn subreports_and_boxes_without_width_stay_put() {
        let mut sub = el(0.0, 90.0, boxed(1, 0.0, 90.0, true));
        sub.content = Content::Subreport {
            report: Box::new(reportcraft_model::Report::new("s")),
            dataset: "d".into(),
            links: Vec::new(),
            hide_when_empty: false,
        };
        let band = Band {
            height_mm: 10.0,
            elements: vec![sub, el(90.0, 90.0, boxed(1, 90.0, 90.0, true)), el(0.0, 10.0, boxed(1, 0.0, 0.0, true))],
            ..Band::default()
        };
        assert_eq!(across(&band, &[false, true, false]), None);
    }
}
