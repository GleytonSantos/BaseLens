//! Unit tests (and two live checks that need Ollama / Claude Code:
//! `cargo test -- --ignored`).

use crate::ai::*;
use crate::db::*;

#[test]
fn finds_sql_blocks() {
    let t = "Try this:\n```sql\nSELECT 1;\n```\nand\n```python\nprint(1)\n```";
    assert_eq!(code_blocks(t), vec!["SELECT 1;".to_string()]);
}

#[test]
fn splits_and_classifies() {
    let s = split_statements("SELECT ';' AS a; -- c;\nUPDATE t SET x=1;");
    assert_eq!(s.len(), 2);
    assert!(is_read_query("SELECT * FROM t"));
    assert!(is_read_query("WITH x AS (SELECT 1) SELECT * FROM x"));
    assert!(!is_read_query("SELECT 1; DROP TABLE t"));
    assert!(!is_read_query("SELECT * INTO new_t FROM t"));
}

#[test]
fn sqlite_roundtrip() {
    let cfg = ConnConfig { kind: DbKind::Sqlite, path: ":memory:".into(), ..Default::default() };
    let h = spawn(cfg, String::new());
    assert!(matches!(h.rx.recv().unwrap(), Response::Connected { .. }));
    let _ = h.rx.recv().unwrap(); // table list
    let sql = "CREATE TABLE a(id INTEGER PRIMARY KEY, j JSON); INSERT INTO a(j) VALUES ('{\"k\":[1,2]}'); SELECT * FROM a";
    assert!(h.send(Request::Query { tag: 7, sql: sql.into(), max_rows: 10, sent: std::time::Instant::now() }));
    match h.rx.recv().unwrap() {
        Response::QueryDone { tag, results, .. } => {
            assert_eq!(tag, 7);
            let r = results.unwrap();
            assert_eq!(r.len(), 3);
            assert_eq!(r[1].affected, Some(1));
            assert!(r[2].rows[0][1].json().is_some());
        }
        _ => panic!("unexpected response"),
    }
}

#[test]
fn secrets_roundtrip() {
    // keep tests out of the real keyring
    std::env::set_var("FAVO_PASSWORD_STORE", "basic");
    let enc = crate::store::encrypt("s3cret");
    assert!(enc.starts_with("enc:"));
    assert!(!enc.contains("s3cret"));
    assert_eq!(crate::store::decrypt(&enc), "s3cret");
}

#[test]
#[ignore]
fn live_ollama() {
    let models = ollama_models(OLLAMA_URL).expect("ollama running");
    let call = Call { provider: Provider::Ollama, url: OLLAMA_URL.into(), model: models[0].clone(), key: String::new(), temperature: 0.2, claude_path: String::new() };
    let rx = ask(call, "Answer with one SQL query in a ```sql block.".into(), vec![ChatMsg { role: Role::User, text: "Count rows in table products".into() }]);
    let answer = rx.recv().unwrap().expect("answer");
    println!("{answer}");
    assert!(!code_blocks(&answer).is_empty());
}

#[test]
#[ignore]
fn live_claude_code() {
    let path = detect_claude().expect("claude installed");
    let call = Call { provider: Provider::ClaudeCode, url: String::new(), model: String::new(), key: String::new(), temperature: 0.7, claude_path: path };
    let rx = ask(call, "Answer with one SQL query in a ```sql block.".into(), vec![ChatMsg { role: Role::User, text: "Count rows in table products".into() }]);
    let answer = rx.recv().unwrap().expect("answer");
    println!("{answer}");
    assert!(!code_blocks(&answer).is_empty());
}

