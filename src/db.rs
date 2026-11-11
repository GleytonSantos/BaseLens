//! Database connections. Each open connection runs on its own worker thread
//! and talks to the UI through channels, so the window never freezes.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub const PG_CONNECT_TIMEOUT_MS: u64 = 15000; // MECHANICS: [db.postgres] connectionTimeout
pub const MYSQL_CONNECT_TIMEOUT_MS: u64 = 60000; // MECHANICS: [db.mysql] "1 minute"
/// Rows asked from the server per round trip (first batch); later batches
/// double up to PG_FETCH_MAX so big results don't need hundreds of trips.
pub const PG_FETCH_BATCH: usize = 200;
/// How long a SQLite file that another program is writing is waited for
/// before "database is locked" (better-sqlite3's default `timeout`, 5000 ms).
/// SQLite's default busy handler (0x685df20 in the original) sleeps
/// 1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100 ms, then 100 ms each time,
/// and the last sleep is cut so the total never passes the timeout.
pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 5000;
/// Oldest TLS version accepted, as in the original: Node's
/// `tls.DEFAULT_MIN_VERSION = 'TLSv1.2'` (no `--tls-min-v1.x` flag), applied
/// by `SecureContext::Init` 0x2ed35b0 / `SetMinProto` 0x2ed4e40 through
/// SSL_CTX_set_min_proto_version (0x303 = TLS 1.2; max 0x304 = TLS 1.3;
/// SSLv2/SSLv3 methods are refused outright).
pub const TLS_MIN_VERSION: &str = "TLSv1.2";
/// Most rows read for one Properties tab.
pub const PROPS_MAX_ROWS: usize = 5000;

/// Picking between IPv6 and IPv4 addresses, as the original does (Node's
/// "network family autoselection", on by default): `EnvironmentOptions`
/// constructor 0x2dd42f0 sets `network_family_autoselection = true` (+0x108)
/// and `network_family_autoselection_attempt_timeout = 0xfa` (+0x110) =
/// 250 ms; `net.js` raises anything below 10 ms to 10.
pub const AUTOSELECT_ATTEMPT_MS: u64 = 250;
pub const AUTOSELECT_MIN_MS: u64 = 10;

