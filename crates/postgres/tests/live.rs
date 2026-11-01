//! Tests against a real PostgreSQL server. They run only when `REPORTCRAFT_TEST_POSTGRES` holds a
//! connection string to a scratch database the tests may fill; otherwise they say so and pass.
//! The database is set up by the tests themselves (schema `rc_live`, role `rc_live_reader`).
//!
//! Example: `REPORTCRAFT_TEST_POSTGRES="host=127.0.0.1 port=54329 user=rc dbname=rc_test" cargo test -p reportcraft-postgres`

use std::sync::Once;
use std::time::Duration;

use postgres::{Client, NoTls};
use reportcraft_core::{Error, FieldType, Value};
use reportcraft_data::{DataSource, SchemaSource, TableKind};
use reportcraft_postgres::PostgresSource;

fn conninfo() -> Option<String> {
    let c = std::env::var("REPORTCRAFT_TEST_POSTGRES").ok().filter(|s| !s.is_empty());
    if c.is_none() {
        eprintln!("REPORTCRAFT_TEST_POSTGRES not set: live PostgreSQL test skipped");
    }
    c
}

static SETUP: Once = Once::new();

/// Creates the test schema once per run (as the connecting user, who must be allowed to).
fn setup(conninfo: &str) {
    SETUP.call_once(|| {
        let mut c = Client::connect(conninfo, NoTls).unwrap();
        c.batch_execute(
            r#"
            DROP SCHEMA IF EXISTS rc_live CASCADE;
            CREATE SCHEMA rc_live;
            CREATE TABLE rc_live.branch (code text PRIMARY KEY, city text NOT NULL);
            CREATE TABLE rc_live.secret (id int PRIMARY KEY, pin text);
            CREATE TABLE rc_live.rental (
              id bigserial PRIMARY KEY,
              branch_code text REFERENCES rc_live.branch(code),
              secret_id int REFERENCES rc_live.secret(id),
              days smallint, amount numeric(10,2), paid boolean, started date, note text, ratio double precision);
            CREATE TABLE rc_live."odd ""name""" (a int, b int, PRIMARY KEY (b, a));
            INSERT INTO rc_live.branch VALUES ('LYO', 'Lyon'), ('ANN', 'Annecy');
            INSERT INTO rc_live.secret VALUES (1, '1234');
            INSERT INTO rc_live.rental (branch_code, secret_id, days, amount, paid, started, note, ratio) VALUES
              ('LYO', 1, 12, 540.00, true, '2026-10-01', 'first', 0.5),
              ('ANN', NULL, 4, 720.50, false, '2026-10-03', NULL, 'NaN'),
              ('LYO', NULL, 3, NULL, NULL, NULL, 'it''s "quoted"', NULL);
            INSERT INTO rc_live."odd ""name""" SELECT g, g FROM generate_series(1, 2500) g;
            CREATE VIEW rc_live.lyon AS SELECT * FROM rc_live.rental WHERE branch_code = 'LYO';
            DO $$ BEGIN
              IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'rc_live_reader') THEN CREATE ROLE rc_live_reader LOGIN; END IF;
            END $$;
            GRANT USAGE ON SCHEMA rc_live TO rc_live_reader;
            GRANT SELECT ON rc_live.branch, rc_live.rental, rc_live.lyon TO rc_live_reader;
            "#,
        )
        .unwrap();
    });
}

fn source(conninfo: &str, sql: &str) -> PostgresSource {
    PostgresSource::new(conninfo, sql)
}