#[test]
fn postgres_errors_are_explained() {
    // port 1 on localhost: nothing listens there
    let cfg = ConnConfig { kind: DbKind::Postgres, host: "127.0.0.1".into(), port: 1, user: "".into(), database: "".into(), ..Default::default() };
    let h = spawn(cfg, String::new());
    match h.rx.recv().unwrap() {
        Response::Failed(e) => {
            println!("{e}");
            assert!(!e.trim().eq_ignore_ascii_case("db error"), "only the vague text: {e}");
            assert!(e.contains("Nothing answered at 127.0.0.1:1"), "{e}");
            assert!(e.contains("Tried:") && e.contains(":1 ·"), "{e}");
            // empty user falls back to the operating-system user, like the original
            let os_user = std::env::var("USER").unwrap_or_default();
            if !os_user.is_empty() {
                assert!(e.contains(&format!("Tried: {os_user}@")), "{e}");
            }
        }
        _ => panic!("should not connect"),
    }
}

#[test]
fn unreadable_saved_password_is_detected() {
    std::env::set_var("FAVO_PASSWORD_STORE", "basic");
    assert_eq!(crate::store::decrypt_checked(""), Some(String::new()));
    let enc = crate::store::encrypt("pw");
    assert_eq!(crate::store::decrypt_checked(&enc).as_deref(), Some("pw"));
    // a value made with another key (or damaged) must not turn into ""
    let damaged = format!("enc:{}", "A".repeat(40));
    assert_eq!(crate::store::decrypt_checked(&damaged), None);
}

#[test]
fn os_username_follows_the_original_order() {
    // passwd lookup alone (no variables) must still find a name
    #[cfg(unix)]
    {
        let saved: Vec<(String, Option<String>)> = crate::db::USERNAME_ENV_ORDER.iter().map(|k| (k.to_string(), std::env::var(k).ok())).collect();
        for k in crate::db::USERNAME_ENV_ORDER {
            std::env::remove_var(k);
        }
        let from_passwd = crate::db::os_username();
        let id = std::process::Command::new("id").arg("-un").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        assert_eq!(from_passwd, id);
        // SUDO_USER wins over USER, as in the original
        std::env::set_var("USER", "plain");
        std::env::set_var("SUDO_USER", "realperson");
        assert_eq!(crate::db::os_username(), "realperson");
        std::env::remove_var("SUDO_USER");
        assert_eq!(crate::db::os_username(), "plain");
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }
}

/// Live check against a PostgreSQL server:
/// `PG_PROBE_HOST=localhost PG_PROBE_PORT=55433 PG_PROBE_USER=… PG_PROBE_PASS=… PG_PROBE_DB=… cargo test probe_pg -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_pg() {
    let v = |k: &str| std::env::var(k).unwrap_or_default();
    let cfg = ConnConfig { kind: DbKind::Postgres, host: v("PG_PROBE_HOST"), port: v("PG_PROBE_PORT").parse().unwrap(), user: v("PG_PROBE_USER"), database: v("PG_PROBE_DB"), ..Default::default() };
    let h = spawn(cfg, v("PG_PROBE_PASS"));
    match h.rx.recv().unwrap() {
        Response::Connected { server, .. } => println!("CONNECTED: {server}"),
        Response::Failed(e) => println!("FAILED:\n{e}"),
        _ => println!("?"),
    }
}

/// Live diagnostics against a PostgreSQL server (same env vars as probe_pg).
#[test]
#[ignore]
fn diagnose_pg() {
    let v = |k: &str| std::env::var(k).unwrap_or_default();
    let cfg = ConnConfig { kind: DbKind::Postgres, host: v("PG_PROBE_HOST"), port: v("PG_PROBE_PORT").parse().unwrap(), user: v("PG_PROBE_USER"), database: v("PG_PROBE_DB"), ..Default::default() };
    for (ok, line) in crate::db::pg_diagnose(&cfg, &v("PG_PROBE_PASS")) {
        println!("{} {line}", if ok { "OK  " } else { "FAIL" });
    }
}

