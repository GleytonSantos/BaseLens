//! Saving settings, connections and history on disk. Secrets (database
//! passwords, AI keys) are encrypted with ChaCha20-Poly1305 using a random
//! master key kept in the system keyring (libsecret or KDE Wallet), or in a
//! private file (mode 0600) when no keyring is available.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// True while the self-test / monkey robot drives the app: no file dialogs,
/// no AI calls, and everything is stored in a throw-away folder.
static ROBOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_robot_mode(dir: &std::path::Path) {
    ROBOT.store(true, std::sync::atomic::Ordering::SeqCst);
    std::env::set_var("BASELENS_HOME", dir);
    std::env::set_var("FAVO_PASSWORD_STORE", "basic");
}

pub fn robot_mode() -> bool {
    ROBOT.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn data_dir() -> PathBuf {
    if let Ok(h) = std::env::var("BASELENS_HOME") {
        let d = PathBuf::from(h);
        let _ = std::fs::create_dir_all(&d);
        return d;
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("baselens");
    // the app was called "Favo DB" before: keep the user's saved data
    let old = base.join("favo-db");
    if !d.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &d);
    }
    let _ = std::fs::create_dir_all(&d);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    d
}

fn write_private(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    let p = data_dir().join(name);
    std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save<T: Serialize>(name: &str, value: &T) {
    let p = data_dir().join(name);
    if let Ok(b) = serde_json::to_vec_pretty(value) {
        let _ = write_private(&p, &b);
    }
}

// ------------------------------------------------------------ master key
//
// Rule from the original's password-store picker (function 0x7505630,
// key_storage_util_linux): the switch value "basic" -> plain backend,
// "gnome-libsecret" -> libsecret, "kwallet" / "kwallet5" / "kwallet6" ->
// KWallet; otherwise the desktop environment decides (KDE -> KWallet, others
// -> libsecret), falling back to the basic backend. Our own code follows the
// same order; the switch is the FAVO_PASSWORD_STORE environment variable.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum KeyBackend {
    Basic,
    Libsecret,
    Kwallet,
}

impl KeyBackend {
    pub fn label(self) -> &'static str {
        match self {
            KeyBackend::Basic => "private file (basic)",
            KeyBackend::Libsecret => "system keyring (Secret Service / libsecret)",
            KeyBackend::Kwallet => "KDE Wallet",
        }
    }
}

fn backend_order() -> Vec<KeyBackend> {
    let choice = std::env::var("FAVO_PASSWORD_STORE").unwrap_or_default().to_lowercase();
    match choice.as_str() {
        "basic" => return vec![KeyBackend::Basic],
        "gnome-libsecret" => return vec![KeyBackend::Libsecret, KeyBackend::Basic],
        "kwallet" | "kwallet5" | "kwallet6" => return vec![KeyBackend::Kwallet, KeyBackend::Basic],
        _ => {}
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
    if desktop.contains("KDE") {
        vec![KeyBackend::Kwallet, KeyBackend::Libsecret, KeyBackend::Basic]
    } else {
        vec![KeyBackend::Libsecret, KeyBackend::Kwallet, KeyBackend::Basic]
    }
}

const KR_APP: &str = "baselens";
const KR_ENTRY: &str = "master-key";
const KW_FOLDER: &str = "BaseLens";
/// names used before the rename, read once and moved over
const KR_APP_OLD: &str = "favo-db";
const KW_FOLDER_OLD: &str = "FavoDB";
const KW_WALLET: &str = "kdewallet";

fn run_with_stdin(cmd: &str, args: &[&str], input: Option<&str>) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut si) = child.stdin.take() {
        if let Some(i) = input {
            let _ = si.write_all(i.as_bytes());
        }
    }
    let out = child.wait_with_output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

fn keyring_get(b: KeyBackend) -> Option<[u8; 32]> {
    keyring_get_named(b, KR_APP, KW_FOLDER)
}

fn keyring_get_named(b: KeyBackend, app: &str, folder: &str) -> Option<[u8; 32]> {
    let txt = match b {
        KeyBackend::Libsecret => run_with_stdin("secret-tool", &["lookup", "application", app, "entry", KR_ENTRY], None)?,
        KeyBackend::Kwallet => run_with_stdin("kwallet-query", &["-r", KR_ENTRY, "-f", folder, KW_WALLET], None)?,
        KeyBackend::Basic => return None,
    };
    let raw = B64.decode(txt.trim()).ok()?;
    if raw.len() != 32 {
        return None;
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&raw);
    Some(k)
}