#[test]
fn reads_typed_rows() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    let rs = source(&c, "SELECT branch_code, days, amount, paid, started, note, ratio, days * 2 AS twice FROM rc_live.rental ORDER BY id")
        .read(None)
        .unwrap();
    let types: Vec<FieldType> = rs.columns.iter().map(|c| c.field_type).collect();
    use FieldType::{Bool, Integer, Number, Text};
    assert_eq!(types, [Text, Integer, Number, Bool, Text, Text, Number, Integer]);
    assert_eq!(rs.rows.len(), 3);
    assert_eq!(rs.rows[0][2], Value::Number(540.0));
    assert_eq!(rs.rows[0][3], Value::Bool(true));
    assert_eq!(rs.rows[0][4], Value::Text("2026-10-01".into()));
    assert_eq!(rs.rows[1][6], Value::Text("NaN".into()), "NaN stays text");
    assert_eq!(rs.rows[2][5], Value::Text(r#"it's "quoted""#.into()));
    assert!(rs.rows[2][2].is_null());
    assert_eq!(rs.rows[2][7], Value::Int(6));
}

#[test]
fn reads_in_batches_up_to_the_limit() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    let all = source(&c, r#"SELECT * FROM rc_live."odd ""name""""#).read(None).unwrap();
    assert_eq!(all.rows.len(), 2500, "more than one cursor batch");
    let some = source(&c, r#"SELECT * FROM rc_live."odd ""name""""#).read(Some(1234)).unwrap();
    assert_eq!(some.rows.len(), 1234);
}

#[test]
fn refuses_anything_that_is_not_a_single_read() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    for q in [
        "DELETE FROM rc_live.rental",
        "UPDATE rc_live.rental SET days = 0",
        "INSERT INTO rc_live.branch VALUES ('X', 'x')",
        "DROP TABLE rc_live.rental",
        "CREATE TABLE rc_live.t (a int)",
        "SELECT 1; DELETE FROM rc_live.rental",
        "WITH d AS (DELETE FROM rc_live.rental RETURNING *) SELECT * FROM d",
        "SELECT nextval('rc_live.rental_id_seq')",
        "COMMIT",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
        "",
    ] {
        assert!(source(&c, q).read(None).is_err(), "{q:?} should be refused");
    }
    let n = source(&c, "SELECT count(*) AS n FROM rc_live.rental").read(None).unwrap();
    assert_eq!(n.rows[0][0], Value::Int(3), "nothing was written");
}

#[test]
fn errors_are_clear_and_long_queries_are_cancelled() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    assert!(matches!(source(&c, "SELEC nope").read(None), Err(Error::Invalid(_))));
    assert!(matches!(source(&c, "SELECT * FROM rc_live.missing").read(None), Err(Error::Invalid(_))));
    let slow = source(&c, "SELECT pg_sleep(5)").with_timeout(Duration::from_millis(300)).read(None);
    assert!(matches!(slow, Err(Error::TooLarge(_))), "{slow:?}");
    let nowhere = PostgresSource::new("host=127.0.0.1 port=1 user=x password=s3cret connect_timeout=2", "SELECT 1").read(None);
    let e = nowhere.unwrap_err();
    assert!(matches!(e, Error::Io(_)) && !e.to_string().contains("s3cret"), "{e}");
}

#[test]
fn describes_schema_keys_and_relationships() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    let schema = PostgresSource::database(&c).schema().unwrap();
    let rental = schema.table("rc_live.rental").unwrap();
    assert_eq!(rental.kind, TableKind::Table);
    let id = &rental.columns[0];
    assert_eq!((id.name.as_str(), id.declared_type.as_str(), id.primary_key, id.nullable), ("id", "bigint", Some(1), false));
    let amount = rental.columns.iter().find(|c| c.name == "amount").unwrap();
    assert_eq!((amount.declared_type.as_str(), amount.field_type), ("numeric(10,2)", FieldType::Number));
    assert_eq!(rental.foreign_keys.len(), 2);
    assert!(rental.foreign_keys.iter().any(|f| f.references_table == "rc_live.branch" && f.references_columns == ["code"]));
    assert_eq!(schema.table("rc_live.lyon").unwrap().kind, TableKind::View);
    let odd = schema.table(r#"rc_live.odd "name""#).unwrap();
    assert_eq!(odd.columns.iter().map(|c| c.primary_key).collect::<Vec<_>>(), [Some(2), Some(1)], "composite key order");
}

#[test]
fn a_reader_sees_only_what_it_may_read() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    let reader = format!("{c} user=rc_live_reader");
    let schema = PostgresSource::database(&reader).schema().unwrap();
    assert!(schema.table("rc_live.rental").is_some());
    assert!(schema.table("rc_live.secret").is_none(), "no SELECT privilege, not listed");
    let fk = schema.table("rc_live.rental").unwrap().foreign_keys.iter().find(|f| f.columns == ["secret_id"]).cloned().unwrap();
    assert!(fk.references_table.starts_with("<table "), "the hidden target is not named");
    assert!(PostgresSource::new(&reader, "SELECT * FROM rc_live.secret").read(None).is_err());
}

#[test]
fn previews_listed_tables_only() {
    let Some(c) = conninfo() else { return };
    setup(&c);
    let src = PostgresSource::database(&c);
    let rs = src.preview(r#"rc_live.odd "name""#, 5).unwrap();
    assert_eq!(rs.rows.len(), 5);
    assert!(matches!(src.preview("rc_live.rental; DROP TABLE rc_live.rental", 5), Err(Error::NotFound(_))));
    assert_eq!(src.preview("rc_live.rental", 10).unwrap().rows.len(), 3);
}
