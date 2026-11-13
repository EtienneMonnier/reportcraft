# Pitfalls

Mistakes already made, and the rule for each. Add one in the same PR when you make a new one.

- **Rust from rustup in `~/.cargo/bin`** may not be on `PATH` in a fresh shell:
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Layout units**: the model is in millimetres from the band's corner; `Document` items are in
  millimetres from the page's corner (margins already added). Don't add the margin twice.
- **Live PostgreSQL tests are skipped unless `REPORTCRAFT_TEST_POSTGRES` is set.** Run them against a
  throwaway server, never a real one (they create and drop the `rc_live` schema):
  `initdb -D /tmp/pg -U rc --auth=trust`, then
  `pg_ctl -D /tmp/pg -o "-p 54329 -c unix_socket_directories='' -c listen_addresses=127.0.0.1" start`
  (the socket is disabled because a deep working folder makes its path longer than PostgreSQL's
  107-byte limit), `createdb -h 127.0.0.1 -p 54329 -U rc rc_test`, and
  `REPORTCRAFT_TEST_POSTGRES="host=127.0.0.1 port=54329 user=rc dbname=rc_test" cargo test -p reportcraft-postgres`.
- **Live SQL Server tests are skipped unless `REPORTCRAFT_TEST_SQLSERVER` is set.** Same rule: a
  throwaway server only (they drop and create the `rc_live` database and create the
  `rc_live_reader` login), for example a container:
  `podman run -d --name rc-mssql -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD=… -p 127.0.0.1:14339:1433 mcr.microsoft.com/mssql/server:2022-latest`,
  then `REPORTCRAFT_TEST_SQLSERVER="Server=tcp:127.0.0.1,14339;User Id=sa;Password=…;TrustServerCertificate=true" cargo test -p reportcraft-sqlserver`.
- **`std::time::Instant::now` is refused by clippy** (it traps on wasm). Native-only crates on
  tokio use `tokio::time::Instant`; others read the clock off wasm with the lint allowed at the call.