#[test]
fn connection_urls() {
    let (c, pw) = crate::db::parse_connection_url("postgresql://estante_admin:p%40ss:w@rd@localhost:5433/estante_crm_prod?sslmode=require").unwrap();
    assert_eq!((c.kind, c.user.as_str(), pw.as_str(), c.host.as_str(), c.port, c.database.as_str(), c.ssl), (DbKind::Postgres, "estante_admin", "p@ss:w@rd", "localhost", 5433, "estante_crm_prod", true));
    let (c, pw) = crate::db::parse_connection_url("mysql://root@[::1]/shop").unwrap();
    assert_eq!((c.kind, c.user.as_str(), pw.as_str(), c.host.as_str(), c.port, c.database.as_str(), c.ssl), (DbKind::Mysql, "root", "", "::1", 3306, "shop", false));
    let (c, _) = crate::db::parse_connection_url("postgres://u:p@h/db?sslmode=disable").unwrap();
    assert!(!c.ssl);
    assert!(crate::db::parse_connection_url("redis://x").is_err());
    assert_eq!(crate::db::password_host_tail("Abc123@172").as_deref(), Some("@172"));
    assert_eq!(crate::db::password_host_tail("Abc@123x"), None);
    assert_eq!(crate::db::password_host_tail("no-at"), None);
}

#[test]
fn transaction_and_cursor_rules() {
    assert!(cursor_friendly("select * from t"));
    assert!(cursor_friendly("WITH x AS (SELECT 1) SELECT * FROM x"));
    assert!(!cursor_friendly("select 1; select 2"));
    assert!(!cursor_friendly("SHOW server_version"));
    assert!(!cursor_friendly("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d"));
}

/// Live check against a real PostgreSQL:
/// `BASELENS_PG_TEST=postgresql://postgres:pw@127.0.0.1:5432/postgres cargo test live_pg -- --ignored`
#[test]
#[ignore]
fn live_pg() {
    use std::time::{Duration, Instant};
    std::env::set_var("BASELENS_HOME", std::env::temp_dir().join("baselens-livepg"));
    let url = std::env::var("BASELENS_PG_TEST").expect("BASELENS_PG_TEST");
    let (cfg, pw) = parse_connection_url(&url).unwrap();
    let h = spawn(cfg, pw);
    let wait = Duration::from_secs(30);
    let next = |want: &str| loop {
        let r = h.rx.recv_timeout(wait).expect(want);
        match (&r, want) {
            (Response::Connected { .. }, "connected") | (Response::Tables(_), "tables") | (Response::AllColumns(_), "all") | (Response::QueryDone { .. }, "done") => return r,
            (Response::Failed(e), _) => panic!("{e}"),
            _ => {}
        }
    };
    next("connected");
    next("tables");
    // all columns in one question, on the second connection
    assert!(h.send(Request::AllColumns));
    let Response::AllColumns(Ok(all)) = next("all") else { panic!() };
    let orders = &all.iter().find(|(k, _)| k == "public.orders").expect("orders").1;
    assert_eq!(orders.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "customer", "total"]);
    assert!(orders[0].pk && !orders[1].nullable && orders[2].nullable);
    let items = &all.iter().find(|(k, _)| k == "public.items").unwrap().1;
    assert_eq!(items.iter().filter(|c| c.pk).count(), 2);
    assert!(all.iter().any(|(k, _)| k == "public.big"));
    let run = |sql: &str, max: usize| {
        assert!(h.send(Request::Query { tag: 1, sql: sql.into(), max_rows: max, sent: Instant::now() }));
        match next("done") {
            Response::QueryDone { results, timing, .. } => (results, timing),
            _ => unreachable!(),
        }
    };
    // big result: stops at the limit, in batches (200, 250, then 1 to see there is more)
    let (r, t) = run("select * from big", 450);
    let r = r.unwrap();
    assert_eq!(r[0].rows.len(), 450);
    assert!(r[0].truncated);
    assert_eq!(t.batches, 3);
    // exact fit: not marked as cut
    let (r, _) = run("select * from generate_series(1,200)", 200);
    assert!(!r.unwrap()[0].truncated);
    // empty result still has its columns
    let (r, _) = run("select id, customer from orders where false", 100);
    assert_eq!(r.unwrap()[0].columns, ["id", "customer"]);
    // an error comes back readable and the connection keeps working
    let (r, _) = run("select * from nope", 100);
    assert!(r.unwrap_err().contains("42P01"));
    let (r, _) = run("select 1 as one", 100);
    assert_eq!(r.unwrap()[0].rows.len(), 1);
    // the person's own transaction is not committed by the cursor
    let (r, _) = run("BEGIN; INSERT INTO orders(customer) VALUES ('tx-test')", 100);
    r.unwrap();
    let (r, _) = run("select count(*) from orders where customer = 'tx-test'", 100);
    assert_eq!(r.unwrap()[0].rows[0][0].display(), "1");
    let (r, _) = run("ROLLBACK", 100);
    r.unwrap();
    let (r, _) = run("select count(*) from orders where customer = 'tx-test'", 100);
    assert_eq!(r.unwrap()[0].rows[0][0].display(), "0");
    // typed values come back as numbers, not text
    let (r, _) = run("select 1::int as i, 2.5::float8 as f, true as b", 10);
    let r = r.unwrap();
    assert!(matches!(r[0].rows[0][0], Cell::Int(1)) && matches!(r[0].rows[0][1], Cell::Float(_)));
    // a long query is stopped on the server
    assert!(h.send(Request::Query { tag: 2, sql: "select pg_sleep(30)".into(), max_rows: 10, sent: Instant::now() }));
    std::thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    assert!(h.cancel());
    match next("done") {
        Response::QueryDone { results, .. } => assert!(results.unwrap_err().contains("57014")),
        _ => unreachable!(),
    }
    assert!(t.elapsed() < Duration::from_secs(5));
    // and the connection still works afterwards
    let (r, _) = run("select 1", 10);
    assert!(r.is_ok());
}

