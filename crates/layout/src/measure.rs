//! Measures and wraps text without the fonts: widths are those of Liberation Sans (Arial's
//! metrics), the first font the PDF writer looks for. Another font measures differently, so a
//! wrapped line keeps a little room spare (`SPARE`).

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// A line of text takes this many times its font size, as the writers draw it.
pub const LINE_HEIGHT: f64 = 1.2;
/// Share of a box's width a wrapped line may fill: room for a wider font than the one measured.
const SPARE: f64 = 0.96;
/// Width of a character outside Latin-1, in thousandths of an em.
const OTHER: u16 = 556;

/// Regular: characters U+0020 to U+00FF, in thousandths of an em.
const REGULAR: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584,
    584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667,
    611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500,
    722, 500, 500, 500, 334, 260, 334, 584, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750,
    750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 552, 400,
    549, 333, 333, 333, 576, 537, 333, 333, 333, 365, 556, 834, 834, 834, 611, 667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278,
    278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556,
    556, 278, 278, 278, 278, 556, 556, 556, 556, 556, 556, 556, 549, 611, 556, 556, 556, 556, 500, 556, 500,
];
/// Bold: characters U+0020 to U+00FF, in thousandths of an em.
const BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584,
    584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667,
    611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556,
    778, 556, 556, 500, 389, 280, 389, 584, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750,
    750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 750, 278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 552, 400,
    549, 333, 333, 333, 576, 556, 333, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278,
    278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556,
    556, 278, 278, 278, 278, 611, 611, 611, 611, 611, 611, 611, 549, 611, 611, 611, 611, 611, 556, 611, 556,
];

/// The width of `text` written at `size_pt`, in millimetres.
pub fn width_mm(text: &str, size_pt: f64, bold: bool) -> f64 {
    let table = if bold { &BOLD } else { &REGULAR };
    let thousandths: u64 = text
        .chars()
        .map(|c| {
            let code = c as u32;
            let i = code.checked_sub(32).and_then(|i| usize::try_from(i).ok());
            u64::from(i.and_then(|i| table.get(i)).copied().unwrap_or(if code < 32 { 0 } else { OTHER }))
        })
        .sum();
    thousandths as f64 / 1000.0 * size_pt / PT_PER_MM
}

/// The height of one line of text at `size_pt`, in millimetres.
pub fn line_mm(size_pt: f64) -> f64 {
    size_pt * LINE_HEIGHT / PT_PER_MM
}

/// `text` cut into the lines a box `box_mm` wide holds: at its line breaks, then between words
/// (a word wider than the box is cut between letters). Spaces at the end of a line are dropped;
/// at most `max_lines` lines, the last holding the rest.
pub fn wrap(text: &str, box_mm: f64, size_pt: f64, bold: bool, max_lines: usize) -> Vec<String> {
    let room = box_mm * SPARE;
    let fits = |s: &str| width_mm(s, size_pt, bold) <= room;
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if fits(&candidate) {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // A word wider than the box: cut between letters.
            for c in word.chars() {
                line.push(c);
                if !fits(&line) && line.chars().count() > 1 {
                    line.pop();
                    lines.push(std::mem::take(&mut line));
                    line.push(c);
                }
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    if lines.len() > max_lines.max(1) {
        let rest = lines.split_off(max_lines.max(1) - 1).join(" ");
        lines.push(rest);
    }
    // Blank lines at the end take no room.
    while lines.len() > 1 && lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_measured_with_arial_metrics() {
        // "Hello" at 10 pt in Arial: 722 + 556 + 222 + 222 + 556 = 2278 thousandths of 10 pt.
        let w = width_mm("Hello", 10.0, false);
        assert!((w - 2.278 * 10.0 / PT_PER_MM).abs() < 1e-9);
        assert!(width_mm("Hello", 10.0, true) > w);
        assert!((line_mm(10.0) - 12.0 / PT_PER_MM).abs() < 1e-9);
    }

    #[test]
    fn text_wraps_between_words_at_line_breaks_and_inside_long_words() {
        let one_word = width_mm("Insurance ", 10.0, false);
        let lines = wrap("Insurance premium base\nrate 7 %", one_word * 1.6, 10.0, false, 99);
        assert_eq!(lines, ["Insurance", "premium base", "rate 7 %"]);
        let cut = wrap("Supercalifragilistic", width_mm("Super", 10.0, false) / SPARE + 0.01, 10.0, false, 99);
        assert_eq!(cut.first().map(String::as_str), Some("Super"));
        assert_eq!(cut.concat(), "Supercalifragilistic");
        assert_eq!(wrap("a b c d", 0.5, 10.0, false, 2), ["a", "b c d"]);
        assert_eq!(wrap("", 50.0, 10.0, false, 9), [""]);
        assert_eq!(wrap("Son\n", 50.0, 10.0, false, 9), ["Son"]);
    }
}
