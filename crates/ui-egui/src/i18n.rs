//! The interface's language, through craft-libs' craft-i18n: catalogs embedded at build time
//! (`build.rs`), English when there are none. `tl!("Text")` is the English text in the current
//! language; `tf!("Opened {name}", name = x)` fills placeholders too.

use std::cell::Cell;
use std::sync::LazyLock;

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The languages the designer's interface is ready for (left-to-right, Latin script for now).
const OFFERED: &[&str] = &["fr", "de", "es", "it", "nl", "pt", "pt-br"];

pub(crate) static I18N: LazyLock<craft_i18n::I18n> = LazyLock::new(|| {
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, OFFERED);
    for p in problems {
        log::warn!("translation catalog: {p}");
    }
    i18n
});

thread_local! {
    pub(crate) static CURRENT: Cell<craft_i18n::Lang> = const { Cell::new(craft_i18n::Lang::EN) };
}

/// The current language.
pub(crate) fn lang() -> craft_i18n::Lang {
    CURRENT.get()
}

/// Uses `code` (`fr`, `de`…; `auto` is the system's language) when it is offered; English otherwise.
pub fn set_language(code: &str) {
    let lang = if code == "auto" { I18N.system_lang() } else { I18N.from_code(code).unwrap_or(craft_i18n::Lang::EN) };
    CURRENT.set(lang);
}

/// The current language's code (`en` when English).
pub fn language() -> String {
    I18N.code(lang()).to_owned()
}

/// The languages to choose from: code and name in itself.
pub(crate) fn languages() -> Vec<(String, String)> {
    I18N.languages().into_iter().map(|l| (I18N.code(l).to_owned(), I18N.name(l).to_owned())).collect()
}

macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::I18N.t($crate::i18n::lang(), $s)
    };
}

macro_rules! tf {
    ($s:expr $(, $k:ident = $v:expr)* $(,)?) => {
        craft_i18n::fmt($crate::i18n::I18N.t($crate::i18n::lang(), $s), &[$((stringify!($k), &$v.to_string())),*])
    };
}
