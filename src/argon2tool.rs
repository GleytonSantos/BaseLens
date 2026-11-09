//! Argon2 tool: make and check password hashes (Argon2id/i/d, PHC strings
//! like `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`), e.g. to fill or
//! check a `password_hash` column. Runs on a background thread so large
//! memory settings never freeze the window. Nothing typed here is saved.

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use std::sync::mpsc::Receiver;

/// Defaults: the `argon2` crate's (OWASP baseline) — 19 MiB, 2 passes, 1 lane, 32-byte hash.
pub const DEFAULT_MEM_KIB: u32 = 19_456;
pub const DEFAULT_TIME: u32 = 2;
pub const DEFAULT_LANES: u32 = 1;
pub const DEFAULT_OUT_LEN: u32 = 32;
/// Limits offered in the tool (memory up to 1 GiB).
pub const MEM_KIB_RANGE: (u32, u32) = (8, 1_048_576);
pub const TIME_RANGE: (u32, u32) = (1, 16);
pub const LANES_RANGE: (u32, u32) = (1, 16);
pub const OUT_LEN_RANGE: (u32, u32) = (16, 64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Id,
    I,
    D,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Id, Variant::I, Variant::D];
    pub fn label(self) -> &'static str {
        match self {
            Variant::Id => "Argon2id (recommended)",
            Variant::I => "Argon2i",
            Variant::D => "Argon2d",
        }
    }
    fn algorithm(self) -> Algorithm {
        match self {
            Variant::Id => Algorithm::Argon2id,
            Variant::I => Algorithm::Argon2i,
            Variant::D => Algorithm::Argon2d,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    pub variant: Variant,
    pub mem_kib: u32,
    pub time: u32,
    pub lanes: u32,
    pub out_len: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { variant: Variant::Id, mem_kib: DEFAULT_MEM_KIB, time: DEFAULT_TIME, lanes: DEFAULT_LANES, out_len: DEFAULT_OUT_LEN }
    }
}

impl Settings {
    fn params(&self) -> Result<Params, String> {
        // Argon2 needs at least 8 KiB of memory per lane
        let mem = self.mem_kib.max(8 * self.lanes);
        Params::new(mem, self.time, self.lanes, Some(self.out_len as usize)).map_err(|e| e.to_string())
    }
}

