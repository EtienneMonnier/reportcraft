//! ReportCraft named connections (L3, native only): a database the user chose once, saved by name
//! so reports, the CLI and MCP agents can use it without repeating where it is or how to log in.
//!
//! Definitions (a SQLite file, or a PostgreSQL or SQL Server host, port, user and database) live in the suite's
//! settings file through craft-libs' `craft-settings` (`reportcraft/config.toml` in the platform's
//! config folder, or `REPORTCRAFT_CONFIG_DIR`). Passwords never go there: they go to a
//! [`SecretStore`], the OS keychain in the application ([`Keychain`]: Secret Service, macOS
//! Keychain, Windows Credential Manager) and memory in tests ([`MemorySecrets`]). Nothing here
//! prints, logs or returns a password: [`Connections::conninfo`] builds the libpq string only to
//! hand it to the driver, and [`Connections::sqlserver`] hands the password over on its own.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::Mutex;

use craft_settings::{Location, Settings, SettingsStore};
use reportcraft_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Most connections kept.
pub const MAX_CONNECTIONS: usize = 200;

/// Where a connection points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    Sqlite {
        path: String,
    },
    Postgres {
        host: String,
        #[serde(default = "default_port")]
        port: u16,
        user: String,
        database: String,
        /// libpq's `sslmode`: `disable`, `prefer` (when unset), `require` or `verify-full`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sslmode: Option<String>,
    },
    SqlServer {
        host: String,
        #[serde(default = "default_sqlserver_port")]
        port: u16,
        user: String,
        database: String,
        /// The same words as PostgreSQL's: `disable` (only the login is encrypted), `prefer` or
        /// `require` (when unset: encrypted, certificate not checked) or `verify-full` (checked).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sslmode: Option<String>,
    },
}

fn default_port() -> u16 {
    5432
}

fn default_sqlserver_port() -> u16 {
    1433
}

impl Target {
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Sqlite { .. } => "sqlite",
            Target::Postgres { .. } => "postgres",
            Target::SqlServer { .. } => "sqlserver",
        }
    }

    /// Where it points, without any secret: the file, or `user@host:port/database`.
    pub fn describe(&self) -> String {
        match self {
            Target::Sqlite { path } => path.clone(),
            Target::Postgres { host, port, user, database, .. } | Target::SqlServer { host, port, user, database, .. } => {
                format!("{user}@{host}:{port}/{database}")
            }
        }
    }
}

/// A saved connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub name: String,
    #[serde(flatten)]
    pub target: Target,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Connection {
    /// Problems that make the connection unusable: a bad name or an empty field.
    pub fn check(&self) -> Result<()> {
        let n = &self.name;
        if n.is_empty() || n.len() > 64 || !n.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
            return Err(Error::Invalid(format!("connection name `{n}` must be 1 to 64 letters, digits, `_`, `-` or `.`")));
        }
        let field = |value: &str, what: &str| {
            if value.trim().is_empty() || value.len() > 1_024 || value.chars().any(char::is_control) {
                Err(Error::Invalid(format!("connection `{n}`: {what} must be given, on one line")))
            } else {
                Ok(())
            }
        };
        match &self.target {
            Target::Sqlite { path } => field(path, "the database file")?,
            Target::Postgres { host, port, user, database, sslmode } | Target::SqlServer { host, port, user, database, sslmode } => {
                if let Some(m) = sslmode
                    && !["disable", "prefer", "require", "verify-ca", "verify-full"].contains(&m.as_str())
                {
                    return Err(Error::Invalid(format!("connection `{n}`: sslmode is disable, prefer, require or verify-full, not `{m}`")));
                }
                field(host, "the host")?;
                field(user, "the user")?;
                field(database, "the database")?;
                if *port == 0 {
                    return Err(Error::Invalid(format!("connection `{n}`: the port can't be 0")));
                }
            }
        }
        if self.description.len() > 1_024 {
            return Err(Error::TooLarge(format!("connection `{n}`: description longer than 1024 characters")));
        }
        Ok(())
    }
}

/// What the settings file holds for ReportCraft.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub connections: Vec<Connection>,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "reportcraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// Where passwords are kept, by connection name.
pub trait SecretStore: Send {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, secret: &str) -> Result<()>;
    /// Removing a secret that isn't there is not an error.
    fn delete(&self, name: &str) -> Result<()>;
}

