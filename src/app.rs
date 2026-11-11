//! The main window: state and logic. Drawing lives in app/panels.rs and
//! app/views.rs.

mod dialogs;
mod nav;
mod panels;
mod robot;
mod views;

use crate::ai::{self, AiSettings, ChatMsg, Provider, Role};
use crate::db::{self, Cell, ColumnInfo, ConnConfig, DbKind, QueryResult, Request, Response, TableInfo};
use crate::store;
use crate::theme::{self, Palette, ThemeKind};
use crate::assist::{self, SchemaInfo, Suggestion};
use crate::filter::{self, FilterSet};
use crate::finder;
use crate::widgets::{ChartCfg, ErState, GridState};
use eframe::egui::{self, Color32, Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::Instant;

// MECHANICS.md
pub const SIDEBAR_W: f32 = 240.0;
pub const SIDEBAR_MIN_W: f32 = 150.0;
pub const PAGE_SIZE: usize = 100;
pub const MAX_RESULTS: usize = 50000;
pub const MIN_PIN_LEN: usize = 6;
pub const IDLE_SECS: u64 = 300;
pub const RECENT_LIMIT: usize = 10;
/// Below these window widths the JSON inspector / side panel step aside.
pub const NARROW_JSON_HIDE: f32 = 1000.0;
pub const NARROW_SIDEBAR_HIDE: f32 = 860.0;
/// Side panels never take more than this share of the window.
pub const PANEL_MAX_SHARE: f32 = 0.3;

/// Value suggestions: rows sampled, distinct values kept.
pub const VALUES_SAMPLE_ROWS: usize = 10000;
pub const VALUES_TOP: usize = 50;
pub const EDITOR_FONT_SIZE: f32 = 14.0; // renderer: settings.editorFontSize?.value || 14

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeKind,
    pub font_scale: f32,
    pub page_size: usize,
    pub max_results: usize,
    pub ai: AiSettings,
    pub pin_hash: String,
    pub pin_salt: String,
    pub lock_on_idle: bool,
    pub idle_secs: u64,
    pub show_json: bool,
    /// remember names and values from my queries for suggestions
    pub remember_suggestions: bool,
    pub recent: Vec<u64>,
    /// stop a query on the server after this many seconds (0 = never)
    pub query_timeout_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeKind::System, // MECHANICS: linuxDefault 'system'
            font_scale: 1.0,
            page_size: PAGE_SIZE,
            max_results: MAX_RESULTS,
            ai: AiSettings::default(),
            pin_hash: String::new(),
            pin_salt: String::new(),
            lock_on_idle: true,
            idle_secs: IDLE_SECS,
            show_json: true,
            remember_suggestions: true,
            recent: vec![],
            query_timeout_secs: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct HistoryItem {
    pub sql: String,
    pub conn: String,
    pub when: String,
    pub ms: f64,
    pub rows: usize,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SavedQuery {
    pub name: String,
    pub sql: String,
}

#[derive(PartialEq)]
enum ConnStatus {
    Connecting,
    Ready,
    Failed(String),
}

struct OpenConn {
    sid: u64,
    cfg: ConnConfig,
    password: String,
    handle: db::Handle,
    status: ConnStatus,
    server: String,
    databases: Vec<String>,
    current_db: String,
    tables: Vec<TableInfo>,
    columns: HashMap<String, Vec<ColumnInfo>>,
    instance: usize,
    /// The one "all columns" question was sent for this table list.
    cols_asked: bool,
    /// manual transactions (auto-commit off) and whether one is open
    manual_tx: bool,
    tx_open: bool,
}

impl OpenConn {
    fn label(&self) -> String {
        let mut s = self.cfg.name.clone();
        if self.instance > 1 {
            s.push_str(&format!(" #{}", self.instance));
        }
        s
    }
    fn color(&self) -> Color32 {
        Color32::from_rgb(self.cfg.color[0], self.cfg.color[1], self.cfg.color[2])
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum View {
    #[default]
    Grid,
    Chart,
    Json,
}

#[derive(Default)]
struct QueryTab {
    sql: String,
    results: Vec<QueryResult>,
    active: usize,
    error: Option<String>,
    running: bool,
    view: View,
    chart: ChartCfg,
    grid: GridState,
    split: f32,
    cursor: (usize, usize),
    ac: AcState,
    editor_op: Option<EditorOp>,
    /// when the running query started, and whether a stop was asked (by
    /// the person, or by the time limit)
    started: Option<Instant>,
    stop_asked: Option<StopReason>,
    /// visual EXPLAIN shown instead of the results
    plan: Option<Result<crate::plan::PlanNode, String>>,
    /// offline syntax check: when the text last changed, the hash checked,
    /// and the error found (None = looks fine)
    edited: Option<Instant>,
    checked: u64,
    syntax: Option<crate::sql::SqlError>,
    /// sqruff lint shown instead of the results, and one being computed
    lint: Option<Result<Vec<crate::sql::Violation>, String>>,
    lint_rx: Option<std::sync::mpsc::Receiver<Result<Vec<crate::sql::Violation>, String>>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum StopReason {
    Person,
    TimeLimit(u64),
}

/// Autocomplete popup state.
#[derive(Default)]
struct AcState {
    open: bool,
    forced: bool,
    sel: usize,
    start: usize,
    items: Vec<Suggestion>,
    last_text_len: usize,
    /// why the popup last closed (shown by the test robot)
    closed_by: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
enum EditorOp {
    Format,
    Comment,
    Complete,
    Lint,
}

#[derive(Default)]
struct TableTab {
    schema: Option<String>,
    table: String,
    is_view: bool,
    cols: Option<Vec<ColumnInfo>>,
    data: Option<QueryResult>,
    page: usize,
    total: Option<u64>,
    filter: FilterSet,
    search_changed: Option<Instant>,
    focus_search: bool,
    show_rules: bool,
    sort: Option<(usize, bool)>,
    edits: HashMap<(usize, usize), Cell>,
    deletes: HashSet<usize>,
    grid: GridState,
    error: Option<String>,
    loading: bool,
    view: View,
    chart: ChartCfg,
    notice: Option<String>,
    /// Properties panel: shown instead of the data, its answer, the open
    /// sub-tab and the request it waits for
    props_open: bool,
    props: Option<crate::props::TableProps>,
    props_tab: usize,
    props_tag: u64,
    props_grid: GridState,
}

#[derive(Default)]
struct AiTab {
    msgs: Vec<ChatMsg>,
    input: String,
    busy: bool,
    rx: Option<Receiver<Result<String, String>>>,
    runs: HashMap<usize, Result<QueryResult, String>>,
    confirm: Option<(usize, String)>,
}

enum TabKind {
    Query(QueryTab),
    Table(TableTab),
    Ai(AiTab),
    Er(ErState),
}

struct Tab {
    id: u64,
    conn: u64,
    title: String,
    kind: TabKind,
}

#[derive(Clone, Copy)]
enum Purpose {
    Query(u64),
    TableData(u64),
    TableCount(u64),
    TableSave(u64),
    AiRun(u64, usize),
    Insert(u64),
    /// ask bot data search: (message index, job index)
    AskData(usize, usize),
    /// value sampling for suggestions: index into `values_jobs`
    Values(usize),
    /// visual EXPLAIN for a query tab
    Explain(u64),
    /// COMMIT / ROLLBACK from the buttons (true = commit)
    EndTx(bool),
}

struct ConnEditor {
    cfg: ConnConfig,
    password: String,
    is_new: bool,
    test: Option<db::Handle>,
    test_msg: Option<(bool, String)>,
}

struct NewRow {
    tab: u64,
    cols: Vec<ColumnInfo>,
    values: Vec<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsPage {
    General,
    Ai,
    Security,
}

struct SettingsUi {
    page: SettingsPage,
    keys: HashMap<&'static str, String>,
    ollama_models: Vec<String>,
    msg: String,
    pin1: String,
    pin2: String,
    claude_ver: String,
}

/// Sections of the side panel, picked from the icon rail.
#[derive(PartialEq, Clone, Copy)]
enum SideTab {
    Connections,
    Tables,
    Ask,
    Saved,
    History,
}

#[derive(PartialEq, Clone, Copy, Default)]
enum AskScope {
    /// the table open in the active tab
    #[default]
    ThisTable,
    /// every table of the active connection
    Database,
    /// every open connection
    Everything,
}

#[derive(Default)]
struct AskMsg {
    user: bool,
    text: String,
    hits: Vec<finder::Hit>,
    value: Option<String>,
    lang: Option<finder::Lang>,
    /// tables where the value was found inside the data: (sid, table, rows)
    data: Vec<(u64, TableInfo, i64)>,
    pending: usize,
    searched: bool,
}

#[derive(Default)]
struct AskState {
    msgs: Vec<AskMsg>,
    input: String,
    scope: AskScope,
    /// data-search jobs: (connection, table)
    jobs: Vec<(u64, TableInfo)>,
    focus_input: bool,
}

#[derive(Clone)]
enum QuickItem {
    Command(Cmd),
    Table(u64, TableInfo, String),
    Conn(ConnConfig),
    Saved(String, String),
}

/// Everything the command palette (Ctrl+K) can run, with its shortcut.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Cmd {
    NewQuery,
    NewAi,
    NewEr,
    NewConnection,
    RunQuery,
    RunCurrent,
    FormatSql,
    LintSql,
    ToggleComment,
    Autocomplete,
    SaveChanges,
    Refresh,
    FocusSearch,
    AddFilter,
    ClearFilters,
    ExportCsv,
    ExportJson,
    CloseTab,
    ReopenTab,
    NextTab,
    PrevTab,
    ToggleSidebar,
    ToggleJson,
    ToggleTheme,
    ShowConnections,
    ShowTables,
    ShowSaved,
    ShowHistory,
    Settings,
    Shortcuts,
    Lock,
    Ask,
    Argon2,
}

impl Cmd {
    const ALL: [Cmd; 33] = [
        Cmd::LintSql,
        Cmd::Ask,
        Cmd::Argon2,
        Cmd::NewQuery,
        Cmd::RunQuery,
        Cmd::RunCurrent,
        Cmd::FormatSql,
        Cmd::ToggleComment,
        Cmd::Autocomplete,
        Cmd::FocusSearch,
        Cmd::AddFilter,
        Cmd::ClearFilters,
        Cmd::SaveChanges,
        Cmd::Refresh,
        Cmd::NewAi,
        Cmd::NewEr,
        Cmd::NewConnection,
        Cmd::ExportCsv,
        Cmd::ExportJson,
        Cmd::CloseTab,
        Cmd::ReopenTab,
        Cmd::NextTab,
        Cmd::PrevTab,
        Cmd::ToggleSidebar,
        Cmd::ToggleJson,
        Cmd::ToggleTheme,
        Cmd::ShowConnections,
        Cmd::ShowTables,
        Cmd::ShowSaved,
        Cmd::ShowHistory,
        Cmd::Settings,
        Cmd::Shortcuts,
        Cmd::Lock,
    ];
    fn label(self) -> &'static str {
        match self {
            Cmd::NewQuery => "New query tab",
            Cmd::NewAi => "New AI assistant tab",
            Cmd::NewEr => "Open relationship diagram",
            Cmd::NewConnection => "New connection",
            Cmd::RunQuery => "Run query (all or selection)",
            Cmd::RunCurrent => "Run statement under cursor",
            Cmd::FormatSql => "Format SQL",
            Cmd::LintSql => "Lint SQL (sqruff)",
            Cmd::ToggleComment => "Comment / uncomment line",
            Cmd::Autocomplete => "Show suggestions",
            Cmd::SaveChanges => "Save table changes",
            Cmd::Refresh => "Refresh data",
            Cmd::FocusSearch => "Search in table",
            Cmd::AddFilter => "Add a filter",
            Cmd::ClearFilters => "Clear all filters",
            Cmd::ExportCsv => "Export result as CSV",
            Cmd::ExportJson => "Export result as JSON",
            Cmd::CloseTab => "Close tab",
            Cmd::ReopenTab => "Reopen closed tab",
            Cmd::NextTab => "Next tab",
            Cmd::PrevTab => "Previous tab",
            Cmd::ToggleSidebar => "Show / hide side panel",
            Cmd::ToggleJson => "Show / hide JSON inspector",
            Cmd::ToggleTheme => "Switch light / dark",
            Cmd::ShowConnections => "Go to connections",
            Cmd::ShowTables => "Go to tables",
            Cmd::ShowSaved => "Go to saved queries",
            Cmd::ShowHistory => "Go to history",
            Cmd::Settings => "Settings",
            Cmd::Shortcuts => "Keyboard shortcuts",
            Cmd::Lock => "Lock the app",
            Cmd::Ask => "Ask Lens: is it in this table?",
            Cmd::Argon2 => "Argon2 / scrypt: make or check a password hash",
        }
    }
    /// Shortcut text for this platform. Rule from the original's accelerator
    /// key table (0xbb7cf68.., filled by relocations at 0x957290..):
    /// "cmdorctrl" / "commandorcontrol" map to key code 0x11 (Control) on
    /// Linux, while "cmd" / "command" map to 0x5B (the Super key). So the
    /// app's "ctrlOrCmd" shortcuts are Ctrl here and ⌘ only on macOS.
    fn keys_label(self) -> String {
        let k = self.keys();
        if cfg!(target_os = "macos") {
            k.replace("Ctrl", "⌘")
        } else {
            k.to_string()
        }
    }

    fn keys(self) -> &'static str {
        match self {
            Cmd::NewQuery => "Ctrl+T",
            Cmd::NewAi => "Ctrl+L",
            Cmd::NewEr => "Ctrl+Shift+E",
            Cmd::NewConnection => "Ctrl+Shift+N",
            Cmd::RunQuery => "Ctrl+Enter · F5",
            Cmd::RunCurrent => "Ctrl+Shift+Enter",
            Cmd::FormatSql => "Ctrl+Shift+F",
            Cmd::LintSql => "Ctrl+Shift+I",
            Cmd::ToggleComment => "Ctrl+/",
            Cmd::Autocomplete => "Ctrl+Space",
            Cmd::SaveChanges => "Ctrl+S",
            Cmd::Refresh => "Ctrl+R · F5",
            Cmd::FocusSearch => "Ctrl+F",
            Cmd::AddFilter => "Ctrl+Shift+A",
            Cmd::ClearFilters => "Ctrl+Shift+X",
            Cmd::ExportCsv => "Ctrl+E",
            Cmd::ExportJson => "Ctrl+Shift+J",
            Cmd::CloseTab => "Ctrl+W",
            Cmd::ReopenTab => "Ctrl+Shift+T",
            Cmd::NextTab => "Ctrl+Tab",
            Cmd::PrevTab => "Ctrl+Shift+Tab",
            Cmd::ToggleSidebar => "Ctrl+B",
            Cmd::ToggleJson => "Ctrl+J",
            Cmd::ToggleTheme => "Ctrl+Shift+L",
            Cmd::ShowConnections => "Alt+Shift+1",
            Cmd::ShowTables => "Alt+Shift+2",
            Cmd::ShowSaved => "Alt+Shift+3",
            Cmd::ShowHistory => "Alt+Shift+4",
            Cmd::Settings => "Ctrl+,",
            Cmd::Shortcuts => "F1",
            Cmd::Lock => "Ctrl+Shift+Q",
            Cmd::Ask => "Ctrl+Shift+K",
            Cmd::Argon2 => "Ctrl+Shift+H",
        }
    }
}

enum Action {
    Run(bool),
    Reload,
    Save,
    SaveQuery(String),
    AiSend,
    AiRun(usize, String),
    OpenInEditor(String),
    ShowJson,
    Export(crate::export::Format),
    AddRow,
    ReloadEr,
    /// sample the distinct values of (table, column) for suggestions
    FetchValues(String, String),
    Settings,
    /// stop the query running in this tab
    StopQuery,
    /// show the query plan (true = ANALYZE: runs the query)
    Explain(bool),
    /// turn manual transactions on/off for this tab's connection
    SetManualCommit(bool),
    /// end the open transaction: true = COMMIT, false = ROLLBACK
    EndTx(bool),
    /// load (or reload) the Properties panel of this table tab
    LoadProps,
    /// open the Argon2 tool: check this hash, or fill this cell (row, column)
    Argon2 { check: Option<String>, cell: Option<(usize, usize)> },
}

pub struct App {
    settings: Settings,
    pal: Palette,
    applied_theme: Option<(ThemeKind, f32)>,
    system_dark: bool,
    theme_rx: Option<Receiver<bool>>,
    theme_checked: Instant,
    connections: Vec<ConnConfig>,
    history: Vec<HistoryItem>,
    saved: Vec<SavedQuery>,
    open: Vec<OpenConn>,
    tabs: Vec<Tab>,
    active_tab: Option<u64>,
    active_conn: Option<u64>,
    closed_tabs: Vec<(u64, String, String)>,
    next_id: u64,
    pending: HashMap<u64, Purpose>,
    /// Debug mode: the panel (F12), traces and the SQL of queries in flight.
    pub(crate) dbg: crate::debug::State,
    dbg_sql: HashMap<u64, String>,
    editor: Option<ConnEditor>,
    new_row: Option<NewRow>,
    settings_ui: Option<SettingsUi>,
    side_tab: SideTab,
    sidebar_open: bool,
    /// the user opened the side panel while the window was narrow
    sidebar_pinned_narrow: bool,
    expanded: HashSet<String>,
    ask: AskState,
    /// screen rectangles of some controls, for the self-test robot
    test_rects: HashMap<String, egui::Rect>,
    robot: Option<robot::Robot>,
    robot_ctx: Option<egui::Context>,
    /// connection editor: step-by-step check and the "show password" eye
    diag_rx: Option<Receiver<Vec<db::DiagLine>>>,
    diag_lines: Vec<db::DiagLine>,
    show_pw: bool,
    /// sampled distinct values: (connection, "table.column") -> (value, rows)
    values_cache: HashMap<(u64, String), Vec<(String, i64)>>,
    values_pending: HashSet<(u64, String)>,
    values_jobs: Vec<(u64, String)>,
    /// what the user wrote in queries, per saved connection id
    learned: HashMap<u64, assist::Learned>,
    show_shortcuts: bool,
    /// Argon2 hash tool window (Ctrl+Shift+H)
    pub(crate) argon2: Option<crate::argon2tool::Tool>,
    table_filter: String,
    json_filter: String,
    quick: Option<(String, usize)>,
    save_query: Option<(u64, String)>,
    toast: Option<(String, Instant, bool)>,
    locked: bool,
    pin_input: String,
    pin_error: bool,
    last_activity: Instant,
    screenshot: Option<std::path::PathBuf>,
    frame: u64,
    shot_requested: bool,
    demo_setup: u8,
    demo_tab: u64,
}

/// Learned suggestions are stored encrypted, like passwords: they can hold
/// real values from the user's data.
fn load_learned() -> HashMap<u64, assist::Learned> {
    let enc: String = store::load("learned.json");
    if enc.is_empty() {
        return HashMap::new();
    }
    serde_json::from_str(&store::decrypt(&enc)).unwrap_or_default()
}

fn save_learned(l: &HashMap<u64, assist::Learned>) {
    if let Ok(js) = serde_json::to_string(l) {
        store::save("learned.json", &store::encrypt(&js));
    }
}

pub(crate) fn now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, (rem % 3600) / 60)
}

const SWATCHES: [[u8; 3]; 8] = [
    [139, 123, 255],
    [56, 214, 196],
    [90, 169, 255],
    [244, 114, 182],
    [245, 165, 36],
    [52, 211, 153],
    [244, 63, 94],
    [148, 156, 178],
];

fn short_path(s: &str) -> String {
    if s.chars().count() > 34 {
        let tail: String = s.chars().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    } else {
        s.to_string()
    }
}

/// Statement that contains char index `at`.
fn statement_at(sql: &str, at: usize) -> String {
    let mut start = 0;
    let mut quote: Option<char> = None;
    let chars: Vec<char> = sql.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if let Some(q) = quote {
            if *c == q {
                quote = None;
            }
            continue;
        }
        if *c == '\'' || *c == '"' || *c == '`' {
            quote = Some(*c);
        } else if *c == ';' {
            if at <= i {
                return chars[start..i].iter().collect::<String>().trim().to_string();
            }
            start = i + 1;
        }
    }
    let rest: String = chars[start.min(chars.len())..].iter().collect::<String>().trim().to_string();
    if rest.is_empty() {
        db::split_statements(sql).pop().unwrap_or_default()
    } else {
        rest
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext, screenshot: Option<std::path::PathBuf>) -> Self {
        let settings: Settings = if screenshot.is_some() { Settings::default() } else { store::load("settings.json") };
        let mut connections: Vec<ConnConfig> = store::load("connections.json");
        // the first version used the original's yellow for the demo: move to our palette
        for c in &mut connections {
            if c.color == [250, 216, 59] {
                c.color = SWATCHES[0];
            }
        }
        let demo_path = store::ensure_demo_db();
        if !connections.iter().any(|c| c.kind == DbKind::Sqlite && c.path == demo_path.to_string_lossy()) {
            connections.insert(
                0,
                ConnConfig {
                    id: 1,
                    name: "Demo (SQLite)".into(),
                    kind: DbKind::Sqlite,
                    path: demo_path.to_string_lossy().to_string(),
                    color: SWATCHES[0],
                    ..Default::default()
                },
            );
        }
        let pal = theme::palette(settings.theme);
        let locked = screenshot.is_none() && !settings.pin_hash.is_empty();
        let mut app = App {
            pal,
            applied_theme: None,
            system_dark: theme::system_prefers_dark(),
            theme_rx: None,
            theme_checked: Instant::now(),
            history: store::load("history.json"),
            saved: store::load("saved_queries.json"),
            next_id: connections.iter().map(|c| c.id).max().unwrap_or(1) + 100,
            connections,
            settings,
            open: vec![],
            tabs: vec![],
            active_tab: None,
            active_conn: None,
            closed_tabs: vec![],
            pending: HashMap::new(),
            dbg: crate::debug::State { show: crate::debug::on(), ..Default::default() },
            dbg_sql: HashMap::new(),
            editor: None,
            new_row: None,
            settings_ui: None,
            side_tab: SideTab::Connections,
            sidebar_open: true,
            sidebar_pinned_narrow: false,
            expanded: HashSet::new(),
            ask: AskState::default(),
            test_rects: HashMap::new(),
            robot_ctx: None,
            diag_rx: None,
            diag_lines: Vec::new(),
            show_pw: false,
            robot: match std::env::var("BASELENS_ROBOT").as_deref() {
                Ok("selftest") => Some(robot::Robot::new(robot::RobotKind::SelfTest)),
                Ok(m) if m.starts_with("monkey:") => Some(robot::Robot::new(robot::RobotKind::Monkey(m[7..].parse().unwrap_or(3000)))),
                _ => None,
            },
            values_cache: HashMap::new(),
            values_pending: HashSet::new(),
            values_jobs: Vec::new(),
            learned: load_learned(),
            show_shortcuts: false,
            argon2: None,
            table_filter: String::new(),
            json_filter: String::new(),
            quick: None,
            save_query: None,
            toast: None,
            locked,
            pin_input: String::new(),
            pin_error: false,
            last_activity: Instant::now(),
            screenshot,
            frame: 0,
            shot_requested: false,
            demo_setup: 0,
            demo_tab: 0,
        };
        if app.settings.ai.claude_path.is_empty() {
            if let Some(p) = ai::detect_claude() {
                app.settings.ai.claude_path = p;
            }
        }
        if app.screenshot.is_some() && std::env::var("FAVO_SCENE").map(|v| v != "welcome").unwrap_or(true) {
            let cfg = app.connections[0].clone();
            app.connect(cfg, false);
            app.demo_setup = 1;
        }
        theme::install_fonts(&cc.egui_ctx);
        app.apply_theme(&cc.egui_ctx);
        app
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn current_theme(&self) -> ThemeKind {
        self.settings.theme.resolve(self.system_dark)
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        // follow desktop light/dark changes (original listens to the portal's
        // SettingChanged signal; we re-read it every 30 s, the idle-check interval)
        if let Some(rx) = &self.theme_rx {
            if let Ok(d) = rx.try_recv() {
                self.system_dark = d;
                self.theme_rx = None;
            }
        }
        if self.settings.theme == ThemeKind::System && self.theme_rx.is_none() && self.theme_checked.elapsed().as_secs() >= 30 {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(theme::system_prefers_dark());
            });
            self.theme_rx = Some(rx);
            self.theme_checked = Instant::now();
        }
        let want = (self.current_theme(), self.settings.font_scale);
        if self.applied_theme != Some(want) {
            self.pal = theme::palette(want.0);
            theme::apply(ctx, &self.pal, self.settings.font_scale);
            self.applied_theme = Some(want);
        }
    }

    fn persist(&self) {
        if self.screenshot.is_some() {
            return;
        }
        store::save("settings.json", &self.settings);
        store::save("connections.json", &self.connections);
    }

    fn toast(&mut self, msg: impl Into<String>, ok: bool) {
        self.toast = Some((msg.into(), Instant::now(), ok));
    }

    // ------------------------------------------------------------ connections

    fn connect(&mut self, cfg: ConnConfig, force_new: bool) {
        if !force_new {
            if let Some(o) = self.open.iter().find(|o| o.cfg.id == cfg.id) {
                let sid = o.sid;
                self.active_conn = Some(sid);
                if let Some(t) = self.tabs.iter().find(|t| t.conn == sid) {
                    self.active_tab = Some(t.id);
                }
                return;
            }
        }
        // a saved password that can't be unlocked must not silently become
        // an empty one: ask for it instead
        let Some(password) = store::decrypt_checked(&cfg.password_enc) else {
            self.toast("The saved password couldn't be unlocked. Please type it again.", false);
            self.editor = Some(ConnEditor {
                cfg,
                password: String::new(),
                is_new: false,
                test: None,
                test_msg: Some((false, "The saved password couldn't be unlocked (the system keyring didn't give back the key). Type the password again and save.".into())),
            });
            return;
        };
        self.spawn_conn(cfg, password);
    }

    fn spawn_conn(&mut self, cfg: ConnConfig, password: String) -> u64 {
        let sid = self.id();
        let instance = self.open.iter().filter(|o| o.cfg.id == cfg.id).count() + 1;
        let handle = db::spawn(cfg.clone(), password.clone());
        self.settings.recent.retain(|x| *x != cfg.id);
        self.settings.recent.insert(0, cfg.id);
        self.settings.recent.truncate(RECENT_LIMIT);
        self.open.push(OpenConn {
            sid,
            cfg,
            password,
            handle,
            status: ConnStatus::Connecting,
            server: String::new(),
            databases: vec![],
            current_db: String::new(),
            tables: vec![],
            columns: HashMap::new(),
            instance,
            cols_asked: false,
            manual_tx: false,
            tx_open: false,
        });
        self.active_conn = Some(sid);
        sid
    }

    fn conn(&self, sid: u64) -> Option<&OpenConn> {
        self.open.iter().find(|o| o.sid == sid)
    }

    fn switch_database(&mut self, sid: u64, dbname: String) {
        let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) else { return };
        let mut cfg = o.cfg.clone();
        cfg.database = dbname;
        o.handle = db::spawn(cfg.clone(), o.password.clone());
        o.cfg = cfg;
        o.status = ConnStatus::Connecting;
        o.tables.clear();
        o.columns.clear();
        o.cols_asked = false;
        o.manual_tx = false;
        o.tx_open = false;
    }

    fn close_conn(&mut self, sid: u64) {
        self.open.retain(|o| o.sid != sid);
        self.tabs.retain(|t| t.conn != sid);
        if self.active_conn == Some(sid) {
            self.active_conn = self.open.first().map(|o| o.sid);
        }
        if !self.tabs.iter().any(|t| Some(t.id) == self.active_tab) {
            self.active_tab = self.tabs.last().map(|t| t.id);
        }
    }

    fn send(&mut self, sid: u64, sql: String, purpose: Purpose, max_rows: usize) -> Result<(), String> {
        let tag = self.id();
        let Some(o) = self.open.iter().find(|o| o.sid == sid) else {
            return Err("Connection is closed.".into());
        };
        if o.cfg.read_only && !db::is_read_query(&sql) {
            return Err("This connection is read-only: only queries that read data are allowed.".into());
        }
        if crate::debug::on() {
            self.dbg_sql.insert(tag, sql.clone());
        }
        let _ = o.handle.send(Request::Query { tag, sql, max_rows, sent: Instant::now() });
        self.pending.insert(tag, purpose);
        Ok(())
    }

    fn poll(&mut self) {
        let mut events = Vec::new();
        for o in &mut self.open {
            while let Ok(r) = o.handle.rx.try_recv() {
                events.push((o.sid, r));
            }
        }
        for (sid, r) in events {
            self.on_response(sid, r);
        }
        // AI answers
        let mut auto_runs = Vec::new();
        let auto = self.settings.ai.allow_read_only_auto;
        let conns_ro: HashMap<u64, bool> = self.open.iter().map(|o| (o.sid, o.cfg.read_only)).collect();
        for t in &mut self.tabs {
            if let TabKind::Ai(a) = &mut t.kind {
                let got = a.rx.as_ref().and_then(|rx| rx.try_recv().ok());
                if let Some(res) = got {
                    a.busy = false;
                    a.rx = None;
                    match res {
                        Ok(text) => {
                            let blocks = ai::code_blocks(&text);
                            a.msgs.push(ChatMsg { role: Role::Assistant, text });
                            let idx = a.msgs.len() - 1;
                            if auto {
                                if let Some(b) = blocks.first() {
                                    // only on a connection the server keeps
                                    // read-only, and never file/sleep/network
                                    // functions (security review #2)
                                    let ro = conns_ro.get(&t.conn).copied().unwrap_or(false);
                                    if ro && db::is_safe_auto_query(b) {
                                        auto_runs.push((t.id, t.conn, idx, b.clone()));
                                    }
                                }
                            }
                        }
                        Err(e) => a.msgs.push(ChatMsg { role: Role::Note, text: format!("⚠ {e}") }),
                    }
                }
            }
        }
        for (tid, sid, idx, sql) in auto_runs {
            let _ = self.send(sid, sql, Purpose::AiRun(tid, idx), 200);
        }
        // connection test in the editor
        if let Some(ed) = &mut self.editor {
            let got = ed.test.as_ref().and_then(|h| h.rx.try_recv().ok());
            if let Some(r) = got {
                ed.test_msg = Some(match r {
                    Response::Connected { server, .. } => (true, format!("Connected: {server}")),
                    Response::Failed(e) => (false, e),
                    _ => (true, "Connected".into()),
                });
                ed.test = None;
            }
        }
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn on_response(&mut self, sid: u64, r: Response) {
        match r {
            Response::Connected { server, databases, current_db, timing } => {
                if crate::debug::on() {
                    let name = self.conn(sid).map(|o| o.label()).unwrap_or_default();
                    self.dbg.connect(format!(
                        "connect [{name}] DNS {:.1} ms · TCP {:.1} ms · TLS + login {:.1} ms · total {:.1} ms · {server}",
                        timing.dns_ms, timing.tcp_ms, timing.tls_login_ms, timing.total_ms
                    ));
                }
                if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                    o.status = ConnStatus::Ready;
                    o.server = server;
                    o.databases = databases;
                    o.current_db = current_db;
                }
                if !self.tabs.iter().any(|t| t.conn == sid) {
                    self.new_query_tab(sid, String::new());
                }
            }
            Response::Failed(e) => {
                if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                    o.status = ConnStatus::Failed(e.clone());
                }
                self.toast(format!("Connection failed: {e}"), false);
            }
            Response::Tables(t) => {
                if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                    // load column details in the background (they feed
                    // autocomplete, the explorer tree and the AI prompt): one
                    // question for all tables, on the second connection, so a
                    // query never waits behind it
                    let missing = t.iter().any(|tb| !o.columns.contains_key(&tb.key()));
                    let changed = t.len() != o.tables.len() || t.iter().zip(&o.tables).any(|(a, b)| a.key() != b.key());
                    if missing && (!o.cols_asked || changed) {
                        o.cols_asked = true;
                        let _ = o.handle.send(Request::AllColumns);
                    }
                    o.tables = t;
                }
                if self.side_tab == SideTab::Connections && self.active_conn == Some(sid) {
                    self.side_tab = SideTab::Tables;
                }
            }
            Response::Columns { table, cols } => {
                if let (Some(o), Ok(c)) = (self.open.iter_mut().find(|o| o.sid == sid), &cols) {
                    o.columns.insert(table.clone(), c.clone());
                }
                let mut reload = vec![];
                for t in &mut self.tabs {
                    if t.conn != sid {
                        continue;
                    }
                    if let TabKind::Table(tt) = &mut t.kind {
                        let key = match &tt.schema {
                            Some(s) => format!("{s}.{}", tt.table),
                            None => tt.table.clone(),
                        };
                        if key == table && tt.cols.is_none() {
                            match &cols {
                                Ok(c) => {
                                    tt.cols = Some(c.clone());
                                    reload.push(t.id);
                                }
                                Err(e) => tt.error = Some(e.clone()),
                            }
                        }
                    }
                }
                for id in reload {
                    self.load_table(id);
                }
            }
            Response::Properties { tag, props } => {
                for t in &mut self.tabs {
                    if let TabKind::Table(tt) = &mut t.kind {
                        if tt.props_tag == tag {
                            tt.props = Some(props.clone());
                            tt.props_tag = 0;
                        }
                    }
                }
            }
            Response::TxState { manual, open, error } => {
                if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                    o.manual_tx = manual;
                    o.tx_open = open;
                }
                match error {
                    Some(e) => self.toast(e, false),
                    None => self.toast(if manual { "Manual transactions: changes wait for Commit or Rollback." } else { "Auto-commit is back on." }, true),
                }
            }
            Response::AllColumns(res) => match res {
                Ok(all) => {
                    let keys: std::collections::HashSet<String> = all.iter().map(|(k, _)| k.clone()).collect();
                    let waiting: Vec<String> = self
                        .tabs
                        .iter()
                        .filter(|t| t.conn == sid)
                        .filter_map(|t| match &t.kind {
                            TabKind::Table(tt) if tt.cols.is_none() => Some(match &tt.schema {
                                Some(s) => format!("{s}.{}", tt.table),
                                None => tt.table.clone(),
                            }),
                            _ => None,
                        })
                        .filter(|k| keys.contains(k))
                        .collect();
                    let mut hand: Vec<(String, Vec<ColumnInfo>)> = Vec::new();
                    if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                        for (k, c) in all {
                            if waiting.contains(&k) {
                                hand.push((k.clone(), c.clone()));
                            }
                            o.columns.insert(k, c);
                        }
                    }
                    for (table, cols) in hand {
                        self.on_response(sid, Response::Columns { table, cols: Ok(cols) });
                    }
                }
                Err(e) => crate::debug::log(&format!("all columns failed: {e}")),
            },
            Response::Graph { tag: _, tables, fks } => {
                for t in &mut self.tabs {
                    if t.conn == sid {
                        if let TabKind::Er(er) = &mut t.kind {
                            if !er.loaded {
                                er.set(tables.clone(), fks.clone());
                            }
                        }
                    }
                }
            }
            Response::QueryDone { tag, results, timing } => {
                let sql = self.dbg_sql.remove(&tag);
                if let Some(o) = self.open.iter_mut().find(|o| o.sid == sid) {
                    o.tx_open = timing.in_tx;
                }
                let Some(purpose) = self.pending.remove(&tag) else {
                    if let Err(e) = results {
                        self.toast(e, false);
                    }
                    return;
                };
                let error = results.is_err();
                let t = Instant::now();
                self.on_query_done(sid, purpose, results);
                if crate::debug::on() {
                    let conn = self.conn(sid).map(|o| o.label()).unwrap_or_default();
                    self.dbg.push(crate::debug::Trace {
                        when: now_string(),
                        conn,
                        sql: sql.unwrap_or_default(),
                        queue_ms: timing.queue_ms,
                        queue_behind: timing.queue_behind,
                        run_ms: timing.run_ms,
                        rows: timing.rows,
                        bytes: timing.bytes,
                        batches: timing.batches,
                        build_ms: t.elapsed().as_secs_f64() * 1000.0,
                        draw_ms: None,
                        error,
                    });
                }
            }
        }
    }

    fn on_query_done(&mut self, sid: u64, purpose: Purpose, results: Result<Vec<QueryResult>, String>) {
        let conn_name = self.conn(sid).map(|o| o.label()).unwrap_or_default();
        match purpose {
            Purpose::EndTx(commit) => match results {
                Ok(_) => self.toast(if commit { "Committed." } else { "Rolled back." }, true),
                Err(e) => self.toast(format!("{} failed: {e}", if commit { "Commit" } else { "Rollback" }), false),
            },
            Purpose::Explain(tid) => {
                let kind = self.conn(sid).map(|o| o.cfg.kind).unwrap_or(db::DbKind::Sqlite);
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Query(q) = &mut t.kind {
                        q.running = false;
                        q.started = None;
                        q.stop_asked = None;
                        q.plan = Some(match results {
                            Ok(r) => match r.iter().rev().find(|x| !x.columns.is_empty()) {
                                Some(x) => crate::plan::parse(kind, x),
                                None => Err("The database returned no plan.".into()),
                            },
                            Err(e) => Err(e),
                        });
                    }
                }
            }
            Purpose::Query(tid) => {
                let mut hist = None;
                let mut learn: Option<(String, Vec<String>)> = None;
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Query(q) = &mut t.kind {
                        q.running = false;
                        q.started = None;
                        let stopped = q.stop_asked.take();
                        match results {
                            Ok(r) => {
                                let rows: usize = r.iter().map(|x| x.rows.len()).sum();
                                let ms: f64 = r.iter().map(|x| x.elapsed_ms).sum();
                                hist = Some(HistoryItem { sql: q.sql.clone(), conn: conn_name, when: now_string(), ms, rows });
                                learn = Some((q.sql.clone(), r.iter().flat_map(|x| x.columns.clone()).collect::<Vec<String>>()));
                                q.active = r.iter().rposition(|x| !x.columns.is_empty()).unwrap_or(r.len().saturating_sub(1));
                                q.results = r;
                                q.error = None;
                                q.grid = GridState::default();
                            }
                            Err(e) => {
                                q.error = Some(match stopped {
                                    Some(StopReason::Person) => format!("Query stopped.\n\n{e}"),
                                    Some(StopReason::TimeLimit(s)) => format!("Query stopped after the {s}-second limit (Settings → General).\n\n{e}"),
                                    None => e,
                                })
                            }
                        }
                    }
                }
                if let (Some((sql, cols)), true) = (learn, self.settings.remember_suggestions && self.screenshot.is_none()) {
                    let schema = self.schema_info(sid);
                    if let Some(cid) = self.conn(sid).map(|o| o.cfg.id) {
                        self.learned.entry(cid).or_default().learn(&sql, &cols, &schema);
                        save_learned(&self.learned);
                    }
                }
                if let Some(h) = hist {
                    self.history.insert(0, h);
                    self.history.truncate(500);
                    if self.screenshot.is_none() {
                        store::save("history.json", &self.history);
                    }
                }
                // refresh the table list, but not inside an open transaction
                // (an error there would abort it)
                if let Some(o) = self.conn(sid).filter(|o| !o.tx_open) {
                    let _ = o.handle.send(Request::Refresh);
                }
            }
            Purpose::TableData(tid) => {
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Table(tt) = &mut t.kind {
                        tt.loading = false;
                        match results {
                            Ok(mut r) if !r.is_empty() => {
                                tt.data = Some(r.remove(0));
                                tt.error = None;
                                tt.edits.clear();
                                tt.deletes.clear();
                            }
                            Ok(_) => {}
                            Err(e) => tt.error = Some(e),
                        }
                    }
                }
            }
            Purpose::TableCount(tid) => {
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Table(tt) = &mut t.kind {
                        tt.total = results
                            .ok()
                            .and_then(|r| r.first().and_then(|x| x.rows.first().and_then(|row| row[0].as_f64())))
                            .map(|v| v as u64);
                    }
                }
            }
            Purpose::TableSave(tid) => {
                let ok = results.is_ok();
                let msg = match &results {
                    Ok(r) => format!("Saved {} change(s).", r.len()),
                    Err(e) => format!("Save failed: {e}"),
                };
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Table(tt) = &mut t.kind {
                        tt.notice = Some(msg.clone());
                    }
                }
                self.toast(msg, ok);
                if ok {
                    self.load_table(tid);
                }
            }
            Purpose::Insert(tid) => {
                let ok = results.is_ok();
                let msg = match &results {
                    Ok(_) => "Row added.".to_string(),
                    Err(e) => format!("Insert failed: {e}"),
                };
                self.toast(msg, ok);
                if ok {
                    self.load_table(tid);
                }
            }
            Purpose::Values(ji) => {
                if let Some(key) = self.values_jobs.get(ji).cloned() {
                    self.values_pending.remove(&key);
                    let vals: Vec<(String, i64)> = results
                        .ok()
                        .and_then(|mut r| if r.is_empty() { None } else { Some(r.remove(0)) })
                        .map(|r| {
                            r.rows
                                .into_iter()
                                .filter(|row| row.len() >= 2 && !row[0].is_null())
                                .map(|row| (row[0].display(), row[1].as_f64().unwrap_or(0.0) as i64))
                                .filter(|(v, _)| !v.is_empty() && v.chars().count() <= assist::VALUE_MAX_CHARS)
                                .collect()
                        })
                        .unwrap_or_default();
                    self.values_cache.insert(key, vals);
                }
            }
            Purpose::AskData(mi, ji) => {
                let n = results
                    .ok()
                    .and_then(|r| r.first().and_then(|x| x.rows.first().and_then(|row| row.first().and_then(|c| c.as_f64()))))
                    .unwrap_or(0.0) as i64;
                let job = self.ask.jobs.get(ji).cloned();
                if let Some(m) = self.ask.msgs.get_mut(mi) {
                    m.pending = m.pending.saturating_sub(1);
                    if let (Some((jsid, t)), true) = (job, n > 0) {
                        m.data.push((jsid, t, n));
                        m.data.sort_by(|a, b| b.2.cmp(&a.2));
                    }
                }
            }
            Purpose::AiRun(tid, idx) => {
                if let Some(t) = self.tab_mut(tid) {
                    if let TabKind::Ai(a) = &mut t.kind {
                        a.runs.insert(idx, results.and_then(|mut r| r.pop().ok_or_else(|| "No result".to_string())));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ tabs

    fn add_tab(&mut self, conn: u64, title: String, kind: TabKind) -> u64 {
        let id = self.id();
        self.tabs.push(Tab { id, conn, title, kind });
        self.active_tab = Some(id);
        self.active_conn = Some(conn);
        id
    }

    fn new_query_tab(&mut self, conn: u64, sql: String) -> u64 {
        let n = self.tabs.iter().filter(|t| matches!(t.kind, TabKind::Query(_))).count() + 1;
        self.add_tab(conn, format!("Query #{n}"), TabKind::Query(QueryTab { sql, split: 0.38, ..Default::default() }))
    }

    fn new_ai_tab(&mut self, conn: u64) {
        if self.settings.ai.provider == Provider::Ollama && self.settings.ai.ollama_model.is_empty() {
            if let Ok(m) = ai::ollama_models(&self.settings.ai.ollama_url) {
                if let Some(first) = m.first() {
                    self.settings.ai.ollama_model = first.clone();
                }
            }
        }
        // column details make the AI's answers better
        if let Some(o) = self.conn(conn) {
            for t in o.tables.iter().take(60) {
                if !o.columns.contains_key(&t.key()) {
                    let _ = o.handle.send(Request::Columns { schema: t.schema.clone(), table: t.name.clone() });
                }
            }
        }
        let mut a = AiTab::default();
        a.msgs.push(ChatMsg {
            role: Role::Note,
            text: "Ask about your data in plain words. I can read the table list and columns of this connection and suggest SQL. Queries that change data always need your OK.".into(),
        });
        self.add_tab(conn, "AI Shell".into(), TabKind::Ai(a));
    }

    fn new_er_tab(&mut self, conn: u64) {
        let tag = self.id();
        if let Some(o) = self.conn(conn) {
            let _ = o.handle.send(Request::Graph { tag });
        }
        self.add_tab(conn, "ER Diagram".into(), TabKind::Er(ErState::default()));
    }

    fn open_table(&mut self, conn: u64, t: &TableInfo) {
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|x| x.conn == conn && matches!(&x.kind, TabKind::Table(tt) if tt.table == t.name && tt.schema == t.schema))
        {
            self.active_tab = Some(existing.id);
            return;
        }
        let cached = self.conn(conn).and_then(|o| o.columns.get(&t.key()).cloned());
        let tt = TableTab {
            schema: t.schema.clone(),
            table: t.name.clone(),
            is_view: t.is_view,
            cols: cached.clone(),
            loading: true,
            ..Default::default()
        };
        let id = self.add_tab(conn, t.name.clone(), TabKind::Table(tt));
        if cached.is_some() {
            self.load_table(id);
        } else if let Some(o) = self.conn(conn) {
            let _ = o.handle.send(Request::Columns { schema: t.schema.clone(), table: t.name.clone() });
        }
    }

    fn qualified(kind: DbKind, schema: &Option<String>, table: &str) -> String {
        match schema {
            Some(s) => format!("{}.{}", kind.quote(s), kind.quote(table)),
            None => kind.quote(table),
        }
    }

    fn load_table(&mut self, tid: u64) {
        let page_size = self.settings.page_size.max(1);
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tid) else { return };
        let sid = t.conn;
        let Some(kind) = self.open.iter().find(|o| o.sid == sid).map(|o| o.cfg.kind) else { return };
        let TabKind::Table(tt) = &mut t.kind else { return };
        tt.loading = true;
        let q = Self::qualified(kind, &tt.schema, &tt.table);
        tt.search_changed = None;
        let col_names: Vec<String> = tt.cols.as_ref().map(|c| c.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
        let clause = filter::where_clause(kind, &tt.filter, &col_names);
        let wh = if clause.is_empty() { String::new() } else { format!(" WHERE {clause}") };
        let order = match (&tt.sort, &tt.cols) {
            (Some((c, asc)), Some(_)) => tt
                .data
                .as_ref()
                .and_then(|d| d.columns.get(*c))
                .map(|name| format!(" ORDER BY {} {}", kind.quote(name), if *asc { "ASC" } else { "DESC" }))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let sql = format!("SELECT * FROM {q}{wh}{order} LIMIT {page_size} OFFSET {}", tt.page * page_size);
        let count = format!("SELECT COUNT(*) FROM {q}{wh}");
        let _ = self.send(sid, sql, Purpose::TableData(tid), page_size);
        // MECHANICS: allowSkipToLastPage = true for sqlite/postgres/mysql
        let _ = self.send(sid, count, Purpose::TableCount(tid), 1);
    }

    fn save_table(&mut self, tid: u64) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tid) else { return };
        let sid = t.conn;
        let Some(o) = self.conn(sid) else { return };
        let kind = o.cfg.kind;
        let TabKind::Table(tt) = &t.kind else { return };
        let (Some(cols), Some(data)) = (&tt.cols, &tt.data) else { return };
        let pks: Vec<&str> = cols.iter().filter(|c| c.pk).map(|c| c.name.as_str()).collect();
        if pks.is_empty() {
            self.toast("This table has no primary key, so rows can't be edited safely.", false);
            return;
        }
        let q = Self::qualified(kind, &tt.schema, &tt.table);
        let where_for = |r: usize| -> String {
            pks.iter()
                .filter_map(|name| {
                    data.columns.iter().position(|c| c == name).map(|di| {
                        let v = &data.rows[r][di];
                        if v.is_null() {
                            format!("{} IS NULL", kind.quote(name))
                        } else {
                            format!("{} = {}", kind.quote(name), v.sql_literal_for(kind))
                        }
                    })
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        let mut stmts = Vec::new();
        let mut by_row: HashMap<usize, Vec<(usize, &Cell)>> = HashMap::new();
        for ((r, c), v) in &tt.edits {
            if !tt.deletes.contains(r) {
                by_row.entry(*r).or_default().push((*c, v));
            }
        }
        let mut rows: Vec<_> = by_row.into_iter().collect();
        rows.sort_by_key(|x| x.0);
        for (r, changes) in rows {
            let sets: Vec<String> = changes.iter().map(|(c, v)| format!("{} = {}", kind.quote(&data.columns[*c]), v.sql_literal_for(kind))).collect();
            stmts.push(format!("UPDATE {q} SET {} WHERE {}", sets.join(", "), where_for(r)));
        }
        let mut dels: Vec<_> = tt.deletes.iter().copied().collect();
        dels.sort();
        for r in dels {
            stmts.push(format!("DELETE FROM {q} WHERE {}", where_for(r)));
        }
        if stmts.is_empty() {
            return;
        }
        let sql = stmts.join(";\n");
        if let Err(e) = self.send(sid, sql, Purpose::TableSave(tid), 1) {
            self.toast(e, false);
        }
    }

    /// Asks the server to stop the query running in a tab.
    pub(super) fn stop_query(&mut self, tid: u64, why: StopReason) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tid) else { return };
        let sid = t.conn;
        let TabKind::Query(q) = &mut t.kind else { return };
        if !q.running || q.stop_asked.is_some() {
            return;
        }
        q.stop_asked = Some(why);
        let ok = self.open.iter().find(|o| o.sid == sid).map(|o| o.handle.cancel()).unwrap_or(false);
        if !ok {
            self.toast("This connection can't stop a query yet.", false);
        }
    }

    /// Time limit for queries (Settings → General).
    fn enforce_query_timeout(&mut self) {
        let limit = self.settings.query_timeout_secs;
        if limit == 0 {
            return;
        }
        let due: Vec<u64> = self
            .tabs
            .iter()
            .filter_map(|t| match &t.kind {
                TabKind::Query(q) if q.running && q.stop_asked.is_none() && q.started.map(|s| s.elapsed().as_secs() >= limit).unwrap_or(false) => Some(t.id),
                _ => None,
            })
            .collect();
        for id in due {
            self.stop_query(id, StopReason::TimeLimit(limit));
        }
    }

    /// Visual EXPLAIN of the selection, or the statement under the cursor.
    pub(super) fn explain_query_tab(&mut self, tid: u64, analyze: bool) {
        let max = self.settings.max_results;
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tid) else { return };
        let sid = t.conn;
        let TabKind::Query(q) = &mut t.kind else { return };
        let (a, b) = (q.cursor.0.min(q.cursor.1), q.cursor.0.max(q.cursor.1));
        let chars: Vec<char> = q.sql.chars().collect();
        let sql: String = if a != b && b <= chars.len() { chars[a..b].iter().collect() } else { statement_at(&q.sql, a) };
        if sql.trim().is_empty() {
            return;
        }
        // ANALYZE really runs the query: only for queries that just read
        if analyze && !db::is_read_query(&sql) {
            self.toast("EXPLAIN ANALYZE runs the query, so it's only offered for queries that just read data.", false);
            return;
        }
        let kind = self.open.iter().find(|o| o.sid == sid).map(|o| o.cfg.kind).unwrap_or(db::DbKind::Sqlite);
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tid) else { return };
        let TabKind::Query(q) = &mut t.kind else { return };
        q.running = true;
        q.error = None;
        q.started = Some(Instant::now());
        q.stop_asked = None;
        let ex = crate::plan::explain_sql(kind, &sql, analyze);
        if let Err(e) = self.send(sid, ex, Purpose::Explain(tid), max) {
            if let Some(t) = self.tab_mut(tid) {
                if let TabKind::Query(q) = &mut t.kind {
                    q.running = false;
                    q.plan = Some(Err(e));
                }
            }
        }
    }

    fn run_query_tab(&mut self, tid: u64, current_only: bool) {
        let max = self.settings.max_results;
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tid) else { return };
        let sid = t.conn;
        let TabKind::Query(q) = &mut t.kind else { return };
        let (a, b) = (q.cursor.0.min(q.cursor.1), q.cursor.0.max(q.cursor.1));
        let chars: Vec<char> = q.sql.chars().collect();
        let sql = if a != b && b <= chars.len() {
            chars[a..b].iter().collect()
        } else if current_only {
            statement_at(&q.sql, a)
        } else {
            q.sql.clone()
        };
        if sql.trim().is_empty() {
            return;
        }
        q.running = true;
        q.error = None;
        q.plan = None;
        q.started = Some(Instant::now());
        q.stop_asked = None;
        if let Err(e) = self.send(sid, sql, Purpose::Query(tid), max) {
            if let Some(t) = self.tab_mut(tid) {
                if let TabKind::Query(q) = &mut t.kind {
                    q.running = false;
                    q.error = Some(e);
                }
            }
        }
    }

    fn close_tab(&mut self, id: u64) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            let t = self.tabs.remove(pos);
            if let TabKind::Query(q) = &t.kind {
                self.closed_tabs.push((t.conn, t.title.clone(), q.sql.clone()));
            }
            if self.active_tab == Some(id) {
                self.active_tab = self.tabs.get(pos.min(self.tabs.len().saturating_sub(1))).map(|t| t.id);
            }
        }
    }

    // ------------------------------------------------------------ AI

    fn ai_system_prompt(&self, sid: u64) -> String {
        let Some(o) = self.conn(sid) else { return String::new() };
        // MECHANICS: system schemas hidden from the AI
        let hidden = ["information_schema", "pg_catalog", "pg_toast", "sys"];
        let mut tables = String::new();
        for t in o.tables.iter().filter(|t| !t.schema.as_deref().map(|s| hidden.contains(&s)).unwrap_or(false)).take(200) {
            tables.push_str(&format!("- {}{}", t.key(), if t.is_view { " (view)" } else { "" }));
            if let Some(cols) = o.columns.get(&t.key()) {
                let c: Vec<String> = cols.iter().map(|c| format!("{} {}{}", c.name, c.data_type, if c.pk { " PK" } else { "" })).collect();
                tables.push_str(&format!(": {}", c.join(", ")));
            }
            tables.push('\n');
        }
        let mut s = format!(
            "You are a database assistant inside a desktop SQL editor.\nToday is {}.\nDatabase engine: {}.\nDatabase: {}.\n{}\nTables:\n{}\n\
             When you write SQL, put each query in a ```sql code block, using the {} dialect. Keep answers short and clear. \
             Prefer read-only queries unless the user clearly asks to change data. If you need query results to answer, propose the query; the user can run it and send you the results.",
            now_string(),
            o.cfg.kind.label(),
            o.current_db,
            if o.cfg.read_only { "The connection is READ ONLY: only suggest queries that do not modify data.\n" } else { "" },
            tables,
            o.cfg.kind.label()
        );
        if !self.settings.ai.custom_instructions.trim().is_empty() {
            s.push_str("\n\nExtra instructions from the user:\n");
            s.push_str(&self.settings.ai.custom_instructions);
        }
        s
    }

    fn ai_call(&self) -> ai::Call {
        let a = &self.settings.ai;
        let (url, model, key) = match a.provider {
            Provider::Ollama => (a.ollama_url.clone(), a.ollama_model.clone(), String::new()),
            Provider::OpenAiCompat => (a.compat_url.clone(), a.compat_model.clone(), store::decrypt(&a.compat_key_enc)),
            Provider::Anthropic => (String::new(), a.anthropic_model.clone(), store::decrypt(&a.anthropic_key_enc)),
            Provider::OpenAi => (String::new(), a.openai_model.clone(), store::decrypt(&a.openai_key_enc)),
            Provider::Google => (String::new(), a.google_model.clone(), store::decrypt(&a.google_key_enc)),
            Provider::ClaudeCode => (String::new(), a.claude_model.clone(), String::new()),
        };
        ai::Call { provider: a.provider, url, model, key, temperature: a.temperature, claude_path: a.claude_path.clone() }
    }

    fn ai_send(&mut self, tid: u64) {
        let Some(sid) = self.tabs.iter().find(|t| t.id == tid).map(|t| t.conn) else { return };
        // ask for column details so later prompts include them
        if let Some(o) = self.conn(sid) {
            for t in o.tables.iter().take(60) {
                if !o.columns.contains_key(&t.key()) {
                    let _ = o.handle.send(Request::Columns { schema: t.schema.clone(), table: t.name.clone() });
                }
            }
        }
        if self.settings.ai.provider == Provider::Ollama && self.settings.ai.ollama_model.is_empty() {
            if let Ok(m) = ai::ollama_models(&self.settings.ai.ollama_url) {
                if let Some(first) = m.first() {
                    self.settings.ai.ollama_model = first.clone();
                }
            }
        }
        let system = self.ai_system_prompt(sid);
        let call = self.ai_call();
        let Some(t) = self.tab_mut(tid) else { return };
        let TabKind::Ai(a) = &mut t.kind else { return };
        if a.input.trim().is_empty() || a.busy {
            return;
        }
        a.msgs.push(ChatMsg { role: Role::User, text: std::mem::take(&mut a.input).trim().to_string() });
        a.busy = true;
        a.rx = Some(ai::ask(call, system, a.msgs.clone()));
    }

    fn open_settings(&mut self, page: SettingsPage) {
        let a = &self.settings.ai;
        let mut keys = HashMap::new();
        keys.insert("anthropic", store::decrypt(&a.anthropic_key_enc));
        keys.insert("openai", store::decrypt(&a.openai_key_enc));
        keys.insert("google", store::decrypt(&a.google_key_enc));
        keys.insert("compat", store::decrypt(&a.compat_key_enc));
        self.settings_ui = Some(SettingsUi {
            page,
            keys,
            ollama_models: vec![],
            msg: String::new(),
            pin1: String::new(),
            pin2: String::new(),
            claude_ver: String::new(),
        });
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // demo scene: open a table that has JSON data and select a row
        let scene = std::env::var("FAVO_SCENE").unwrap_or_default();
        if self.demo_setup == 1 && scene == "query" {
            if let Some(sid) = self.open.first().filter(|o| o.status == ConnStatus::Ready).map(|o| o.sid) {
                let sql = if std::env::var("FAVO_CHART").is_ok() { "SELECT status, total FROM orders;".to_string() } else { "-- Revenue per category\nSELECT category, revenue, units\nFROM sales_by_category\nORDER BY revenue DESC;".to_string() };
                let id = self.new_query_tab(sid, sql);
                if let Some(t) = self.tab_mut(id) {
                    if let TabKind::Query(q) = &mut t.kind {
                        q.view = View::Chart;
                    }
                }
                self.run_query_tab(id, false);
                self.demo_setup = 3;
            }
        } else if self.demo_setup == 1 && (scene == "ac" || scene == "palette" || scene == "values") {
            if let Some(sid) = self.open.first().filter(|o| o.status == ConnStatus::Ready && !o.tables.is_empty() && o.columns.len() >= 4).map(|o| o.sid) {
                let sql = if scene == "values" { "SELECT *\nFROM orders o\nWHERE o.status = '" } else { "SELECT p.\nFROM products p\nWHERE p.price > 20;" };
                let id = self.new_query_tab(sid, sql.to_string());
                if scene == "palette" {
                    self.quick = Some(("ord".to_string(), 0));
                    self.demo_setup = 3;
                } else {
                    self.demo_setup = 4;
                    self.demo_tab = id;
                }
            }
        } else if self.demo_setup == 4 && self.frame > 60 {
            let id = self.demo_tab;
            if let Some(t) = self.tab_mut(id) {
                if let TabKind::Query(q) = &mut t.kind {
                    let end = q.sql.chars().count();
                    let at = if scene == "values" { end } else { 9 };
                    q.cursor = (at, at);
                    q.editor_op = Some(EditorOp::Complete);
                }
            }
            self.demo_setup = 3;
        } else if self.demo_setup == 1 && scene == "conn" {
            let cfg = ConnConfig { name: "Prod via proxy".into(), kind: DbKind::Postgres, host: "localhost".into(), port: std::env::var("FAVO_PG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(55433), user: "estante_admin".into(), database: "estante_crm_prod".into(), ..Default::default() };
            let pw = std::env::var("FAVO_PG_PASS").unwrap_or_default();
            let (tx, rx) = std::sync::mpsc::channel();
            let (c2, p2) = (cfg.clone(), pw.clone());
            std::thread::spawn(move || {
                let _ = tx.send(db::pg_diagnose(&c2, &p2));
            });
            self.diag_rx = Some(rx);
            self.editor = Some(ConnEditor { cfg, password: pw, is_new: false, test: None, test_msg: None });
            self.demo_setup = 3;
        } else if self.demo_setup == 1 && (scene == "ai" || scene == "er") {
            if let Some(sid) = self.open.first().filter(|o| o.status == ConnStatus::Ready && !o.tables.is_empty()).map(|o| o.sid) {
                if scene == "ai" {
                    self.new_ai_tab(sid);
                } else {
                    self.new_er_tab(sid);
                }
                self.demo_setup = 3;
            }
        } else if self.demo_setup == 1 {
            if scene == "light" {
                self.settings.theme = ThemeKind::Daylight;
            }
            let found = self.open.first().and_then(|o| o.tables.iter().find(|t| t.name == "products").cloned().map(|t| (o.sid, t)));
            if let Some((sid, t)) = found {
                self.open_table(sid, &t);
                self.demo_setup = 2;
            }
        } else if self.demo_setup == 2 {
            let mut reload = None;
            for t in &mut self.tabs {
                if let TabKind::Table(tt) = &mut t.kind {
                    if tt.data.is_some() {
                        tt.grid.selected = Some((1, 6));
                        self.demo_setup = 3;
                        if scene == "filter" {
                            tt.filter.search = "a".into();
                            tt.filter.rules.push(filter::Rule { column: "price".into(), op: filter::Op::Greater, value: "12".into(), enabled: true });
                            tt.filter.rules.push(filter::Rule { column: "category".into(), op: filter::Op::InList, value: "Food, Garden".into(), enabled: true });
                            tt.show_rules = true;
                            tt.grid.selected = Some((0, 1));
                            reload = Some(t.id);
                        }
                    }
                }
            }
            if let Some(id) = reload {
                self.load_table(id);
            }
            if scene == "ask" && self.demo_setup == 3 {
                self.side_tab = SideTab::Ask;
                self.ask.scope = AskScope::ThisTable;
                self.ask.input = "tem email nessa tabela?".into();
                self.ask_send();
                self.ask.scope = AskScope::Database;
                self.ask.input = "tem 'Honey' em algum lugar?".into();
                self.ask_send();
            }
            if scene == "nav" && self.demo_setup == 3 {
                if let Some(o) = self.open.first() {
                    self.expanded.insert(format!("{}:products", o.sid));
                }
            }
        }
        if self.frame >= 120 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let mut bytes = Vec::with_capacity(w * h * 4);
            for px in &img.pixels {
                bytes.extend_from_slice(&px.to_array());
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match image::save_buffer(&path, &bytes, w as u32, h as u32, image::ExtendedColorType::Rgba8) {
                Ok(_) => eprintln!("screenshot saved to {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            std::process::exit(0);
        }
        if self.frame > 900 {
            eprintln!("screenshot timed out");
            std::process::exit(1);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let ctrl = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let alt_s = Modifiers::ALT | Modifiers::SHIFT;
        // longer chords first so Ctrl+Shift+X is not read as Ctrl+X
        let table: &[(Modifiers, Key, Cmd)] = &[
            (cs, Key::Enter, Cmd::RunCurrent),
            (Modifiers::SHIFT, Key::F5, Cmd::RunCurrent),
            (cs, Key::F, Cmd::FormatSql),
            (cs, Key::I, Cmd::LintSql),
            (cs, Key::T, Cmd::ReopenTab),
            (cs, Key::N, Cmd::NewConnection),
            (cs, Key::E, Cmd::NewEr),
            (cs, Key::A, Cmd::AddFilter),
            (cs, Key::X, Cmd::ClearFilters),
            (cs, Key::J, Cmd::ExportJson),
            (cs, Key::L, Cmd::ToggleTheme),
            (cs, Key::Q, Cmd::Lock),
            (cs, Key::H, Cmd::Argon2),
            (cs, Key::P, Cmd::Shortcuts),
            (cs, Key::K, Cmd::Ask),
            (alt_s, Key::Num5, Cmd::Ask),
            (Modifiers::CTRL | Modifiers::SHIFT, Key::Tab, Cmd::PrevTab),
            (alt_s, Key::Num1, Cmd::ShowConnections),
            (alt_s, Key::Num2, Cmd::ShowTables),
            (alt_s, Key::Num3, Cmd::ShowSaved),
            (alt_s, Key::Num4, Cmd::ShowHistory),
            (ctrl, Key::Enter, Cmd::RunQuery),
            (ctrl, Key::Slash, Cmd::ToggleComment),
            (ctrl, Key::Space, Cmd::Autocomplete),
            (ctrl, Key::S, Cmd::SaveChanges),
            (ctrl, Key::R, Cmd::Refresh),
            (ctrl, Key::F, Cmd::FocusSearch),
            (ctrl, Key::E, Cmd::ExportCsv),
            (ctrl, Key::W, Cmd::CloseTab),
            (ctrl, Key::T, Cmd::NewQuery),
            (ctrl, Key::L, Cmd::NewAi),
            (ctrl, Key::B, Cmd::ToggleSidebar),
            (ctrl, Key::J, Cmd::ToggleJson),
            (ctrl, Key::Comma, Cmd::Settings),
            (ctrl, Key::PageDown, Cmd::NextTab),
            (ctrl, Key::PageUp, Cmd::PrevTab),
            (Modifiers::CTRL, Key::Tab, Cmd::NextTab),
            (Modifiers::NONE, Key::F1, Cmd::Shortcuts),
        ];
        let mut fired = Vec::new();
        ctx.input_mut(|i| {
            for (m, k, c) in table {
                if i.consume_key(*m, *k) {
                    fired.push(*c);
                }
            }
        });
        // F5: run in a query tab, refresh in a table tab
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
            fired.push(if self.active_kind_is(|k| matches!(k, TabKind::Query(_))) { Cmd::RunQuery } else { Cmd::Refresh });
        }
        let palette = ctx.input_mut(|i| i.consume_key(ctrl, Key::K) || i.consume_key(ctrl, Key::P));
        if palette {
            self.quick = Some((String::new(), 0));
        }
        let editing_text = ctx.wants_keyboard_input();
        let (pg_next, pg_prev) = if editing_text {
            (false, false)
        } else {
            ctx.input_mut(|i| (i.consume_key(ctrl, Key::ArrowRight), i.consume_key(ctrl, Key::ArrowLeft)))
        };
        let mut switch = None;
        ctx.input_mut(|i| {
            let keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
            for (n, k) in keys.iter().enumerate() {
                if i.consume_key(Modifiers::ALT, *k) {
                    switch = Some(n);
                }
            }
        });
        if let Some(n) = switch {
            if let Some(t) = self.tabs.get(n) {
                self.active_tab = Some(t.id);
            }
        }
        for c in fired {
            self.run_command(c);
        }
        if let Some(id) = self.active_tab {
            if self.active_kind_is(|k| matches!(k, TabKind::Table(_))) && (pg_next || pg_prev) {
                let ps = self.settings.page_size.max(1) as u64;
                if let Some(t) = self.tab_mut(id) {
                    if let TabKind::Table(tt) = &mut t.kind {
                        let last = tt.total.map(|n| (n.saturating_sub(1) / ps) as usize).unwrap_or(usize::MAX);
                        if pg_next && tt.page < last {
                            tt.page += 1;
                        }
                        if pg_prev && tt.page > 0 {
                            tt.page -= 1;
                        }
                    }
                }
                self.load_table(id);
            }
        }
        if let Some(t) = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)) {
            self.active_conn = Some(t.conn);
        }
    }

    fn active_kind_is(&self, f: fn(&TabKind) -> bool) -> bool {
        self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id)).map(|t| f(&t.kind)).unwrap_or(false)
    }

    fn run_command(&mut self, c: Cmd) {
        let active = self.active_tab;
        let sid = self.active_conn;
        match c {
            Cmd::NewQuery => {
                if let Some(s) = sid {
                    self.new_query_tab(s, String::new());
                }
            }
            Cmd::NewAi => {
                if let Some(s) = sid {
                    self.new_ai_tab(s);
                }
            }
            Cmd::NewEr => {
                if let Some(s) = sid {
                    self.new_er_tab(s);
                }
            }
            Cmd::NewConnection => {
                self.editor = Some(ConnEditor { cfg: ConnConfig::default(), password: String::new(), is_new: true, test: None, test_msg: None });
            }
            Cmd::RunQuery | Cmd::RunCurrent => {
                if let Some(id) = active {
                    if self.active_kind_is(|k| matches!(k, TabKind::Query(_))) {
                        self.run_query_tab(id, c == Cmd::RunCurrent);
                    } else if self.active_kind_is(|k| matches!(k, TabKind::Table(_))) {
                        self.load_table(id);
                    }
                }
            }
            Cmd::FormatSql | Cmd::ToggleComment | Cmd::Autocomplete | Cmd::LintSql => {
                let op = match c {
                    Cmd::FormatSql => EditorOp::Format,
                    Cmd::LintSql => EditorOp::Lint,
                    Cmd::ToggleComment => EditorOp::Comment,
                    _ => EditorOp::Complete,
                };
                if let Some(t) = active.and_then(|id| self.tabs.iter_mut().find(|t| t.id == id)) {
                    if let TabKind::Query(q) = &mut t.kind {
                        q.editor_op = Some(op);
                    }
                }
            }
            Cmd::SaveChanges => {
                if let Some(id) = active {
                    if self.active_kind_is(|k| matches!(k, TabKind::Table(_))) {
                        self.save_table(id);
                    }
                }
            }
            Cmd::Refresh => {
                if let Some(id) = active {
                    if self.active_kind_is(|k| matches!(k, TabKind::Table(_))) {
                        self.load_table(id);
                    }
                }
                if let Some(o) = sid.and_then(|s| self.conn(s)) {
                    let _ = o.handle.send(Request::Refresh);
                }
            }
            Cmd::FocusSearch | Cmd::AddFilter | Cmd::ClearFilters => {
                let mut reload = false;
                if let Some(t) = active.and_then(|id| self.tabs.iter_mut().find(|t| t.id == id)) {
                    if let TabKind::Table(tt) = &mut t.kind {
                        match c {
                            Cmd::FocusSearch => tt.focus_search = true,
                            Cmd::AddFilter => {
                                let col = tt.cols.as_ref().and_then(|c| c.first()).map(|c| c.name.clone()).unwrap_or_default();
                                tt.filter.rules.push(filter::Rule { column: col, op: filter::Op::Contains, value: String::new(), enabled: true });
                                tt.show_rules = true;
                            }
                            _ => {
                                tt.filter = FilterSet::default();
                                tt.page = 0;
                                reload = true;
                            }
                        }
                    }
                }
                if reload {
                    if let Some(id) = active {
                        self.load_table(id);
                    }
                }
            }
            Cmd::ExportCsv | Cmd::ExportJson => {
                if let (Some(id), Some(s)) = (active, sid) {
                    self.do_action(id, s, Action::Export(if c == Cmd::ExportJson { crate::export::Format::Json } else { crate::export::Format::Csv }));
                }
            }
            Cmd::CloseTab => {
                if let Some(id) = active {
                    self.close_tab(id);
                }
            }
            Cmd::ReopenTab => {
                if let Some((cn, _title, sql)) = self.closed_tabs.pop() {
                    if self.conn(cn).is_some() {
                        self.new_query_tab(cn, sql);
                    }
                }
            }
            Cmd::NextTab | Cmd::PrevTab => {
                if !self.tabs.is_empty() {
                    let pos = active.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(0);
                    let n = self.tabs.len();
                    let np = if c == Cmd::NextTab { (pos + 1) % n } else { (pos + n - 1) % n };
                    self.active_tab = Some(self.tabs[np].id);
                }
            }
            Cmd::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                // an explicit request wins over the narrow-window auto-hide
                self.sidebar_pinned_narrow = self.sidebar_open;
            }
            Cmd::ToggleJson => self.settings.show_json = !self.settings.show_json,
            Cmd::ToggleTheme => {
                self.settings.theme = self.current_theme().flipped();
                self.persist();
            }
            Cmd::ShowConnections | Cmd::ShowTables | Cmd::ShowSaved | Cmd::ShowHistory => {
                self.side_tab = match c {
                    Cmd::ShowConnections => SideTab::Connections,
                    Cmd::ShowTables => SideTab::Tables,
                    Cmd::ShowSaved => SideTab::Saved,
                    _ => SideTab::History,
                };
                self.sidebar_open = true;
                self.sidebar_pinned_narrow = true;
            }
            Cmd::Settings => self.open_settings(SettingsPage::General),
            Cmd::Shortcuts => self.show_shortcuts = !self.show_shortcuts,
            Cmd::Argon2 => {
                self.argon2 = if self.argon2.is_some() { None } else { Some(crate::argon2tool::Tool::new(crate::argon2tool::Mode::Hash)) };
            }
            Cmd::Ask => {
                self.side_tab = SideTab::Ask;
                self.sidebar_open = true;
                self.sidebar_pinned_narrow = true;
                self.ask.focus_input = true;
            }
            Cmd::Lock => {
                if self.settings.pin_hash.is_empty() {
                    self.toast("Set a PIN in Settings → Security first.", false);
                } else {
                    self.locked = true;
                }
            }
        }
    }

    // ------------------------------------------------------------ ask bot

    fn ask_focus(&self) -> Option<(u64, String)> {
        let t = self.active_tab.and_then(|id| self.tabs.iter().find(|t| t.id == id))?;
        match &t.kind {
            TabKind::Table(tt) => Some((t.conn, tt.table.clone())),
            _ => None,
        }
    }

    /// Tables the bot looks at, by scope.
    fn ask_tables(&self) -> Vec<(u64, String, TableInfo, Vec<ColumnInfo>)> {
        let focus = self.ask_focus();
        let mut out = Vec::new();
        for o in &self.open {
            let in_scope = match self.ask.scope {
                AskScope::Everything => true,
                _ => Some(o.sid) == self.active_conn,
            };
            if !in_scope {
                continue;
            }
            for t in &o.tables {
                if self.ask.scope == AskScope::ThisTable {
                    // names: whole database (to suggest where else it is);
                    // the answer is still about the focused table
                }
                let cols = o.columns.get(&t.key()).cloned().unwrap_or_default();
                out.push((o.sid, o.label(), t.clone(), cols));
            }
        }
        let _ = focus;
        out
    }

    fn ask_send(&mut self) {
        let q = self.ask.input.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.ask.input.clear();
        let tables = self.ask_tables();
        let cands: Vec<finder::Candidate> = tables.iter().map(|(sid, conn, t, c)| finder::Candidate { sid: *sid, conn: conn.clone(), table: t, cols: c }).collect();
        let focus = if self.ask.scope == AskScope::ThisTable { self.ask_focus() } else { None };
        let mut r = finder::answer(&q, &cands, focus);
        if self.open.is_empty() {
            r.text = if r.lang == Some(finder::Lang::Pt) { "Conecte a um banco primeiro, aí eu procuro.".into() } else { "Connect to a database first, then I can look.".into() };
            r.hits.clear();
            r.auto_search = false;
            r.value = None;
        }
        self.ask.msgs.push(AskMsg { user: true, text: q, ..Default::default() });
        let auto = r.auto_search;
        self.ask.msgs.push(AskMsg { user: false, text: r.text, hits: r.hits, value: r.value, lang: r.lang, ..Default::default() });
        if auto {
            let mi = self.ask.msgs.len() - 1;
            self.ask_search_data(mi);
        }
    }

    /// Count rows containing the message's value in every table in scope.
    fn ask_search_data(&mut self, mi: usize) {
        let Some(value) = self.ask.msgs.get(mi).and_then(|m| m.value.clone()) else { return };
        let focus = if self.ask.scope == AskScope::ThisTable { self.ask_focus() } else { None };
        let mut jobs = Vec::new();
        for (sid, _conn, t, cols) in self.ask_tables() {
            if t.is_view || cols.is_empty() {
                continue;
            }
            if let Some((fs, ft)) = &focus {
                if *fs != sid || *ft != t.name {
                    continue;
                }
            }
            jobs.push((sid, t, cols));
        }
        jobs.truncate(80);
        let mut sent = 0;
        for (sid, t, cols) in jobs {
            let Some(kind) = self.conn(sid).map(|o| o.cfg.kind) else { continue };
            let names: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();
            let fs = FilterSet { search: value.clone(), ..Default::default() };
            let wh = filter::where_clause(kind, &fs, &names);
            if wh.is_empty() {
                continue;
            }
            let sql = format!("SELECT COUNT(*) FROM {} WHERE {wh}", Self::qualified(kind, &t.schema, &t.name));
            self.ask.jobs.push((sid, t));
            let ji = self.ask.jobs.len() - 1;
            if self.send(sid, sql, Purpose::AskData(mi, ji), 1).is_ok() {
                sent += 1;
            }
        }
        if let Some(m) = self.ask.msgs.get_mut(mi) {
            m.pending += sent;
            m.searched = true;
        }
    }

    /// Open a table found by the bot; highlight a column or pre-fill a search.
    fn ask_open(&mut self, sid: u64, t: &TableInfo, column: Option<String>, search: Option<String>) {
        self.active_conn = Some(sid);
        self.open_table(sid, t);
        let mut reload = None;
        if let Some(tab) = self.active_tab.and_then(|id| self.tabs.iter_mut().find(|x| x.id == id)) {
            if let TabKind::Table(tt) = &mut tab.kind {
                tt.grid.highlight_col = column;
                if let Some(v) = search {
                    tt.filter.search = v;
                    tt.page = 0;
                    reload = Some(tab.id);
                }
            }
        }
        if let Some(id) = reload {
            self.load_table(id);
        }
    }

    /// Schema snapshot for autocomplete.
    /// Distinct values of a column, sampled cheaply: only the first
    /// VALUES_SAMPLE_ROWS rows are read, then the VALUES_TOP most common
    /// values are kept. Results are cached per connection and column.
    fn fetch_values(&mut self, sid: u64, table: &str, column: &str) {
        let key = (sid, assist::value_key(table, column));
        if self.values_cache.contains_key(&key) || self.values_pending.contains(&key) {
            return;
        }
        let Some(o) = self.conn(sid) else { return };
        let kind = o.cfg.kind;
        let Some(t) = o.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).cloned() else { return };
        let c = kind.quote(column);
        let sql = format!(
            "SELECT v, COUNT(*) AS n FROM (SELECT {c} AS v FROM {} WHERE {c} IS NOT NULL LIMIT {VALUES_SAMPLE_ROWS}) s GROUP BY v ORDER BY n DESC, v LIMIT {VALUES_TOP}",
            Self::qualified(kind, &t.schema, &t.name)
        );
        self.values_jobs.push(key.clone());
        let ji = self.values_jobs.len() - 1;
        if self.send(sid, sql, Purpose::Values(ji), VALUES_TOP).is_ok() {
            self.values_pending.insert(key);
        }
    }

    fn schema_info(&self, sid: u64) -> SchemaInfo {
        let Some(o) = self.conn(sid) else { return SchemaInfo::default() };
        let learned = self.learned.get(&o.cfg.id);
        // values: remembered ones first, then the sampled ones
        let mut values: HashMap<String, Vec<assist::ValueHint>> = HashMap::new();
        if let Some(l) = learned {
            for (k, vs) in &l.values {
                values.insert(k.clone(), vs.iter().map(|(v, n)| assist::ValueHint { text: v.clone(), count: None, used: *n }).collect());
            }
        }
        for ((csid, k), vs) in &self.values_cache {
            if *csid != sid {
                continue;
            }
            let list = values.entry(k.clone()).or_default();
            for (v, n) in vs {
                match list.iter_mut().find(|h| h.text == *v) {
                    Some(h) => h.count = Some(*n),
                    None => list.push(assist::ValueHint { text: v.clone(), count: Some(*n), used: 0 }),
                }
            }
        }
        SchemaInfo {
            learned: learned.map(|l| l.words.iter().map(|(w, n)| (w.clone(), *n)).collect()).unwrap_or_default(),
            values,
            kind: o.cfg.kind,
            tables: o
                .tables
                .iter()
                .map(|t| {
                    let cols = o.columns.get(&t.key()).map(|c| c.iter().map(|x| (x.name.clone(), x.data_type.clone())).collect()).unwrap_or_default();
                    (t.name.clone(), t.is_view, cols)
                })
                .collect(),
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.robot_input(raw_input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.apply_theme(ctx);
        self.poll();
        self.enforce_query_timeout();
        self.screenshot_step(ctx);

        // activity / idle lock (MECHANICS: idleThresholdSeconds = 300)
        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_activity = Instant::now();
        }
        if !self.locked
            && !self.settings.pin_hash.is_empty()
            && self.settings.lock_on_idle
            && self.last_activity.elapsed().as_secs() >= self.settings.idle_secs
        {
            self.locked = true;
        }
        if self.locked {
            self.lock_screen(ctx);
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }

        self.shortcuts(ctx);
        // narrow windows: the side panels give way so the main area stays usable
        let screen_w = ctx.screen_rect().width();
        let user_sidebar = self.sidebar_open;
        if screen_w < NARROW_SIDEBAR_HIDE && !self.sidebar_pinned_narrow {
            self.sidebar_open = false;
        }
        self.status_bar(ctx);
        self.rail(ctx);
        self.sidebar(ctx);
        if screen_w < NARROW_SIDEBAR_HIDE && !self.sidebar_pinned_narrow {
            // remember the choice; a click on the rail while narrow pins it open
            if self.sidebar_open {
                self.sidebar_pinned_narrow = true;
            } else {
                self.sidebar_open = user_sidebar;
            }
        }
        let json_ok = self
            .active_tab
            .and_then(|id| self.tabs.iter().find(|t| t.id == id))
            .map(|t| matches!(t.kind, TabKind::Query(_) | TabKind::Table(_)))
            .unwrap_or(false);
        if self.settings.show_json && json_ok && screen_w >= NARROW_JSON_HIDE {
            self.json_sidebar(ctx);
        }
        self.header(ctx);
        let draw_t = Instant::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.pal.bg).inner_margin(egui::Margin { left: 8, right: 12, top: 4, bottom: 8 }))
            .show(ctx, |ui| {
                ui.set_min_width(200.0); // MECHANICS: mainContentMinWidth
                self.central(ui);
            });
        if crate::debug::on() {
            self.dbg.drew(draw_t.elapsed().as_secs_f64() * 1000.0);
            self.dbg.frame(ctx.input(|i| i.unstable_dt) * 1000.0);
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F12)) {
                self.dbg.show = !self.dbg.show;
            }
            self.debug_panel(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.windows(ctx);
        if self.robot.is_some() {
            self.robot_step(ctx);
        }

        // type-to-filter: reload a table 350 ms after the search box stops changing
        let due: Vec<u64> = self
            .tabs
            .iter()
            .filter_map(|t| match &t.kind {
                TabKind::Table(tt) if tt.search_changed.map(|i| i.elapsed().as_millis() >= 350).unwrap_or(false) => Some(t.id),
                _ => None,
            })
            .collect();
        for id in due {
            if let Some(t) = self.tab_mut(id) {
                if let TabKind::Table(tt) = &mut t.kind {
                    tt.page = 0;
                }
            }
            self.load_table(id);
        }
        if self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Table(tt) if tt.search_changed.is_some())) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let busy = !self.pending.is_empty()
            || self.open.iter().any(|o| o.status == ConnStatus::Connecting)
            || self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Ai(a) if a.busy))
            || self.editor.as_ref().map(|e| e.test.is_some()).unwrap_or(false);
        if busy || self.screenshot.is_some() {
            ctx.request_repaint();
        } else {
            // MECHANICS: idleCheckIntervalSeconds = 30
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
    }
}
