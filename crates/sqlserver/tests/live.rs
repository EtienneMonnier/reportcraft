//! Tests against a real SQL Server. They run only when `REPORTCRAFT_TEST_SQLSERVER` holds an
//! ADO.NET connection string to a scratch server where the login may create a database;
//! otherwise they say so and pass. The tests set up database `rc_live` (dropped and created
//! again) and login `rc_live_reader` themselves.
//!
//! Example: `REPORTCRAFT_TEST_SQLSERVER="Server=tcp:127.0.0.1,14339;User Id=sa;Password=…;TrustServerCertificate=true" cargo test -p reportcraft-sqlserver`

use std::collections::BTreeMap;
use std::sync::Once;
use std::time::Duration;

use reportcraft_core::{Error, FieldType, Value};
use reportcraft_data::{DataSource, Dialect, SchemaSource, TableKind, bind};
use reportcraft_model::ParamValue;
use reportcraft_sqlserver::SqlServerSource;
use tokio_util::compat::TokioAsyncWriteCompatExt;

const READER_PASSWORD: &str = "Reader-Live-2026!";

fn server() -> Option<String> {
    let c = std::env::var("REPORTCRAFT_TEST_SQLSERVER").ok().filter(|s| !s.is_empty());
    if c.is_none() {
        eprintln!("REPORTCRAFT_TEST_SQLSERVER not set: live SQL Server test skipped");
    }
    c
}

/// The test database, as the configured login.
fn database(server: &str) -> String {
    format!("{server};Database=rc_live")
}

static SETUP: Once = Once::new();

/// Runs each batch in turn on `connection`.
fn execute(connection: &str, batches: &[&str]) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let config = tiberius::Config::from_ado_string(connection).unwrap();
        let tcp = tokio::net::TcpStream::connect(config.get_addr()).await.unwrap();
        let mut client = tiberius::Client::connect(config, tcp.compat_write()).await.unwrap();
        for b in batches {
            let run = async { client.simple_query(*b).await?.into_results().await };
            run.await.unwrap_or_else(|e| panic!("{b}: {e}"));
        }
    });
}

/// Creates the test database once per run.
fn setup(server: &str) {
    SETUP.call_once(|| {
        execute(
            server,
            &[
                "IF DB_ID('rc_live') IS NOT NULL BEGIN ALTER DATABASE rc_live SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE rc_live END",
                "CREATE DATABASE rc_live",
                &format!(
                    "IF SUSER_ID('rc_live_reader') IS NULL CREATE LOGIN rc_live_reader WITH PASSWORD = '{READER_PASSWORD}', CHECK_POLICY = OFF"
                ),
            ],
        );
        execute(
            &database(server),
            &[
                "CREATE SCHEMA sales",
                "CREATE TABLE branch (code nvarchar(10) PRIMARY KEY, city nvarchar(40) NOT NULL);
                 CREATE TABLE secret (id int PRIMARY KEY, pin nvarchar(10));
                 CREATE TABLE sales.rental (
                   id bigint IDENTITY PRIMARY KEY,
                   branch_code nvarchar(10) REFERENCES branch(code),
                   secret_id int REFERENCES secret(id),
                   days smallint, amount decimal(10,2), paid bit, started date, note nvarchar(max), ratio float,
                   stamp datetime2(0), tiny tinyint);
                 CREATE TABLE [odd ]]name] (a int NOT NULL, b int NOT NULL, PRIMARY KEY (b, a));
                 INSERT INTO branch VALUES ('LYO', 'Lyon'), ('ANN', 'Annecy');
                 INSERT INTO secret VALUES (1, '1234');
                 INSERT INTO sales.rental (branch_code, secret_id, days, amount, paid, started, note, ratio, stamp, tiny) VALUES
                   ('LYO', 1, 12, 540.00, 1, '2026-10-01', N'first', 0.5, '2026-10-01 08:30:00', 7),
                   ('ANN', NULL, 4, 720.50, 0, '2026-10-03', NULL, NULL, NULL, NULL),
                   ('LYO', NULL, 3, NULL, NULL, NULL, N'it''s \"quoted\" é', NULL, NULL, NULL);
                 INSERT INTO [odd ]]name] SELECT TOP 2500 n, n FROM
                   (SELECT ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS n FROM sys.all_objects a CROSS JOIN sys.all_objects b) g",
                "CREATE VIEW lyon AS SELECT * FROM sales.rental WHERE branch_code = 'LYO'",
                "CREATE USER rc_live_reader FOR LOGIN rc_live_reader;
                 GRANT SELECT ON branch TO rc_live_reader;
                 GRANT SELECT ON sales.rental TO rc_live_reader;
                 GRANT SELECT ON lyon TO rc_live_reader",
            ],
        );
    });
}