/// The OS keychain, under the service `reportcraft` and the account `connection/<name>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

impl Keychain {
    fn entry(name: &str) -> Result<keyring::v1::Entry> {
        keyring::v1::Entry::new("reportcraft", &format!("connection/{name}")).map_err(keychain_error)
    }
}

/// A keychain failure, without anything that could carry the secret.
fn keychain_error(e: keyring::v1::Error) -> Error {
    Error::Io(format!("the system keychain: {e}"))
}

impl SecretStore for Keychain {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match Self::entry(name)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn set(&self, name: &str, secret: &str) -> Result<()> {
        Self::entry(name)?.set_password(secret).map_err(keychain_error)
    }

    fn delete(&self, name: &str) -> Result<()> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

/// Secrets in memory, for tests and sessions that must not touch the keychain.
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.0.lock().map_err(|_| Error::Io("secrets lock".into()))?.get(name).cloned())
    }

    fn set(&self, name: &str, secret: &str) -> Result<()> {
        self.0.lock().map_err(|_| Error::Io("secrets lock".into()))?.insert(name.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        self.0.lock().map_err(|_| Error::Io("secrets lock".into()))?.remove(name);
        Ok(())
    }
}

/// The saved connections and their passwords.
pub struct Connections {
    store: SettingsStore<Prefs>,
    secrets: Box<dyn SecretStore>,
}

impl std::fmt::Debug for Connections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connections").field("settings", &self.store.path()).field("count", &self.list().len()).finish_non_exhaustive()
    }
}

impl Connections {
    /// The user's connections: the settings file the suite's rules choose, and the OS keychain.
    pub fn open_user() -> Self {
        Self::open(Location::Auto, Box::new(Keychain))
    }

    /// Connections kept in memory only, with secrets in memory (tests).
    pub fn in_memory() -> Self {
        Self::open(Location::Memory, Box::<MemorySecrets>::default())
    }

    pub fn open(location: Location, secrets: Box<dyn SecretStore>) -> Self {
        let (store, _report) = SettingsStore::open(location);
        Self { store, secrets }
    }

    /// The settings file, when there is one.
    pub fn settings_path(&self) -> Option<&std::path::Path> {
        self.store.path()
    }

    pub fn list(&self) -> &[Connection] {
        &self.store.get().connections
    }

    pub fn get(&self, name: &str) -> Option<&Connection> {
        self.list().iter().find(|c| c.name == name)
    }

    /// Whether a password is kept for the connection.
    pub fn has_password(&self, name: &str) -> Result<bool> {
        Ok(self.secrets.get(name)?.is_some())
    }

    /// Saves (or replaces) a connection. `password`: `Some("")` removes the kept one, `None`
    /// keeps whatever is there. The settings file is written at once.
    pub fn save(&mut self, connection: Connection, password: Option<&str>) -> Result<()> {
        connection.check()?;
        let replacing = self.get(&connection.name).is_some();
        if !replacing && self.list().len() >= MAX_CONNECTIONS {
            return Err(Error::TooLarge(format!("more than {MAX_CONNECTIONS} connections")));
        }
        match password {
            Some("") => self.secrets.delete(&connection.name)?,
            Some(p) if !matches!(connection.target, Target::Sqlite { .. }) => self.secrets.set(&connection.name, p)?,
            Some(_) => return Err(Error::Invalid("a SQLite file has no password".into())),
            None => {}
        }
        self.store.update(|prefs| match prefs.connections.iter_mut().find(|c| c.name == connection.name) {
            Some(existing) => *existing = connection,
            None => prefs.connections.push(connection),
        });
        self.flush()
    }

    /// Forgets a connection and its password. `false` when there was none by that name.
    pub fn delete(&mut self, name: &str) -> Result<bool> {
        if self.get(name).is_none() {
            return Ok(false);
        }
        self.secrets.delete(name)?;
        self.store.update(|prefs| prefs.connections.retain(|c| c.name != name));
        self.flush()?;
        Ok(true)
    }

