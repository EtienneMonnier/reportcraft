//! What the designer's control channel and its clients share: where the access token is kept.
//!
//! `reportcraft --control PORT` listens on `127.0.0.1:PORT` for JSON lines. The first line of a
//! connection must be `{"auth": TOKEN}`, the token the app wrote, readable by its user only, to
//! [`token_path`]; then each line `{"id", "method", "params"}` gets one reply
//! `{"id", "ok", "result" | "error"}`. `reportcraft-cli send PORT METHOD [JSON]` does this.

use std::path::PathBuf;

/// The file holding the token of the app listening on `port`: in the user's runtime folder
/// (`XDG_RUNTIME_DIR`, private to them) when there is one, else the temporary folder.
pub fn token_path(port: u16) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("reportcraft-control-{port}.token"))
}

/// A fresh random token: 128 bits from the standard library's randomly seeded hashers, as hex.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(u64::from(std::process::id()));
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// Writes `token` for the app on `port`, readable by its user only.
pub fn write_token(port: u16, token: &str) -> std::io::Result<PathBuf> {
    let path = token_path(port);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&path)?, token.as_bytes())?;
    Ok(path)
}

/// The token of the app on `port`, as it wrote it.
pub fn read_token(port: u16) -> std::io::Result<String> {
    std::fs::read_to_string(token_path(port)).map(|t| t.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_private() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        let port = 40_000 + (std::process::id() % 20_000) as u16;
        let path = write_token(port, &a).unwrap();
        assert_eq!(read_token(port).unwrap(), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(path);
    }
}
