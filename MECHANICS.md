# Mechanics sheet

The target binary (`beekeeper-studio-bin`) is the Electron/Chromium runtime; the
app's own logic lives in `resources/app.asar` (JavaScript bundle + CSS) and the
plain settings file `resources/default.config.ini`. The ELF was opened
(target t_c63780578ca4) and the native functions behind the features we
rebuilt were decompiled with the built-in lifter (Ghidra could not open this
190 MB file). No decompiled code was copied; only the rules below.

## Native program (decompiled)

| function | what it does | rule we use | where in our code |
|---|---|---|---|
| `0x2bc6690` (node/electron bootstrap) | builds `appSearchPaths` = `app.asar`, `app`, `default_app.asar`; checks fuse `IsOnlyLoadAppFromAsarEnabled` before setting `appSearchPathsOnlyLoadASAR`, then emits `appCodeLoaded` | app code only comes from its own package; nothing loads external code. Ours: no plugins/extensions loaded; SQLite runtime extensions stay off | `db.rs open_sqlite` |
| `0x7505630` (key_storage_util_linux: pick password store) | string switch: `basic` (5 chars) → plain backend; `kwallet` (7) → KWallet; `kwallet5` (8) → KWallet5; `kwallet6` (8) → KWallet6; `gnome-libsecret` (15) → libsecret; anything else → chosen from the desktop environment (log "Password storage detected desktop environment: …") | master key for saved secrets lives in the OS keyring. Switch = `FAVO_PASSWORD_STORE` (same values). Auto: KDE → KDE Wallet then libsecret; others → libsecret then KDE Wallet; last resort `basic` = private file 0600. An older file key is moved into the keyring | `store.rs backend_order / key_and_backend` |
| `0xb2eb7c0` (dark_mode_manager_linux: Read reply) | reads portal `org.freedesktop.portal.Settings.Read("org.freedesktop.appearance", "color-scheme")` as uint32; sets dark when value == 1 | "System" theme = Dark when color-scheme == 1, else Light | `theme.rs system_prefers_dark` |
| `0xb2eb4c0` (dark_mode_manager_linux: SettingChanged) | on signal with namespace `org.freedesktop.appearance` (26 chars) and key `color-scheme` (12 chars) re-reads the value and updates the theme; also handles `accent-color` | theme follows desktop changes while running; we re-read every 30 s (idle-check interval) in a background thread | `app.rs apply_theme` |

Related values found in the JS bundle while following these:
| value | number | source |
|---|---|---|
| theme default on Linux | `system` | `app.asar:/dist/main.js` migration `UPDATE user_setting SET macDefault='system', windowsDefault='system', linuxDefault='system' WHERE key='theme'` (first default was `dark`) |
| theme menu order | System, Light, Dark, Solarized, Solarized Dark | `main.js` menu `checked:t?.theme?.value===…` |
| dark decision in main process | `nativeTheme.shouldUseDarkColors \|\| theme.includes("dark")` | `main.js` window options |
| SQL editor font size | 14 | renderer `settings.editorFontSize?.value \|\| 14` |

Sources:
- `INI` = `/opt/Beekeeper Studio/resources/default.config.ini`
- `CSS` = `app.asar:/dist/renderer/assets/index-CaFA456d.css`
- `RJS` = `app.asar:/dist/renderer/assets/index-BX4-pU21.js`
- `AI`  = `bundled_plugins/@beekeeperstudio/bks-ai-shell` (source map: `src/config.ts`, `src/tools/index.ts`, `src/stores/configuration.ts`, `src/providers/OllamaProvider.ts`, `src/instructions.ts`)

## Layout
| value | number | source |
|---|---|---|
| primary sidebar width (default) | 240 px | RJS `primarySidebarWidth-v3` default `sxe=240` |
| secondary sidebar width (JSON viewer) | 240 px | RJS `secondarySidebarWidth-v3` default `cxe=240` |
| main content min width | 200 px | INI `[ui.layout] mainContentMinWidth` |
| primary sidebar min width | 150 px | INI `primarySidebarMinWidth` |
| secondary sidebar min width | 150 px | INI `secondarySidebarMinWidth` |
| default sidebar section | tables | INI `defaultGlobalSidebarItem` |
| table list item height | 22.8 px | INI `[ui.tableList] itemHeight` |
| recent connections shown | 10 | INI `recentConnectionsLimit` |

## Data grid
| value | number | source |
|---|---|---|
| table page size | 100 rows | INI `[ui.tableTable] pageSize` |
| default column width | 125 px | INI `defaultColumnWidth` |
| min column width | 100 px | INI `minColumnWidth` |
| max initial column width | 500 px | INI `maxInitialWidth` |
| max column width | 1000 px | INI `maxColumnWidth` |
| large field width | 300 px | INI `largeFieldWidth` |
| cell edit trigger | double click | INI `editTrigger` |
| query max results | 50000 rows | INI `[ui.queryEditor] maxResults` |
| binary display | hex | INI `[ui.general] binaryEncoding` |
| skip to last page (count query) | sqlite/postgres/mysql = true | INI `[db.*] allowSkipToLastPage` |

## Connections
| value | number | source |
|---|---|---|
| postgres connect timeout | 15000 ms | INI `[db.postgres] connectionTimeout` |
| mysql connect timeout | 3600000 ms (comment says 1 minute) — we use 60000 ms | INI `[db.mysql] connectTimeout` |
| supported engine list | 25 engines (MySQL, Postgres, SQLite, SQL Server, …) | RJS `{name:'MySQL',value:'mysql'}…` |
| this rebuild supports | SQLite, PostgreSQL, MySQL/MariaDB | decision |

## Security
| value | number | source |
|---|---|---|
| lock mode | disabled by default, option "pin" | INI `[security] lockMode` |
| min PIN length | 6 | INI `minPinLength` |
| idle threshold before lock | 300 s | INI `idleThresholdSeconds` |
| idle check interval | 30 s | INI `idleCheckIntervalSeconds` |
| runtime extensions | off | INI `allowRuntimeExtensions = false` |
| saved secrets | encrypted (AI keys via `getEncryptedData`) | AI `stores/configuration.ts` |
| master key storage | OS keyring picked by desktop (libsecret / KWallet), `basic` fallback | native `0x7505630` |
| rebuild: secrets | ChaCha20-Poly1305; master key in keyring (file 0600 only as `basic` fallback) | own design + `0x7505630` rule |

## Keyboard
| action | keys | source |
|---|---|---|
| run query | Ctrl+Enter / F5 | INI `primaryQueryAction` |
| run current statement | Ctrl+Shift+Enter / Shift+F5 | INI `secondaryQueryAction` |
| refresh | F5 / Ctrl+R | INI `refresh` |
| save changes | Ctrl+S | INI `save` |
| close tab | Ctrl+W | INI `closeTab` |
| next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | INI `nextTab`, `previousTab` |
| switch to tab N | Alt+1..9 | INI `switchTab1..9` |
| quick search | Ctrl+P | INI `openQuickSearch` |
| table next / prev page | Ctrl+→ / Ctrl+← | INI `[keybindings.tableTable]` |
| set NULL | Backspace | INI `nullSelection` |
| new AI shell | Ctrl+L (plugin) | INI `[keybindings.plugins.bks-ai-shell]` |

