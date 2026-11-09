//! SSH tunnels for databases behind a bastion host.
//!
//! BaseLens starts the system's own `ssh` (OpenSSH: Linux, macOS, Windows
//! 10+) with a local port forward, so the person's keys, ssh-agent,
//! `~/.ssh/config` and known_hosts all work as usual:
//!
//! ```text
//! ssh -N -L 127.0.0.1:<free port>:<db host>:<db port> -p <ssh port> [-i key] -- user@bastion
//! ```
//!
//! The database connection then goes to 127.0.0.1:<free port>. The tunnel
//! lives as long as the connection's worker; dropping it stops `ssh`.
//! Logins that need a typed password aren't supported (BatchMode): use a
//! key or ssh-agent.

use crate::db::ConnConfig;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for ssh to log in and open the forward.
pub const TUNNEL_START_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Tunnel {
    child: Child,
    pub port: u16,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A host, user or path that can't be mistaken for an ssh option.
fn safe(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The ssh command line for a tunnel to `cfg`'s database through its
/// bastion, forwarding 127.0.0.1:`local_port`.
pub fn ssh_args(cfg: &ConnConfig, local_port: u16) -> Result<Vec<String>, String> {
    let (host, user) = (cfg.ssh_host.trim(), cfg.ssh_user.trim());
    if !safe(host) {
        return Err("Type the SSH host (the bastion), without spaces.".into());
    }
    if !user.is_empty() && !safe(user) {
        return Err("The SSH user can't start with “-” or contain spaces.".into());
    }
    let db_host = cfg.host.trim();
    if !safe(db_host) {
        return Err("Type the database host as seen from the SSH server (often localhost).".into());
    }
    let db_host = if db_host.contains(':') { format!("[{db_host}]") } else { db_host.to_string() };
    let mut a: Vec<String> = [
        "-N", "-T",
        "-o", "ExitOnForwardFailure=yes",
        "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=15",
        "-o", "ServerAliveInterval=30",
        // trust a new server once, refuse a changed key (like most clients)
        "-o", "StrictHostKeyChecking=accept-new",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push("-L".into());
    a.push(format!("127.0.0.1:{local_port}:{db_host}:{}", cfg.port));
    a.push("-p".into());
    a.push(cfg.ssh_port.to_string());
    let key = cfg.ssh_key.trim();
    if !key.is_empty() {
        if key.starts_with('-') {
            return Err("The key file path can't start with “-”.".into());
        }
        a.push("-i".into());
        a.push(key.to_string());
        a.push("-o".into());
        a.push("IdentitiesOnly=yes".into());
    }
    a.push("--".into());
    a.push(if user.is_empty() { host.to_string() } else { format!("{user}@{host}") });
    Ok(a)
}

fn free_port() -> Result<u16, String> {
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("no free local port: {e}"))?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

/// Plain advice for common ssh failures.
fn advice(stderr: &str) -> &'static str {
    let s = stderr.to_lowercase();
    if s.contains("permission denied") {
        "The SSH server didn't accept the login. Check the SSH user and key file (password logins aren't supported: use a key or ssh-agent)."
    } else if s.contains("host key verification failed") || s.contains("remote host identification has changed") {
        "The SSH server's key is different from the one saved in ~/.ssh/known_hosts. If the server really changed, remove the old line; otherwise someone may be in between."
    } else if s.contains("could not resolve") {
        "The SSH host name couldn't be found. Check it for typos."
    } else if s.contains("connection refused") || s.contains("timed out") || s.contains("no route") {
        "Nothing answered on the SSH port. Check the SSH host and port."
    } else if s.contains("forwarding failed") || s.contains("administratively prohibited") {
        "The SSH server refused to forward to the database. Check the database host and port as seen from the SSH server."
    } else {
        "The SSH tunnel couldn't be opened."
    }
}

/// Starts ssh and waits until the local end of the tunnel accepts connections.
pub fn open(cfg: &ConnConfig) -> Result<Tunnel, String> {
    let port = free_port()?;
    let args = ssh_args(cfg, port)?;
    let mut cmd = Command::new("ssh");
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().map_err(|e| format!("Couldn't start ssh ({e}). Install OpenSSH (it comes with Linux, macOS and Windows 10+)."))?;
    let mut t = Tunnel { child, port };
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let start = Instant::now();
    loop {
        if let Ok(Some(_)) = t.child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = t.child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            let err = err.trim();
            return Err(format!("{}\n\nssh: {}", advice(err), if err.is_empty() { "(no message)" } else { err }));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            // keep ssh's messages flowing so its pipe never fills up
            if let Some(mut e) = t.child.stderr.take() {
                std::thread::spawn(move || {
                    let mut s = String::new();
                    let _ = e.read_to_string(&mut s);
                    if !s.trim().is_empty() {
                        crate::debug::log(&format!("ssh: {}", s.trim()));
                    }
                });
            }
            return Ok(t);
        }
        if start.elapsed() > TUNNEL_START_TIMEOUT {
            return Err(format!("The SSH tunnel didn't open within {} seconds. Check the SSH host, port and login.", TUNNEL_START_TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ConnConfig {
        ConnConfig { host: "db.internal".into(), port: 5432, ssh: true, ssh_host: "bastion.example.com".into(), ssh_user: "me".into(), ssh_port: 2222, ..Default::default() }
    }

    #[test]
    fn builds_a_safe_ssh_command() {
        let a = ssh_args(&cfg(), 40000).unwrap();
        assert!(a.windows(2).any(|w| w == ["-L", "127.0.0.1:40000:db.internal:5432"]));
        assert!(a.windows(2).any(|w| w == ["-p", "2222"]));
        assert_eq!(a[a.len() - 2..], ["--".to_string(), "me@bastion.example.com".to_string()]);
        assert!(a.contains(&"BatchMode=yes".to_string()) && a.contains(&"StrictHostKeyChecking=accept-new".to_string()));
        // a key file adds -i and IdentitiesOnly
        let mut c = cfg();
        c.ssh_key = "/home/me/.ssh/id_ed25519".into();
        let a = ssh_args(&c, 1).unwrap();
        assert!(a.windows(2).any(|w| w == ["-i", "/home/me/.ssh/id_ed25519"]));
        // IPv6 database hosts get brackets
        c.host = "fd00::5".into();
        assert!(ssh_args(&c, 1).unwrap().iter().any(|x| x == "127.0.0.1:1:[fd00::5]:5432"));
    }

    #[test]
    fn refuses_values_that_look_like_options() {
        let mut c = cfg();
        c.ssh_host = "-oProxyCommand=evil".into();
        assert!(ssh_args(&c, 1).is_err());
        let mut c = cfg();
        c.ssh_user = "-F/tmp/x".into();
        assert!(ssh_args(&c, 1).is_err());
        let mut c = cfg();
        c.host = "a b".into();
        assert!(ssh_args(&c, 1).is_err());
        let mut c = cfg();
        c.ssh_key = "-oProxyCommand=x".into();
        assert!(ssh_args(&c, 1).is_err());
    }
}