/// SQLite's default busy handler, as read in the original (0x685df20):
/// delays[i] for the i-th wait, totals[i] already waited; past 11 waits,
/// 100 ms each; the last wait is cut to what is left of the timeout.
fn busy_sleep(count: usize, timeout_ms: i64) -> Option<i64> {
    const DELAYS: [i64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
    const TOTALS: [i64; 12] = [0, 1, 3, 8, 18, 33, 53, 78, 103, 128, 178, 228];
    let (delay, prior) = if count < 12 { (DELAYS[count], TOTALS[count]) } else { (100, TOTALS[11] + (count as i64 - 11) * 100) };
    if prior + delay > timeout_ms {
        let left = timeout_ms - prior;
        return if left <= 0 { None } else { Some(left) };
    }
    Some(delay)
}

#[test]
fn busy_handler_schedule_matches_the_original() {
    // the table and formula: 12th wait onwards = count*100 - 872 ms already waited
    assert_eq!(busy_sleep(12, i64::MAX), Some(100));
    assert_eq!(TOTAL_AT_12, 328);
    let mut waited = 0;
    let mut n = 0;
    while let Some(d) = busy_sleep(n, SQLITE_BUSY_TIMEOUT_MS as i64) {
        waited += d;
        n += 1;
    }
    assert_eq!(waited, 5000);
    assert_eq!(n, 59);
}
const TOTAL_AT_12: i64 = 12 * 100 - 872;

#[test]
fn sqlite_waits_for_a_busy_file_instead_of_failing() {
    use std::time::Duration;
    let dir = std::env::temp_dir().join(format!("baselens-busy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("busy.sqlite");
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("CREATE TABLE IF NOT EXISTS t(x); BEGIN IMMEDIATE; INSERT INTO t VALUES (1);").unwrap();
    let cfg = ConnConfig { kind: DbKind::Sqlite, path: path.to_string_lossy().into(), ..Default::default() };
    let h = spawn(cfg, String::new());
    assert!(matches!(h.rx.recv().unwrap(), Response::Connected { .. }));
    let _ = h.rx.recv().unwrap();
    // the other program finishes its write after 400 ms
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        other.execute_batch("COMMIT").unwrap();
    });
    assert!(h.send(Request::Query { tag: 1, sql: "INSERT INTO t VALUES (2)".into(), max_rows: 10, sent: std::time::Instant::now() }));
    match h.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Response::QueryDone { results, .. } => assert!(results.is_ok(), "{:?}", results.err()),
        _ => panic!("unexpected response"),
    }
    release.join().unwrap();
}