    fn flush(&mut self) -> Result<()> {
        self.store.flush().map_err(|e| Error::Io(format!("saving the settings: {e}")))
    }

    /// The libpq connection string for a PostgreSQL connection, with its kept password if any
    /// (else the driver's own `PGPASSWORD` applies). Hand it to the driver; never show it.
    pub fn conninfo(&self, name: &str) -> Result<String> {
        let c = self.get(name).ok_or_else(|| self.unknown(name))?;
        let Target::Postgres { host, port, user, database, sslmode } = &c.target else {
            return Err(Error::Invalid(format!("connection `{name}` is a {} database, not PostgreSQL", c.target.kind())));
        };
        let mut out = format!("host={} port={port} user={} dbname={}", quote(host), quote(user), quote(database));
        if let Some(m) = sslmode {
            out.push_str(&format!(" sslmode={m}"));
        }
        if let Some(p) = self.secrets.get(name)? {
            out.push_str(" password=");
            out.push_str(&quote(&p));
        }
        Ok(out)
    }

    /// The ADO.NET connection string of a SQL Server connection, without the password, and its kept
    /// password if any (else the driver's `SQLCMDPASSWORD` applies). Hand both to the driver; never
    /// show them.
    pub fn sqlserver(&self, name: &str) -> Result<(String, Option<String>)> {
        let c = self.get(name).ok_or_else(|| self.unknown(name))?;
        let Target::SqlServer { host, port, user, database, sslmode } = &c.target else {
            return Err(Error::Invalid(format!("connection `{name}` is a {} database, not SQL Server", c.target.kind())));
        };
        let encryption = match sslmode.as_deref() {
            Some("disable") => "Encrypt=false",
            Some("verify-full" | "verify-ca") => "Encrypt=true",
            _ => "Encrypt=true;TrustServerCertificate=true",
        };
        let ado =
            format!("Server={};User Id={};Database={};{encryption}", ado_quote(&format!("tcp:{host},{port}")), ado_quote(user), ado_quote(database));
        Ok((ado, self.secrets.get(name)?))
    }

    /// The database file of a SQLite connection.
    pub fn sqlite_path(&self, name: &str) -> Result<String> {
        let c = self.get(name).ok_or_else(|| self.unknown(name))?;
        match &c.target {
            Target::Sqlite { path } => Ok(path.clone()),
            other => Err(Error::Invalid(format!("connection `{name}` is a {} database, not SQLite", other.kind()))),
        }
    }

    fn unknown(&self, name: &str) -> Error {
        let names: Vec<&str> = self.list().iter().map(|c| c.name.as_str()).collect();
        if names.is_empty() {
            Error::NotFound(format!("connection `{name}` (no connection is saved yet)"))
        } else {
            Error::NotFound(format!("connection `{name}` (saved: {})", names.join(", ")))
        }
    }
}

