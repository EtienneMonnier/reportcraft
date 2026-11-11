//! Reading a few keys of an ADO.NET connection string (`Server=…;User Id=…;Database=…`) for
//! listings and the password fallback. The driver parses the whole string itself.

/// The `key=value` pairs, keys lower-cased; values may be quoted with `'…'`, `"…"` (the quote
/// doubled inside) or `{…}`.
pub(crate) fn pairs(s: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && chars[i] != '=' && chars[i] != ';' {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect::<String>().trim().to_lowercase();
        if chars.get(i) != Some(&'=') {
            i += 1;
            continue;
        }
        i += 1;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        let mut value = String::new();
        match chars.get(i) {
            Some(&q @ ('\'' | '"')) => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == q {
                        if chars.get(i + 1) == Some(&q) {
                            value.push(q);
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    value.push(chars[i]);
                    i += 1;
                }
                while i < chars.len() && chars[i] != ';' {
                    i += 1;
                }
            }
            Some('{') => {
                i += 1;
                while i < chars.len() && chars[i] != '}' {
                    value.push(chars[i]);
                    i += 1;
                }
                while i < chars.len() && chars[i] != ';' {
                    i += 1;
                }
            }
            _ => {
                while i < chars.len() && chars[i] != ';' {
                    value.push(chars[i]);
                    i += 1;
                }
                value = value.trim().to_owned();
            }
        }
        i += 1;
        if !key.is_empty() {
            out.push((key, value));
        }
    }
    out
}

/// The value of the last of `keys` the string sets (a later setting wins, as in ADO.NET).
pub(crate) fn get(pairs: &[(String, String)], keys: &[&str]) -> Option<String> {
    pairs.iter().rev().find(|(k, _)| keys.contains(&k.as_str())).map(|(_, v)| v.clone())
}

pub(crate) const USER: &[&str] = &["user id", "uid", "user", "username"];
pub(crate) const PASSWORD: &[&str] = &["password", "pwd"];
pub(crate) const DATABASE: &[&str] = &["database", "initial catalog"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_quoted_values() {
        let p = pairs("Server=tcp:db,1433; User Id = rc ;Password='a;b''c';Database={odd;name};TrustServerCertificate=true");
        assert_eq!(get(&p, &["server"]).as_deref(), Some("tcp:db,1433"));
        assert_eq!(get(&p, USER).as_deref(), Some("rc"));
        assert_eq!(get(&p, PASSWORD).as_deref(), Some("a;b'c"));
        assert_eq!(get(&p, DATABASE).as_deref(), Some("odd;name"));
        assert_eq!(get(&pairs("server=x;;junk;uid=u"), USER).as_deref(), Some("u"));
        assert_eq!(get(&pairs(""), USER), None);
    }
}
