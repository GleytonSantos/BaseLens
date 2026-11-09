//! Debug mode: `baselens --debug` or `BASELENS_DEBUG=1`.
//! Times every query (queue, server, rows, bytes, building the table,
//! drawing it), shows them in a panel (F12) and appends them to
//! `~/.local/share/baselens/debug.log`. `baselens --bench "<sql>"` runs a
//! query 10 times and prints min / average / max of each step.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Runs of `--bench`.
pub const BENCH_RUNS: usize = 10;
/// Queries kept in the F12 panel.
pub const TRACES_KEPT: usize = 50;

pub fn init(args: &[String]) {
    let env = std::env::var("BASELENS_DEBUG").map(|v| !v.is_empty() && v != "0").unwrap_or(false);
    if env || args.iter().any(|a| a == "--debug") {
        ON.store(true, Ordering::SeqCst);
    }
}

pub fn on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// True for the unoptimised build (target/debug), which is much slower,
/// especially for TLS.
pub const DEV_BUILD: bool = cfg!(debug_assertions);

pub fn log_path() -> PathBuf {
    // tests and the robot keep their files in their own folder
    if let Ok(h) = std::env::var("BASELENS_HOME") {
        return PathBuf::from(h).join("debug.log");
    }
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("baselens").join("debug.log")
}

/// Appends one line to debug.log (only in debug mode).
pub fn log(line: &str) {
    if !on() {
        return;
    }
    let p = log_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
        // the log holds SQL text: private folder (0700) and file (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
        }
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(&p);
    #[cfg(unix)]
    {
        // a log made by an older version keeps its old mode until fixed here
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    if let Ok(mut f) = file {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let _ = writeln!(f, "{} :{:02}.{:03} {line}", crate::app::now_string(), (ms / 1000) % 60, ms % 1000);
    }
}

/// One query's timeline.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub when: String,
    pub conn: String,
    pub sql: String,
    pub queue_ms: f64,
    pub queue_behind: usize,
    pub run_ms: f64,
    pub rows: usize,
    pub bytes: usize,
    pub batches: usize,
    pub build_ms: f64,
    /// Filled on the next frame that draws the result.
    pub draw_ms: Option<f64>,
    pub error: bool,
}

impl Trace {
    pub fn line(&self) -> String {
        format!(
            "[{}] queue {:.1} ms ({} behind) · server {:.1} ms · {} rows · {} · {} batch(es) · build {:.1} ms · draw {} · {}{}",
            self.conn,
            self.queue_ms,
            self.queue_behind,
            self.run_ms,
            self.rows,
            human_bytes(self.bytes),
            self.batches,
            self.build_ms,
            self.draw_ms.map(|d| format!("{d:.1} ms")).unwrap_or_else(|| "…".into()),
            if self.error { "ERROR · " } else { "" },
            one_line(&self.sql, 80)
        )
    }
}

pub fn human_bytes(n: usize) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

pub fn one_line(s: &str, max: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max {
        format!("{}…", t.chars().take(max).collect::<String>())
    } else {
        t
    }
}

/// What the F12 panel shows.
#[derive(Default)]
pub struct State {
    pub show: bool,
    pub traces: VecDeque<Trace>,
    pub connects: VecDeque<String>,
    /// Recent frame times in ms, for frames per second.
    pub frames: VecDeque<f32>,
    /// Time spent drawing the main area in the last frame.
    pub last_draw_ms: f64,
}

impl State {
    pub fn push(&mut self, t: Trace) {
        log(&t.line());
        self.traces.push_front(t);
        self.traces.truncate(TRACES_KEPT);
    }
    pub fn connect(&mut self, line: String) {
        log(&line);
        self.connects.push_front(line);
        self.connects.truncate(10);
    }
    pub fn frame(&mut self, dt_ms: f32) {
        self.frames.push_back(dt_ms);
        while self.frames.len() > 120 {
            self.frames.pop_front();
        }
    }
    pub fn fps(&self) -> f32 {
        let n = self.frames.len();
        if n == 0 {
            return 0.0;
        }
        let avg = self.frames.iter().sum::<f32>() / n as f32;
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }
    /// Gives the newest result its draw time once a frame has drawn it.
    pub fn drew(&mut self, ms: f64) {
        self.last_draw_ms = ms;
        for t in self.traces.iter_mut().filter(|t| t.draw_ms.is_none()) {
            t.draw_ms = Some(ms);
            log(&format!("draw {ms:.1} ms for: {}", one_line(&t.sql, 60)));
        }
    }
}

