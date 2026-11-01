//! EAN-13 (ISO/IEC 15420): 12 digits plus a check digit; the first digit is carried by the
//! parity pattern of the next six.

use crate::Symbol;

/// Left-hand odd-parity (set A) patterns of digits 0 to 9; a 1 is a dark module.
const SET_A: [u8; 10] = [0b0001101, 0b0011001, 0b0010011, 0b0111101, 0b0100011, 0b0110001, 0b0101111, 0b0111011, 0b0110111, 0b0001011];

/// Which of the six left-hand digits use even parity (set B), by first digit; bit 5 is the
/// leftmost of the six.
const PARITY: [u8; 10] = [0b000000, 0b001011, 0b001101, 0b001110, 0b010011, 0b011001, 0b011100, 0b010101, 0b010110, 0b011010];

/// The check digit of 12 digits: weights 1 and 3 from the left, up to the next multiple of 10.
fn check_digit(digits: &[u8]) -> u8 {
    let sum: u32 = digits.iter().enumerate().map(|(i, &d)| u32::from(d) * if i % 2 == 0 { 1 } else { 3 }).sum();
    u8::try_from((10 - sum % 10) % 10).unwrap_or(0)
}

fn push_bits(pattern: u8, out: &mut Vec<bool>) {
    out.extend((0..7).rev().map(|bit| pattern >> bit & 1 == 1));
}

pub fn encode(value: &str) -> Result<Symbol, String> {
    let t = value.trim();
    if !(t.len() == 12 || t.len() == 13) || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err("EAN-13 needs 12 or 13 digits".into());
    }
    let mut digits: Vec<u8> = t.bytes().map(|b| b - b'0').collect();
    let check = check_digit(digits.get(..12).unwrap_or_default());
    match digits.get(12) {
        Some(&given) if given != check => return Err(format!("EAN-13 check digit should be {check}, not {given}")),
        Some(_) => {}
        None => digits.push(check),
    }
    let first = usize::from(digits.first().copied().unwrap_or(0));
    let parity = PARITY.get(first).copied().unwrap_or(0);
    let mut modules = Vec::with_capacity(95);
    modules.extend([true, false, true]);
    for (i, &d) in digits.iter().skip(1).take(6).enumerate() {
        let a = SET_A.get(usize::from(d)).copied().unwrap_or(0);
        // Set B is set C (the right-hand complement) read backwards.
        let even = parity >> (5 - i) & 1 == 1;
        push_bits(if even { (!a & 0x7F).reverse_bits() >> 1 } else { a }, &mut modules);
    }
    modules.extend([false, true, false, true, false]);
    for &d in digits.iter().skip(7) {
        push_bits(!SET_A.get(usize::from(d)).copied().unwrap_or(0) & 0x7F, &mut modules);
    }
    modules.extend([true, false, true]);
    let text = digits.iter().map(|d| char::from(b'0' + d)).collect();
    Ok(Symbol { columns: modules.len(), rows: 1, quiet: 11, modules, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_or_checks_the_check_digit() {
        assert_eq!(encode("400638133393").unwrap().text, "4006381333931");
        assert_eq!(encode("4006381333931").unwrap().columns, 95);
        assert!(encode("4006381333932").unwrap_err().contains("should be 1"));
        assert!(encode("40063813339").is_err());
        assert!(encode("40063813339x").is_err());
    }

    #[test]
    fn set_b_is_set_c_reversed() {
        // Digit 0: set A 0001101, set C 1110010, set B 0100111.
        let a = SET_A[0];
        assert_eq!((!a & 0x7F).reverse_bits() >> 1, 0b0100111);
    }
}