fn source(server: &str, sql: &str) -> SqlServerSource {
    SqlServerSource::new(database(server), sql)
}

#[test]
fn reads_typed_rows() {
    let Some(s) = server() else { return };
    setup(&s);
    let rs = source(&s, "SELECT branch_code, days, amount, paid, started, note, ratio, days * 2 AS twice, stamp, tiny FROM sales.rental ORDER BY id")
        .read(None)
        .unwrap();
    let types: Vec<FieldType> = rs.columns.iter().map(|c| c.field_type).collect();
    use FieldType::{Bool, Integer, Number, Text};
    assert_eq!(types, [Text, Integer, Number, Bool, Text, Text, Number, Integer, Text, Integer]);
    assert_eq!(rs.rows.len(), 3);
    assert_eq!(rs.rows[0][2], Value::Number(540.0));
    assert_eq!(rs.rows[1][2], Value::Number(720.5));
    assert_eq!(rs.rows[0][3], Value::Bool(true));
    assert_eq!(rs.rows[0][4], Value::Text("2026-10-01".into()));
    assert_eq!(rs.rows[0][6], Value::Number(0.5));
    assert_eq!(rs.rows[0][8], Value::Text("2026-10-01 08:30:00".into()));
    assert_eq!(rs.rows[0][9], Value::Int(7));
    assert_eq!(rs.rows[2][5], Value::Text(r#"it's "quoted" é"#.into()));
    assert!(rs.rows[2][2].is_null());
    assert_eq!(rs.rows[2][7], Value::Int(6));
}

#[test]
fn reads_up_to_the_limit() {
    let Some(s) = server() else { return };
    setup(&s);
    let all = source(&s, "SELECT * FROM [odd ]]name]").read(None).unwrap();
    assert_eq!(all.rows.len(), 2500);
    let some = source(&s, "SELECT * FROM [odd ]]name] ORDER BY a").read(Some(1234)).unwrap();
    assert_eq!(some.rows.len(), 1234);
    // The connection stopped mid-stream is cleaned up; the next read works.
    assert_eq!(source(&s, "SELECT 1 AS one").read(None).unwrap().rows, vec![vec![Value::Int(1)]]);
}

#[test]
fn refuses_anything_that_is_not_a_single_read() {
    let Some(s) = server() else { return };
    setup(&s);
    for q in [
        "DELETE FROM sales.rental",
        "UPDATE sales.rental SET days = 0",
        "INSERT INTO branch VALUES ('X', 'x')",
        "DROP TABLE sales.rental",
        "CREATE TABLE t (a int)",
        "SELECT 1; DELETE FROM sales.rental",
        "SELECT 1 DELETE FROM sales.rental",
        "WITH d AS (SELECT * FROM sales.rental) DELETE FROM d",
        "SELECT * INTO copy FROM branch",
        "COMMIT",
        "SELECT 1 COMMIT",
        "EXEC sp_who",
        "SET IMPLICIT_TRANSACTIONS OFF",
        "",
    ] {
        assert!(source(&s, q).read(None).is_err(), "{q:?} should be refused");
    }
    let n = source(&s, "SELECT COUNT(*) AS n FROM sales.rental").read(None).unwrap();
    assert_eq!(n.rows[0][0], Value::Int(3), "nothing was written");
    let n = source(&s, "SELECT COUNT(*) AS n FROM sys.tables WHERE name = 'copy'").read(None).unwrap();
    assert_eq!(n.rows[0][0], Value::Int(0), "nothing was created");
}

/// Long enough to be stopped: counting to a hundred million, one step at a time.
const SLOW: &str = "WITH n AS (SELECT CAST(1 AS bigint) AS i UNION ALL SELECT i + 1 FROM n WHERE i < 100000000)
                    SELECT MAX(i) AS last_i FROM n OPTION (MAXRECURSION 0)";

#[test]
fn errors_are_clear_and_long_queries_are_cancelled() {
    let Some(s) = server() else { return };
    setup(&s);
    assert!(matches!(source(&s, "SELECT nope FROM").read(None), Err(Error::Invalid(_))));
    assert!(matches!(source(&s, "SELECT * FROM missing").read(None), Err(Error::Invalid(_))));
    #[allow(clippy::disallowed_methods)] // native test
    let started = std::time::Instant::now();
    let slow = source(&s, SLOW).with_timeout(Duration::from_millis(500)).read(None);
    assert!(matches!(slow, Err(Error::TooLarge(_))), "{slow:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "stopped at the time limit");
    let cancel = reportcraft_core::Cancel::default();
    let raiser = cancel.clone();
    #[allow(clippy::disallowed_methods)] // native test
    let started = std::time::Instant::now();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        raiser.cancel();
    });
    let stopped = source(&s, SLOW).with_cancel(cancel).read(None);
    stopper.join().unwrap();
    assert!(matches!(stopped, Err(Error::Cancelled(_))), "{stopped:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "cancelled, not left to finish");
    let big = source(&s, "SELECT REPLICATE('x', 1000) AS x FROM [odd ]]name]").with_max_bytes(1 << 20).read(None);
    assert!(matches!(&big, Err(Error::TooLarge(m)) if m.contains("MiB")), "{big:?}");
    let nowhere = SqlServerSource::new("Server=tcp:127.0.0.1,1;User Id=x;Password=s3cret", "SELECT 1").read(None);
    let e = nowhere.unwrap_err();
    assert!(matches!(e, Error::Io(_)) && !e.to_string().contains("s3cret"), "{e}");
    let wrong = SqlServerSource::new(format!("{s};User Id=rc_live_reader;Password=wrong-s3cret"), "SELECT 1").read(None).unwrap_err();
    assert!(matches!(wrong, Error::Io(_)) && wrong.to_string().contains("Login failed") && !wrong.to_string().contains("s3cret"), "{wrong}");
    let apart = SqlServerSource::new(format!("{};User Id=rc_live_reader", database(&s)), "SELECT COUNT(*) AS n FROM branch")
        .with_password(Some(READER_PASSWORD.to_owned()))
        .read(None)
        .unwrap();
    assert_eq!(apart.rows[0][0], Value::Int(2), "a password given apart logs in");
}

#[test]
fn describes_schema_keys_and_relationships() {
    let Some(s) = server() else { return };
    setup(&s);
    let schema = SqlServerSource::database(database(&s)).schema().unwrap();
    let rental = schema.table("sales.rental").unwrap();
    assert_eq!(rental.kind, TableKind::Table);
    let id = &rental.columns[0];
    assert_eq!((id.name.as_str(), id.declared_type.as_str(), id.primary_key, id.nullable), ("id", "bigint", Some(1), false));
    let amount = rental.columns.iter().find(|c| c.name == "amount").unwrap();
    assert_eq!((amount.declared_type.as_str(), amount.field_type), ("decimal(10,2)", FieldType::Number));
    let note = rental.columns.iter().find(|c| c.name == "note").unwrap();
    assert_eq!(note.declared_type, "nvarchar(max)");
    assert_eq!(schema.table("branch").unwrap().columns[0].declared_type, "nvarchar(10)");
    assert_eq!(rental.foreign_keys.len(), 2);
    assert!(rental.foreign_keys.iter().any(|f| f.references_table == "branch" && f.references_columns == ["code"]));
    assert_eq!(schema.table("lyon").unwrap().kind, TableKind::View);
    let odd = schema.table("odd ]name").unwrap();
    assert_eq!(odd.columns.iter().map(|c| c.primary_key).collect::<Vec<_>>(), [Some(2), Some(1)], "composite key order");
}

#[test]
fn a_reader_sees_only_what_it_may_read() {
    let Some(s) = server() else { return };
    setup(&s);
    let reader = format!("{};User Id=rc_live_reader;Password={READER_PASSWORD}", database(&s));
    let schema = SqlServerSource::database(&reader).schema().unwrap();
    assert!(schema.table("sales.rental").is_some());
    assert!(schema.table("secret").is_none(), "no SELECT permission, not listed");
    let fk = schema.table("sales.rental").unwrap().foreign_keys.iter().find(|f| f.columns == ["secret_id"]).cloned();
    if let Some(fk) = fk {
        assert!(fk.references_table.starts_with("<table "), "the hidden target is not named");
    }
    let denied = SqlServerSource::new(&reader, "SELECT * FROM secret").read(None).unwrap_err();
    assert!(denied.to_string().contains("not allowed"), "{denied}");
}

#[test]
fn previews_listed_tables_only() {
    let Some(s) = server() else { return };
    setup(&s);
    let src = SqlServerSource::database(database(&s));
    assert_eq!(src.preview("odd ]name", 5).unwrap().rows.len(), 5);
    assert!(matches!(src.preview("branch; DROP TABLE branch", 5), Err(Error::NotFound(_))));
    assert_eq!(src.preview("sales.rental", 10).unwrap().rows.len(), 3);
}

#[test]
fn binds_typed_parameters() {
    let Some(s) = server() else { return };
    setup(&s);
    let d = |t: &str| chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").unwrap();
    let values = BTreeMap::from([
        ("branches".to_owned(), ParamValue::List(vec![ParamValue::Text("LYO".into()), ParamValue::Text("ANN".into())])),
        ("min".to_owned(), ParamValue::Integer(4)),
        ("amount".to_owned(), ParamValue::Number(500.25)),
        ("period".to_owned(), ParamValue::DateRange(d("2026-10-01"), d("2026-10-02"))),
        ("paid".to_owned(), ParamValue::Boolean(true)),
    ]);
    let declared = ["branches", "min", "amount", "period", "paid", "nothing"];
    let sql = "SELECT id FROM sales.rental WHERE branch_code IN (:branches) AND days >= :min AND amount > :amount
               AND started BETWEEN :period.from AND :period.to AND paid = :paid AND (:nothing IS NULL) ORDER BY id";
    let b = bind(sql, &values, &declared, Dialect::SqlServer).unwrap();
    let rs = SqlServerSource::new(database(&s), b.sql).with_binds(b.values).read(None).unwrap();
    assert_eq!(rs.rows, vec![vec![Value::Int(1)]], "smallint, decimal, date, bit and a list compared with typed values");
    let evil = bind(
        "SELECT COUNT(*) FROM sales.rental WHERE branch_code = :b",
        &BTreeMap::from([("b".to_owned(), ParamValue::Text("x' OR '1'='1".into()))]),
        &["b"],
        Dialect::SqlServer,
    )
    .unwrap();
    let n = SqlServerSource::new(database(&s), evil.sql).with_binds(evil.values).read(None).unwrap();
    assert_eq!(n.rows[0][0], Value::Int(0));
}