/// A libpq value in single quotes, `\` and `'` escaped.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// An ADO.NET value in double quotes, inner quotes doubled.
fn ado_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(name: &str) -> Connection {
        Connection {
            name: name.into(),
            target: Target::Postgres { host: "db.example".into(), port: 5433, user: "report's".into(), database: "odoo".into(), sslmode: None },
            description: String::new(),
        }
    }

    #[test]
    fn saves_lists_and_forgets_without_writing_passwords_to_settings() {
        let dir = tempfile::tempdir().unwrap();
        let open = || Connections::open(Location::Dir(dir.path().to_owned()), Box::<MemorySecrets>::default());
        let mut c = open();
        c.save(pg("prod"), Some("s3cr'et\\")).unwrap();
        c.save(Connection { name: "local".into(), target: Target::Sqlite { path: "/data/x.db".into() }, description: "test".into() }, None).unwrap();
        assert_eq!(c.conninfo("prod").unwrap(), r"host='db.example' port=5433 user='report\'s' dbname='odoo' password='s3cr\'et\\'");
        assert_eq!(c.sqlite_path("local").unwrap(), "/data/x.db");
        assert!(c.conninfo("local").unwrap_err().to_string().contains("not PostgreSQL"));
        assert!(c.sqlite_path("nope").unwrap_err().to_string().contains("saved: prod, local"));
        let file = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(file.contains("db.example") && !file.contains("s3cr"), "{file}");
        // Reopened from the file (secrets are in a fresh memory store, so none here).
        let mut again = open();
        assert_eq!(again.list().len(), 2);
        assert_eq!(again.conninfo("prod").unwrap(), r"host='db.example' port=5433 user='report\'s' dbname='odoo'");
        assert!(again.delete("prod").unwrap());
        assert!(!again.delete("prod").unwrap());
        assert_eq!(again.list().len(), 1);
    }

    #[test]
    fn passwords_are_kept_replaced_or_removed() {
        let mut c = Connections::in_memory();
        c.save(pg("p"), Some("one")).unwrap();
        c.save(pg("p"), None).unwrap();
        assert!(c.has_password("p").unwrap(), "None keeps it");
        c.save(pg("p"), Some("")).unwrap();
        assert!(!c.has_password("p").unwrap(), "empty removes it");
        let sqlite = Connection { name: "f".into(), target: Target::Sqlite { path: "a.db".into() }, description: String::new() };
        assert!(c.save(sqlite, Some("x")).is_err());
        assert_eq!(c.list().len(), 1);
    }

    #[test]
    fn bad_definitions_are_refused() {
        let mut c = Connections::in_memory();
        for name in ["", "a b", "x/y", &"n".repeat(65)] {
            assert!(c.save(Connection { name: name.into(), ..pg("x") }, None).is_err(), "{name:?}");
        }
        let empty_host = Connection {
            target: Target::Postgres { host: " ".into(), port: 5432, user: "u".into(), database: "d".into(), sslmode: None },
            ..pg("x")
        };
        assert!(c.save(empty_host, None).is_err());
        let newline = Connection { target: Target::Sqlite { path: "a\nb".into() }, ..pg("x") };
        assert!(c.save(newline, None).is_err());
        assert!(c.list().is_empty());
        let bad_mode = Connection {
            target: Target::Postgres { host: "h".into(), port: 1, user: "u".into(), database: "d".into(), sslmode: Some("maybe".into()) },
            ..pg("x")
        };
        assert!(c.save(bad_mode, None).is_err());
        let tls = Connection {
            target: Target::Postgres { host: "h".into(), port: 1, user: "u".into(), database: "d".into(), sslmode: Some("verify-full".into()) },
            ..pg("t")
        };
        c.save(tls, None).unwrap();
        assert!(c.conninfo("t").unwrap().ends_with(" sslmode=verify-full"));
        c.delete("t").unwrap();
        // The port defaults to 5432 (JSON exercises the same serde derive as the settings file).
        let read: Connection = serde_json::from_str(r#"{"name":"n","kind":"postgres","host":"h","user":"u","database":"d"}"#).unwrap();
        assert_eq!(read.target, Target::Postgres { host: "h".into(), port: 5432, user: "u".into(), database: "d".into(), sslmode: None });
    }

    #[test]
    fn sql_server_connections_hand_the_password_apart() {
        let mut c = Connections::in_memory();
        let read: Connection =
            serde_json::from_str(r#"{"name":"mssql","kind":"sqlserver","host":"db","user":"rc \"x\"","database":"rent"}"#).unwrap();
        assert_eq!(read.target.describe(), "rc \"x\"@db:1433/rent", "port 1433 by default");
        c.save(read, Some("s3;cr\"et")).unwrap();
        let (ado, password) = c.sqlserver("mssql").unwrap();
        assert_eq!(ado, r#"Server="tcp:db,1433";User Id="rc ""x""";Database="rent";Encrypt=true;TrustServerCertificate=true"#);
        assert_eq!(password.as_deref(), Some("s3;cr\"et"));
        assert!(c.conninfo("mssql").unwrap_err().to_string().contains("not PostgreSQL"));
        c.save(pg("pg"), None).unwrap();
        assert!(c.sqlserver("pg").unwrap_err().to_string().contains("not SQL Server"));
        let strict = Connection {
            target: Target::SqlServer { host: "h".into(), port: 1, user: "u".into(), database: "d".into(), sslmode: Some("verify-full".into()) },
            ..pg("strict")
        };
        c.save(strict, None).unwrap();
        assert!(c.sqlserver("strict").unwrap().0.ends_with(";Encrypt=true"));
    }
}
