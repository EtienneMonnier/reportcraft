//! How a field or total is written: numbers, currencies, percentages, dates, yes or no and text,
//! following the report's locale. Locales are a typed table (data over code); an unknown tag falls back to `en`.

use serde::{Deserialize, Serialize};

/// Most decimals a format may ask for.
pub const MAX_DECIMALS: u8 = 10;

/// A format for one element's value. Values the format doesn't apply to (text in a number
/// format, a number in a date format) are written as they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Format {
    /// `1,234.50`: fixed decimals, optional thousands grouping.
    Number {
        #[serde(default)]
        decimals: u8,
        #[serde(default = "yes")]
        grouping: bool,
    },
    /// `1,234.50 €` or `$1,234.50`.
    Currency {
        symbol: String,
        #[serde(default = "two")]
        decimals: u8,
        /// Symbol after the amount (with the locale's space), else before it.
        #[serde(default)]
        symbol_after: bool,
    },
    /// `12.5%`: the value times 100.
    Percent {
        #[serde(default)]
        decimals: u8,
    },
    /// A date or date-time, with a `strftime`-style pattern (`%d/%m/%Y`), or the locale's date.
    Date {
        #[serde(default)]
        pattern: Option<String>,
    },
    /// `Yes` or `No` in the report's language (`Oui`/`Non` in French), or the texts given, for a
    /// true or false value: a boolean, 1 or 0, `true` or `false`.
    Boolean {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        true_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        false_text: Option<String>,
    },
    /// The value as text, in upper, lower or title case, after a prefix and before a suffix
    /// (`Ref. 1042`, `12 kg`); a blank value stays blank, without them.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        case: Option<TextCase>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        prefix: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        suffix: String,
    },
}

/// The case a text format writes letters in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    Upper,
    Lower,
    /// The first letter of each word in capitals, the others in small letters.
    Title,
}

/// Most characters in a format's own texts (a prefix, a suffix, a yes or no).
pub const MAX_FORMAT_TEXT: usize = 200;

fn yes() -> bool {
    true
}

fn two() -> u8 {
    2
}

impl Format {
    /// The decimals asked for, when the format has them.
    pub fn decimals(&self) -> Option<u8> {
        match self {
            Format::Number { decimals, .. } | Format::Currency { decimals, .. } | Format::Percent { decimals } => Some(*decimals),
            Format::Date { .. } | Format::Boolean { .. } | Format::Text { .. } => None,
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Format::Number { .. } | Format::Currency { .. } | Format::Percent { .. })
    }

    /// The texts the format writes itself, each at most `MAX_FORMAT_TEXT` characters.
    pub fn texts_are_valid(&self) -> bool {
        let short = |t: &str| t.chars().count() <= MAX_FORMAT_TEXT;
        match self {
            Format::Boolean { true_text, false_text } => [true_text, false_text].into_iter().flatten().all(|t| short(t)),
            Format::Text { prefix, suffix, .. } => short(prefix) && short(suffix),
            Format::Currency { symbol, .. } => short(symbol),
            _ => true,
        }
    }
}

/// A locale's conventions for writing numbers and dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocaleStyle {
    pub tag: &'static str,
    pub decimal: char,
    pub group: char,
    /// Between a number and `%` or a trailing currency symbol (empty when none).
    pub unit_space: &'static str,
    pub date_pattern: &'static str,
    /// Day and month names (`%A`, `%B`, `%a`, `%b`) and AM/PM in this language.
    pub names: chrono::Locale,
    /// How a boolean format writes true and false.
    pub yes: &'static str,
    pub no: &'static str,
}

const NNBSP: char = '\u{202F}';

