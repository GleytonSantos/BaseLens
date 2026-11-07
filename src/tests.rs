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
}
