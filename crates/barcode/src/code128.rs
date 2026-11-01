//! Code 128 (ISO/IEC 15417): code set B for printable ASCII, code set C for runs of digits
//! (two digits per symbol), switching where it makes the symbol shorter.

use crate::{Symbol, widths_to_modules};

/// Longest value accepted.
const MAX_LEN: usize = 80;
const START_B: u8 = 104;
const START_C: u8 = 105;
const CODE_B: u8 = 100;
const CODE_C: u8 = 99;
const STOP: [u8; 7] = [2, 3, 3, 1, 1, 1, 2];

/// Bar and space widths of symbol values 0 to 105: bar, space, bar, space, bar, space.
const PATTERNS: [[u8; 6]; 106] = [
    [2, 1, 2, 2, 2, 2],
    [2, 2, 2, 1, 2, 2],
    [2, 2, 2, 2, 2, 1],
    [1, 2, 1, 2, 2, 3],
    [1, 2, 1, 3, 2, 2],
    [1, 3, 1, 2, 2, 2],
    [1, 2, 2, 2, 1, 3],
    [1, 2, 2, 3, 1, 2],
    [1, 3, 2, 2, 1, 2],
    [2, 2, 1, 2, 1, 3],
    [2, 2, 1, 3, 1, 2],
    [2, 3, 1, 2, 1, 2],
    [1, 1, 2, 2, 3, 2],
    [1, 2, 2, 1, 3, 2],
    [1, 2, 2, 2, 3, 1],
    [1, 1, 3, 2, 2, 2],
    [1, 2, 3, 1, 2, 2],
    [1, 2, 3, 2, 2, 1],
    [2, 2, 3, 2, 1, 1],
    [2, 2, 1, 1, 3, 2],
    [2, 2, 1, 2, 3, 1],
    [2, 1, 3, 2, 1, 2],
    [2, 2, 3, 1, 1, 2],
    [3, 1, 2, 1, 3, 1],
    [3, 1, 1, 2, 2, 2],
    [3, 2, 1, 1, 2, 2],
    [3, 2, 1, 2, 2, 1],
    [3, 1, 2, 2, 1, 2],
    [3, 2, 2, 1, 1, 2],
    [3, 2, 2, 2, 1, 1],
    [2, 1, 2, 1, 2, 3],
    [2, 1, 2, 3, 2, 1],
    [2, 3, 2, 1, 2, 1],
    [1, 1, 1, 3, 2, 3],
    [1, 3, 1, 1, 2, 3],
    [1, 3, 1, 3, 2, 1],
    [1, 1, 2, 3, 1, 3],
    [1, 3, 2, 1, 1, 3],
    [1, 3, 2, 3, 1, 1],
    [2, 1, 1, 3, 1, 3],
    [2, 3, 1, 1, 1, 3],
    [2, 3, 1, 3, 1, 1],
    [1, 1, 2, 1, 3, 3],
    [1, 1, 2, 3, 3, 1],
    [1, 3, 2, 1, 3, 1],
    [1, 1, 3, 1, 2, 3],
    [1, 1, 3, 3, 2, 1],
    [1, 3, 3, 1, 2, 1],
    [3, 1, 3, 1, 2, 1],
    [2, 1, 1, 3, 3, 1],
    [2, 3, 1, 1, 3, 1],
    [2, 1, 3, 1, 1, 3],
    [2, 1, 3, 3, 1, 1],
    [2, 1, 3, 1, 3, 1],
    [3, 1, 1, 1, 2, 3],
    [3, 1, 1, 3, 2, 1],
    [3, 3, 1, 1, 2, 1],
    [3, 1, 2, 1, 1, 3],
    [3, 1, 2, 3, 1, 1],
    [3, 3, 2, 1, 1, 1],
    [3, 1, 4, 1, 1, 1],
    [2, 2, 1, 4, 1, 1],
    [4, 3, 1, 1, 1, 1],
    [1, 1, 1, 2, 2, 4],
    [1, 1, 1, 4, 2, 2],
    [1, 2, 1, 1, 2, 4],
    [1, 2, 1, 4, 2, 1],
    [1, 4, 1, 1, 2, 2],
    [1, 4, 1, 2, 2, 1],
    [1, 1, 2, 2, 1, 4],
    [1, 1, 2, 4, 1, 2],
    [1, 2, 2, 1, 1, 4],
    [1, 2, 2, 4, 1, 1],
    [1, 4, 2, 1, 1, 2],
    [1, 4, 2, 2, 1, 1],
    [2, 4, 1, 2, 1, 1],
    [2, 2, 1, 1, 1, 4],
    [4, 1, 3, 1, 1, 1],
    [2, 4, 1, 1, 1, 2],
    [1, 3, 4, 1, 1, 1],
    [1, 1, 1, 2, 4, 2],
    [1, 2, 1, 1, 4, 2],
    [1, 2, 1, 2, 4, 1],
    [1, 1, 4, 2, 1, 2],
    [1, 2, 4, 1, 1, 2],
    [1, 2, 4, 2, 1, 1],
    [4, 1, 1, 2, 1, 2],
    [4, 2, 1, 1, 1, 2],
    [4, 2, 1, 2, 1, 1],
    [2, 1, 2, 1, 4, 1],
    [2, 1, 4, 1, 2, 1],
    [4, 1, 2, 1, 2, 1],
    [1, 1, 1, 1, 4, 3],
    [1, 1, 1, 3, 4, 1],
    [1, 3, 1, 1, 4, 1],
    [1, 1, 4, 1, 1, 3],
    [1, 1, 4, 3, 1, 1],
    [4, 1, 1, 1, 1, 3],
    [4, 1, 1, 3, 1, 1],
    [1, 1, 3, 1, 4, 1],
    [1, 1, 4, 1, 3, 1],
    [3, 1, 1, 1, 4, 1],
    [4, 1, 1, 1, 3, 1],
    [2, 1, 1, 4, 1, 2],
    [2, 1, 1, 2, 1, 4],
    [2, 1, 1, 2, 3, 2],
];