/// Known locales. Matching takes the full tag first (`de-CH`), then its language (`de`).
pub const LOCALES: &[LocaleStyle] = &[
    LocaleStyle { tag: "en", decimal: '.', group: ',', unit_space: "", date_pattern: "%Y-%m-%d", names: chrono::Locale::en_US, yes: "Yes", no: "No" },
    LocaleStyle {
        tag: "en-US",
        decimal: '.',
        group: ',',
        unit_space: "",
        date_pattern: "%m/%d/%Y",
        names: chrono::Locale::en_US,
        yes: "Yes",
        no: "No",
    },
    LocaleStyle {
        tag: "en-GB",
        decimal: '.',
        group: ',',
        unit_space: "",
        date_pattern: "%d/%m/%Y",
        names: chrono::Locale::en_GB,
        yes: "Yes",
        no: "No",
    },
    LocaleStyle {
        tag: "fr",
        decimal: ',',
        group: NNBSP,
        unit_space: "\u{a0}",
        date_pattern: "%d/%m/%Y",
        names: chrono::Locale::fr_FR,
        yes: "Oui",
        no: "Non",
    },
    LocaleStyle {
        tag: "fr-CH",
        decimal: '.',
        group: '\u{2019}',
        unit_space: "\u{a0}",
        date_pattern: "%d.%m.%Y",
        names: chrono::Locale::fr_CH,
        yes: "Oui",
        no: "Non",
    },
    LocaleStyle {
        tag: "de",
        decimal: ',',
        group: '.',
        unit_space: "\u{a0}",
        date_pattern: "%d.%m.%Y",
        names: chrono::Locale::de_DE,
        yes: "Ja",
        no: "Nein",
    },
    LocaleStyle {
        tag: "de-CH",
        decimal: '.',
        group: '\u{2019}',
        unit_space: "\u{a0}",
        date_pattern: "%d.%m.%Y",
        names: chrono::Locale::de_CH,
        yes: "Ja",
        no: "Nein",
    },
    LocaleStyle {
        tag: "es",
        decimal: ',',
        group: '.',
        unit_space: "\u{a0}",
        date_pattern: "%d/%m/%Y",
        names: chrono::Locale::es_ES,
        yes: "Sí",
        no: "No",
    },
    LocaleStyle {
        tag: "it",
        decimal: ',',
        group: '.',
        unit_space: "\u{a0}",
        date_pattern: "%d/%m/%Y",
        names: chrono::Locale::it_IT,
        yes: "Sì",
        no: "No",
    },
    LocaleStyle {
        tag: "nl",
        decimal: ',',
        group: '.',
        unit_space: "\u{a0}",
        date_pattern: "%d-%m-%Y",
        names: chrono::Locale::nl_NL,
        yes: "Ja",
        no: "Nee",
    },
    LocaleStyle {
        tag: "pt",
        decimal: ',',
        group: '.',
        unit_space: "\u{a0}",
        date_pattern: "%d/%m/%Y",
        names: chrono::Locale::pt_PT,
        yes: "Sim",
        no: "Não",
    },
];

/// The style for a locale tag, if known (case-insensitive, `_` or `-`).
pub fn locale_style(tag: &str) -> Option<LocaleStyle> {
    let tag = tag.replace('_', "-");
    let exact = LOCALES.iter().find(|l| l.tag.eq_ignore_ascii_case(&tag));
    let language = || {
        let lang = tag.split('-').next().unwrap_or("");
        LOCALES.iter().find(|l| l.tag.eq_ignore_ascii_case(lang))
    };
    exact.or_else(language).copied()
}

/// The style for a locale tag, `en` when unknown.
pub fn locale_or_default(tag: &str) -> LocaleStyle {
    locale_style(tag).unwrap_or(DEFAULT_LOCALE)
}

const DEFAULT_LOCALE: LocaleStyle =
    LocaleStyle { tag: "en", decimal: '.', group: ',', unit_space: "", date_pattern: "%Y-%m-%d", names: chrono::Locale::en_US, yes: "Yes", no: "No" };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_match_by_tag_then_language() {
        assert_eq!(locale_style("fr_FR").map(|l| l.tag), Some("fr"));
        assert_eq!(locale_style("DE-ch").map(|l| l.tag), Some("de-CH"));
        assert_eq!(locale_style("en-US").map(|l| l.date_pattern), Some("%m/%d/%Y"));
        assert!(locale_style("xx").is_none());
        assert_eq!(locale_or_default("xx").tag, "en");
        assert_eq!(LOCALES.first(), Some(&DEFAULT_LOCALE));
    }

    #[test]
    fn formats_read_from_json_with_defaults() {
        let f: Format = serde_json::from_str(r#"{"kind":"currency","symbol":"€","symbol_after":true}"#).unwrap();
        assert_eq!(f, Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true });
        let n: Format = serde_json::from_str(r#"{"kind":"number"}"#).unwrap();
        assert_eq!(n, Format::Number { decimals: 0, grouping: true });
    }
}