## Themes (CSS variables)
| theme | bg | text | primary | secondary | editor bg | sidebar | cell edited | row add | row delete |
|---|---|---|---|---|---|---|---|---|---|
| dark | #181818 | #ffffffab | #fad83b | #4ad0ff | #090909 | #101010 | #50492a | #104633 | #8a3937 |
| light | #f8f8f8 | #000000ab | #f9ce09 | #0099ff | #ffffff | #efefef (estimated) | #fcf3c9 (estimated) | #d0f5e5 (estimated) | #fcd0cf (estimated) |
| solarized | #eee8d5 | #657b83 | #b58900 | #268bd2 | #fdf6e3 | #eee8d5 | #eddfa1 | #d9da9f | #fc8a87 |
| solarized dark | #073642 | #eee8d5 | #b58900 | #268bd2 | #002b36 | #073642 | #eddfa1 | #284c26 | #8a3937 |

Brand colours (dark): info #4ad0ff, success #15db95, warning #ff8d21, danger #ff5d59, purple #9858ff, pink #ff78f7.
Brand colours (light): info #3498db, success #15db95, warning #dc700c, danger #ff5d59, purple #9858ff, pink #ff00f0.
Solarized: info #268bd2, success #859900, warning #b58900, danger #dc322f, purple #6c71c4, pink #d33682.

## AI shell
| value | number | source |
|---|---|---|
| default temperature | 0.7 | AI `config.ts defaultTemperature` |
| Ollama base URL | http://localhost:11434 (+ `v1/` OpenAI-compatible path) | AI `OllamaProvider.ts` |
| Anthropic base | https://api.anthropic.com/v1 | AI bundle string |
| OpenAI base | https://api.openai.com/v1 | AI bundle string |
| Google base | https://generativelanguage.googleapis.com/v1beta | AI bundle string |
| providers | anthropic, openai, google, ollama, openai-compatible, deepseek, zai | AI `config.ts` |
| tools given to the model | get_tables, get_columns, run_query | AI `tools/index.ts` |
| run_query approval | needed unless "allow read-only queries" is on and query is read-only | AI `tools/index.ts needsApproval` |
| allow read-only queries without asking | false | AI `configuration.ts` default |
| system schemas hidden from AI | information_schema, pg_catalog, pg_toast, sys | AI `instructions.ts` |
| rebuild extra: Claude Code | runs the natively installed `claude` CLI (`~/.local/bin/claude` first) in print mode | person's request |