fn keyring_put(b: KeyBackend, k: &[u8; 32]) -> bool {
    let enc = B64.encode(k);
    let stored = match b {
        KeyBackend::Libsecret => run_with_stdin(
            "secret-tool",
            &["store", "--label=BaseLens master key", "application", KR_APP, "entry", KR_ENTRY],
            Some(&enc),
        ),
        KeyBackend::Kwallet => run_with_stdin("kwallet-query", &["-w", KR_ENTRY, "-f", KW_FOLDER, KW_WALLET], Some(&enc)),
        KeyBackend::Basic => None,
    };
    stored.is_some() && keyring_get(b).as_ref() == Some(k)
}

fn file_key_get(p: &PathBuf) -> Option<[u8; 32]> {
    let b = std::fs::read(p).ok()?;
    if b.len() != 32 {
        return None;
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&b);
    Some(k)
}

static KEY: std::sync::OnceLock<([u8; 32], KeyBackend)> = std::sync::OnceLock::new();

/// Where the master key lives (shown in Settings → Security).
pub fn key_backend() -> KeyBackend {
    key_and_backend().1
}

fn key() -> [u8; 32] {
    key_and_backend().0
}

fn key_and_backend() -> ([u8; 32], KeyBackend) {
    *KEY.get_or_init(|| {
        let file = data_dir().join("secret.key");
        let order = backend_order();
        // 1. a key already in a keyring
        for b in &order {
            if let Some(k) = keyring_get(*b) {
                return (k, *b);
            }
        }
        // 1b. a key saved under the app's old name: copy it to the new name
        for b in &order {
            if let Some(k) = keyring_get_named(*b, KR_APP_OLD, KW_FOLDER_OLD) {
                let _ = keyring_put(*b, &k);
                return (k, *b);
            }
        }
        // 2. existing file key (older versions / basic backend): move it into
        //    the keyring when one works, so old secrets stay readable
        let k = file_key_get(&file).unwrap_or_else(|| {
            let mut k = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut k);
            k
        });
        for b in &order {
            if *b == KeyBackend::Basic {
                break;
            }
            if keyring_put(*b, &k) {
                if file.exists() {
                    let _ = std::fs::remove_file(&file);
                }
                return (k, *b);
            }
        }
        // 3. basic backend: private file, mode 0600
        let _ = write_private(&file, &k);
        (k, KeyBackend::Basic)
    })
}

pub fn encrypt(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    let k = key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&k));
    let mut n = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut n);
    match cipher.encrypt(Nonce::from_slice(&n), plain.as_bytes()) {
        Ok(ct) => {
            let mut out = n.to_vec();
            out.extend(ct);
            format!("enc:{}", B64.encode(out))
        }
        Err(_) => String::new(),
    }
}

/// None when something is stored but can't be decrypted (for example the
/// key in the system keyring couldn't be read); Some("") when nothing is stored.
pub fn decrypt_checked(stored: &str) -> Option<String> {
    if stored.is_empty() {
        return Some(String::new());
    }
    let v = decrypt(stored);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

pub fn decrypt(stored: &str) -> String {
    let Some(b64) = stored.strip_prefix("enc:") else {
        return String::new();
    };
    let Ok(raw) = B64.decode(b64) else {
        return String::new();
    };
    if raw.len() < 13 {
        return String::new();
    }
    let k = key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&k));
    cipher
        .decrypt(Nonce::from_slice(&raw[..12]), &raw[12..])
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

/// Hash for the app-lock PIN: Argon2id (memory-hard, 19 MiB, 2 passes, the
/// OWASP baseline) in PHC form `$argon2id$...`, with its own random salt.
pub fn hash_pin_argon2(pin: &str) -> String {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    argon2::Argon2::default().hash_password(pin.as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
}

/// Checks a PIN against a stored hash. Returns (matches, needs_upgrade):
/// hashes from older versions (SHA-256 x 50 000) still unlock, and are then
/// replaced by an Argon2id hash.
pub fn verify_pin(pin: &str, salt: &str, stored: &str) -> (bool, bool) {
    if stored.starts_with("$argon2") {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let ok = PasswordHash::new(stored).map(|h| argon2::Argon2::default().verify_password(pin.as_bytes(), &h).is_ok()).unwrap_or(false);
        (ok, false)
    } else {
        let ok = !stored.is_empty() && hash_pin(pin, salt) == stored;
        (ok, ok)
    }
}

/// Old PIN hash (SHA-256 stretched 50 000 times), kept to read older settings.
pub fn hash_pin(pin: &str, salt: &str) -> String {
    let mut h = Sha256::digest(format!("{salt}:{pin}").as_bytes()).to_vec();
    for _ in 0..50_000 {
        let mut s = Sha256::new();
        s.update(&h);
        s.update(salt.as_bytes());
        h = s.finalize().to_vec();
    }
    B64.encode(h)
}

pub fn new_salt() -> String {
    let mut s = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut s);
    B64.encode(s)
}