/// Length of the run of ASCII digits starting at `i`.
fn digits_from(b: &[u8], i: usize) -> usize {
    b.get(i..).map_or(0, |rest| rest.iter().take_while(|c| c.is_ascii_digit()).count())
}

pub fn encode(value: &str) -> Result<Symbol, String> {
    if value.is_empty() {
        return Err("Code 128 needs a value".into());
    }
    if value.len() > MAX_LEN {
        return Err(format!("Code 128 holds at most {MAX_LEN} characters"));
    }
    if let Some(c) = value.chars().find(|c| !(' '..='~').contains(c)) {
        return Err(format!("Code 128 can't hold {c:?}: printable ASCII only"));
    }
    let b = value.as_bytes();
    // Code set C pays off from 4 digits on (or a value that is only digits, in pairs).
    let mut values: Vec<u8> = Vec::with_capacity(b.len() + 4);
    let first_run = digits_from(b, 0);
    let mut in_c = first_run.is_multiple_of(2) && (first_run >= 4 || (first_run == b.len() && first_run >= 2));
    values.push(if in_c { START_C } else { START_B });
    let mut i = 0;
    while i < b.len() {
        let run = digits_from(b, i);
        if in_c {
            if run >= 2 {
                values.push((b[i] - b'0') * 10 + (b[i + 1] - b'0'));
                i += 2;
                continue;
            }
            values.push(CODE_B);
            in_c = false;
        }
        // An odd run puts its first digit in code set B, so the rest pairs up exactly.
        if run >= 4 && run.is_multiple_of(2) {
            values.push(CODE_C);
            in_c = true;
            continue;
        }
        values.push(b[i] - b' ');
        i += 1;
    }
    let check = values.iter().enumerate().fold(0usize, |sum, (pos, &v)| sum + pos.max(1) * usize::from(v)) % 103;
    values.push(u8::try_from(check).unwrap_or(0));
    let mut modules = Vec::with_capacity(values.len() * 11 + 13);
    for v in &values {
        if let Some(p) = PATTERNS.get(usize::from(*v)) {
            widths_to_modules(p.iter().copied(), &mut modules);
        }
    }
    widths_to_modules(STOP, &mut modules);
    Ok(Symbol { columns: modules.len(), rows: 1, quiet: 10, modules, text: value.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_is_eleven_modules() {
        for (i, p) in PATTERNS.iter().enumerate() {
            assert_eq!(p.iter().map(|&w| u32::from(w)).sum::<u32>(), 11, "pattern {i}");
        }
        assert_eq!(STOP.iter().map(|&w| u32::from(w)).sum::<u32>(), 13);
    }

    #[test]
    fn digits_use_code_set_c() {
        // Start C, 12, 34, check, stop: 4 symbols of 11 plus 13.
        assert_eq!(encode("1234").unwrap().columns, 4 * 11 + 13);
        // "AB" then "123456" in C: start B, A, B, code C, 12, 34, 56, check, stop.
        assert_eq!(encode("AB123456").unwrap().columns, 8 * 11 + 13);
        // An odd run: "12345" is 1 in B, then 23 45 in C.
        assert_eq!(encode("X12345").unwrap().columns, 7 * 11 + 13);
        assert_eq!(encode("Hello, World!").unwrap().columns, 15 * 11 + 13);
    }

    #[test]
    fn check_symbol_is_the_weighted_sum_modulo_103() {
        // Start B (104) + "A" (33) × 1 = 137, and 137 mod 103 = 34.
        let s = encode("A").unwrap();
        let mut expected = Vec::new();
        widths_to_modules(PATTERNS[34], &mut expected);
        assert_eq!(s.modules[22..33], expected[..]);
    }

    #[test]
    fn refuses_what_it_cannot_hold() {
        assert!(encode("").is_err());
        assert!(encode("café").is_err());
        assert!(encode("tab\there").is_err());
        assert!(encode(&"9".repeat(MAX_LEN + 1)).is_err());
    }
}
