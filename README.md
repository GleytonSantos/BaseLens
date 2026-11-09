<div align="center">

<img src="assets/baselens-256.png" width="128" alt="BaseLens logo">

# BaseLens

### See into every database.

A free, private desktop editor for **SQLite**, **PostgreSQL** and **MySQL** — written in Rust with [egui](https://github.com/emilk/egui). One fast, native window. No telemetry. Your data never leaves your machine.

[![Release](https://img.shields.io/github/v/release/GleytonSantos/BaseLens?color=8b7bff&label=release)](https://github.com/GleytonSantos/BaseLens/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-38d6c4.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584.svg?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/Linux%20·%20Windows%20·%20macOS-0d1017.svg)](#-install)

[**Download**](https://github.com/GleytonSantos/BaseLens/releases/latest) · [Install](#-install) · [Features](#-features) · [Build](#-build-from-source)

<img src="docs/screenshot.png" width="860" alt="BaseLens screenshot">

</div>

---

## ✨ Features

| | |
| --- | --- |
| 🗂️ **One navigator for every database** | Several connections — and several instances of the same one — side by side. Switch databases without reconnecting. |
| 🔎 **Filters without SQL** | Click a column, pick a condition, or search every text column at once. |
| ⚡ **Smart editor** | Context-aware autocomplete (tables after `FROM`, columns after `alias.`), real values suggested inside `WHERE`, words learned from your queries, a case-preserving formatter, command palette (`Ctrl+K`) and shortcuts (`F1`). |
| 🧭 **Lens** | Ask *"is there an email column?"* or *"where is 'Maria'?"* and it tells you which table holds it. |
| 🔑 **Argon2 tool** | Make or check password hashes (Argon2id/i/d, adjustable memory, passes, lanes and length) — `Ctrl+Shift+H`, or right-click a cell to check its hash or fill it with a new one. |
| 📊 **See your data** | Built-in JSON viewer, charts and an ER diagram. |
| 🤖 **AI assistant** | Local (Ollama / any OpenAI-compatible server), cloud (Anthropic, OpenAI, Google), or the native Claude Code CLI. Read-only queries can run without asking. |
| 🔐 **Private by default** | Passwords encrypted on your computer with the system keyring, optional PIN lock on idle, read-only connections, verified TLS. |
| 🌗 **Dark & light themes** | Follows your system, or pick one. |
| 🚀 **Fast on remote servers** | Queries never wait behind schema loading; results arrive in batches and stop at the limit. |

## 🗄️ Database support

| Engine | Connect by | Notes |
| --- | --- | --- |
| **SQLite** | file path | opened read-only on request; waits out a locked file |
| **PostgreSQL** | host / URL | TLS with certificate verification, cursor-paged results |
| **MySQL / MariaDB** | host / URL | TLS, batched fetch |

Paste a connection URL (`postgresql://user:pass@host:5432/db?sslmode=require`) and BaseLens fills the form for you.

## 📦 Install

### ➡️ [**Download the latest release**](https://github.com/GleytonSantos/BaseLens/releases/latest)

All versions: **https://github.com/GleytonSantos/BaseLens/releases**

Grab the file for your system:

| System | Package | Install |
| --- | --- | --- |
| **Debian · Ubuntu · Mint** | `baselens_<version>_amd64.deb` | `sudo apt install ./baselens_*_amd64.deb` |
| **Fedora · RHEL · openSUSE** | `baselens-<version>.x86_64.rpm` | `sudo dnf install ./baselens-*.x86_64.rpm` |
| **Arch · Manjaro · CachyOS** | `baselens-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U ./baselens-*.pkg.tar.zst` |
| **Any Linux (portable)** | `BaseLens-<version>-x86_64.AppImage` | `chmod +x BaseLens-*.AppImage && ./BaseLens-*.AppImage` |
| **Any Linux (tarball)** | `baselens-<version>-linux-x86_64.tar.gz` | extract and run `./baselens` |
| **Flatpak** | `BaseLens-<version>-x86_64.flatpak` | `flatpak install --user BaseLens-*.flatpak` |
| **Windows 10 / 11** | `…-windows-x64-setup.exe` | run the installer (Start-menu + desktop shortcut, no admin) |
| **Windows (portable)** | `…-windows-x64.zip` | unzip, run `BaseLens.exe` |
| **macOS 11+ (Apple Silicon & Intel)** | `…-macos-universal.dmg` | open the `.dmg`, drag to Applications |

<details>
<summary><b>Platform notes</b></summary>

- **AppImage** installs its own menu entry and icon on first launch; remove with `./BaseLens-*.AppImage --remove-desktop-entry`.
- **Windows** builds are unsigned — SmartScreen may warn once: *More info → Run anyway*.
- **macOS** is not notarised — the first time, **right-click the app → Open**.
- **Arch**: you can also build it yourself with `makepkg -si` inside `packaging/arch`.

</details>

## 🔐 Security

- Database passwords and API keys are encrypted with **ChaCha20-Poly1305**; the master key lives in your **system keyring** (libsecret / KDE Wallet), or in a private `0600` file when no keyring is available.
- **TLS certificates are verified** by default; the automatic plaintext→TLS upgrade verifies too.
- **Read-only connections** are enforced at the driver level (SQLite open flag, `SET TRANSACTION READ ONLY`), and the AI never auto-runs anything but a verified read query.
- Optional **PIN lock** when the app sits idle.

Nothing is sent anywhere unless *you* pick a cloud AI provider.

## 🛠️ Build from source

```sh
cargo run --release
```

Linux needs `pkg-config` and the OpenSSL headers (`libssl-dev` / `openssl-devel`).

<details>
<summary><b>Build the distributable packages</b></summary>

| Command | Makes | Runs on |
| --- | --- | --- |
| `scripts/build-all.sh` | everything this machine can make | Linux + Docker |
| `scripts/package-linux.sh` | `.deb`, `.rpm`, AppImage, `.tar.gz` | `packaging/docker` (Debian 12) |
| `scripts/package-windows.sh` | NSIS installer + portable `.zip` | `packaging/docker` (cross-compiled) |
| `scripts/package-flatpak.sh` | `.flatpak` bundle | Linux + `flatpak` |
| `scripts/package-arch.sh` | `.pkg.tar.zst` | Arch-based Linux |
| `scripts/package-macos.sh` | universal `.app` in `.dmg` / `.zip` | macOS |

Everything lands in `dist/`. Releases are built automatically by [`.github/workflows/release.yml`](.github/workflows/release.yml) on every `v*` tag.

</details>

## ⌨️ Command-line options

| Option | What it does |
| --- | --- |
| `--debug` | debug mode: timings panel (`F12`) and `debug.log` |
| `--bench "<sql>" [--conn <name>]` | times a query 10× on a saved connection |
| `--screenshot <file.png>` | opens with demo data, saves a picture, quits |
| `--self-test` | test robot: clicks and types through the app, reports problems |
| `--install-desktop-entry` / `--remove-desktop-entry` | add / remove menu entry and icon for a non-packaged copy |
| `--app-id` | prints the window id the desktop uses for the icon |
| `--export-brand <folder>` | writes the logo (SVG, PNGs, `.ico`) and `.desktop` file |

## 🧪 Tests

```sh
cargo test                       # unit tests
cargo run -- --self-test         # UI test robot
BASELENS_PG_TEST=postgresql://user:pw@localhost/db cargo test live_pg -- --ignored
```

## 📖 How it works

[MECHANICS.md](MECHANICS.md) lists every value and rule the app uses and where each one comes from; [mechanics.json](mechanics.json) has the same, machine-readable.

## 📄 License

[MIT](LICENSE) © Gleyton Santos

<div align="center">

**[⬆ Download the latest release](https://github.com/GleytonSantos/BaseLens/releases/latest)**

</div>