#[test]
fn read_only_gate_blocks_side_effect_functions() {
    // security review #2: a SELECT that writes or runs SQL given as text
    assert!(!is_read_query("SELECT setval('s', 1)"));
    assert!(!is_read_query("select nextval('orders_id_seq')"));
    assert!(!is_read_query("SELECT lo_import('/etc/shadow')"));
    assert!(!is_read_query("SELECT query_to_xml('delete from t', true, true, '')"));
    assert!(!is_read_query("SELECT pg_terminate_backend(123)"));
    assert!(is_read_query("SELECT count(*) FROM orders"));
    // the AI may only auto-run reads without files, sleeps or network links
    for q in ["SELECT pg_read_file('/etc/passwd')", "SELECT pg_sleep(9999)", "select * from dblink('host=evil', 'select 1') as t(x int)", "SELECT LOAD_FILE('/etc/passwd')", "select sleep(100)", "SELECT readfile('x')"] {
        assert!(!is_safe_auto_query(q), "{q}");
    }
    assert!(is_safe_auto_query("SELECT name, total FROM orders WHERE total > 10 LIMIT 20"));
}

#[test]
fn pin_hash_is_argon2id_and_old_hashes_still_unlock() {
    let h = crate::store::hash_pin_argon2("123456");
    assert!(h.starts_with("$argon2id$"));
    assert_eq!(crate::store::verify_pin("123456", "", &h), (true, false));
    assert_eq!(crate::store::verify_pin("654321", "", &h).0, false);
    // a settings file from an older version: unlocks and asks to be upgraded
    let old = crate::store::hash_pin("123456", "salt");
    assert_eq!(crate::store::verify_pin("123456", "salt", &old), (true, true));
    assert_eq!(crate::store::verify_pin("000000", "salt", &old), (false, false));
    assert_eq!(crate::store::verify_pin("", "", "").0, false);
}

#[cfg(unix)]
#[test]
fn debug_log_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("baselens-dbglog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("BASELENS_HOME", &dir);
    std::env::set_var("BASELENS_DEBUG", "1");
    crate::debug::init(&[]);
    crate::debug::log("select secret from t");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir.join("debug.log")), 0o600);
    assert_eq!(mode(&dir), 0o700);
}

#[test]
fn tls_floor_is_tls12() {
    assert_eq!(TLS_MIN_VERSION, "TLSv1.2");
    for v in ["TLSv1", "TLSv1.1", "SSLv3"] {
        assert!(tls_version_too_old(v), "{v}");
    }
    for v in ["TLSv1.2", "TLSv1.3"] {
        assert!(!tls_version_too_old(v), "{v}");
    }
}

#[test]
fn splits_postgres_dollar_quoted_bodies() {
    let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RAISE NOTICE 'a;b'; RETURN 1; END; $$ LANGUAGE plpgsql; SELECT f();";
    let s = split_statements(sql);
    assert_eq!(s.len(), 2, "{s:?}");
    assert!(s[0].ends_with("LANGUAGE plpgsql"));
    assert_eq!(s[1], "SELECT f()");
    // named tags, another tag inside, and $1 parameters
    let s = split_statements("DO $body$ BEGIN PERFORM $x$;$x$; END $body$; SELECT $1::int; SELECT 'it''s; fine'");
    assert_eq!(s.len(), 3, "{s:?}");
    assert_eq!(s[1], "SELECT $1::int");
    // a DO block is not a read
    let w = ["DE", "LETE"].concat();
    assert!(!is_read_query(&format!("DO $$ BEGIN {w} FROM t; END $$")));
}

#[test]
fn comment_marks_inside_strings_do_not_hide_writes() {
    // security: `--` inside a string used to hide the rest of the line
    let w = ["DE", "LETE"].concat();
    assert!(!is_read_query("SELECT '--', pg_terminate_backend(1)"));
    assert!(!is_read_query(&format!("SELECT '/*', 1 FROM t; {w} FROM t -- */")));
    assert!(!is_read_query("SELECT $$--$$, setval('s', 1)"));
    assert!(is_read_query(&format!("SELECT '-- not a comment' AS x -- a real one: {w}")));
    assert!(is_read_query(&format!("/* {w} */ SELECT 1")));
}