/// A new hash of `text` with a fresh random salt, as a PHC string.
pub fn hash(text: &str, s: &Settings) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::new(s.variant.algorithm(), Version::V0x13, s.params()?)
        .hash_password(text.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

/// Plain description of a PHC string's settings ("Argon2id · 19 MiB · 2 passes · 1 lane").
pub fn describe(phc: &str) -> Result<String, String> {
    let h = PasswordHash::new(phc.trim()).map_err(|e| format!("Not an Argon2 hash: {e}"))?;
    let alg = h.algorithm.as_str();
    if !alg.starts_with("argon2") {
        return Err(format!("This is a {alg} hash, not Argon2."));
    }
    let p = |k: &str| h.params.get_decimal(k);
    let mem = p("m").unwrap_or(0);
    let mem_txt = if mem >= 1024 && mem % 1024 == 0 { format!("{} MiB", mem / 1024) } else { format!("{mem} KiB") };
    let t = p("t").unwrap_or(0);
    let l = p("p").unwrap_or(0);
    let name = match alg {
        "argon2id" => "Argon2id",
        "argon2i" => "Argon2i",
        "argon2d" => "Argon2d",
        x => x,
    };
    let out = h.hash.map(|o| o.len()).unwrap_or(0);
    Ok(format!(
        "{name} · version {} · {mem_txt} · {t} pass{} · {l} lane{} · {out}-byte hash",
        h.version.map(|v| format!("0x{v:x}")).unwrap_or_else(|| "?".into()),
        if t == 1 { "" } else { "es" },
        if l == 1 { "" } else { "s" },
    ))
}

/// Does `text` match the PHC string? Uses the settings stored in the string.
pub fn verify(text: &str, phc: &str) -> Result<bool, String> {
    let h = PasswordHash::new(phc.trim()).map_err(|e| format!("Not an Argon2 hash: {e}"))?;
    if !h.algorithm.as_str().starts_with("argon2") {
        return Err(format!("This is a {} hash, not Argon2.", h.algorithm.as_str()));
    }
    Ok(Argon2::default().verify_password(text.as_bytes(), &h).is_ok())
}

/// True for text that looks like an Argon2 PHC string.
pub fn looks_like_hash(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("$argon2id$") || s.starts_with("$argon2i$") || s.starts_with("$argon2d$")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hash,
    Verify,
}

/// What the tool window shows.
pub struct Tool {
    pub mode: Mode,
    pub text: String,
    pub show_text: bool,
    pub settings: Settings,
    pub output: String,
    pub check_hash: String,
    /// last outcome: Ok(message) or Err(message)
    pub result: Option<Result<String, String>>,
    pub busy: Option<Receiver<Result<String, String>>>,
    /// a table cell to fill: (tab id, row, column)
    pub target: Option<(u64, usize, usize)>,
}

impl Tool {
    pub fn new(mode: Mode) -> Self {
        Tool {
            mode,
            text: String::new(),
            show_text: false,
            settings: Settings::default(),
            output: String::new(),
            check_hash: String::new(),
            result: None,
            busy: None,
            target: None,
        }
    }

    /// Starts the work on a background thread.
    pub fn start(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (mode, text, settings, phc) = (self.mode, self.text.clone(), self.settings, self.check_hash.clone());
        std::thread::spawn(move || {
            let r = match mode {
                Mode::Hash => hash(&text, &settings),
                Mode::Verify => verify(&text, &phc).map(|ok| if ok { "match".to_string() } else { "no match".to_string() }),
            };
            let _ = tx.send(r);
        });
        self.busy = Some(rx);
        self.result = None;
    }

    /// Picks up a finished result (call every frame).
    pub fn poll(&mut self) {
        let Some(rx) = &self.busy else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.busy = None;
        match (self.mode, r) {
            (Mode::Hash, Ok(h)) => {
                self.output = h;
                self.result = Some(Ok("Hash made with a new random salt.".into()));
            }
            (Mode::Verify, Ok(m)) if m == "match" => self.result = Some(Ok("✔ The password matches this hash.".into())),
            (Mode::Verify, Ok(_)) => self.result = Some(Err("✘ The password does not match this hash.".into())),
            (_, Err(e)) => self.result = Some(Err(e)),
        }
        // nothing typed stays around longer than needed
        self.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_check_and_describe() {
        let s = Settings { mem_kib: 1024, time: 1, ..Default::default() };
        let h = hash("hunter2", &s).unwrap();
        assert!(looks_like_hash(&h) && h.starts_with("$argon2id$v=19$m=1024,t=1,p=1$"));
        assert_eq!(verify("hunter2", &h), Ok(true));
        assert_eq!(verify("hunter3", &h), Ok(false));
        assert_eq!(describe(&h).unwrap(), "Argon2id · version 0x13 · 1 MiB · 1 pass · 1 lane · 32-byte hash");
        // two hashes of the same text differ (random salt) but both check
        let h2 = hash("hunter2", &s).unwrap();
        assert_ne!(h, h2);
        assert_eq!(verify("hunter2", &h2), Ok(true));
        // other variants and lengths
        let d = hash("x", &Settings { variant: Variant::D, mem_kib: 64, time: 1, lanes: 2, out_len: 16 }).unwrap();
        assert!(d.starts_with("$argon2d$v=19$m=64,t=1,p=2$"));
        assert_eq!(verify("x", &d), Ok(true));
        assert!(describe(&d).unwrap().contains("16-byte hash"));
        // the app-lock PIN hash is readable by the tool too
        let pin = crate::store::hash_pin_argon2("123456");
        assert_eq!(verify("123456", &pin), Ok(true));
        assert!(describe(&pin).unwrap().starts_with("Argon2id · version 0x13 · 19 MiB · 2 passes"));
    }

    #[test]
    fn refuses_what_is_not_argon2() {
        assert!(verify("x", "not a hash").is_err());
        assert!(verify("x", "$2b$12$abcdefghijklmnopqrstuuJ2nqZ6F0pY3d1u0sY7sZ1m3Qp4o5r6S").is_err());
        assert!(!looks_like_hash("$2b$12$abc"));
        assert!(looks_like_hash("  $argon2i$v=19$m=8,t=1,p=1$c2FsdHNhbHQ$aGFzaA "));
    }
}