/// Builds a small sample SQLite database (our own data) for first use.
pub fn ensure_demo_db() -> PathBuf {
    let p = data_dir().join("demo.sqlite");
    if p.exists() {
        return p;
    }
    if let Ok(c) = rusqlite::Connection::open(&p) {
        let _ = c.execute_batch(DEMO_SCHEMA);
        let countries = ["Brazil", "Portugal", "Canada", "Japan", "Kenya", "Chile", "Norway"];
        let first = ["Ana", "Bruno", "Carla", "Diego", "Elena", "Felipe", "Gabi", "Hugo", "Iris", "João", "Kai", "Lia"];
        let last = ["Silva", "Moreau", "Tanaka", "Okafor", "Lund", "Rossi", "Costa"];
        let tx = c.unchecked_transaction().ok();
        for i in 0..36 {
            let n = format!("{} {}", first[i % first.len()], last[(i * 5) % last.len()]);
            let email = format!("{}{}@example.com", first[i % first.len()].to_lowercase().replace('ã', "a"), i + 1);
            let plan = ["free", "pro", "team"][i % 3];
            let tags: &[&str] = [&["early", "beta"][..], &["vip"][..], &["support", "trial", "edu"][..]][i % 3];
            let city = ["Recife", "Lisboa", "Toronto", "Osaka", "Nairobi", "Santiago", "Bergen"][i % 7];
            let profile = serde_json::json!({
                "plan": plan,
                "newsletter": i % 2 == 0,
                "tags": tags,
                "address": {"city": city, "zip": format!("{:05}", 10000 + i * 137)}
            });
            let _ = c.execute(
                "INSERT INTO customers(name,email,country,created_at,profile) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![n, email, countries[i % countries.len()], format!("2026-{:02}-{:02} 10:{:02}:00", 1 + i % 10, 1 + (i * 3) % 27, i % 60), profile.to_string()],
            );
        }
        let products = [
            ("Honey Jar", "Food", 12.5), ("Beeswax Candle", "Home", 18.0), ("Wooden Hive", "Garden", 149.9),
            ("Smoker", "Tools", 39.0), ("Veil Hat", "Apparel", 24.5), ("Gloves", "Apparel", 15.0),
            ("Pollen Pack", "Food", 9.9), ("Hive Tool", "Tools", 11.0), ("Frame Set", "Garden", 54.0),
            ("Royal Jelly", "Food", 29.9), ("Wax Wraps", "Home", 14.0), ("Observation Kit", "Garden", 89.0),
        ];
        for (i, (n, cat, price)) in products.iter().enumerate() {
            let colors: &[&str] = [&["amber"][..], &["gold", "white"][..], &["natural"][..]][i % 3];
            let attrs = serde_json::json!({"weight_g": 100 + i * 75, "organic": i % 3 != 1, "colors": colors, "dimensions": {"w": 10 + i, "h": 6 + i % 4}});
            let _ = c.execute(
                "INSERT INTO products(name,category,price,stock,attributes) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![n, cat, price, 5 + (i * 17) % 120, attrs.to_string()],
            );
        }
        let statuses = ["paid", "shipped", "delivered", "pending", "refunded"];
        for i in 0..80usize {
            let cust = 1 + (i * 7) % 36;
            let _ = c.execute(
                "INSERT INTO orders(customer_id,status,created_at,total) VALUES (?1,?2,?3,0)",
                rusqlite::params![cust as i64, statuses[(i * 3) % 5], format!("2026-{:02}-{:02}", 1 + i % 10, 1 + (i * 7) % 28)],
            );
            let oid = c.last_insert_rowid();
            for k in 0..(1 + i % 3) {
                let pid = 1 + (i + k * 5) % products.len();
                let qty = 1 + (i + k) % 4;
                let _ = c.execute(
                    "INSERT INTO order_items(order_id,product_id,qty,unit_price) VALUES (?1,?2,?3,(SELECT price FROM products WHERE id=?2))",
                    rusqlite::params![oid, pid as i64, qty as i64],
                );
            }
        }
        let _ = c.execute_batch("UPDATE orders SET total = (SELECT ROUND(SUM(qty*unit_price),2) FROM order_items WHERE order_id = orders.id);");
        if let Some(t) = tx {
            let _ = t.commit();
        }
    }
    p
}

const DEMO_SCHEMA: &str = "
CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT UNIQUE, country TEXT, created_at TEXT, profile JSON);
CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, category TEXT, price REAL, stock INTEGER, attributes JSON);
CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), status TEXT, created_at TEXT, total REAL);
CREATE TABLE order_items (id INTEGER PRIMARY KEY, order_id INTEGER REFERENCES orders(id), product_id INTEGER REFERENCES products(id), qty INTEGER, unit_price REAL);
CREATE VIEW sales_by_category AS SELECT p.category, ROUND(SUM(oi.qty*oi.unit_price),2) AS revenue, SUM(oi.qty) AS units FROM order_items oi JOIN products p ON p.id = oi.product_id GROUP BY p.category;
";