/// Node's order (`lookupAndConnectMultiple` in net.js): addresses grouped by
/// family, the family of the first answer first, each address once, then
/// alternating between the two families.
pub fn interleave_families(addrs: &[std::net::SocketAddr]) -> Vec<std::net::SocketAddr> {
    let Some(first) = addrs.first() else { return vec![] };
    let first_v6 = first.is_ipv6();
    let (mut a, mut b): (Vec<std::net::SocketAddr>, Vec<std::net::SocketAddr>) = (vec![], vec![]);
    for x in addrs {
        let group = if x.is_ipv6() == first_v6 { &mut a } else { &mut b };
        if !group.iter().any(|y| y.ip() == x.ip()) {
            group.push(*x);
        }
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    for i in 0..a.len().max(b.len()) {
        if let Some(x) = a.get(i) {
            out.push(*x);
        }
        if let Some(x) = b.get(i) {
            out.push(*x);
        }
    }
    out
}

/// When a host name has several addresses (typically IPv6 and IPv4), finds
/// one that answers, trying them in Node's order and giving each 250 ms, so a
/// broken IPv6 route doesn't cost the driver's whole connect timeout. None
/// means "let the driver try as usual" (one address, an IP literal, or none
/// answered that fast).
pub fn autoselect_address(host: &str, port: u16) -> Option<std::net::IpAddr> {
    use std::net::{TcpStream, ToSocketAddrs};
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let addrs: Vec<_> = (host, port).to_socket_addrs().ok()?.collect();
    let order = interleave_families(&addrs);
    if order.len() < 2 {
        return None;
    }
    let wait = Duration::from_millis(AUTOSELECT_ATTEMPT_MS.max(AUTOSELECT_MIN_MS));
    order.iter().find(|a| TcpStream::connect_timeout(a, wait).is_ok()).map(|a| a.ip())
}

/// True for a negotiated TLS version older than TLS 1.2 ("TLSv1", "TLSv1.1",
/// "SSLv3"...).
pub fn tls_version_too_old(v: &str) -> bool {
    let v = v.trim();
    matches!(v, "TLSv1" | "TLSv1.0" | "TLSv1.1" | "SSLv3" | "SSLv2")
}
/// TCP keep-alive probes once a socket is on, as libuv sets them
/// (uv__tcp_keepalive 0x2a19260): TCP_KEEPINTVL = 1 s, TCP_KEEPCNT = 10.
pub const TCP_KEEPALIVE_INTERVAL_S: u64 = 1;
pub const TCP_KEEPALIVE_COUNT: u32 = 10;
/// Idle time before the first probe. libuv refuses 0 (-EINVAL), so a value is
/// needed; 60 s keeps proxies (Cloud SQL Proxy, NAT) from dropping an idle
/// connection without the person noticing. BaseLens choice.
pub const TCP_KEEPALIVE_IDLE_S: u64 = 60;
pub const PG_FETCH_MAX: usize = 6400;
/// Tables whose columns are loaded one by one when a database can't list
/// them all in one query (SQLite).
pub const COLUMNS_PREFETCH_TABLES: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum DbKind {
    #[default]
    Sqlite,
    Postgres,
    Mysql,
}

impl DbKind {
    pub const ALL: [DbKind; 3] = [DbKind::Sqlite, DbKind::Postgres, DbKind::Mysql];
    pub fn label(self) -> &'static str {
        match self {
            DbKind::Sqlite => "SQLite",
            DbKind::Postgres => "PostgreSQL",
            DbKind::Mysql => "MySQL / MariaDB",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            DbKind::Sqlite => "sqlite",
            DbKind::Postgres => "postgresql",
            DbKind::Mysql => "mysql",
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Sqlite => 0,
            DbKind::Postgres => 5432,
            DbKind::Mysql => 3306,
        }
    }
    pub fn quote(self, ident: &str) -> String {
        match self {
            DbKind::Mysql => format!("`{}`", ident.replace('`', "``")),
            _ => format!("\"{}\"", ident.replace('"', "\"\"")),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ConnConfig {
    pub id: u64,
    pub name: String,
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Encrypted with store::encrypt; never stored in clear text.
    pub password_enc: String,
    pub save_password: bool,
    pub database: String,
    pub path: String,
    pub ssl: bool,
    pub ssl_verify: bool,
    pub read_only: bool,
    pub color: [u8; 3],
    /// reach the database through an SSH tunnel (see tunnel.rs)
    pub ssh: bool,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    /// private key file; empty = ssh-agent / ~/.ssh defaults
    pub ssh_key: String,
    /// local end of the tunnel while connected (never saved)
    #[serde(skip)]
    pub via: Option<u16>,
}

impl Default for ConnConfig {
    fn default() -> Self {
        ConnConfig {
            id: 0,
            name: String::new(),
            kind: DbKind::Postgres,
            host: "localhost".into(),
            port: 5432,
            user: String::new(),
            password_enc: String::new(),
            save_password: true,
            database: String::new(),
            path: String::new(),
            ssl: false,
            ssl_verify: true,
            read_only: false,
            color: [74, 208, 255],
            ssh: false,
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_user: String::new(),
            ssh_key: String::new(),
            via: None,
        }
    }
}

impl ConnConfig {
    /// "via bastion" when the connection goes through an SSH tunnel.
    pub fn via_label(&self) -> Option<String> {
        (self.ssh && self.kind != DbKind::Sqlite && !self.ssh_host.trim().is_empty()).then(|| format!("via ssh {}", self.ssh_host.trim()))
    }
    pub fn subtitle(&self) -> String {
        match self.kind {
            DbKind::Sqlite => self.path.clone(),
            _ => format!("{}@{}:{}/{}", self.user, self.host, self.port, self.database),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Cell {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl Cell {
    pub fn display(&self) -> String {
        match self {
            Cell::Null => "(NULL)".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Cell::Text(s) => s.clone(),
            Cell::Bytes(b) => {
                // MECHANICS: binaryEncoding = hex
                let mut s = String::from("0x");
                for x in b.iter().take(64) {
                    s.push_str(&format!("{x:02x}"));
                }
                if b.len() > 64 {
                    s.push('…');
                }
                s
            }
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(*i as f64),
            Cell::Float(f) => Some(*f),
            Cell::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    /// Text holding a JSON object or array.
    pub fn json(&self) -> Option<serde_json::Value> {
        if let Cell::Text(s) = self {
            let t = s.trim_start();
            if t.starts_with('{') || t.starts_with('[') {
                return serde_json::from_str(s).ok();
            }
        }
        None
    }
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            Cell::Null => Value::Null,
            Cell::Int(i) => Value::from(*i),
            Cell::Float(f) => serde_json::Number::from_f64(*f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Cell::Text(s) => self.json().unwrap_or_else(|| Value::String(s.clone())),
            Cell::Bytes(_) => Value::String(self.display()),
        }
    }
    /// The value as SQL for one database: PostgreSQL writes bytes as
    /// '\x…'::bytea (X'…' is a bit string there) and NaN/Infinity quoted.
    pub fn sql_literal_for(&self, kind: DbKind) -> String {
        match (self, kind) {
            (Cell::Bytes(b), DbKind::Postgres) => {
                let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
                format!("'\\x{h}'::bytea")
            }
            (Cell::Float(f), DbKind::Postgres) if !f.is_finite() => {
                format!("'{}'::float8", if f.is_nan() { "NaN" } else if *f > 0.0 { "Infinity" } else { "-Infinity" })
            }
            _ => self.sql_literal(),
        }
    }
    pub fn sql_literal(&self) -> String {
        match self {
            Cell::Null => "NULL".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Float(f) => f.to_string(),
            Cell::Text(s) => sql_string(s),
            Cell::Bytes(b) => {
                let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
                format!("X'{h}'")
            }
        }
    }
}

pub fn sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[derive(Clone, Debug, Default)]
pub struct QueryResult {
    pub sql: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub affected: Option<u64>,
    pub elapsed_ms: f64,
    pub truncated: bool,
    /// Round trips used to read the rows (0 = all in one reply).
    pub batches: usize,
}

/// Where the time of one query went (shown by the debug mode).
#[derive(Clone, Debug, Default)]
pub struct Timing {
    /// Waiting behind other work on the connection's queue.
    pub queue_ms: f64,
    /// Sending the query and receiving every row.
    pub run_ms: f64,
    pub rows: usize,
    /// Size of the received values as text.
    pub bytes: usize,
    pub batches: usize,
    /// Requests still waiting on the queue when this one started.
    pub queue_behind: usize,
    /// a transaction is open on the connection after this query
    pub in_tx: bool,
}

/// Where the time of opening a connection went (debug mode only measures
/// DNS and TCP separately; TLS + login is the rest).
#[derive(Clone, Debug, Default)]
pub struct ConnectTiming {
    pub dns_ms: f64,
    pub tcp_ms: f64,
    pub tls_login_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug)]
pub struct TableInfo {
    pub schema: Option<String>,
    pub name: String,
    pub is_view: bool,
}

impl TableInfo {
    pub fn key(&self) -> String {
        match &self.schema {
            Some(s) => format!("{s}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub pk: bool,
}

#[derive(Clone, Debug)]
pub struct ForeignKey {
    pub from_table: String,
    pub from_col: String,
    pub to_table: String,
    pub to_col: String,
}

pub enum Request {
    Refresh,
    Columns { schema: Option<String>, table: String },
    /// Columns of every table at once (one query on Postgres/MySQL).
    AllColumns,
    Query { tag: u64, sql: String, max_rows: usize, sent: Instant },
    Graph { tag: u64 },
    /// manual transactions on/off (auto-commit off = every change waits for
    /// Commit or Rollback)
    SetManualCommit(bool),
    /// the Properties panel of one table (catalog queries, read-only)
    Properties { tag: u64, kind: DbKind, schema: Option<String>, table: String, is_view: bool },
}

impl Request {
    /// Schema questions go to the second connection so they never make a
    /// query wait.
    fn is_meta(&self) -> bool {
        matches!(self, Request::Columns { .. } | Request::AllColumns | Request::Graph { .. } | Request::Properties { .. })
    }
}

pub enum Response {
    Connected { server: String, databases: Vec<String>, current_db: String, timing: ConnectTiming },
    Failed(String),
    Tables(Vec<TableInfo>),
    Columns { table: String, cols: Result<Vec<ColumnInfo>, String> },
    AllColumns(Result<Vec<(String, Vec<ColumnInfo>)>, String>),
    QueryDone { tag: u64, results: Result<Vec<QueryResult>, String>, timing: Timing },
    Graph { tag: u64, tables: Vec<(String, Vec<ColumnInfo>)>, fks: Vec<ForeignKey> },
    /// manual-commit mode and whether a transaction is open now
    TxState { manual: bool, open: bool, error: Option<String> },
    Properties { tag: u64, props: crate::props::TableProps },
}

trait Backend {
    fn server(&mut self) -> String;
    fn databases(&mut self) -> Vec<String>;
    fn current_db(&mut self) -> String;
    fn tables(&mut self) -> Result<Vec<TableInfo>, String>;
    fn columns(&mut self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>, String>;
    fn foreign_keys(&mut self, tables: &[TableInfo]) -> Vec<ForeignKey>;
    fn run(&mut self, sql: &str, max_rows: usize) -> Result<Vec<QueryResult>, String>;
    /// How to stop a running query from another thread.
    fn canceller(&self) -> Option<Canceller> {
        None
    }
    /// Manual transactions: when on, the first statement after a COMMIT or
    /// ROLLBACK opens a transaction that stays open until the next one.
    fn set_manual_commit(&mut self, _on: bool) -> Result<(), String> {
        Err("Manual transactions aren't available for this database.".into())
    }
    /// A transaction is open right now.
    fn in_transaction(&mut self) -> bool {
        false
    }
    /// Columns of every table, keyed like TableInfo::key. Default: one
    /// question per table (fine for local SQLite files).
    fn all_columns(&mut self) -> Result<Vec<(String, Vec<ColumnInfo>)>, String> {
        let tables = self.tables()?;
        Ok(tables
            .iter()
            .take(COLUMNS_PREFETCH_TABLES)
            .map(|t| (t.key(), self.columns(t.schema.as_deref(), &t.name).unwrap_or_default()))
            .collect())
    }
}

/// Stops the query running on a connection, from another thread (PostgreSQL
/// cancel request, SQLite interrupt, MySQL `KILL QUERY` on a second link).
pub type Canceller = std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub struct Handle {
    /// Queries, refreshes: the main connection.
    pub tx: Sender<Request>,
    /// Schema questions: a second connection (None for SQLite, which is local).
    meta: Option<Sender<Request>>,
    pub rx: Receiver<Response>,
    /// Requests waiting on the main connection right now.
    pub queue: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// set by the worker once connected
    canceller: std::sync::Arc<std::sync::Mutex<Option<Canceller>>>,
}

impl Handle {
    pub fn send(&self, req: Request) -> bool {
        use std::sync::atomic::Ordering;
        if req.is_meta() {
            if let Some(m) = &self.meta {
                return m.send(req).is_ok();
            }
        }
        self.queue.fetch_add(1, Ordering::SeqCst);
        let ok = self.tx.send(req).is_ok();
        if !ok {
            self.queue.fetch_sub(1, Ordering::SeqCst);
        }
        ok
    }
    /// Asks the server to stop the running query (in the background).
    /// Returns false when this connection can't be stopped yet.
    pub fn cancel(&self) -> bool {
        let c = self.canceller.lock().ok().and_then(|g| g.clone());
        match c {
            Some(c) => {
                std::thread::spawn(move || {
                    if let Err(e) = c() {
                        crate::debug::log(&format!("cancel failed: {e}"));
                    }
                });
                true
            }
            None => false,
        }
    }
    pub fn queued(&self) -> usize {
        self.queue.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn open_backend(cfg: &ConnConfig, password: &str) -> Result<Box<dyn Backend>, String> {
    match cfg.kind {
        DbKind::Sqlite => open_sqlite(cfg).map(|b| Box::new(b) as Box<dyn Backend>),
        DbKind::Postgres => open_pg(cfg, password).map(|b| Box::new(b) as Box<dyn Backend>),
        DbKind::Mysql => open_mysql(cfg, password).map(|b| Box::new(b) as Box<dyn Backend>),
    }
}

/// DNS and TCP measured on their own (debug mode): the drivers don't report
/// their steps, so the rest of the connect time is TLS + login.
fn probe_network(cfg: &ConnConfig) -> (f64, f64) {
    use std::net::{TcpStream, ToSocketAddrs};
    if cfg.kind == DbKind::Sqlite {
        return (0.0, 0.0);
    }
    let t = Instant::now();
    let addrs: Vec<_> = (cfg.host.as_str(), cfg.port).to_socket_addrs().map(|a| a.collect()).unwrap_or_default();
    let dns = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let tcp = addrs.first().map(|a| TcpStream::connect_timeout(a, Duration::from_secs(5)).is_ok()).unwrap_or(false);
    let tcp_ms = if tcp { t.elapsed().as_secs_f64() * 1000.0 } else { 0.0 };
    (dns, tcp_ms)
}

/// Size of the values as text (what came over the wire, roughly).
pub fn result_bytes(r: &QueryResult) -> usize {
    r.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| match c {
                    Cell::Null => 0,
                    Cell::Int(_) | Cell::Float(_) => 8,
                    Cell::Text(t) => t.len(),
                    Cell::Bytes(b) => b.len(),
                })
                .sum::<usize>()
        })
        .sum()
}

fn graph_of(b: &mut Box<dyn Backend>) -> (Vec<(String, Vec<ColumnInfo>)>, Vec<ForeignKey>) {
    let tables = b.tables().unwrap_or_default();
    let all: std::collections::HashMap<String, Vec<ColumnInfo>> = b.all_columns().unwrap_or_default().into_iter().collect();
    let out = tables
        .iter()
        .filter(|t| !t.is_view)
        .take(80)
        .map(|t| (t.name.clone(), all.get(&t.key()).cloned().unwrap_or_default()))
        .collect();
    let fks = b.foreign_keys(&tables);
    (out, fks)
}

/// The table and its direct neighbours by foreign key, for the per-table diagram.
fn table_erd(b: &mut Box<dyn Backend>, schema: Option<&str>, table: &str) -> Option<crate::props::Erd> {
    let tables = b.tables().ok()?;
    let fks: Vec<ForeignKey> = b.foreign_keys(&tables).into_iter().filter(|f| f.from_table == table || f.to_table == table).collect();
    let mut names: Vec<String> = vec![table.to_string()];
    for f in &fks {
        for n in [&f.from_table, &f.to_table] {
            if !names.contains(n) && names.len() <= crate::props::ERD_MAX_NEIGHBOURS {
                names.push(n.clone());
            }
        }
    }
    let cols: Vec<(String, Vec<ColumnInfo>)> = names
        .iter()
        .map(|n| {
            // prefer the same schema when a name exists in several
            let t = tables.iter().find(|t| &t.name == n && t.schema.as_deref() == schema).or_else(|| tables.iter().find(|t| &t.name == n));
            let c = t.and_then(|t| b.columns(t.schema.as_deref(), &t.name).ok()).unwrap_or_default();
            (n.clone(), c)
        })
        .collect();
    let fks = fks.into_iter().filter(|f| names.contains(&f.from_table) && names.contains(&f.to_table)).collect();
    Some(crate::props::Erd { center: table.to_string(), tables: cols, fks })
}

/// Answers a schema question (on whichever connection runs it).
fn answer_meta(b: &mut Box<dyn Backend>, req: Request, tx: &Sender<Response>) {
    match req {
        Request::Columns { schema, table } => {
            let cols = b.columns(schema.as_deref(), &table);
            let key = match &schema {
                Some(s) => format!("{s}.{table}"),
                None => table.clone(),
            };
            let _ = tx.send(Response::Columns { table: key, cols });
        }
        Request::AllColumns => {
            let _ = tx.send(Response::AllColumns(b.all_columns()));
        }
        Request::Graph { tag } => {
            let (tables, fks) = graph_of(b);
            let _ = tx.send(Response::Graph { tag, tables, fks });
        }
        Request::Properties { tag, kind, schema, table, is_view } => {
            // one query per tab; a failing one only fills its own tab
            let answers = crate::props::queries(kind, schema.as_deref(), &table, is_view)
                .into_iter()
                .map(|(name, sql)| {
                    let r = b.run(&sql, PROPS_MAX_ROWS).and_then(|v| v.into_iter().rev().find(|x| !x.columns.is_empty()).ok_or_else(|| "no answer".to_string()));
                    (name, r)
                })
                .collect();
            let mut props = crate::props::assemble(kind, schema.as_deref(), &table, is_view, answers);
            props.erd = table_erd(b, schema.as_deref(), &table);
            let _ = tx.send(Response::Properties { tag, props });
        }
        _ => {}
    }
}

pub fn spawn(cfg: ConnConfig, password: String) -> Handle {
    use std::sync::atomic::Ordering;
    let (tx_req, rx_req) = channel::<Request>();
    let (tx_res, rx_res) = channel::<Response>();
    let queue = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // local port of the SSH tunnel, once the main worker opened it
    let via_slot: std::sync::Arc<std::sync::Mutex<Option<u16>>> = Default::default();
    let via_main = via_slot.clone();
    // second connection for schema questions; opened on the first question,
    // which only comes after the main connection worked
    let meta = if cfg.kind == DbKind::Sqlite {
        None
    } else {
        let (tx_meta, rx_meta) = channel::<Request>();
        let (mut cfg2, pw2, tx2) = (cfg.clone(), password.clone(), tx_res.clone());
        let via2 = via_slot.clone();
        std::thread::spawn(move || {
            let mut b: Option<Box<dyn Backend>> = None;
            while let Ok(req) = rx_meta.recv() {
                if b.is_none() {
                    cfg2.via = via2.lock().ok().and_then(|v| *v);
                    match open_backend(&cfg2, &pw2) {
                        Ok(x) => b = Some(x),
                        Err(e) => {
                            let _ = match req {
                                Request::Columns { table, .. } => tx2.send(Response::Columns { table, cols: Err(e) }),
                                Request::AllColumns => tx2.send(Response::AllColumns(Err(e))),
                                Request::Graph { tag } => tx2.send(Response::Graph { tag, tables: vec![], fks: vec![] }),
                                Request::Properties { tag, .. } => tx2.send(Response::Properties {
                                    tag,
                                    props: crate::props::TableProps { sections: vec![crate::props::Section { name: "Error", body: crate::props::Body::Note(e.clone()) }], erd: None },
                                }),
                                _ => Ok(()),
                            };
                            continue;
                        }
                    }
                }
                if let Some(b) = b.as_mut() {
                    answer_meta(b, req, &tx2);
                }
            }
        });
        Some(tx_meta)
    };
    let q = queue.clone();
    let canceller: std::sync::Arc<std::sync::Mutex<Option<Canceller>>> = Default::default();
    let cancel_slot = canceller.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let (dns_ms, tcp_ms) = if crate::debug::on() { probe_network(&cfg) } else { (0.0, 0.0) };
        // SSH tunnel first; it lives as long as this worker
        let mut cfg = cfg;
        let _tunnel = if cfg.ssh && cfg.kind != DbKind::Sqlite {
            match crate::tunnel::open(&cfg) {
                Ok(t) => {
                    cfg.via = Some(t.port);
                    if let Ok(mut v) = via_main.lock() {
                        *v = Some(t.port);
                    }
                    Some(t)
                }
                Err(e) => {
                    let _ = tx_res.send(Response::Failed(format!("SSH tunnel to {}: {e}", cfg.ssh_host)));
                    return;
                }
            }
        } else {
            None
        };
        let t1 = Instant::now();
        let mut b = match open_backend(&cfg, &password) {
            Ok(b) => b,
            Err(e) => {
                let _ = tx_res.send(Response::Failed(e));
                return;
            }
        };
        let login_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let timing = ConnectTiming {
            dns_ms,
            tcp_ms,
            tls_login_ms: (login_ms - if crate::debug::on() { dns_ms + tcp_ms } else { 0.0 }).max(0.0),
            total_ms: t0.elapsed().as_secs_f64() * 1000.0,
        };
        if let Ok(mut slot) = cancel_slot.lock() {
            *slot = b.canceller();
        }
        let server = b.server();
        let databases = b.databases();
        let current_db = b.current_db();
        let _ = tx_res.send(Response::Connected { server, databases, current_db, timing });
        let send_tables = |b: &mut Box<dyn Backend>| match b.tables() {
            Ok(t) => {
                let _ = tx_res.send(Response::Tables(t));
            }
            Err(e) => {
                let _ = tx_res.send(Response::QueryDone { tag: 0, results: Err(e), timing: Timing::default() });
            }
        };
        send_tables(&mut b);
        while let Ok(req) = rx_req.recv() {
            let behind = q.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
            match req {
                Request::Refresh => send_tables(&mut b),
                Request::SetManualCommit(on) => {
                    let error = b.set_manual_commit(on).err();
                    let open = b.in_transaction();
                    let _ = tx_res.send(Response::TxState { manual: on && error.is_none(), open, error });
                }
                Request::Query { tag, sql, max_rows, sent } => {
                    let queue_ms = sent.elapsed().as_secs_f64() * 1000.0;
                    let t = Instant::now();
                    let results = b.run(&sql, max_rows);
                    let mut timing = Timing { queue_ms, run_ms: t.elapsed().as_secs_f64() * 1000.0, queue_behind: behind, ..Default::default() };
                    timing.in_tx = b.in_transaction();
                    if let Ok(r) = &results {
                        timing.rows = r.iter().map(|x| x.rows.len()).sum();
                        timing.batches = r.iter().map(|x| x.batches).sum();
                        if crate::debug::on() {
                            timing.bytes = r.iter().map(result_bytes).sum();
                        }
                    }
                    if tx_res.send(Response::QueryDone { tag, results, timing }).is_err() {
                        break;
                    }
                }
                other => answer_meta(&mut b, other, &tx_res),
            }
        }
    });
    Handle { tx: tx_req, meta, rx: rx_res, queue, canceller }
}

/// Length of a PostgreSQL dollar-quote tag starting at `i` (`$$` or
/// `$name$`, name = letter or `_` then letters, digits, `_`), or None. `$1`
/// (a parameter) is not a tag, nor is `$` inside a word (`a$b$`).
fn dollar_tag(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'$') {
        return None;
    }
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    let mut j = i + 1;
    if chars.get(j) == Some(&'$') {
        return Some(2);
    }
    match chars.get(j) {
        Some(c) if c.is_alphabetic() || *c == '_' => {}
        _ => return None,
    }
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    (chars.get(j) == Some(&'$')).then_some(j + 1 - i)
}

/// `sqlite3_complete`'s transition table, read from the original at
/// 0x20fd790 (used by the function at 0x685c030). Rows are states
/// 0 INVALID, 1 START, 2 NORMAL, 3 EXPLAIN, 4 CREATE, 5 TRIGGER, 6 SEMI,
/// 7 END; columns are token classes SEMI, WS, OTHER, EXPLAIN, CREATE, TEMP,
/// TRIGGER, END.
pub const SQLITE_COMPLETE_TRANS: [[u8; 8]; 8] = [
    [1, 0, 2, 3, 4, 2, 2, 2],
    [1, 1, 2, 3, 4, 2, 2, 2],
    [1, 2, 2, 2, 2, 2, 2, 2],
    [1, 3, 3, 2, 4, 2, 2, 2],
    [1, 4, 2, 2, 2, 4, 5, 2],
    [6, 5, 5, 5, 5, 5, 5, 5],
    [6, 6, 5, 5, 5, 5, 5, 7],
    [1, 7, 5, 5, 5, 5, 5, 5],
];

/// `sqlite3_complete` (0x685c030 in the original): true when the text ends
/// with a complete statement. Whitespace and comments are class WS; '…', "…",
/// `…` and […] are OTHER (unterminated → incomplete); `;` is SEMI; the words
/// EXPLAIN, CREATE, TEMP/TEMPORARY, TRIGGER and END have their own classes
/// (compared without case, through the upper/lower table at 0x20fd090).
/// Inside CREATE … TRIGGER only "; END ;" ends the statement, so a CASE … END
/// in the body doesn't.
pub fn sqlite_complete(sql: &str) -> bool {
    const SEMI: usize = 0;
    const WS: usize = 1;
    const OTHER: usize = 2;
    let z = sql.as_bytes();
    let mut state = 0usize;
    let mut i = 0;
    while i < z.len() {
        let c = z[i];
        let token = match c {
            b';' => SEMI,
            b' ' | b'\r' | b'\t' | b'\n' | 0x0c => WS,
            b'/' if z.get(i + 1) == Some(&b'*') => {
                match sql[i + 2..].find("*/") {
                    Some(e) => i += 2 + e + 1,
                    None => return false,
                }
                WS
            }
            b'-' if z.get(i + 1) == Some(&b'-') => {
                while i < z.len() && z[i] != b'\n' {
                    i += 1;
                }
                if i >= z.len() {
                    return state == 1;
                }
                WS
            }
            b'[' => {
                match sql[i + 1..].find(']') {
                    Some(e) => i += 1 + e,
                    None => return false,
                }
                OTHER
            }
            b'`' | b'"' | b'\'' => {
                match sql[i + 1..].find(c as char) {
                    Some(e) => i += 1 + e,
                    None => return false,
                }
                OTHER
            }
            _ if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                let start = i;
                while i + 1 < z.len() && (z[i + 1].is_ascii_alphanumeric() || z[i + 1] == b'_' || z[i + 1] == b'$' || z[i + 1] >= 0x80) {
                    i += 1;
                }
                match sql[start..=i].to_ascii_uppercase().as_str() {
                    "EXPLAIN" => 3,
                    "CREATE" => 4,
                    "TEMP" | "TEMPORARY" => 5,
                    "TRIGGER" => 6,
                    "END" => 7,
                    _ => OTHER,
                }
            }
            _ => OTHER,
        };
        state = SQLITE_COMPLETE_TRANS[state][token] as usize;
        i += 1;
    }
    state == 1
}

/// True while a `CREATE [TEMP] TRIGGER … BEGIN` body hasn't reached its END
/// yet (`bare` = the statement so far, strings kept whole, comments out). The
/// end is decided by SQLite's own rule (`sqlite_complete`): "; END ;". Only
/// for triggers with a BEGIN body (SQLite, MySQL); PostgreSQL triggers
/// (EXECUTE FUNCTION …) have no END and end at their first `;`.
fn trigger_body_open(bare: &str) -> bool {
    let up = bare.to_ascii_uppercase();
    // words only, ignoring the inside of strings and quoted names
    let mut words: Vec<String> = Vec::new();
    let mut w = String::new();
    let mut quote: Option<char> = None;
    for ch in up.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' || ch == '`' || ch == '[' => {
                quote = Some(if ch == '[' { ']' } else { ch });
                if !w.is_empty() {
                    words.push(std::mem::take(&mut w));
                }
            }
            None if ch.is_ascii_alphanumeric() || ch == '_' => w.push(ch),
            None => {
                if !w.is_empty() {
                    words.push(std::mem::take(&mut w));
                }
            }
        }
    }
    if !w.is_empty() {
        words.push(w);
    }
    if words.first().map(String::as_str) != Some("CREATE") || !words.iter().take(4).any(|x| x == "TRIGGER") {
        return false;
    }
    if !words.iter().any(|x| x == "BEGIN") {
        return false;
    }
    !sqlite_complete(&format!("{bare};"))
}

/// One pass over SQL that knows about '…' "…" `…` strings (a doubled quote
/// stays inside), PostgreSQL $tag$…$tag$ bodies, -- line comments and
/// /* */ comments. Returns the statements split on `;` outside all of
/// those, each with and without its comments.
fn scan_sql(sql: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let (mut cur, mut bare) = (String::new(), String::new());
    let mut i = 0;
    let flush = |cur: &mut String, bare: &mut String, out: &mut Vec<(String, String)>| {
        if !bare.trim().is_empty() {
            out.push((cur.trim().to_string(), bare.trim().to_string()));
        }
        cur.clear();
        bare.clear();
    };
    while i < n {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' {
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == c {
                    if chars.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            let t: String = chars[start..i].iter().collect();
            cur.push_str(&t);
            bare.push_str(&t);
        } else if let Some(len) = dollar_tag(&chars, i) {
            let tag: Vec<char> = chars[i..i + len].to_vec();
            let start = i;
            i += len;
            // the body runs to the same tag
            while i < n && !(chars[i] == '$' && chars[i..].starts_with(&tag)) {
                i += 1;
            }
            i = (i + len).min(n);
            let t: String = chars[start..i].iter().collect();
            cur.push_str(&t);
            bare.push_str(&t);
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < n && chars[i] != '\n' {
                cur.push(chars[i]);
                i += 1;
            }
            bare.push(' ');
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i < n && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(n);
            cur.extend(chars[start..i].iter());
            bare.push(' ');
        } else if c == ';' && trigger_body_open(&bare) {
            // inside CREATE TRIGGER … BEGIN … END the `;` ends a step, not the statement
            cur.push(';');
            bare.push(';');
            i += 1;
        } else if c == ';' {
            flush(&mut cur, &mut bare, &mut out);
            i += 1;
        } else {
            cur.push(c);
            bare.push(c);
            i += 1;
        }
    }
    flush(&mut cur, &mut bare, &mut out);
    out
}

/// Splits a script into statements on `;`, ignoring semicolons inside
/// quotes, PostgreSQL dollar-quoted bodies and comments.
pub fn split_statements(sql: &str) -> Vec<String> {
    scan_sql(sql).into_iter().map(|(s, _)| s).collect()
}

/// The SQL without its comments; strings and dollar-quoted bodies are kept
/// whole (a `--` inside a string is not a comment).
fn strip_comments(s: &str) -> String {
    scan_sql(s).into_iter().map(|(_, b)| b).collect::<Vec<_>>().join(";\n")
}

/// Functions that change data or the server even inside a SELECT (or run a
/// query given as text). They make a statement count as a write.
pub const WRITE_FUNCTIONS: &[&str] = &[
    // PostgreSQL
    "NEXTVAL", "SETVAL", "LO_IMPORT", "LO_EXPORT", "LO_UNLINK", "LO_CREATE", "LO_CREAT", "LO_FROM_BYTEA", "LO_PUT",
    "LOWRITE", "LO_TRUNCATE", "PG_TERMINATE_BACKEND", "PG_CANCEL_BACKEND", "PG_RELOAD_CONF", "PG_ROTATE_LOGFILE",
    "SET_CONFIG", "DBLINK_EXEC", "PG_SWITCH_WAL", "PG_CREATE_RESTORE_POINT", "PG_PROMOTE", "PG_LOGICAL_EMIT_MESSAGE",
    "PG_CREATE_LOGICAL_REPLICATION_SLOT", "PG_CREATE_PHYSICAL_REPLICATION_SLOT", "PG_DROP_REPLICATION_SLOT",
    "PG_ADVISORY_LOCK", "PG_ADVISORY_XACT_LOCK", "PG_NOTIFY", "QUERY_TO_XML", "QUERY_TO_XMLSCHEMA",
    "QUERY_TO_XML_AND_XMLSCHEMA", "CURSOR_TO_XML", "TS_STAT",
    // MySQL
    "GET_LOCK", "RELEASE_LOCK", "RELEASE_ALL_LOCKS",
    // SQLite (shell/extension functions)
    "WRITEFILE", "LOAD_EXTENSION", "EDIT", "FTS3_TOKENIZER",
];

/// Functions that only read but must never run without the person seeing
/// them first: server files, sleeps, network, large objects (security review #2).
pub const RISKY_READ_FUNCTIONS: &[&str] = &[
    // PostgreSQL
    "PG_READ_FILE", "PG_READ_BINARY_FILE", "PG_LS_DIR", "PG_LS_LOGDIR", "PG_LS_WALDIR", "PG_LS_TMPDIR",
    "PG_LS_ARCHIVE_STATUSDIR", "PG_STAT_FILE", "PG_FILE_READ", "PG_SLEEP", "PG_SLEEP_FOR", "PG_SLEEP_UNTIL",
    "DBLINK", "DBLINK_CONNECT", "DBLINK_CONNECT_U", "DBLINK_SEND_QUERY", "DBLINK_GET_RESULT", "DBLINK_OPEN",
    "DBLINK_FETCH", "LO_GET", "LOREAD", "LO_OPEN", "CURRENT_SETTING", "PG_SHOW_ALL_SETTINGS", "INET_SERVER_ADDR",
    "GENERATE_SERIES",
    // MySQL
    "LOAD_FILE", "SLEEP", "BENCHMARK",
    // SQLite
    "READFILE", "RANDOMBLOB", "ZEROBLOB",
];

fn tokens_upper(s: &str) -> Vec<String> {
    s.to_ascii_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// True when every statement only reads data (used for read-only connections
/// and, with `is_safe_auto_query`, for the AI "run without asking" rule).
pub fn is_read_query(sql: &str) -> bool {
    let stmts = split_statements(sql);
    !stmts.is_empty()
        && stmts.iter().all(|s| {
            let s = strip_comments(s);
            let first = s
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            let ok_first = matches!(
                first.as_str(),
                "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC" | "VALUES" | "TABLE"
            ) || (first == "PRAGMA" && !s.contains('='));
            let upper = s.to_ascii_uppercase();
            let writes = [
                "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "TRUNCATE", "INTO", "GRANT",
                "REVOKE", "REPLACE", "MERGE", "ATTACH", "VACUUM",
            ];
            // Conservative: any write keyword or write function anywhere (even
            // inside a string or as a column name) makes it count as a write.
            let _ = &upper;
            ok_first
                && !tokens_upper(&s)
                    .iter()
                    .any(|t| writes.contains(&t.as_str()) || WRITE_FUNCTIONS.contains(&t.as_str()))
        })
}

/// What the AI may run without asking: a read query with no file, sleep,
/// network or large-object functions. The caller must also require a
/// read-only connection.
pub fn is_safe_auto_query(sql: &str) -> bool {
    is_read_query(sql) && !tokens_upper(&strip_comments(sql)).iter().any(|t| RISKY_READ_FUNCTIONS.contains(&t.as_str()))
}

// ---------------------------------------------------------------- SQLite

struct Sqlite {
    c: rusqlite::Connection,
    path: String,
    manual: bool,
}

fn open_sqlite(cfg: &ConnConfig) -> Result<Sqlite, String> {
    use rusqlite::OpenFlags;
    let mut flags = OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if cfg.read_only {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    }
    if cfg.path.trim().is_empty() {
        return Err("Choose a database file first.".into());
    }
    let c = rusqlite::Connection::open_with_flags(&cfg.path, flags).map_err(|e| e.to_string())?;
    let _ = c.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS));
    // Loading native extensions stays off (MECHANICS: allowRuntimeExtensions = false).
    let _ = c.execute_batch("PRAGMA foreign_keys = ON;");
    Ok(Sqlite { c, path: cfg.path.clone(), manual: false })
}

impl Backend for Sqlite {
    fn canceller(&self) -> Option<Canceller> {
        let h = self.c.get_interrupt_handle();
        Some(std::sync::Arc::new(move || {
            h.interrupt();
            Ok(())
        }))
    }
    fn server(&mut self) -> String {
        format!("SQLite {}", rusqlite::version())
    }
    fn databases(&mut self) -> Vec<String> {
        vec!["main".into()]
    }
    fn current_db(&mut self) -> String {
        std::path::Path::new(&self.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".into())
    }
    fn tables(&mut self) -> Result<Vec<TableInfo>, String> {
        let mut st = self
            .c
            .prepare("SELECT name, type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY type DESC, name")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(TableInfo {
                    schema: None,
                    name: r.get::<_, String>(0)?,
                    is_view: r.get::<_, String>(1)? == "view",
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
    fn columns(&mut self, _schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>, String> {
        let sql = format!("PRAGMA table_info({})", DbKind::Sqlite.quote(table));
        let mut st = self.c.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(ColumnInfo {
                    name: r.get(1)?,
                    data_type: r.get::<_, String>(2).unwrap_or_default(),
                    nullable: r.get::<_, i64>(3).unwrap_or(0) == 0,
                    pk: r.get::<_, i64>(5).unwrap_or(0) > 0,
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
    fn foreign_keys(&mut self, tables: &[TableInfo]) -> Vec<ForeignKey> {
        let mut out = Vec::new();
        for t in tables.iter().filter(|t| !t.is_view) {
            let sql = format!("PRAGMA foreign_key_list({})", DbKind::Sqlite.quote(&t.name));
            if let Ok(mut st) = self.c.prepare(&sql) {
                if let Ok(rows) = st.query_map([], |r| {
                    Ok(ForeignKey {
                        from_table: t.name.clone(),
                        to_table: r.get(2)?,
                        from_col: r.get(3)?,
                        to_col: r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "id".into()),
                    })
                }) {
                    out.extend(rows.filter_map(|r| r.ok()));
                }
            }
        }
        out
    }
    fn set_manual_commit(&mut self, on: bool) -> Result<(), String> {
        if !on && !self.c.is_autocommit() {
            return Err("Commit or roll back the open transaction first.".into());
        }
        self.manual = on;
        Ok(())
    }
    fn in_transaction(&mut self) -> bool {
        !self.c.is_autocommit()
    }
    fn run(&mut self, sql: &str, max_rows: usize) -> Result<Vec<QueryResult>, String> {
        use rusqlite::types::ValueRef;
        if self.manual && self.c.is_autocommit() && !split_statements(sql).iter().all(|s| tx_effect(s).is_some()) {
            self.c.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        }
        let mut out = Vec::new();
        for stmt_sql in split_statements(sql) {
            let t0 = Instant::now();
            let mut st = self.c.prepare(&stmt_sql).map_err(|e| e.to_string())?;
            let columns: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
            let mut res = QueryResult { sql: stmt_sql.clone(), columns: columns.clone(), ..Default::default() };
            if columns.is_empty() {
                let n = st.raw_execute().map_err(|e| e.to_string())?;
                res.affected = Some(n as u64);
            } else {
                let mut rows = st.query([]).map_err(|e| e.to_string())?;
                while let Some(r) = rows.next().map_err(|e| e.to_string())? {
                    if res.rows.len() >= max_rows {
                        res.truncated = true;
                        break;
                    }
                    let mut v = Vec::with_capacity(columns.len());
                    for i in 0..columns.len() {
                        v.push(match r.get_ref(i).map_err(|e| e.to_string())? {
                            ValueRef::Null => Cell::Null,
                            ValueRef::Integer(i) => Cell::Int(i),
                            ValueRef::Real(f) => Cell::Float(f),
                            ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).to_string()),
                            ValueRef::Blob(b) => Cell::Bytes(b.to_vec()),
                        });
                    }
                    res.rows.push(v);
                }
            }
            res.elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            out.push(res);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- PostgreSQL

struct Pg {
    c: postgres::Client,
    /// (TLS on, certificate verified) of the live connection, for cancel requests
    tls: (bool, bool),
    read_only: bool,
    /// The person opened a transaction (BEGIN) that is still going.
    in_tx: bool,
    /// manual transactions: BEGIN before the first statement after a COMMIT/ROLLBACK
    manual: bool,
}

/// User to log in with: the typed one, else the operating-system user
/// (same rule as the original's `configurePool`: `user || osUser`).
pub fn effective_user(cfg: &ConnConfig) -> String {
    if !cfg.user.trim().is_empty() {
        return cfg.user.trim().to_string();
    }
    os_username()
}

/// Environment variables checked first, in the original's order
/// (its `username` module in preload.js).
pub const USERNAME_ENV_ORDER: [&str; 6] = ["SUDO_USER", "C9_USER", "LOGNAME", "USER", "LNAME", "USERNAME"];

/// The operating-system user name, found the way the original finds it:
/// 1. the first non-empty of SUDO_USER, C9_USER, LOGNAME, USER, LNAME, USERNAME;
/// 2. the passwd entry of the current user — the original calls Node's
///    `os.userInfo()`, i.e. `getUserInfo` at 0x2dd9530 → `uv_os_get_passwd`
///    → `getpwuid_r(geteuid())`;
/// 3. `id -un <uid>`;
/// 4. `no-username-<uid>`.
pub fn os_username() -> String {
    for k in USERNAME_ENV_ORDER {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    #[cfg(unix)]
    {
        if let Some(u) = passwd_username() {
            return u;
        }
        let uid = std::process::Command::new("id").arg("-u").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if !uid.is_empty() {
            if let Ok(o) = std::process::Command::new("id").args(["-un", &uid]).output() {
                let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if o.status.success() && !n.is_empty() {
                    return n;
                }
            }
            return format!("no-username-{uid}");
        }
    }
    String::new()
}

#[cfg(unix)]
fn passwd_username() -> Option<String> {
    // getpwuid_r with a growing buffer, like libuv's uv_os_get_passwd
    unsafe {
        let uid = libc::geteuid();
        let mut buf: Vec<libc::c_char> = vec![0; 1024];
        loop {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let r = libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result);
            if r == libc::ERANGE && buf.len() < 1 << 20 {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if r != 0 || result.is_null() || pwd.pw_name.is_null() {
                return None;
            }
            let name = std::ffi::CStr::from_ptr(pwd.pw_name).to_string_lossy().to_string();
            return if name.is_empty() { None } else { Some(name) };
        }
    }
}

/// Full, readable text for a PostgreSQL error. The driver's own text is
/// just "db error"; the real reason (code, message, detail, hint) and any
/// network cause are inside it.
pub fn pg_error_text(e: &postgres::Error) -> String {
    if let Some(d) = e.as_db_error() {
        let mut s = format!("{} {}: {}", d.severity(), d.code().code(), d.message());
        if let Some(x) = d.detail() {
            s.push_str(&format!("\nDetail: {x}"));
        }
        if let Some(x) = d.hint() {
            s.push_str(&format!("\nHint: {x}"));
        }
        return s;
    }
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(x) = src {
        let t = x.to_string();
        if !s.contains(&t) {
            s.push_str(&format!(": {t}"));
        }
        src = x.source();
    }
    s
}

/// A query error with a hint for the cases that need the person to act.
pub fn pg_query_error(e: &postgres::Error) -> String {
    let t = pg_error_text(e);
    match e.as_db_error().map(|d| d.code().code()) {
        Some("25P02") => format!("{t}\n\nAn earlier statement in this transaction failed, so PostgreSQL ignores the rest until it ends. Press Rollback (or run ROLLBACK) to continue."),
        _ => t,
    }
}

/// A plain-language next step for common connection failures.
pub fn pg_advice(e: &postgres::Error, cfg: &ConnConfig, user: &str) -> Option<String> {
    let port = cfg.port;
    if let Some(d) = e.as_db_error() {
        let msg = d.message().to_lowercase();
        return Some(match d.code().code() {
            "28P01" => format!("The server rejected the password for user “{user}”. If the password is saved, open Edit connection and type it again."),
            "28000" if msg.contains("encryption") || msg.contains("ssl") => "The server only accepts encrypted connections: turn on “Use TLS/SSL” in the connection (BaseLens also tries this automatically).".into(),
            "28000" => format!("The server does not allow user “{user}” from this computer to this database (pg_hba.conf). Check the user and database, or ask the server's admin."),
            "3D000" => {
                if cfg.database.trim().is_empty() {
                    format!("No database was chosen, so PostgreSQL tried one named after the user (“{user}”), which doesn't exist. Type a database name, e.g. “postgres”.")
                } else {
                    format!("The database “{}” doesn't exist on this server. Check the name (it is case-sensitive).", cfg.database)
                }
            }
            "53300" => "The server has too many open connections right now. Close some, or try again later.".into(),
            "25P02" => "An earlier statement in this transaction failed, so PostgreSQL ignores the rest until it ends. Press Rollback (or run ROLLBACK) to continue.".into(),
            "57P03" => "The server is starting up or shutting down. Try again in a moment.".into(),
            _ => return None,
        });
    }
    let t = pg_error_text(e).to_lowercase();
    if t.contains("refused") {
        Some(format!("Nothing answered at {}:{port}. Check that the server is running and the port is right (PostgreSQL usually uses 5432; yours says {port}).", cfg.host))
    } else if t.contains("timed out") || t.contains("timeout") {
        Some(format!("No reply from {}:{port} within {} seconds. Check the host, the port and any firewall/VPN.", cfg.host, PG_CONNECT_TIMEOUT_MS / 1000))
    } else if t.contains("failed to lookup") || t.contains("name or service not known") || t.contains("resolve") {
        Some(format!("The host name “{}” could not be found. Check it for typos.", cfg.host))
    } else if t.contains("protocol version") || t.contains("unsupported protocol") {
        Some(format!("The server only offers an old TLS version (before {TLS_MIN_VERSION}), which is no longer safe; BaseLens, like the original app, needs {TLS_MIN_VERSION} or newer. Ask the server's admin to enable it."))
    } else if t.contains("certificate") {
        Some("The server's TLS certificate couldn't be verified (self-signed or for another name). Untick “Verify certificate” only if you trust this network and server: without the check, someone in between could pretend to be the server and read the password.".into())
    } else if t.contains("server does not support tls") || t.contains("tls") && cfg.ssl {
        Some("The server doesn't accept TLS/SSL. Turn off “Use TLS/SSL” in the connection.".into())
    } else {
        None
    }
}

/// `verify`: check the server's certificate and host name. Only an explicit
/// "Use TLS/SSL" with "Verify certificate" unticked turns it off; an automatic
/// upgrade to TLS always verifies (security review #1).
fn pg_connect(cfg: &ConnConfig, password: &str, user: &str, tls: bool, verify: bool) -> Result<postgres::Client, postgres::Error> {
    let mut pc = postgres::Config::new();
    pc.host(&cfg.host);
    match cfg.via {
        // through the SSH tunnel: connect to its local end, while TLS still
        // checks the certificate against the real host name (each call to
        // port()/hostaddr() adds an entry, so exactly one of each)
        Some(local) => {
            pc.hostaddr(std::net::IpAddr::from([127, 0, 0, 1])).port(local);
        }
        None => {
            if let Some(ip) = autoselect_address(&cfg.host, cfg.port) {
                pc.hostaddr(ip);
            }
            pc.port(cfg.port);
        }
    }
    pc
        .user(user)
        .connect_timeout(Duration::from_millis(PG_CONNECT_TIMEOUT_MS))
        .keepalives(true)
        .keepalives_idle(Duration::from_secs(TCP_KEEPALIVE_IDLE_S))
        .keepalives_interval(Duration::from_secs(TCP_KEEPALIVE_INTERVAL_S))
        .keepalives_retries(TCP_KEEPALIVE_COUNT)
        .application_name("baselens");
    if !password.is_empty() {
        pc.password(password);
    }
    if !cfg.database.is_empty() {
        pc.dbname(&cfg.database);
    }
    if tls {
        pc.ssl_mode(postgres::config::SslMode::Require);
        let connector = native_tls::TlsConnector::builder()
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .danger_accept_invalid_certs(!verify)
            .danger_accept_invalid_hostnames(!verify)
            .build();
        match connector {
            Ok(c) => pc.connect(postgres_native_tls::MakeTlsConnector::new(c)),
            // never fall back to plain text: with SslMode::Require and no TLS
            // the driver refuses to connect
            Err(_) => pc.connect(postgres::NoTls),
        }
    } else {
        pc.connect(postgres::NoTls)
    }
}

fn open_pg(cfg: &ConnConfig, password: &str) -> Result<Pg, String> {
    let user = effective_user(cfg);
    let first = pg_connect(cfg, password, &user, cfg.ssl, cfg.ssl && cfg.ssl_verify);
    let mut tls = (cfg.ssl, cfg.ssl && cfg.ssl_verify);
    let mut c = match first {
        Ok(c) => c,
        Err(e) => {
            // the server demands encryption: try once more with TLS
            let needs_tls = !cfg.ssl
                && e.as_db_error().map(|d| d.code().code() == "28000" && {
                    let m = d.message().to_lowercase();
                    m.contains("encryption") || m.contains("ssl")
                }).unwrap_or(false);
            // the upgrade nobody asked for must not weaken anything: the
            // certificate is always verified here
            let retried = if needs_tls { Some(pg_connect(cfg, password, &user, true, true)) } else { None };
            let retry_err = match retried {
                Some(Ok(c)) => Ok(c),
                Some(Err(e2)) => Err(Some(e2)),
                None => Err(None),
            };
            match retry_err {
                Ok(c) => {
                    tls = (true, true);
                    c
                }
                Err(e2) => {
                    if let Some(e2) = e2 {
                        let t = pg_error_text(&e2);
                        if t.to_lowercase().contains("certificate") || t.to_lowercase().contains("verify") {
                            return Err(format!(
                                "The server only accepts encrypted connections, and its TLS certificate couldn't be verified, so BaseLens didn't send the password.\n\nTurn on “Use TLS/SSL” in the connection. Untick “Verify certificate” only if you trust this network and server (for example a self-signed certificate behind Cloud SQL Proxy).\n\n{t}"
                            ));
                        }
                    }
                    let mut msg = pg_error_text(&e);
                    if let Some(a) = pg_advice(&e, cfg, &user) {
                        msg = format!("{a}\n\n{msg}");
                    }
                    let db = if cfg.database.is_empty() { format!("(none → “{user}”)") } else { cfg.database.clone() };
                    msg.push_str(&format!(
                        "\n\nTried: {user}@{}:{} · database {db} · TLS {} · password {}",
                        cfg.host,
                        cfg.port,
                        if cfg.ssl { "on" } else { "off" },
                        &if password.is_empty() { "empty".to_string() } else { format!("given ({} characters)", password.chars().count()) }
                    ));
                    return Err(msg);
                }
            }
        }
    };
    if cfg.read_only {
        let _ = c.batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY");
    }
    Ok(Pg { c, tls, read_only: cfg.read_only, in_tx: false, manual: false })
}


// ------------------------------------------------------------ diagnostics

/// One line of a connection check: ok?, text.
pub type DiagLine = (bool, String);

fn pg_read_message(s: &mut std::net::TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    use std::io::Read;
    let mut head = [0u8; 5];
    s.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut body = vec![0u8; len.saturating_sub(4).min(1 << 16)];
    s.read_exact(&mut body)?;
    Ok((head[0], body))
}

fn pg_error_fields(body: &[u8]) -> String {
    // fields: type byte + C string, ended by 0
    let mut out = Vec::new();
    for f in body.split(|b| *b == 0) {
        if f.len() > 1 && matches!(f[0], b'S' | b'C' | b'M') {
            out.push(String::from_utf8_lossy(&f[1..]).to_string());
        }
    }
    out.join(" ")
}

/// Step-by-step check of a PostgreSQL connection. Nothing secret is sent
/// until the last step (the real login).
pub fn pg_diagnose(cfg: &ConnConfig, password: &str) -> Vec<DiagLine> {
    use std::io::Write;
    use std::net::{TcpStream, ToSocketAddrs};
    let mut out: Vec<DiagLine> = Vec::new();
    let user = effective_user(cfg);
    let db = if cfg.database.trim().is_empty() { user.clone() } else { cfg.database.trim().to_string() };
    // 1. where does the host name point?
    let addrs: Vec<std::net::SocketAddr> = match (cfg.host.as_str(), cfg.port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            out.push((false, format!("Host “{}” could not be resolved: {e}", cfg.host)));
            return out;
        }
    };
    out.push((true, format!("“{}” points to {}", cfg.host, addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "))));
    // 2. who answers on each address?
    let timeout = Duration::from_secs(4);
    for a in &addrs {
        match TcpStream::connect_timeout(a, timeout) {
            Err(e) => out.push((false, format!("{a}: no answer ({e})"))),
            Ok(mut s) => {
                let _ = s.set_read_timeout(Some(timeout));
                // SSLRequest: length 8, code 80877103
                let mut req = Vec::new();
                req.extend_from_slice(&8u32.to_be_bytes());
                req.extend_from_slice(&80877103u32.to_be_bytes());
                let ssl = if s.write_all(&req).is_ok() {
                    let mut b = [0u8; 1];
                    match std::io::Read::read_exact(&mut s, &mut b) {
                        Ok(_) if b[0] == b'S' => "supports TLS".to_string(),
                        Ok(_) if b[0] == b'N' => "no TLS".to_string(),
                        Ok(_) => format!("answered {:?} (not PostgreSQL?)", b[0] as char),
                        Err(e) => format!("closed the connection ({e})"),
                    }
                } else {
                    "write failed".into()
                };
                out.push((true, format!("{a}: a PostgreSQL server answers ({ssl})")));
            }
        }
    }
    // 3. which login does it ask for? (start-up message, no password sent)
    if let Some(a) = addrs.iter().find(|a| TcpStream::connect_timeout(a, timeout).is_ok()) {
        if let Ok(mut s) = TcpStream::connect_timeout(a, timeout) {
            let _ = s.set_read_timeout(Some(timeout));
            let mut params = Vec::new();
            for (k, v) in [("user", user.as_str()), ("database", db.as_str()), ("application_name", "baselens-diagnose")] {
                params.extend_from_slice(k.as_bytes());
                params.push(0);
                params.extend_from_slice(v.as_bytes());
                params.push(0);
            }
            params.push(0);
            let mut msg = Vec::new();
            msg.extend_from_slice(&((params.len() + 8) as u32).to_be_bytes());
            msg.extend_from_slice(&196608u32.to_be_bytes()); // protocol 3.0
            msg.extend_from_slice(&params);
            let line = match s.write_all(&msg).and_then(|_| pg_read_message(&mut s)) {
                Ok((b'R', body)) if body.len() >= 4 => {
                    let code = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                    match code {
                        0 => (true, format!("{a} lets “{user}” in without a password (trust)")),
                        3 => (true, format!("{a} asks “{user}” for a plain-text password")),
                        5 => (true, format!("{a} asks “{user}” for an MD5 password")),
                        10 => {
                            let mechs: Vec<String> = body[4..].split(|b| *b == 0).filter(|m| !m.is_empty()).map(|m| String::from_utf8_lossy(m).to_string()).collect();
                            (true, format!("{a} asks “{user}” for a SCRAM password ({})", mechs.join(", ")))
                        }
                        c => (true, format!("{a} asks for login method #{c}")),
                    }
                }
                Ok((b'E', body)) => (false, format!("{a} refused before the password: {}", pg_error_fields(&body))),
                Ok((t, _)) => (false, format!("{a} sent an unexpected reply ({})", t as char)),
                Err(e) => (false, format!("{a}: start-up failed ({e})")),
            };
            out.push(line);
        }
    }
    // 4. the password itself (never shown)
    let n = password.chars().count();
    let mut notes = Vec::new();
    if password.is_empty() {
        notes.push("is EMPTY".to_string());
    } else {
        notes.push(format!("has {n} characters"));
    }
    if password != password.trim() {
        notes.push("starts or ends with a space".into());
    }
    if let Some(t) = password_host_tail(password) {
        notes.push(format!("ends with “{t}”, which looks like part of an address pasted with it"));
    }
    let dead: Vec<char> = password.chars().filter(|c| "'\"`´~^¨".contains(*c)).collect();
    if !dead.is_empty() {
        notes.push(format!("contains {} accent/quote character(s) — on ABNT2 keyboards these are typed with dead keys, use the 👁 button to check them", dead.len()));
    }
    if password.chars().any(|c| !c.is_ascii()) {
        notes.push("contains non-ASCII letters".into());
    }
    out.push((!password.is_empty() && password == password.trim() && password_host_tail(password).is_none(), format!("The password {}", notes.join(", "))));
    // 5. the real login
    match open_pg(cfg, password) {
        Ok(mut c) => out.push((true, format!("Login OK: {}", c.server()))),
        Err(e) => out.push((false, format!("Login failed: {}", e.lines().find(|l| l.contains("FATAL") || l.contains("ERROR")).unwrap_or(e.lines().next().unwrap_or(""))))),
    }
    out
}

/// What a statement does to an open transaction: Some(true) opens one,
/// Some(false) ends it.
fn tx_effect(stmt: &str) -> Option<bool> {
    let s = strip_comments(stmt).to_ascii_uppercase();
    let mut w = s.split_whitespace();
    match (w.next(), w.next()) {
        (Some("BEGIN") | Some("START"), _) => Some(true),
        (Some("COMMIT") | Some("END") | Some("ABORT"), _) => Some(false),
        (Some("ROLLBACK"), Some("TO")) => None,
        (Some("ROLLBACK"), _) => Some(false),
        (Some("PREPARE"), Some("TRANSACTION")) => Some(false),
        _ => None,
    }
}

/// A single read statement that a cursor can page through.
pub fn cursor_friendly(stmt: &str) -> bool {
    let s = strip_comments(stmt);
    let first = s.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    matches!(first.as_str(), "SELECT" | "WITH" | "VALUES" | "TABLE") && is_read_query(stmt) && split_statements(stmt).len() == 1
}

/// A PostgreSQL value (text form) as a typed cell, from its column type:
/// integers → Int, float4/float8 → Float, numeric → Float only when it
/// round-trips exactly (otherwise text, so no digits are lost), bool →
/// true/false, bytea (`\x…`) → bytes; everything else stays text.
pub fn pg_cell(v: Option<&str>, ty: Option<&postgres::types::Type>) -> Cell {
    use postgres::types::Type;
    let Some(v) = v else { return Cell::Null };
    let Some(ty) = ty else { return Cell::Text(v.to_string()) };
    match *ty {
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => v.parse().map(Cell::Int).unwrap_or_else(|_| Cell::Text(v.to_string())),
        Type::FLOAT4 | Type::FLOAT8 => match v {
            "NaN" => Cell::Float(f64::NAN),
            "Infinity" => Cell::Float(f64::INFINITY),
            "-Infinity" => Cell::Float(f64::NEG_INFINITY),
            _ => v.parse().map(Cell::Float).unwrap_or_else(|_| Cell::Text(v.to_string())),
        },
        Type::NUMERIC => match v.parse::<f64>() {
            // keep it a number only if nothing is lost
            Ok(f) if f.is_finite() && numeric_same(v, f) => Cell::Float(f),
            _ => Cell::Text(v.to_string()),
        },
        Type::BOOL => Cell::Text(if v == "t" { "true".into() } else if v == "f" { "false".into() } else { v.to_string() }),
        Type::BYTEA => match v.strip_prefix("\\x") {
            Some(hex) if hex.len() % 2 == 0 => (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map(Cell::Bytes)
                .unwrap_or_else(|_| Cell::Text(v.to_string())),
            _ => Cell::Text(v.to_string()),
        },
        _ => Cell::Text(v.to_string()),
    }
}

/// True when the numeric text and the float mean the same value.
fn numeric_same(text: &str, f: f64) -> bool {
    let norm = |t: &str| -> String {
        let t = t.trim().trim_start_matches('+');
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t),
        };
        let (int, frac) = t.split_once('.').unwrap_or((t, ""));
        let int = int.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        let body = match (int.is_empty(), frac.is_empty()) {
            (true, true) => "0".to_string(),
            (true, false) => format!("0.{frac}"),
            (false, true) => int.to_string(),
            (false, false) => format!("{int}.{frac}"),
        };
        if neg && body != "0" { format!("-{body}") } else { body }
    };
    !text.contains(['e', 'E']) && norm(text) == norm(&format!("{f}"))
}

impl Pg {
    /// Reads a SELECT through a cursor, PG_FETCH_BATCH rows at first, and
    /// stops at max_rows instead of downloading the whole result.
    fn run_batched(&mut self, stmt: &str, max_rows: usize) -> Result<Vec<QueryResult>, String> {
        use postgres::SimpleQueryMessage as M;
        let t0 = Instant::now();
        let own_tx = !self.in_tx;
        const CUR: &str = "baselens_cursor";
        // column types (one extra round trip, before any transaction so a
        // failure can't abort one): numbers, booleans and bytea then arrive
        // as real values instead of text. Inside the person's own
        // transaction, or for statements that can't be described (with $1
        // parameters), values stay text.
        let types: Vec<postgres::types::Type> = if own_tx || !stmt.contains('$') {
            self.c.prepare(stmt).map(|st| st.columns().iter().map(|c| c.type_().clone()).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let declare = format!("{}DECLARE {CUR} NO SCROLL CURSOR FOR {stmt}", if own_tx { "BEGIN; " } else { "" });
        let declared = self.c.batch_execute(&declare);
        if let Err(e) = declared {
            if own_tx {
                let _ = self.c.batch_execute("ROLLBACK");
            }
            return Err(pg_error_text(&e));
        }
        let mut res = QueryResult { sql: stmt.to_string(), ..Default::default() };
        let mut batch = PG_FETCH_BATCH;
        let outcome: Result<(), String> = loop {
            let left = max_rows.saturating_sub(res.rows.len());
            // at the limit: ask for one more row only to know if there is more
            let want = if left == 0 { 1 } else { batch.min(left) };
            let msgs = match self.c.simple_query(&format!("FETCH {want} FROM {CUR}")) {
                Ok(m) => m,
                Err(e) => break Err(pg_query_error(&e)),
            };
            res.batches += 1;
            let mut got = 0;
            for m in msgs {
                match m {
                    M::RowDescription(cols) if res.columns.is_empty() => {
                        res.columns = cols.iter().map(|c| c.name().to_string()).collect();
                    }
                    M::Row(row) => {
                        got += 1;
                        if res.columns.is_empty() {
                            res.columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                        }
                        if left == 0 {
                            res.truncated = true;
                            continue;
                        }
                        res.rows.push((0..row.len()).map(|i| pg_cell(row.get(i), types.get(i))).collect());
                    }
                    _ => {}
                }
            }
            if left == 0 || got < want {
                break Ok(());
            }
            batch = (batch * 2).min(PG_FETCH_MAX);
        };
        let end = if own_tx { format!("CLOSE {CUR}; COMMIT") } else { format!("CLOSE {CUR}") };
        if self.c.batch_execute(&end).is_err() && own_tx {
            let _ = self.c.batch_execute("ROLLBACK");
        }
        outcome?;
        res.elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok(vec![res])
    }

    fn text_rows(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, String> {
        let msgs = self.c.simple_query(sql).map_err(|e| pg_error_text(&e))?;
        let mut out = Vec::new();
        for m in msgs {
            if let postgres::SimpleQueryMessage::Row(r) = m {
                out.push((0..r.len()).map(|i| r.get(i).map(|s| s.to_string())).collect());
            }
        }
        Ok(out)
    }
}

impl Backend for Pg {
    fn canceller(&self) -> Option<Canceller> {
        let token = self.c.cancel_token();
        let (tls, verify) = self.tls;
        Some(std::sync::Arc::new(move || {
            let r = if tls {
                let connector = native_tls::TlsConnector::builder()
                    .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
                    .danger_accept_invalid_certs(!verify)
                    .danger_accept_invalid_hostnames(!verify)
                    .build()
                    .map_err(|e| e.to_string())?;
                token.cancel_query(postgres_native_tls::MakeTlsConnector::new(connector))
            } else {
                token.cancel_query(postgres::NoTls)
            };
            r.map_err(|e| pg_error_text(&e))
        }))
    }
    fn server(&mut self) -> String {
        self.text_rows("SHOW server_version")
            .ok()
            .and_then(|r| r.first().and_then(|x| x[0].clone()))
            .map(|v| format!("PostgreSQL {v}"))
            .unwrap_or_else(|| "PostgreSQL".into())
            + if self.read_only { " (read-only)" } else { "" }
    }
    fn databases(&mut self) -> Vec<String> {
        self.text_rows("SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY datname")
            .map(|r| r.into_iter().filter_map(|x| x[0].clone()).collect())
            .unwrap_or_default()
    }
    fn current_db(&mut self) -> String {
        self.text_rows("SELECT current_database()")
            .ok()
            .and_then(|r| r.first().and_then(|x| x[0].clone()))
            .unwrap_or_default()
    }
    fn tables(&mut self) -> Result<Vec<TableInfo>, String> {
        let rows = self.text_rows(
            "SELECT table_schema, table_name, table_type FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog','information_schema','pg_toast') \
             ORDER BY table_schema = 'public' DESC, table_schema, table_name",
        )?;
        Ok(rows
            .into_iter()
            .map(|r| TableInfo {
                schema: r[0].clone(),
                name: r[1].clone().unwrap_or_default(),
                is_view: r[2].as_deref() == Some("VIEW"),
            })
            .collect())
    }
    fn columns(&mut self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>, String> {
        let schema = schema.unwrap_or("public");
        let sql = format!(
            "SELECT c.column_name, c.data_type, c.is_nullable, \
             EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
               JOIN information_schema.key_column_usage k ON k.constraint_name = tc.constraint_name AND k.table_schema = tc.table_schema \
               WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema AND tc.table_name = c.table_name AND k.column_name = c.column_name) \
             FROM information_schema.columns c WHERE c.table_schema = {} AND c.table_name = {} ORDER BY c.ordinal_position",
            sql_string(schema),
            sql_string(table)
        );
        Ok(self
            .text_rows(&sql)?
            .into_iter()
            .map(|r| ColumnInfo {
                name: r[0].clone().unwrap_or_default(),
                data_type: r[1].clone().unwrap_or_default(),
                nullable: r[2].as_deref() == Some("YES"),
                pk: r[3].as_deref() == Some("t"),
            })
            .collect())
    }
    fn foreign_keys(&mut self, _tables: &[TableInfo]) -> Vec<ForeignKey> {
        self.text_rows(
            "SELECT kcu.table_name, kcu.column_name, ccu.table_name, ccu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY'",
        )
        .map(|rows| {
            rows.into_iter()
                .map(|r| ForeignKey {
                    from_table: r[0].clone().unwrap_or_default(),
                    from_col: r[1].clone().unwrap_or_default(),
                    to_table: r[2].clone().unwrap_or_default(),
                    to_col: r[3].clone().unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
    }
    fn all_columns(&mut self) -> Result<Vec<(String, Vec<ColumnInfo>)>, String> {
        // one query on pg_catalog instead of one information_schema query per table
        let rows = self.text_rows(
            "SELECT n.nspname, c.relname, a.attname, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
               COALESCE(a.attnum = ANY(i.indkey), false) \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_catalog.pg_index i ON i.indrelid = c.oid AND i.indisprimary \
             WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r','v','m','p','f') \
               AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg\\_toast%' \
               AND n.nspname NOT LIKE 'pg\\_temp%' \
             ORDER BY n.nspname, c.relname, a.attnum",
        )?;
        let mut out: Vec<(String, Vec<ColumnInfo>)> = Vec::new();
        for r in rows {
            let key = format!("{}.{}", r[0].clone().unwrap_or_default(), r[1].clone().unwrap_or_default());
            let col = ColumnInfo {
                name: r[2].clone().unwrap_or_default(),
                data_type: r[3].clone().unwrap_or_default(),
                nullable: r[4].as_deref() == Some("t"),
                pk: r[5].as_deref() == Some("t"),
            };
            match out.last_mut() {
                Some((k, v)) if *k == key => v.push(col),
                _ => out.push((key, vec![col])),
            }
        }
        Ok(out)
    }
    fn set_manual_commit(&mut self, on: bool) -> Result<(), String> {
        if !on && self.in_tx {
            return Err("Commit or roll back the open transaction first.".into());
        }
        self.manual = on;
        Ok(())
    }
    fn in_transaction(&mut self) -> bool {
        self.in_tx
    }
    fn run(&mut self, sql: &str, max_rows: usize) -> Result<Vec<QueryResult>, String> {
        use postgres::SimpleQueryMessage as M;
        let t0 = Instant::now();
        let stmts = split_statements(sql);
        // manual transactions: open one before anything but COMMIT/ROLLBACK/BEGIN
        if self.manual && !self.in_tx && !stmts.iter().all(|s| tx_effect(s).is_some()) {
            self.c.batch_execute("BEGIN").map_err(|e| pg_error_text(&e))?;
            self.in_tx = true;
        }
        if stmts.len() == 1 && cursor_friendly(&stmts[0]) {
            return self.run_batched(&stmts[0], max_rows);
        }
        for st in &stmts {
            if let Some(open) = tx_effect(st) {
                self.in_tx = open;
            }
        }
        let msgs = self.c.simple_query(sql).map_err(|e| pg_query_error(&e))?;
        let mut out = Vec::new();
        let mut cur: Option<QueryResult> = None;
        for m in msgs {
            match m {
                M::RowDescription(cols) => {
                    let mut r = QueryResult::default();
                    r.columns = cols.iter().map(|c| c.name().to_string()).collect();
                    cur = Some(r);
                }
                M::Row(row) => {
                    let r = cur.get_or_insert_with(|| QueryResult {
                        columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                        ..Default::default()
                    });
                    if r.rows.len() >= max_rows {
                        r.truncated = true;
                        continue;
                    }
                    r.rows.push(
                        (0..row.len())
                            .map(|i| match row.get(i) {
                                None => Cell::Null,
                                Some(s) => Cell::Text(s.to_string()),
                            })
                            .collect(),
                    );
                }
                M::CommandComplete(n) => {
                    let mut r = cur.take().unwrap_or_default();
                    if r.columns.is_empty() {
                        r.affected = Some(n);
                    }
                    r.sql = stmts.get(out.len()).cloned().unwrap_or_default();
                    out.push(r);
                }
                _ => {}
            }
        }
        if let Some(mut r) = cur.take() {
            r.sql = stmts.get(out.len()).cloned().unwrap_or_default();
            out.push(r);
        }
        let el = t0.elapsed().as_secs_f64() * 1000.0;
        for r in &mut out {
            r.elapsed_ms = el;
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- MySQL

struct My {
    c: mysql::Conn,
    read_only: bool,
    /// to open the second connection that stops a running query
    opts: mysql::Opts,
    manual: bool,
    /// a statement ran since the last COMMIT/ROLLBACK while manual
    in_tx: bool,
}

fn open_mysql(cfg: &ConnConfig, password: &str) -> Result<My, String> {
    use mysql::prelude::Queryable;
    let (host, port) = match cfg.via {
        Some(local) => ("127.0.0.1".to_string(), local),
        None => {
            // the MySQL driver checks the certificate against this name, so
            // only swap in the address that answered when that check is off
            let pick = if cfg.ssl && cfg.ssl_verify { None } else { autoselect_address(&cfg.host, cfg.port) };
            (pick.map(|ip| ip.to_string()).unwrap_or_else(|| cfg.host.clone()), cfg.port)
        }
    };
    let mut b = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(host))
        .tcp_port(port)
        .user(Some(cfg.user.clone()))
        .tcp_connect_timeout(Some(Duration::from_millis(MYSQL_CONNECT_TIMEOUT_MS)))
        .tcp_keepalive_time_ms(Some((TCP_KEEPALIVE_IDLE_S * 1000) as u32));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        b = b
        .tcp_keepalive_probe_interval_secs(Some(TCP_KEEPALIVE_INTERVAL_S as u32))
        .tcp_keepalive_probe_count(Some(TCP_KEEPALIVE_COUNT));
    }
    if !password.is_empty() {
        b = b.pass(Some(password.to_string()));
    }
    if !cfg.database.is_empty() {
        b = b.db_name(Some(cfg.database.clone()));
    }
    if cfg.ssl {
        b = b.ssl_opts(Some(
            mysql::SslOpts::default()
                .with_danger_accept_invalid_certs(!cfg.ssl_verify)
                .with_danger_skip_domain_validation(!cfg.ssl_verify),
        ));
    }
    let opts: mysql::Opts = b.into();
    let mut c = mysql::Conn::new(opts.clone()).map_err(|e| e.to_string())?;
    if cfg.ssl {
        // the MySQL driver can't be told a minimum TLS version, so check the
        // one the server agreed to and refuse anything older than TLS 1.2
        let v: Option<(String, String)> = c.query_first("SHOW SESSION STATUS LIKE 'Ssl_version'").unwrap_or(None);
        if let Some((_, v)) = v {
            if v.trim().is_empty() {
                return Err("“Use TLS/SSL” is on, but the server didn't encrypt the connection. Nothing was sent; check the server's TLS settings.".into());
            }
            if tls_version_too_old(&v) {
                return Err(format!("The server only offers {v}, which is no longer safe; BaseLens needs {TLS_MIN_VERSION} or newer, like the original app. Ask the server's admin to enable {TLS_MIN_VERSION}."));
            }
        }
    }
    if cfg.read_only {
        let _ = c.query_drop("SET SESSION TRANSACTION READ ONLY");
    }
    Ok(My { c, read_only: cfg.read_only, opts, manual: false, in_tx: false })
}

fn my_cell(v: mysql::Value) -> Cell {
    use mysql::Value as V;
    match v {
        V::NULL => Cell::Null,
        V::Int(i) => Cell::Int(i),
        V::UInt(u) => Cell::Int(u as i64),
        V::Float(f) => Cell::Float(f as f64),
        V::Double(f) => Cell::Float(f),
        V::Bytes(b) => match String::from_utf8(b) {
            Ok(s) => Cell::Text(s),
            Err(e) => Cell::Bytes(e.into_bytes()),
        },
        V::Date(y, m, d, h, mi, s, us) => {
            if h == 0 && mi == 0 && s == 0 && us == 0 {
                Cell::Text(format!("{y:04}-{m:02}-{d:02}"))
            } else {
                Cell::Text(format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
            }
        }
        V::Time(neg, d, h, m, s, _) => Cell::Text(format!(
            "{}{:02}:{m:02}:{s:02}",
            if neg { "-" } else { "" },
            d * 24 + h as u32
        )),
    }
}

impl My {
    fn strings(&mut self, sql: &str) -> Vec<Vec<String>> {
        self.run(sql, 100_000)
            .ok()
            .and_then(|mut v| if v.is_empty() { None } else { Some(v.remove(0)) })
            .map(|r| r.rows.into_iter().map(|row| row.iter().map(|c| c.display()).collect()).collect())
            .unwrap_or_default()
    }
}

impl Backend for My {
    fn canceller(&self) -> Option<Canceller> {
        use mysql::prelude::Queryable;
        let id = self.c.connection_id();
        let opts = self.opts.clone();
        Some(std::sync::Arc::new(move || {
            let mut k = mysql::Conn::new(opts.clone()).map_err(|e| e.to_string())?;
            k.query_drop(format!("KILL QUERY {id}")).map_err(|e| e.to_string())
        }))
    }
    fn server(&mut self) -> String {
        let v = self.strings("SELECT VERSION()");
        let ver = v.first().and_then(|r| r.first()).cloned().unwrap_or_default();
        let name = if ver.to_lowercase().contains("mariadb") { "MariaDB" } else { "MySQL" };
        format!("{name} {ver}{}", if self.read_only { " (read-only)" } else { "" })
    }
    fn databases(&mut self) -> Vec<String> {
        self.strings("SHOW DATABASES").into_iter().filter_map(|r| r.into_iter().next()).collect()
    }
    fn current_db(&mut self) -> String {
        self.strings("SELECT DATABASE()")
            .first()
            .and_then(|r| r.first())
            .cloned()
            .filter(|s| s != "(NULL)")
            .unwrap_or_default()
    }
    fn tables(&mut self) -> Result<Vec<TableInfo>, String> {
        Ok(self
            .strings("SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name")
            .into_iter()
            .map(|r| TableInfo { schema: None, name: r[0].clone(), is_view: r[1].contains("VIEW") })
            .collect())
    }
    fn columns(&mut self, _schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>, String> {
        let sql = format!(
            "SELECT column_name, column_type, is_nullable, column_key FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = {} ORDER BY ordinal_position",
            sql_string(table)
        );
        Ok(self
            .strings(&sql)
            .into_iter()
            .map(|r| ColumnInfo {
                name: r[0].clone(),
                data_type: r[1].clone(),
                nullable: r[2] == "YES",
                pk: r[3] == "PRI",
            })
            .collect())
    }
    fn all_columns(&mut self) -> Result<Vec<(String, Vec<ColumnInfo>)>, String> {
        let rows = self.strings(
            "SELECT table_name, column_name, column_type, is_nullable, column_key FROM information_schema.columns \
             WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position",
        );
        let mut out: Vec<(String, Vec<ColumnInfo>)> = Vec::new();
        for r in rows {
            let col = ColumnInfo { name: r[1].clone(), data_type: r[2].clone(), nullable: r[3] == "YES", pk: r[4] == "PRI" };
            match out.last_mut() {
                Some((k, v)) if *k == r[0] => v.push(col),
                _ => out.push((r[0].clone(), vec![col])),
            }
        }
        Ok(out)
    }
    fn foreign_keys(&mut self, _tables: &[TableInfo]) -> Vec<ForeignKey> {
        self.strings(
            "SELECT table_name, column_name, referenced_table_name, referenced_column_name FROM information_schema.key_column_usage \
             WHERE table_schema = DATABASE() AND referenced_table_name IS NOT NULL",
        )
        .into_iter()
        .map(|r| ForeignKey { from_table: r[0].clone(), from_col: r[1].clone(), to_table: r[2].clone(), to_col: r[3].clone() })
        .collect()
    }
    fn set_manual_commit(&mut self, on: bool) -> Result<(), String> {
        use mysql::prelude::Queryable;
        if !on && self.in_tx {
            return Err("Commit or roll back the open transaction first.".into());
        }
        self.c.query_drop(if on { "SET autocommit = 0" } else { "SET autocommit = 1" }).map_err(|e| e.to_string())?;
        self.manual = on;
        Ok(())
    }
    fn in_transaction(&mut self) -> bool {
        self.manual && self.in_tx
    }
    fn run(&mut self, sql: &str, max_rows: usize) -> Result<Vec<QueryResult>, String> {
        use mysql::prelude::Queryable;
        if self.manual {
            for s in split_statements(sql) {
                self.in_tx = tx_effect(&s).unwrap_or(true);
            }
        }
        let mut out = Vec::new();
        for stmt in split_statements(sql) {
            let t0 = Instant::now();
            let mut qr = self.c.query_iter(&stmt).map_err(|e| e.to_string())?;
            while let Some(mut set) = qr.iter() {
                let columns: Vec<String> = set.columns().as_ref().iter().map(|c| c.name_str().to_string()).collect();
                let mut res = QueryResult { sql: stmt.clone(), columns, ..Default::default() };
                for row in set.by_ref() {
                    let row = row.map_err(|e| e.to_string())?;
                    if res.rows.len() >= max_rows {
                        res.truncated = true;
                        continue;
                    }
                    res.rows.push(row.unwrap().into_iter().map(my_cell).collect());
                }
                if res.columns.is_empty() {
                    res.affected = Some(set.affected_rows());
                }
                res.elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
                out.push(res);
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------ connection URLs

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Fills a connection from a URL such as
/// `postgresql://user:p%40ss@host:5433/db?sslmode=require`.
/// Like the original (`parseUrl` in main.js): any `sslmode` other than
/// `disable` turns SSL on. Returns the password separately.
pub fn parse_connection_url(url: &str) -> Result<(ConnConfig, String), String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").ok_or("A URL starts with e.g. postgresql:// or mysql://")?;
    let kind = match scheme.to_lowercase().as_str() {
        "postgres" | "postgresql" | "psql" => DbKind::Postgres,
        "mysql" | "mariadb" => DbKind::Mysql,
        "sqlite" | "file" => DbKind::Sqlite,
        other => return Err(format!("“{other}://” isn't supported here (use postgresql://, mysql:// or sqlite://)")),
    };
    let mut cfg = ConnConfig { kind, port: kind.default_port(), ..Default::default() };
    if kind == DbKind::Sqlite {
        cfg.path = percent_decode(rest.split('?').next().unwrap_or(""));
        return Ok((cfg, String::new()));
    }
    let (main, query) = match rest.split_once('?') {
        Some((m, q)) => (m, q),
        None => (rest, ""),
    };
    let (authority, path) = match main.split_once('/') {
        Some((a, p)) => (a, p),
        None => (main, ""),
    };
    // the LAST @ separates the credentials, so passwords may contain @
    let (cred, hostport) = match authority.rfind('@') {
        Some(i) => (&authority[..i], &authority[i + 1..]),
        None => ("", authority),
    };
    let mut password = String::new();
    if !cred.is_empty() {
        match cred.split_once(':') {
            Some((u, p)) => {
                cfg.user = percent_decode(u);
                password = percent_decode(p);
            }
            None => cfg.user = percent_decode(cred),
        }
    }
    // [ipv6]:port or host:port
    if let Some(stripped) = hostport.strip_prefix('[') {
        let (h, after) = stripped.split_once(']').ok_or("Unclosed [ in the host")?;
        cfg.host = h.to_string();
        if let Some(p) = after.strip_prefix(':') {
            cfg.port = p.parse().map_err(|_| format!("“{p}” isn't a port number"))?;
        }
    } else if let Some((h, p)) = hostport.rsplit_once(':') {
        cfg.host = h.to_string();
        cfg.port = p.parse().map_err(|_| format!("“{p}” isn't a port number"))?;
    } else if !hostport.is_empty() {
        cfg.host = hostport.to_string();
    }
    if cfg.host.is_empty() {
        cfg.host = "localhost".into();
    }
    cfg.database = percent_decode(path.trim_end_matches('/'));
    for pair in query.split('&').filter(|x| !x.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k.to_lowercase().as_str() {
            "sslmode" | "ssl-mode" => {
                cfg.ssl = !v.eq_ignore_ascii_case("disable");
                cfg.ssl_verify = v.eq_ignore_ascii_case("verify-full") || v.eq_ignore_ascii_case("verify-ca");
            }
            "ssl" => cfg.ssl = matches!(v, "true" | "1"),
            "user" if cfg.user.is_empty() => cfg.user = percent_decode(v),
            "password" if password.is_empty() => password = percent_decode(v),
            "dbname" if cfg.database.is_empty() => cfg.database = percent_decode(v),
            _ => {}
        }
    }
    Ok((cfg, password))
}

/// The end of a password that looks like part of a host was pasted with it
/// (e.g. "secret@172" copied from "user:secret@172.16.0.4").
pub fn password_host_tail(pw: &str) -> Option<String> {
    let i = pw.rfind('@')?;
    let tail = &pw[i..];
    let rest = &tail[1..];
    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') && rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        Some(tail.to_string())
    } else {
        None
    }
}