#[test]
fn postgres_values_get_their_types() {
    use postgres::types::Type;
    assert!(matches!(pg_cell(Some("42"), Some(&Type::INT4)), Cell::Int(42)));
    assert!(matches!(pg_cell(Some("-9007199254740993"), Some(&Type::INT8)), Cell::Int(-9007199254740993)));
    assert!(matches!(pg_cell(Some("1.5"), Some(&Type::FLOAT8)), Cell::Float(f) if f == 1.5));
    assert!(matches!(pg_cell(Some("12.50"), Some(&Type::NUMERIC)), Cell::Float(f) if f == 12.5));
    // too many digits for a float: kept as text, nothing lost
    assert!(matches!(pg_cell(Some("12345678901234567890.123"), Some(&Type::NUMERIC)), Cell::Text(_)));
    assert!(matches!(pg_cell(Some("t"), Some(&Type::BOOL)), Cell::Text(ref s) if s == "true"));
    assert!(matches!(pg_cell(Some("\\xdeadbeef"), Some(&Type::BYTEA)), Cell::Bytes(ref b) if b == &vec![0xde, 0xad, 0xbe, 0xef]));
    assert!(matches!(pg_cell(Some("2026-01-02 03:04:05"), Some(&Type::TIMESTAMP)), Cell::Text(_)));
    assert!(matches!(pg_cell(None, Some(&Type::INT4)), Cell::Null));
    assert!(matches!(pg_cell(Some("7"), None), Cell::Text(_)));
}

#[test]
fn literals_follow_the_database() {
    let b = Cell::Bytes(vec![0xde, 0xad]);
    assert_eq!(b.sql_literal_for(DbKind::Postgres), "'\\xdead'::bytea");
    assert_eq!(b.sql_literal_for(DbKind::Sqlite), "X'dead'");
    assert_eq!(Cell::Float(f64::NAN).sql_literal_for(DbKind::Postgres), "'NaN'::float8");
    assert_eq!(Cell::Int(3).sql_literal_for(DbKind::Postgres), "3");
}

#[test]
fn sqlite_query_can_be_stopped() {
    use std::time::{Duration, Instant};
    let cfg = ConnConfig { kind: DbKind::Sqlite, path: ":memory:".into(), ..Default::default() };
    let h = spawn(cfg, String::new());
    assert!(matches!(h.rx.recv().unwrap(), Response::Connected { .. }));
    let _ = h.rx.recv().unwrap();
    // a query that would run for a very long time
    let slow = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
    assert!(h.send(Request::Query { tag: 9, sql: slow.into(), max_rows: 10, sent: Instant::now() }));
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    assert!(h.cancel());
    match h.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Response::QueryDone { results, .. } => assert!(results.unwrap_err().to_lowercase().contains("interrupt")),
        _ => panic!("unexpected response"),
    }
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn address_order_follows_node() {
    use std::net::SocketAddr;
    let a = |s: &str| s.parse::<SocketAddr>().unwrap();
    // first answer IPv6: v6, v4, v6, v4…; duplicates dropped
    let got = interleave_families(&[a("[2001:db8::1]:5432"), a("[2001:db8::2]:5432"), a("192.0.2.1:5432"), a("[2001:db8::1]:5432"), a("192.0.2.2:5432")]);
    let want = vec![a("[2001:db8::1]:5432"), a("192.0.2.1:5432"), a("[2001:db8::2]:5432"), a("192.0.2.2:5432")];
    assert_eq!(got, want);
    // first answer IPv4: v4 first
    let got = interleave_families(&[a("192.0.2.1:1"), a("[2001:db8::1]:1")]);
    assert!(got[0].is_ipv4() && got[1].is_ipv6());
    assert_eq!(AUTOSELECT_ATTEMPT_MS, 250);
    // literals and single addresses are left to the driver
    assert_eq!(autoselect_address("127.0.0.1", 1), None);
}