/// min, average, max
pub fn stats(v: &[f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let min = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (min, v.iter().sum::<f64>() / v.len() as f64, max)
}

/// `--bench "<sql>" [--conn <name>]`: opens the saved connection (by name,
/// else the most recent), runs the query BENCH_RUNS times, prints each step.
pub fn bench(sql: &str, conn: Option<&str>) -> Result<String, String> {
    use crate::db::{spawn, ConnConfig, Request, Response};
    use std::time::{Duration, Instant};
    ON.store(true, Ordering::SeqCst);
    let conns: Vec<ConnConfig> = crate::store::load("connections.json");
    let recent: Vec<u64> = crate::store::load::<crate::app::Settings>("settings.json").recent;
    let cfg = match conn {
        Some(n) => conns.iter().find(|c| c.name.eq_ignore_ascii_case(n)).cloned().ok_or_else(|| {
            format!("No saved connection named “{n}”. Saved: {}", conns.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", "))
        })?,
        None => recent
            .iter()
            .find_map(|id| conns.iter().find(|c| c.id == *id))
            .or(conns.first())
            .cloned()
            .ok_or("No saved connections yet.")?,
    };
    let password = crate::store::decrypt_checked(&cfg.password_enc).ok_or("The saved password couldn't be unlocked; open BaseLens and type it again.")?;
    let mut out = String::new();
    out.push_str(&format!(
        "BaseLens bench · build: {} · connection “{}” ({}) · {} runs\n{}\n\n",
        if DEV_BUILD { "development (unoptimised, slow TLS — use target/release/baselens to compare)" } else { "release" },
        cfg.name,
        cfg.subtitle(),
        BENCH_RUNS,
        one_line(sql, 120)
    ));
    let wait = Duration::from_secs(120);
    let mut connect = (vec![], vec![], vec![], vec![]);
    let (mut queue, mut server, mut rows, mut bytes, mut batches, mut total) = (vec![], vec![], 0, 0, 0, vec![]);
    for run in 0..BENCH_RUNS {
        let h = spawn(cfg.clone(), password.clone());
        // connect
        loop {
            match h.rx.recv_timeout(wait).map_err(|_| "timed out waiting for the connection".to_string())? {
                Response::Connected { timing, .. } => {
                    connect.0.push(timing.dns_ms);
                    connect.1.push(timing.tcp_ms);
                    connect.2.push(timing.tls_login_ms);
                    connect.3.push(timing.total_ms);
                    break;
                }
                Response::Failed(e) => return Err(e),
                _ => {}
            }
        }
        let t = Instant::now();
        h.send(Request::Query { tag: 1, sql: sql.to_string(), max_rows: crate::app::MAX_RESULTS, sent: Instant::now() });
        loop {
            match h.rx.recv_timeout(wait).map_err(|_| "timed out waiting for the query".to_string())? {
                Response::QueryDone { tag: 1, results, timing } => {
                    if let Err(e) = results {
                        return Err(format!("run {}: {e}", run + 1));
                    }
                    queue.push(timing.queue_ms);
                    server.push(timing.run_ms);
                    total.push(t.elapsed().as_secs_f64() * 1000.0);
                    rows = timing.rows;
                    bytes = timing.bytes;
                    batches = timing.batches;
                    break;
                }
                _ => {}
            }
        }
    }
    let row = |name: &str, v: &[f64]| {
        let (a, b, c) = stats(v);
        format!("{name:<24} {a:>9.1} {b:>9.1} {c:>9.1}\n")
    };
    out.push_str(&format!("{:<24} {:>9} {:>9} {:>9}   (ms)\n", "step", "min", "avg", "max"));
    out.push_str(&row("connect: DNS", &connect.0));
    out.push_str(&row("connect: TCP", &connect.1));
    out.push_str(&row("connect: TLS + login", &connect.2));
    out.push_str(&row("connect: total", &connect.3));
    out.push_str(&row("query: waiting in queue", &queue));
    out.push_str(&row("query: server + rows", &server));
    out.push_str(&row("query: total", &total));
    out.push_str(&format!("\n{rows} rows · {} · {batches} round trip(s)\n", human_bytes(bytes)));
    log(&out.replace('\n', " | "));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_and_sizes() {
        assert_eq!(stats(&[3.0, 1.0, 2.0]), (1.0, 2.0, 3.0));
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(one_line("select *\n  from t", 80), "select * from t");
    }
}