## Redesign (round 3: person asked for a different, nicer, easier look)
Everything visual below is our own design, not the original's:
- Themes: System (follows desktop, rule from `0xb2eb7c0`), Daylight, Midnight (violet #8b7bff + teal #38d6c4), Ocean (teal), Latte (orange). The original's CSS colours are no longer used.
- Layout: 56 px icon rail → card-style side panel (connections / tables with column tree / saved / history) → pill tabs with a command field → main card (12 px corners) → optional JSON inspector card.
- Filters without SQL: search box over every column (type-to-filter after 350 ms), rule rows (column · operator · value, AND/OR), right-click a cell to keep/hide that value, header menu to sort/filter; raw `WHERE` stays as an advanced "SQL" toggle. LIKE escape is `!`.
- Autocomplete: tables after FROM/JOIN/INTO/UPDATE, columns of an alias after `alias.`, columns of tables in the query, keywords/functions/snippets, fuzzy matching; keyword casing = preserve and quoting = auto (from `[ui.queryEditor.autocomplete]` in `default.config.ini`).
- Shortcuts: one command table drives both the keys and the Ctrl+K palette (30 commands); F1 lists them all. Kept from the original: Ctrl+Enter/F5, Ctrl+Shift+Enter, Ctrl+S, Ctrl+W, Ctrl+T, Ctrl+Tab, Alt+1..9, Ctrl+←/→, Ctrl+L, Ctrl+P.

## Native program, round 4 (filters and shortcuts)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| `0x6934ee0` (SQLite `likeFunc`, built into the runtime) | rejects a pattern longer than the connection's LIKE-pattern limit with "LIKE or GLOB pattern too complex"; with an ESCAPE argument, the escape must be exactly one UTF-8 character ("ESCAPE expression must be a single character"); if the escape equals the match-all or match-one wildcard, that wildcard is switched off; then calls `patternCompare` (`0x68580d0`) and returns true when it matches | our filters use the single-character escape `!` (never `%` or `_`); the search/value pattern is cut so that `%…%` stays within the limit, never ending on a lone escape | `filter.rs LIKE_ESCAPE, like_escape` |
| `0x20fd990` (SQLite hard-limit table) | 1000000000 length, 1000000000 SQL length, 2000 columns, **1000 expression depth**, 500 compound select, 250000000 VDBE ops, 1000 function args, 10 attached, **50000 LIKE pattern bytes**, 32766 variables, 1000 trigger depth, 8 worker threads | `LIKE_PATTERN_MAX_BYTES = 50000`; "search every column" ORs at most `SEARCH_COLUMNS_MAX = 900` columns (90 % of the 1000 depth limit) so very wide tables still work | `filter.rs where_clause` (tested against a real SQLite with 1500 columns and a 60000-char search) |
| key-name table at `0xbb7cf68…` (24-byte entries {name, length, key code}, filled by R_X86_64_RELATIVE relocations at `0x957290…`) | `altgr`→0xE1, `backspace`→0x08, `capslock`→0x14, `cmd`→0x5B, **`cmdorctrl`→0x11**, `command`→0x5B, **`commandorcontrol`→0x11**, `control`→0x11 | "Ctrl-or-Cmd" shortcuts are Control on Linux/Windows (key code 0x11) and ⌘ only on macOS; `cmd`/`command` alone mean the Super key. Our key table uses egui's COMMAND modifier (Ctrl here) and shows "Ctrl" (or ⌘ on macOS) | `app.rs Cmd::keys_label`, `shortcuts` |

Searched but not decompilable statically: the accelerator parser itself (string "Invalid accelerator token: " at `0x13d524e` and `accelerator_util.cc` have no static code references; the table is reached through computed pointers). `0x407bed0` (references "super") was read: a 2781-line V8/CSS routine, unrelated to app shortcuts.

## Round 5: navigator, Ask bot, modern look (own design)
- **One navigator for every engine**: connection → database → schema (only when there is more than one) → table → columns. Other databases on the same server are listed under the connection: click = switch, right-click = open side by side. A breadcrumb (connection › database › schema › table) sits above the open tab. A single search box finds tables *and* columns across every open connection.
- **Ask bot** (`finder.rs`, offline, no AI): detects Portuguese/English, drops filler words, singularises, expands a list of synonym groups (preço↔price, telefone↔phone, cliente↔customer…), scores names (exact 120, word part 100, prefix 70, contains 50, +10 for the word as typed). Scope: this table / this database / everywhere. Quoted text (or a lone number) is searched inside the data with one `COUNT(*)` per table (max 80 tables), reusing the filter engine (so the SQLite limits from `0x20fd990` / `0x6934ee0` still apply). Clicking a hit opens the table with the column highlighted; clicking a data hit opens it pre-filtered.
- **Look**: system UI font when available (Adwaita Sans — Inter-based — then Inter, Noto Sans; mono: Adwaita Mono, JetBrains Mono, Noto Sans Mono), read at start-up, never bundled. Grid header band, numbers right-aligned in mono, NULL shown as a pill, gradient hero on the start page.

## Native program, round 6 (Ask bot data search, fonts)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| `0x68580d0` (SQLite `patternCompare`, called by `likeFunc` 0x6934ee0) | walks pattern and text as UTF-8; for a literal character it compares through the case table **only when `(c1 \| c2) <= 0x7f`**, otherwise the two code points must be identical; `%` recurses (with a `strcspn` fast path for ASCII), `_` takes one character, the escape character makes the next one literal | SQLite LIKE ignores case for A–Z only, so "ação" never matches "AÇÃO". For SQLite and a value with non-ASCII letters our filters and the Ask bot also try lower-case, UPPER-case and Capitalised forms (OR for "contains", AND for "doesn't contain"); the number of columns searched is divided by the number of forms so the OR chain stays under the 1000 expression-depth limit (`0x20fd990`) | `filter.rs case_variants, like_any, where_clause` (tested on a real SQLite: plain LIKE finds 1 of 2 rows, ours finds 2) |
| `0x20fd090` (SQLite `sqlite3UpperToLower`) | 256-byte table; bytes 0x41–0x5A map to 0x61–0x7A, every other byte maps to itself | confirms the ASCII-only folding above | — |
| `0xb653560` (references "sans") | 5113-line routine whose strings are "math", "monoflac", "sans": media/MIME handling, not UI fonts | nothing taken; our UI-font choice stays our own design | — |

## Round 7: BaseLens identity (own design)
- Name **BaseLens**, tagline “See into every database.” Wordmark: “Base” in the text colour + “Lens” in the accent.
- Mark (`brand.rs`): rounded tile (corner 0.28 of its size) with a violet #8b7bff → teal #38d6c4 diagonal gradient; a white lens ring (radius 0.235, stroke 0.075) looking into three rows of data, a handle at 45° from the ring, a small glint. The same numbers draw the in-app logo, the window icon (rasterised at 128 px, 4×4 supersampling) and `assets/baselens.svg` / `baselens-256.png` (`--export-brand <folder>`), plus `baselens.desktop`.
- The Ask bot is now **Lens** (“Ask Lens”), with the lens glyph in the rail and panel.
- Data folder is now `~/.config/baselens`; the old `favo-db` folder and keyring entries (`favo-db` / `FavoDB`) are moved over automatically so nothing saved is lost.

## Round 8: value suggestions and learned suggestions (own design)
- **Values while writing WHERE**: when the cursor is on the value side of `col = …`, `<>`, `!=`, `<`, `>`, `IN (…, …`, `LIKE …` (with or without an opening quote), the editor asks the database once per column for
  `SELECT v, COUNT(*) AS n FROM (SELECT col AS v FROM table WHERE col IS NOT NULL LIMIT 10000) s GROUP BY v ORDER BY n DESC, v LIMIT 50`
  — a DISTINCT over a 10000-row sample, top 50 by frequency, so it stays fast on big tables. Results are cached per connection/column; the popup shows up to 14, filtered by what is typed, quoted for text columns and bare for numeric ones. The same values appear in the filter rows (▾ next to the value).
- **Learning**: after each successful query, identifiers (aliases, computed column names, used columns) and the values written in `=`, `<>`, `IN (…)` and `LIKE` comparisons are remembered per saved connection (max 500 words, 300 columns, 30 values each, values up to 80 characters). They show up in autocomplete as “used N×”. Stored encrypted in `learned.json`; can be turned off or erased in Settings → Security.

## Native program, round 9 (value suggestions)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| `0x68a7000` (SQLite `valueFromExpr`, with `sqlite3AffinityType` inlined for CAST; the byte pattern of `'char'` = 0x63686172 is found at `0x68a7152` and in 5 other inlined copies) | turns a literal/CAST expression into a value. For the declared type it keeps `h = (h << 8) \| lower(c)` (lower-casing through the table at `0x20fd090`) and: `h == 'char'/'clob'/'text'` (0x63686172 / 0x636c6f62 / 0x74657874) → TEXT (0x42); `h == 'blob'` (0x626c6f62) and affinity still NUMERIC or REAL → BLOB (0x41); `h == 'real'/'floa'/'doub'` (0x7265616c / 0x666c6f61 / 0x646f7562) and still NUMERIC → REAL (0x45); `(h & 0xffffff) == 'int'` (0x696e74) → INTEGER (0x44) immediately; otherwise NUMERIC (0x43). Also: unary minus negates (with the i64::MIN → float special case), `X'…'` hex blobs are decoded two digits per byte | for SQLite, a suggested value goes **without quotes** only when the column's affinity is INTEGER, REAL or NUMERIC and the value is a number; TEXT/BLOB columns always get a quoted string (so `'007'` keeps its zeros in a VARCHAR column, while a `FLOATING POINT` column — INTEGER by this rule — gets bare numbers). Other engines keep our type-name heuristic | `assist.rs sqlite_affinity, value_literal` (tested against SQLite: `typeof(CAST('12' AS …))` for FLOATING POINT, VARCHAR, DOUBLE, JSON, BLOB) |

## Round 10: bug hunt (charts, tabs, layout) and the test robot
Found and fixed (own design):
- **Tab ×**: the whole tab pill was a click area drawn on top of the × button and swallowed its click, so × only selected the tab. Now a click inside the × area closes, elsewhere selects. Proven by the robot: with the old behaviour put back, 2 checks fail.
- **Charts**: settings now picked automatically per result (label = first text column, value = last number column that isn't an id, “Number of rows” when there are no numbers instead of all-zero bars); rows with the same label are combined (Sum / Average / Count / Highest / Lowest / Each row); order Biggest first / By label (dates) / As returned; at most 20 points by default (1–200), the rest folded into “Other”; dates become a line. Settings stay when the same query runs again; zoom resets when data changes (drag/zoom off, so the chart can't get “lost”); hover shows label and value; labels cut at 14 characters; the chart and its settings (now two rows) never grow past the card.
- **Layout**: wide results scroll sideways inside the card (before, they pushed the card off-screen and hid “+ Row”/“Export”); filter rows size to the card; side panels take at most 30 % of the window; below 1000 px the JSON inspector steps aside, below 860 px the side panel does (any explicit open — rail click, Ctrl+B, Alt+Shift+N — keeps it open).

Test robot (`app/src/app/robot.rs`): `--self-test` drives the real window with injected mouse/keyboard events and checks 23 things (tabs, ×, Ctrl+T/W/K, charts, search, Lens, value suggestions, navigator); `--monkey <frames>` sends random clicks/keys/text/scroll/drags (seed: `BASELENS_SEED`), checking after every frame that no tab or connection selection dangles. Both use a throw-away data folder, no file dialogs, no AI calls; exit code 0 = all good. Runs done: self-test at 1280×800 and 780×500 — 23/23; monkey 4000 + 3×3000 + 3000 (small window) frames — no crash.

## Native program, round 11 (chart sums and averages)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| FuncDef table in `.data.rel.ro` around `0xbe3ac..` (relocations at `0xdaf880…0xdaf940`): `sum` → step `0x693d4b0`, final `0x693da70`; `total` → step `0x693d4b0`, final `0x693e130`; inverse `0x693dba0` | SQLite's built-in aggregate list | located the code behind `sum()`/`avg()` | — |
| `0x693d4b0` (SQLite `sumStep`, read with objdump; only reached through the table) | NULLs are skipped; integers are added exactly into a 64-bit sum with an overflow check (`0x693d837…`); the first real value (or an overflow) switches to a **Kahan-Babuska-Neumaier** sum kept in two doubles: `s = sum + x`; if `|sum| > |x|` then `err += (sum − s) + x` else `err += (x − s) + sum`; `sum = s` (`0x693d6a3…0x693d702`). Integers with `|x| ≥ 2^52` (test `x + 0xfff0000000000000 ≥ 0xffe0000000000001`) are split into `x − x%16384` and `x%16384` (`0x693d67b`, mask 0x3fff/0xc000) | chart Sum/Average use the same algorithm (`SqlSum`): exact integer sums, compensated float sums, the 2^52 split. Our bars now equal `SELECT sum(v)` to the last bit (test with 0.1×10, 1e16+1−1e16, values near 2^53, mixed int/real) | `widgets.rs SqlSum, chart_series` |

## Round 12: PostgreSQL connections that “just say db error”
Rules taken from the original's connection code (`app.asar:/dist/utility.js`, `PostgresClient.configDatabase` / `configurePool`):
- user = the typed user, else the **operating-system user** (`server.config.user || server.config.osUser`) — we do the same (`db.rs effective_user`);
- database = the chosen one, else none (PostgreSQL then uses the user name) — same; when that fails with 3D000 we now say so and suggest “postgres”;
- password omitted when empty — same;
- with SSL on and no CA/cert/key files, `rejectUnauthorized = false` (certificate not checked). We keep checking by default (safer) and explain how to untick “Verify certificate” when a certificate is refused;
- connect timeout `[db.postgres] connectionTimeout = 15000` ms — same.

Fixes: the driver's text is only “db error”; the real error (severity, SQLSTATE code, message, detail, hint, network cause) is now shown, with a plain next step (wrong password 28P01, pg_hba 28000, missing database 3D000, too many connections 53300, server starting 57P03, refused port, timeout, unknown host, certificate) and a “Tried: user@host:port · database · TLS · password empty/given” line. If the server demands encryption (28000 “no encryption”), BaseLens retries once with TLS. A saved password that can't be decrypted (system keyring didn't return the key) no longer turns into an empty password: the connection editor opens asking for it. The port is a plain text box, and changing the database type no longer overwrites a custom port (e.g. 5433).

## Native program, round 13 (the “system user” used when the user field is empty)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| `0x2dd9530` (Node `os.getUserInfo`, registered by `node::os::Initialize` at `0x2dd825a`) | calls `uv_os_get_passwd` (→ `getpwuid_r` for the effective uid); on error throws a UV exception tagged "uv_os_get_passwd"; returns {uid, gid, username, homedir, shell} | step 2 of the user lookup: our own `getpwuid_r(geteuid())` with a growing buffer | `db.rs passwd_username` |
| `preload.js` `username` module (called by `fetchUsername`, passed as `osUser` to `configurePool`) | order: env `SUDO_USER` → `C9_USER` → `LOGNAME` → `USER` → `LNAME` → `USERNAME`; then `os.userInfo().username`; then `id -un $(id -u)`; then `no-username-<uid>` | same order in `os_username()`; used when the connection's user field is empty | `db.rs os_username, effective_user` (tested: passwd lookup = `id -un`; SUDO_USER beats USER) |

## Round 14: “password authentication failed” although the password is right
Investigation (evidence, nothing guessed):
- Only `cloud-sql-proxy --address 0.0.0.0 --port 5433` listens on 5433; a throw-away PostgreSQL 17 (SCRAM-SHA-256, password with symbols and “ç”) accepted BaseLens's client through `localhost` → the client and SCRAM work.
- Comparing the stored passwords (lengths and equality only, never shown): BaseLens had **36 characters whose first 32 equal Beekeeper's password, plus 4 extra (a symbol and 3 digits)** → the server was right to refuse.

Original's storage format, from `app.asar:/dist/main.js` (used to import, read-only):
- saved connections: `<config>/beekeeper-studio/app.db`, table `saved_connection` (name, connectionType, host, port, username, defaultDatabase, password, ssl, sslRejectUnauthorized, readOnlyMode, labelColor…);
- `password` column through `EncryptTransformer` = `simple-encryptor`: key = SHA-256(key string); text = hex HMAC-SHA256(rest) + hex IV (16 bytes) + base64 AES-256-CBC(JSON);
- the key string is `encryptionKey` inside `<config>/beekeeper-studio/.key`, itself encrypted the same way with the built-in `defaultEncryptionKey` "38782F41…29482B".

Changes: “Import from Beekeeper” (Connections panel) adds Beekeeper's connections and, for ones that already exist (same type/host/port/user/database), replaces only the password with Beekeeper's (`import_bk.rs`, tested on a copy: the estante_admin password became identical); “🔑 Beekeeper's password” in the connection editor; 👁 show-password; a live “N characters · space at start/end · accent/quote (dead keys)” note under the password; “🔎 Diagnose” for PostgreSQL (address → who answers (TLS yes/no) → login type asked, without sending the password → password facts → real login); the error line now says “password given (N characters)”.

## Native program, round 15 (decrypting Beekeeper's saved passwords)

| address | what it is | rule we use | where in our code |
|---|---|---|---|
| `0x6d33120` (BoringSSL `EVP_DecryptFinal_ex`, `crypto/fipsmodule/cipher/cipher.cc.inc`; the cipher name "aes-256-cbc" is at `0x1160f56`, used by `0x6e00e20`) | called by Node's `decipher.final()` inside `simple-encryptor`. Errors (library 0x1e = CIPHER): called on an encrypting context → 0x42; padding off and bytes left over → 0x6a `DATA_NOT_MULTIPLE_OF_BLOCK_LENGTH`; bytes left over / no final block → 0x7b `WRONG_FINAL_BLOCK_LENGTH`; last byte `n` with `n == 0` or `n > block size` → 0x65 `BAD_DECRYPT`; any of the last `n` bytes ≠ `n` → 0x65 `BAD_DECRYPT`; otherwise outputs `block − n` bytes | our importer decrypts with AES-256-CBC without padding and then removes PKCS#7 with exactly these checks (`bssl_unpad`), reporting *why* a password couldn't be read (wrong key/HMAC, incomplete block, bad padding, bad JSON) instead of silently skipping it | `import_bk.rs bssl_unpad, simple_decrypt_detailed` (tested: same result as RustCrypto's PKCS#7 on 2000 random blocks) |

Also from this round: the extra 4 characters in the saved password were “@172” (start of an IP pasted with it). The editor now warns when a password ends in “@digits”, and accepts a whole connection URL (`postgresql://user:password@host:port/db?sslmode=…`, last `@` splits credentials, `%xx` decoded; like the original's `parseUrl`, any `sslmode` other than `disable` turns SSL on).

## Round 16 — same behaviour for `select` and `SELECT`; Beekeeper import removed

- Suggestions: snippets now come after keywords and follow the case the person typed, so `sel` + Enter gives `select` and `SEL` + Enter gives `SELECT` (checked by the test robot, steps 16–19).
- Format SQL keeps the person's keyword case (lowercase if most keywords were written in lowercase).
- The connection failure was a password copied with three extra characters, so the Beekeeper import (connections and "Beekeeper's password" button) and its AES/CBC/HMAC code were removed at the person's request. The URL fill, password eye and Postgres diagnose stay. The four mechanics.json values that only served the import (Beekeeper password storage, key file, AES block size, PKCS#7 padding) were dropped.

## Round 16b — decompiled: how the original matches keywords in any case

Read with native_function_pseudocode on **0x6903ab0** (SQLite's identifier writer, reached from the keyword text at 0x20fd320):

- **Case fold**: every byte of the name is compared after `& 0xdf` (lines `r11l &= 0xdf`, `r12l &= 0xdf`, `bl &= 0xdf`), so `select`, `Select` and `SELECT` are the same keyword.
- **Hash**: `h = ((charMap[z[0]] << 2) ^ (charMap[z[n-1]] * 3) ^ n) % 127` (the `* 0x2040811 … >> 6` sequence is the compiler's divide by 127); charMap at 0x20fd090, hash heads at 0x20fdeb0, chain at 0x20fdfd0. Indexes are stored +1, so the length table is read from 0x20fd6f1 and offsets from 0x20fd5c2.
- **Only names of 2+ bytes** are looked up (`r8d >= 2`).
- **Keyword list**: 147 words decoded from those tables (ABORT … WITHOUT, including PRIMARY).
- **Quote rule**: a name is written in double quotes when it is empty, starts with a digit (`c - 0x3a <= 0xf5` fails), has a byte that is not a letter/digit (ctype table 0x20fd210, mask 0x06) or `_`, or is a keyword; embedded `"` are doubled.

Used in the build: `assist.rs` now has `SQLITE_KEYWORDS` (the 147 words) and `is_sql_keyword` with the same `& 0xdf` fold and 2-byte minimum, and autocomplete quotes table/column names by the same rule, so a table called `order`, `Order` or `ORDER` is inserted as `"order"` (MySQL: `` `order` ``) instead of breaking the query. Test `keywords_match_in_any_case_and_get_quoted` checks it.
- Editor colouring (`widgets.rs::is_sql_keyword`) now also uses the 147 keywords from the original, matched in any case, so `select`, `Select`, `SELECT`, `window`, `over` and `partition` are all coloured as keywords. The leftover `src/import_bk.rs` is no longer compiled; it holds only a one-line comment because this environment refuses to delete files.

## Round 17 — decompiled: the original's SQL tokenizer, used for editor colouring

Read with native_function_pseudocode on **0x685a620** (sqlite3GetToken; it is the other function that reads the keyword text 0x20fd320). It looks up each first byte in the class table **0x20fe070** (e.g. `$`=4, `@ : #`=5, `?`=6, quotes `' " \``=8, `[`=9, `-`=11, `/`=16, `.`=26, `x`=0, digits=3) and jumps through 0x20fb304. Rules found:

- **Comments**: `--` runs to the newline (`ecx == 10`); `/*` runs to `*/` (the `dl ^ 0x2a | r8l ^ 0x2f == 0` test) or to the end of the text.
- **Strings and quoted names**: the opening quote ends the token only if it is not doubled (`al == cl` and next byte `== cl` → keep going). `'…'` is a string (token 0x76); `"…"` and `` `…` `` are names (0x3c); `[…]` runs to `]` and is a name.
- **Blobs**: `x'…'`/`X'…'` with hex digits (ctype bit 0x08).
- **Numbers**: `0x`/`0X` + hex digits; otherwise digits, then `.` + digits, then `e`/`E` (byte `| 0x20 == 0x65`) with optional `+`/`-` before a digit; `_` between digits is allowed (token 0xb7). A `.` followed by a digit starts a number.
- **Parameters**: `?` + digits; `:name`, `@name`, `#name`; `$name`.
- **Identifier bytes**: `ctype & 0x46` from 0x20fd210 — letters (0x02), digits (0x04) and 0x40 (`$`, `_`, every byte ≥ 0x80, so `ação` is one name).
- **Keywords**: same `& 0xdf` fold and hash `((charMap[z0]<<2) ^ (charMap[zN-1]*3) ^ n) % 127` as 0x6903ab0.

Used in the build: `widgets.rs::sql_tokens` follows these rules and `sql_job` colours from it, so block comments, `'it''s'`, `"a""b"`, `[col name]`, `0x1F`, `1.5e-3`, `1_000`, `x'ab'` and `?1 :n @m $v` are coloured as one piece each. One BaseLens addition: in Postgres `x::int` the `::` is a cast, not a `:name` parameter. Test `tokens_follow_the_original_tokenizer`.

## Round 18 — speed: queries no longer wait behind schema loading; debug mode

Problems found in BaseLens's own code (not the original's): one worker per connection ran everything in order, and right after connecting it asked for the columns of up to 300 tables one information_schema query at a time, so the person's query waited behind hundreds of round trips (worse through Cloud SQL Proxy). The editor also downloaded the whole result before cutting it at the limit.

Changes (`db.rs`):
- **Second connection for schema questions** (Postgres/MySQL): `Handle::send` routes `Columns`, `AllColumns` and `Graph` to it; it opens on the first such question. Queries and refreshes keep the main connection to themselves. SQLite keeps one connection (local file; `:memory:` databases can't be shared).
- **All columns in one query**: Postgres reads `pg_catalog.pg_attribute` + `pg_class` + `pg_namespace` + `pg_index` (primary key = `attnum = ANY(indkey)`), type from `format_type`; MySQL reads `information_schema.columns` for `DATABASE()` once. The app asks once per table list (`cols_asked`), not once per table.
- **Results in batches**: a single read statement (`SELECT`/`WITH`/`VALUES`/`TABLE` that passes `is_read_query`) runs as `DECLARE baselens_cursor NO SCROLL CURSOR FOR …` + `FETCH n`, n = 200 first and doubling up to 6400, stopping at the row limit; one extra `FETCH 1` tells whether the result was cut. Outside a transaction it is wrapped in `BEGIN … CLOSE; COMMIT`; inside the person's own `BEGIN` it only declares/closes, so their transaction is never committed by BaseLens (`tx_effect` tracks BEGIN/COMMIT/ROLLBACK/END/ABORT). Other statements keep the simple protocol.

Debug mode (`debug.rs`): `baselens --debug` or `BASELENS_DEBUG=1`.
- Per query: time waiting on the queue (and how many requests were ahead), server + rows time, rows, size as text, round trips, time to build the tab, time to draw it on the next frame.
- Per connection: DNS and TCP measured separately; TLS + login is the rest of the driver's connect time.
- Panel **F12** (also the 🐞 label in the status bar): last 50 queries, live queue size per connection, frames per second.
- Log: `~/.local/share/baselens/debug.log` (same numbers).
- `baselens --bench "<sql>" [--conn <name>]`: saved connection by name (else the most recent), 10 runs, min/avg/max of DNS, TCP, TLS + login, connect total, queue, server + rows, total.
- The status bar shows **DEV BUILD · slow** when running the unoptimised `target/debug` build; the optimised one is `target/release/baselens`.

Checked against a throwaway local PostgreSQL 17 (test `live_pg`, run with `BASELENS_PG_TEST=… cargo test live_pg -- --ignored`): all columns with primary keys in one answer; `select * from big` (100 000 rows) with limit 450 returns 450 rows, marked cut, in 3 round trips; an open `BEGIN … INSERT` is still rolled back by the person's `ROLLBACK`.

## Round 18b — decompiled: how the original's sockets are set up

Read with native_function_pseudocode (symbols from the original's dynamic table):

- **`uv_tcp_nodelay` 0x2a19340**: `setsockopt(fd, IPPROTO_TCP=6, TCP_NODELAY=1, &on, 4)`. The original's PostgreSQL driver (node-postgres `Connection.connect` in app.asar) calls `stream.setNoDelay(true)` before connecting, so small protocol messages leave at once (no Nagle delay). BaseLens's driver (tokio-postgres) also turns TCP_NODELAY on.
- **`uv__tcp_keepalive` 0x2a19260**: `SO_KEEPALIVE` (`SOL_SOCKET=1`, option 9); when on, a zero delay is refused (`-EINVAL`), then `TCP_KEEPIDLE=delay` (6, 4), `TCP_KEEPINTVL=1` (6, 5), `TCP_KEEPCNT=10` (6, 6).
- **`node::TCPWrap::SetKeepAlive` 0x2eafb20**: reads `enable` (Int32) and `delay` (Uint32, seconds) from JavaScript and calls `uv_tcp_keepalive`. node-postgres turns it on only when its `keepAlive` option is set (`keepAliveInitialDelayMillis || 0`).

## Round 19 — packages for Linux, Windows and macOS; git repository

- `scripts/build-all.sh` builds in Docker (`packaging/docker`, Debian 12, so the binary needs only glibc 2.36): `.deb` (cargo-deb), `.rpm` (cargo-generate-rpm), AppImage (carries libssl/libcrypto 3), `.tar.gz`, Windows `.zip` (MinGW cross-build, icon put in the .exe by `build.rs`); then a Flatpak bundle (`dev.baselens.BaseLens`, runtime org.freedesktop.Platform 25.08) and the Arch package (`packaging/arch/PKGBUILD`).
- macOS needs a Mac: `scripts/package-macos.sh` makes a universal (arm64 + x86_64, macOS 11+) `.app`, `.dmg` and `.zip`. `.github/workflows/release.yml` builds every package on GitHub (Linux, Flatpak, Arch, Windows native, macOS) and publishes them on tags `v*`.
- Icons: `--export-brand` writes PNGs at 16, 24, 32, 48, 64, 128, 256 and 512 px and a PNG-based `.ico`; inside Flatpak the window's Wayland app id follows `FLATPAK_ID` so the desktop finds the icon.

## Round 20 — decompiled: keep-alive probes, used on BaseLens connections

Read with native_function_pseudocode:

- **`uv_tcp_keepalive` 0x2a193b0**: with no socket yet (`fd == -1`) it only remembers the choice (flag bit 0x2000000 at handle+0x58); otherwise it calls `uv__tcp_keepalive` and sets the flag on success.
- **`uv__tcp_keepalive` 0x2a19260** (read before, Round 18b): `SO_KEEPALIVE=on`, then refuses a zero delay (`-EINVAL`), `TCP_KEEPIDLE = delay`, **`TCP_KEEPINTVL = 1`**, **`TCP_KEEPCNT = 10`**.
- The original's own Postgres settings (app.asar) never set `keepAlive`, so its node-postgres sockets have no probes; an idle connection behind a proxy or NAT can be dropped silently and the next query hangs until the OS gives up.

Used in the build (`db.rs`): PostgreSQL and MySQL connections now turn keep-alive on with libuv's probe rule — 1 s between probes, 10 probes — after 60 s idle (`TCP_KEEPALIVE_IDLE_S`, a BaseLens choice, since libuv refuses 0). Checked on a live connection to a local PostgreSQL: `ss -tnoe` shows `timer:(keepalive,59sec,0)`.

## Round 21 — icons and the rest in every package

- **Windows**: `BaseLens.exe` carries the icon (16, 24, 32, 48, 64, 256 px), version info and an application manifest (`packaging/windows/baselens.exe.manifest`: per-monitor DPI v2 for sharp text, Common Controls 6, UTF-8 code page, long paths, runs without admin). New installer `BaseLens-<v>-windows-x64-setup.exe` (NSIS, `packaging/windows/installer.nsi`): per-user install in `%LOCALAPPDATA%\Programs\BaseLens`, Start menu and desktop shortcuts with the icon, an entry in Apps with icon and version, and an uninstaller that keeps the person's settings. Checked under Wine: files, shortcut and Apps entry are created.
- **Linux**: `.deb`, `.rpm` and Arch now all ship the icon at 16, 24 (deb/rpm/Arch), 32, 48, 64, 128, 256, 512 px plus SVG, the `.desktop` file and AppStream metadata; AppImage has `.DirIcon` and the full icon set; Flatpak uses `dev.baselens.BaseLens` icons. The AppImage is now written beside and renamed into place, so an open copy can't make the build fail.
- **macOS**: `.icns` built from the same PNGs (unchanged).

## Round 22 — decompiled: how SQLite waits for a busy file

Found by searching the original for SQLite's busy-wait tables (delays at **0x2101a90**: `01 02 05 0a 0f 14 19 19 19 32 32 64`; running totals at **0x2101a9c**: `00 01 03 08 12 21 35 4e 67 80 b2 e4`), both used only by **0x685df20** (`sqliteDefaultBusyCallback`), read with native_function_pseudocode:

- wait number `n` ≤ 11: sleep `delays[n]` ms, having already waited `totals[n]` ms;
- `n` ≥ 12: sleep 100 ms, already waited `n·100 − 872` ms (the constant `0xfffffc98`);
- if `waited + sleep > timeout` the sleep is cut to `timeout − waited`; when nothing is left it gives up (returns 0 → "database is locked");
- the sleep is passed to the VFS as microseconds (`ms · 1000`, `0x3e8`).
- The timeout itself is better-sqlite3's default `timeout: 5000` ms (`app.asar.unpacked/node_modules/better-sqlite3/lib/database.js`).

Used in the build: SQLite connections now set `busy_timeout(5000 ms)` (`SQLITE_BUSY_TIMEOUT_MS`), which uses this same handler, so a file another program is writing is waited for (59 waits adding up to exactly 5000 ms) instead of failing at once with "database is locked". Tests `busy_handler_schedule_matches_the_original` and `sqlite_waits_for_a_busy_file_instead_of_failing` (another connection holds a write for 400 ms; BaseLens's insert waits and succeeds).

## Round 23 — AppImage icon on Wayland

Cause, checked on the person's KDE Plasma (Wayland) with a KWin script: the BaseLens window reports app id `baselens` (`desktopFileName=baselens`). On Wayland a program can't hand its icon to the window (winit 0.30's Wayland `set_window_icon` does nothing); the desktop takes it from a `baselens.desktop` menu entry. A packaged install ships one; an AppImage doesn't, so Plasma showed its generic "W" (or nothing).

Fix (`src/integrate.rs`): when started from an AppImage (`$APPIMAGE` set by the AppImage runtime), BaseLens writes its icons (16–512 px PNG + SVG, `~/.local/share/icons/hicolor/*/apps/baselens.*`) and `~/.local/share/applications/baselens.desktop` (`Exec` = the AppImage's path, `Icon=baselens`, `StartupWMClass=baselens`, marked `X-BaseLens-Integrated=true`), then asks the desktop to refresh its menus (`update-desktop-database`, `kbuildsycoca6`; a GTK icon cache only if one already exists). It never overwrites a `baselens.desktop` it didn't write, updates its own if the AppImage moves, and can be turned off with `BASELENS_NO_INTEGRATION=1`. `--install-desktop-entry` does the same for the .tar.gz; `--remove-desktop-entry` removes only what BaseLens wrote. Test `writes_updates_and_removes_only_its_own_files`; checked with the rebuilt AppImage in a scratch `XDG_DATA_HOME`.

## Round 24 — decompiled: how the original names itself from its menu entry

Found from the string `CHROME_DESKTOP` (0x11c2814), read with native_function_pseudocode:

- **0x2c0cc60**: creates the environment reader and reads `CHROME_DESKTOP` (14 bytes, the `"CHROME_"…` literal).
- **0x2c0a940** (Electron's Linux application name): 1) a name set by the app wins; 2) otherwise, if `$CHROME_DESKTOP` is set, `g_desktop_app_info_new(<it>)` opens that menu entry and `g_desktop_app_info_get_string(…, "Name")` gives the name; 3) otherwise the 8-byte constant `0x6e6f727463656c45` = **"Electron"**. The original's own entry is `beekeeper-studio.desktop` with `StartupWMClass=beekeeper-studio`.

Used in the build (`integrate::app_id`): BaseLens also takes its identity from the menu entry that launched it, so the desktop finds the right icon. `$CHROME_DESKTOP` is inherited by every child process (a terminal inside VS Code carries `code.desktop`), so BaseLens reads GLib's launch variables instead, which carry the launched process id: if `GIO_LAUNCHED_DESKTOP_FILE_PID` is BaseLens's own pid and that entry's `Exec` starts BaseLens, the entry's file name (without `.desktop`) becomes the Wayland app id. This makes entries made by AppImage managers (Gear Lever, AppImageLauncher…) show their icon, and the AppImage then doesn't add a second `baselens.desktop`. Otherwise: `$FLATPAK_ID` in Flatpak, else `baselens` (fallback, like the original's "Electron"). `baselens --app-id` prints the id. Test `app_id_follows_the_entry_that_launched_it`; checked with `WAYLAND_DEBUG=1`: `set_app_id("gearlever_baselens_test")` when launched from such an entry, `set_app_id("baselens")` otherwise.

## Round 25 — security review fixes

1. **TLS auto-upgrade now verifies the certificate.** When a server without "Use TLS/SSL" answers `28000 … no encryption`, BaseLens retries with TLS and **always** checks the certificate and host name (`pg_connect(…, tls, verify)`; before, the retry accepted any certificate because `cfg.ssl` was still false). If the check fails it stops before sending the password and explains how to turn TLS on. Only an explicit "Use TLS/SSL" with "Verify certificate" unticked (or `sslmode=require`, which keeps libpq's meaning: encrypted, not verified) skips the check, and the editor and status bar now say "TLS not verified". Checked live against a PostgreSQL that rejects plain connections with a self-signed certificate: automatic → refused with the explanation; TLS without verify → connects; TLS with verify → refused.
2. **Read-only gate and AI auto-run.** `is_read_query` also treats write/side-effect functions as writes (`WRITE_FUNCTIONS`: `nextval`, `setval`, `lo_import`/`lo_export`/…, `pg_terminate_backend`, `set_config`, `dblink_exec`, `query_to_xml` (runs SQL given as text), MySQL `get_lock`, SQLite `writefile`/`load_extension`…). The AI runs a query without asking only if the connection is **read-only** and `is_safe_auto_query` passes (no `pg_read_file`, `pg_ls_dir`, `pg_sleep`, `dblink`, `lo_get`, `current_setting`, `generate_series`, MySQL `load_file`/`sleep`/`benchmark`, SQLite `readfile`…, see `RISKY_READ_FUNCTIONS`).
3. **MySQL read-only** is labelled "best effort" in the editor and status bar, with advice to use a user with only SELECT rights.
4. **debug.log** is created with mode 0600 in a 0700 folder (and older logs are fixed on the next write).
5. **No keyring ("basic")**: the Security page now says plainly that the key file next to the encrypted settings doesn't protect them from someone who can read the home folder, and how to fix it.
6. **App-lock PIN** is hashed with Argon2id (PHC string, `argon2` defaults: 19 MiB, 2 passes, 1 lane). Old SHA-256 hashes still unlock once and are replaced by Argon2id.

Tests: `read_only_gate_blocks_side_effect_functions`, `pin_hash_is_argon2id_and_old_hashes_still_unlock`, `debug_log_is_private`.

## Round 26 — decompiled: the original's TLS version limits

Read with native_function_pseudocode:

- **`node::crypto::SecureContext::Init` 0x2ed35b0**: refuses `SSLv2_*` and `SSLv3_*` methods outright ("SSLv2/SSLv3 methods disabled"); `TLS_method` starts with min = 0 and max = **0x304 (TLS 1.3)**; the `TLSv1_*`, `TLSv1_1_*`, `TLSv1_2_*` methods pin min and max to 0x301 / 0x302 / 0x303; then `SSL_CTX_set_min_proto_version` (0x6de2ca0) and `SSL_CTX_set_max_proto_version` (0x6de2ce0) are applied, plus 16-byte session-ticket keys.
- **`SecureContext::SetMinProto` 0x2ed4e40**: takes an Int32 from JavaScript and passes it to the same `SSL_CTX_set_min_proto_version`.
- The JavaScript side (Node's `tls.js`, embedded at 0x1b6f6e4; flags parsed by `EnvironmentOptionsParser` at 0x2dcc76d…0x2dcc7d0): with no `--tls-min-v1.x` flag, **`DEFAULT_MIN_VERSION = 'TLSv1.2'`** and `DEFAULT_MAX_VERSION = 'TLSv1.3'`.

Used in the build: PostgreSQL TLS now sets `min_protocol_version(TLS 1.2)` (`TLS_MIN_VERSION`); the MySQL driver has no such option, so after connecting BaseLens reads `SHOW SESSION STATUS LIKE 'Ssl_version'` and refuses anything older than TLS 1.2 (or an unencrypted session when TLS was asked for). A server stuck on old TLS gets a clear message instead of "doesn't accept TLS". Checked live: a PostgreSQL limited to TLS 1.0–1.1 is refused with that message; the same server on its defaults connects. Test `tls_floor_is_tls12`.

## Round 27 — Argon2 as a tool

`src/argon2tool.rs` + window in `dialogs.rs`, opened with **Ctrl+Shift+H**, the command palette ("Argon2: make or check a password hash"), or a cell's right-click menu:

- **Make a hash**: Argon2id (default), Argon2i or Argon2d; memory 8 KiB–1 GiB (default 19 456 KiB), passes 1–16 (2), lanes 1–16 (1), hash length 16–64 bytes (32) — the same defaults as the app-lock PIN (`argon2` crate defaults, OWASP baseline), version 0x13. A new random salt each time; output as a PHC string (`$argon2id$v=19$m=…,t=…,p=…$salt$hash`) with Copy, "Copy as SQL text" and, when opened from an editable cell, **Use in the cell** (puts it in the cell editor; Enter + Save writes it).
- **Check a password**: paste a PHC hash (or right-click a cell holding one, "Check a password against this Argon2 hash…"); the tool shows its settings in plain words (e.g. "Argon2id · version 0x13 · 19 MiB · 2 passes · 1 lane · 32-byte hash") and says whether the password matches. Non-Argon2 hashes (bcrypt…) are refused with a clear message.
- Work runs on a background thread (large memory settings never freeze the window); the typed text is cleared after each run and never saved.

Tests `hashes_check_and_describe`, `refuses_what_is_not_argon2`; robot step 20 opens the tool with Ctrl+Shift+H and checks a real hash.

## Round 28 — decompiled: the original's scrypt, added to the hash tool

The original has no Argon2 (no `argon2` string anywhere), but it ships scrypt as its password-hashing function (Node `crypto.scrypt`). Read with native_function_pseudocode:

- **Defaults** (Node's `lib/internal/crypto/scrypt.js`, embedded at **0x16dc1e2**): `N: 16384, r: 8, p: 1, maxmem: 32 << 20` (32 MiB, "matches SCRYPT_MAX_MEM"); 0 for any of them means the default.
- **`node::crypto::ScryptTraits::AdditionalConfig` 0x2f02270**: refuses passwords and salts of 2^31 bytes or more ("pass is too large" / "salt is too large"), reads N, r, p as Uint32, maxmem as an integer and the key length as Int32 >= 0, then asks BoringSSL whether the settings are valid and throws "Invalid scrypt params[: reason]" otherwise.
- **0x6841de0 → 0x6d81b80** (BoringSSL `EVP_PBE_scrypt`, `crypto/evp/scrypt.cc`): r and p not 0; `p <= (2^30 - 1) / r`; `2 <= N <= 2^32` and N a power of two (`N & (N-1) == 0`); `N >> (16·r) == 0` when `16·r < 64`; memory: with no maxmem the limit is **0x4100000** (65 MiB); it needs `p < maxmem / (128·r)` and `maxmem/(128·r) - p - 1 >= N`, i.e. **128·r·(N + p + 1) bytes <= maxmem** (the work buffer is `(2p + 2N + 2)·r·64` bytes).

Used in the build (`argon2tool.rs`): the hash tool now also makes and checks **scrypt** hashes (PHC form `$scrypt$ln=14,r=8,p=1$salt$hash`) with the original's defaults (N = 2^14, r = 8, p = 1, 32 MiB limit) and the same parameter rules (`scrypt_params_ok`), shown live under the settings ("uses ≈16.0 MiB", or why it's refused — e.g. N = 2^15 with r = 8 needs 32 MiB + 2 KiB, over the 32 MiB limit, as Node refuses it). Checking a scrypt hash applies the same rules before spending the memory. Test `scrypt_follows_the_originals_rules`.

Also fixed in this round (found by the test robot): the suggestion popup could close in the middle of typing when the editor's response missed its focus for a single frame. It now closes only when focus really moved to another widget (`ctx.memory().focused() != editor`), and the robot logs why the popup last closed. Five robot runs in a row under full CPU load: 31/31 checks.


## Round 29 — review fixes: $$ bodies, typed PostgreSQL values, real Stop, SSH tunnels, package channels

- **Statement splitter** (`db.rs::scan_sql`): one pass that knows '…' "…" `…` strings (doubled quote stays inside), PostgreSQL dollar quotes `$$…$$` / `$tag$…$tag$` (tag = letter or `_` then letters/digits/`_`; `$1` is a parameter, not a tag), `--` and `/* */` comments. `split_statements` and the comment stripper both use it, so `CREATE FUNCTION … AS $$ …; … $$` and `DO $$ … $$` stay whole. Security fix found on the way: a `--` or `/*` inside a string used to hide the rest of the line from the read-only check (e.g. `SELECT '--', pg_terminate_backend(1)` passed as a read); now it doesn't.
- **Typed PostgreSQL values** (`pg_cell`): single SELECTs (the cursor path) look up the column types first (one prepared-statement round trip, outside any transaction; skipped inside the person's own transaction or when the statement has `$1` parameters): int2/int4/int8/oid → integers, float4/float8 → floats (NaN/±Infinity too), numeric → float only when it round-trips exactly (else text, no digits lost), bool → true/false, bytea `\x…` → bytes; other types stay text. Saving edits writes PostgreSQL bytes as `'\x…'::bytea` and NaN/Infinity quoted (`sql_literal_for`).
- **Stop** (`Handle::cancel`): a ■ Stop button while a query runs. PostgreSQL sends a cancel request over the same TLS as the connection (`CancelToken`, server error 57014); SQLite uses `sqlite3_interrupt`; MySQL opens a second connection and runs `KILL QUERY <connection id>`. **Time limit**: Settings → General → "Stop queries after (seconds, 0 = never)" uses the same stop. The error says whether the person or the limit stopped it. Checked live: `select pg_sleep(30)` stopped in under 1 s and the connection kept working; a never-ending SQLite recursive query interrupted.
- **SSH tunnels** (`tunnel.rs`): `ssh -N -T -o ExitOnForwardFailure=yes -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30 -o StrictHostKeyChecking=accept-new -L 127.0.0.1:<free port>:<db host>:<db port> -p <port> [-i key -o IdentitiesOnly=yes] -- user@host`; waits up to 20 s for the local end; values that start with `-` or hold spaces are refused (no option injection); `ssh` is stopped when the connection closes; PostgreSQL TLS still checks the certificate against the real host name (`hostaddr` = 127.0.0.1). Checked live through a throwaway OpenSSH server in front of PostgreSQL: query answered by the database behind the bastion; wrong user → "didn't accept the login" with ssh's message; `-oProxyCommand=…` as host → refused before ssh runs.
- **Package channels** (`packaging/aur/baselens-bin`, `packaging/homebrew`, `packaging/winget`, `scripts/update-channels.sh`): checksums filled from the public v0.1.0 release; the AUR package was built from it with makepkg.
