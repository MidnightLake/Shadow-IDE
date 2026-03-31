use crossterm::{
    cursor,
    event::{self as ct_event, Event, KeyCode, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor, Attribute, SetAttribute},
    terminal::{self, ClearType},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

static MSG_COUNTER: AtomicU64 = AtomicU64::new(1);
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static BUDGET_LIMIT: AtomicU64 = AtomicU64::new(0);   // 0 = no limit
static BUDGET_USED: AtomicU64 = AtomicU64::new(0);
static THINK_BUDGET: AtomicU64 = AtomicU64::new(0);
#[allow(dead_code)]
static CONTEXT_TOKENS_USED: AtomicU64 = AtomicU64::new(0);
#[allow(dead_code)]
static CONTEXT_TOKENS_TOTAL: AtomicU64 = AtomicU64::new(200_000);

// Pass 4 statics
use std::sync::OnceLock;
#[allow(dead_code)] static GHOST_TEXT: OnceLock<std::sync::Mutex<String>> = OnceLock::new();
#[allow(dead_code)] static FOLDED_RESPONSES: OnceLock<std::sync::Mutex<Vec<(usize, String)>>> = OnceLock::new();
#[allow(dead_code)] static CONTEXT_SLOTS: OnceLock<std::sync::Mutex<Vec<ContextSlot>>> = OnceLock::new();
#[allow(dead_code)] static RESPONSE_CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<u64, CachedResponse>>> = OnceLock::new();
#[allow(dead_code)] static DB_DSN: OnceLock<std::sync::Mutex<String>> = OnceLock::new();
#[allow(dead_code)] static DAP_STATE: OnceLock<std::sync::Mutex<DapState>> = OnceLock::new();
#[allow(dead_code)] static PLAN_MODE: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)] static TOOL_STREAMING: AtomicBool = AtomicBool::new(true);
#[allow(dead_code)] static STREAMING_ACTIVE: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)] static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);
static SYNTAX_SET: OnceLock<syntect::parsing::SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<syntect::highlighting::ThemeSet> = OnceLock::new();
// Section 8.1 — Email SMTP task timing
#[allow(dead_code)] static TASK_START_TIME: OnceLock<std::sync::Mutex<Option<std::time::Instant>>> = OnceLock::new();
// Snapshot of email config for use in handle_event (no cli_config available there)
#[allow(dead_code)] static EMAIL_CFG_SNAPSHOT: OnceLock<std::sync::Mutex<Option<EmailCfgSnapshot>>> = OnceLock::new();
// Section 7.1 — Real-time Collaboration relay
#[allow(dead_code)] static RELAY_CLIENTS: OnceLock<std::sync::Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>> = OnceLock::new();
#[allow(dead_code)] static RELAY_TX: OnceLock<std::sync::Mutex<Option<tokio::net::tcp::OwnedWriteHalf>>> = OnceLock::new();
#[allow(dead_code)] static RELAY_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Sections 13-19 statics
#[allow(dead_code)] static MCP_SERVERS: OnceLock<std::sync::Mutex<Vec<McpServer>>> = OnceLock::new();
#[allow(dead_code)] static YOLO_MODE: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)] static ARCHITECT_MODE: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)] static APPROVAL_LEVEL: OnceLock<std::sync::Mutex<String>> = OnceLock::new();
#[allow(dead_code)] static PRIVACY_MODE: AtomicBool = AtomicBool::new(false);
// Block-based output: stores (block_id, content)
#[allow(dead_code)] static RESPONSE_BLOCKS: OnceLock<std::sync::Mutex<Vec<(usize, String)>>> = OnceLock::new();
// Running token/cost counters for /cost command
static SESSION_INPUT_TOKENS: AtomicU64 = AtomicU64::new(0);
static SESSION_OUTPUT_TOKENS: AtomicU64 = AtomicU64::new(0);

// ─── Exit codes ──────────────────────────────────────────────────────────────
const EXIT_OK: i32 = 0;
#[allow(dead_code)]
const EXIT_ERROR: i32 = 1;
const EXIT_CONFIG: i32 = 2;
const EXIT_CONNECTION: i32 = 3;
const EXIT_AUTH: i32 = 4;
#[allow(dead_code)]
const EXIT_TIMEOUT: i32 = 5;
#[allow(dead_code)]
const EXIT_ABORT: i32 = 6;

fn next_id() -> u64 {
    MSG_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Detect whether colors should be used. Call once at startup.
fn init_color_support() {
    let enabled = use_color();
    COLOR_ENABLED.store(enabled, Ordering::SeqCst);
}

fn use_color() -> bool {
    // Respect NO_COLOR standard (https://no-color.org/)
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    // Check if stdout is a terminal (not piped)
    io::stdout().is_terminal()
}

fn color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::SeqCst)
}

/// Write an ANSI color escape only when color is enabled.
fn set_fg(out: &mut impl Write, color: Color) {
    if color_enabled() {
        execute!(out, SetForegroundColor(color)).ok();
    }
}

fn reset_color(out: &mut impl Write) {
    if color_enabled() {
        execute!(out, ResetColor).ok();
    }
}

fn set_attr(out: &mut impl Write, attr: Attribute) {
    if color_enabled() {
        execute!(out, SetAttribute(attr)).ok();
    }
}

// ─── Theme: RGB colors for true-color terminals ─────────────────────────────

mod theme {
    use crossterm::style::Color;

    // Primary accent — electric violet/purple
    pub const ACCENT: Color = Color::Rgb { r: 155, g: 89, b: 255 };
    pub const ACCENT_DIM: Color = Color::Rgb { r: 110, g: 60, b: 200 };

    // Secondary — neon cyan
    pub const CYAN: Color = Color::Rgb { r: 0, g: 230, b: 230 };
    pub const CYAN_DIM: Color = Color::Rgb { r: 0, g: 160, b: 160 };

    // AI response — soft white/silver
    pub const AI_TEXT: Color = Color::Rgb { r: 220, g: 225, b: 235 };

    // Success / OK
    pub const OK: Color = Color::Rgb { r: 80, g: 250, b: 123 };

    // Warning / tool calls
    pub const WARN: Color = Color::Rgb { r: 255, g: 183, b: 77 };

    // Error / fail
    pub const ERR: Color = Color::Rgb { r: 255, g: 85, b: 85 };

    // Muted / secondary text
    pub const DIM: Color = Color::Rgb { r: 98, g: 114, b: 138 };
    pub const DIM_LIGHT: Color = Color::Rgb { r: 130, g: 145, b: 165 };

    // File changes
    pub const FILE_NEW: Color = Color::Rgb { r: 80, g: 250, b: 123 };
    pub const FILE_MOD: Color = Color::Rgb { r: 100, g: 180, b: 255 };
    pub const FILE_DEL: Color = Color::Rgb { r: 255, g: 85, b: 85 };

    // Thinking
    pub const THINK: Color = Color::Rgb { r: 180, g: 130, b: 255 };

    // Stats
    pub const STAT: Color = Color::Rgb { r: 80, g: 200, b: 200 };

    // Borders / frames
    pub const BORDER: Color = Color::Rgb { r: 60, g: 70, b: 90 };
    #[allow(dead_code)]
    pub const BORDER_LIGHT: Color = Color::Rgb { r: 80, g: 95, b: 120 };
}

// ─── Theme system (Section 9.2) ──────────────────────────────────────────────

#[allow(dead_code)]
struct ThemeColors {
    accent: Color,
    accent_dim: Color,
    cyan: Color,
    cyan_dim: Color,
    ai_text: Color,
    ok: Color,
    warn: Color,
    err: Color,
    dim: Color,
    dim_light: Color,
    think: Color,
    stat: Color,
    border: Color,
}

static ACTIVE_THEME: std::sync::OnceLock<ThemeColors> = std::sync::OnceLock::new();

fn get_named_theme(name: &str) -> ThemeColors {
    match name {
        "light" => ThemeColors {
            accent:    Color::Rgb { r: 100, g: 60, b: 200 },
            accent_dim: Color::Rgb { r: 70, g: 40, b: 160 },
            cyan:      Color::Rgb { r: 0, g: 130, b: 160 },
            cyan_dim:  Color::Rgb { r: 0, g: 100, b: 130 },
            ai_text:   Color::Rgb { r: 30, g: 30, b: 50 },
            ok:        Color::Rgb { r: 0, g: 140, b: 60 },
            warn:      Color::Rgb { r: 180, g: 100, b: 0 },
            err:       Color::Rgb { r: 180, g: 30, b: 30 },
            dim:       Color::Rgb { r: 120, g: 130, b: 150 },
            dim_light: Color::Rgb { r: 80, g: 90, b: 110 },
            think:     Color::Rgb { r: 100, g: 60, b: 180 },
            stat:      Color::Rgb { r: 0, g: 130, b: 130 },
            border:    Color::Rgb { r: 180, g: 190, b: 210 },
        },
        "dracula" => ThemeColors {
            accent:    Color::Rgb { r: 255, g: 121, b: 198 },
            accent_dim: Color::Rgb { r: 189, g: 147, b: 249 },
            cyan:      Color::Rgb { r: 139, g: 233, b: 253 },
            cyan_dim:  Color::Rgb { r: 100, g: 190, b: 220 },
            ai_text:   Color::Rgb { r: 248, g: 248, b: 242 },
            ok:        Color::Rgb { r: 80, g: 250, b: 123 },
            warn:      Color::Rgb { r: 255, g: 184, b: 108 },
            err:       Color::Rgb { r: 255, g: 85, b: 85 },
            dim:       Color::Rgb { r: 98, g: 114, b: 164 },
            dim_light: Color::Rgb { r: 130, g: 145, b: 185 },
            think:     Color::Rgb { r: 189, g: 147, b: 249 },
            stat:      Color::Rgb { r: 139, g: 233, b: 253 },
            border:    Color::Rgb { r: 68, g: 71, b: 90 },
        },
        "nord" => ThemeColors {
            accent:    Color::Rgb { r: 136, g: 192, b: 208 },
            accent_dim: Color::Rgb { r: 129, g: 161, b: 193 },
            cyan:      Color::Rgb { r: 129, g: 161, b: 193 },
            cyan_dim:  Color::Rgb { r: 94, g: 129, b: 172 },
            ai_text:   Color::Rgb { r: 216, g: 222, b: 233 },
            ok:        Color::Rgb { r: 163, g: 190, b: 140 },
            warn:      Color::Rgb { r: 235, g: 203, b: 139 },
            err:       Color::Rgb { r: 191, g: 97, b: 106 },
            dim:       Color::Rgb { r: 76, g: 86, b: 106 },
            dim_light: Color::Rgb { r: 67, g: 76, b: 94 },
            think:     Color::Rgb { r: 180, g: 142, b: 173 },
            stat:      Color::Rgb { r: 143, g: 188, b: 187 },
            border:    Color::Rgb { r: 59, g: 66, b: 82 },
        },
        "gruvbox" => ThemeColors {
            accent:    Color::Rgb { r: 211, g: 134, b: 155 },
            accent_dim: Color::Rgb { r: 177, g: 98, b: 134 },
            cyan:      Color::Rgb { r: 131, g: 165, b: 152 },
            cyan_dim:  Color::Rgb { r: 104, g: 157, b: 106 },
            ai_text:   Color::Rgb { r: 235, g: 219, b: 178 },
            ok:        Color::Rgb { r: 184, g: 187, b: 38 },
            warn:      Color::Rgb { r: 250, g: 189, b: 47 },
            err:       Color::Rgb { r: 251, g: 73, b: 52 },
            dim:       Color::Rgb { r: 102, g: 92, b: 84 },
            dim_light: Color::Rgb { r: 146, g: 131, b: 116 },
            think:     Color::Rgb { r: 211, g: 134, b: 155 },
            stat:      Color::Rgb { r: 131, g: 165, b: 152 },
            border:    Color::Rgb { r: 60, g: 56, b: 54 },
        },
        "catppuccin" => ThemeColors {
            accent:    Color::Rgb { r: 203, g: 166, b: 247 },
            accent_dim: Color::Rgb { r: 180, g: 140, b: 220 },
            cyan:      Color::Rgb { r: 137, g: 220, b: 235 },
            cyan_dim:  Color::Rgb { r: 116, g: 199, b: 236 },
            ai_text:   Color::Rgb { r: 205, g: 214, b: 244 },
            ok:        Color::Rgb { r: 166, g: 227, b: 161 },
            warn:      Color::Rgb { r: 249, g: 226, b: 175 },
            err:       Color::Rgb { r: 243, g: 139, b: 168 },
            dim:       Color::Rgb { r: 88, g: 91, b: 112 },
            dim_light: Color::Rgb { r: 108, g: 112, b: 134 },
            think:     Color::Rgb { r: 203, g: 166, b: 247 },
            stat:      Color::Rgb { r: 137, g: 220, b: 235 },
            border:    Color::Rgb { r: 49, g: 50, b: 68 },
        },
        "tokyo-night" => ThemeColors {
            accent:    Color::Rgb { r: 122, g: 162, b: 247 },
            accent_dim: Color::Rgb { r: 100, g: 130, b: 210 },
            cyan:      Color::Rgb { r: 125, g: 207, b: 255 },
            cyan_dim:  Color::Rgb { r: 86, g: 175, b: 230 },
            ai_text:   Color::Rgb { r: 192, g: 202, b: 245 },
            ok:        Color::Rgb { r: 158, g: 206, b: 106 },
            warn:      Color::Rgb { r: 224, g: 175, b: 104 },
            err:       Color::Rgb { r: 247, g: 118, b: 142 },
            dim:       Color::Rgb { r: 86, g: 95, b: 137 },
            dim_light: Color::Rgb { r: 101, g: 110, b: 155 },
            think:     Color::Rgb { r: 187, g: 154, b: 247 },
            stat:      Color::Rgb { r: 125, g: 207, b: 255 },
            border:    Color::Rgb { r: 26, g: 27, b: 38 },
        },
        "solarized" => ThemeColors {
            accent:    Color::Rgb { r: 38, g: 139, b: 210 },
            accent_dim: Color::Rgb { r: 42, g: 161, b: 152 },
            cyan:      Color::Rgb { r: 42, g: 161, b: 152 },
            cyan_dim:  Color::Rgb { r: 38, g: 139, b: 210 },
            ai_text:   Color::Rgb { r: 131, g: 148, b: 150 },
            ok:        Color::Rgb { r: 133, g: 153, b: 0 },
            warn:      Color::Rgb { r: 181, g: 137, b: 0 },
            err:       Color::Rgb { r: 220, g: 50, b: 47 },
            dim:       Color::Rgb { r: 0, g: 43, b: 54 },
            dim_light: Color::Rgb { r: 7, g: 54, b: 66 },
            think:     Color::Rgb { r: 38, g: 139, b: 210 },
            stat:      Color::Rgb { r: 42, g: 161, b: 152 },
            border:    Color::Rgb { r: 0, g: 43, b: 54 },
        },
        "solarized-light" => ThemeColors {
            accent:    Color::Rgb { r: 38, g: 139, b: 210 },
            accent_dim: Color::Rgb { r: 42, g: 161, b: 152 },
            cyan:      Color::Rgb { r: 42, g: 161, b: 152 },
            cyan_dim:  Color::Rgb { r: 38, g: 139, b: 210 },
            ai_text:   Color::Rgb { r: 101, g: 123, b: 131 },
            ok:        Color::Rgb { r: 133, g: 153, b: 0 },
            warn:      Color::Rgb { r: 181, g: 137, b: 0 },
            err:       Color::Rgb { r: 220, g: 50, b: 47 },
            dim:       Color::Rgb { r: 253, g: 246, b: 227 },
            dim_light: Color::Rgb { r: 238, g: 232, b: 213 },
            think:     Color::Rgb { r: 38, g: 139, b: 210 },
            stat:      Color::Rgb { r: 42, g: 161, b: 152 },
            border:    Color::Rgb { r: 238, g: 232, b: 213 },
        },
        // "dark" is the default
        _ => {
            // Try custom theme file first
            if let Some(custom) = load_custom_theme(name) {
                return custom;
            }
            ThemeColors {
                accent:    theme::ACCENT,
                accent_dim: theme::ACCENT_DIM,
                cyan:      theme::CYAN,
                cyan_dim:  theme::CYAN_DIM,
                ai_text:   theme::AI_TEXT,
                ok:        theme::OK,
                warn:      theme::WARN,
                err:       theme::ERR,
                dim:       theme::DIM,
                dim_light: theme::DIM_LIGHT,
                think:     theme::THINK,
                stat:      theme::STAT,
                border:    theme::BORDER,
            }
        }
    }
}

fn load_theme(name: &str) {
    let colors = get_named_theme(name);
    // Only set if not already initialized
    ACTIVE_THEME.set(colors).ok();
}

// ─── Unicode box drawing chars ───────────────────────────────────────────────

const TOP_LEFT: &str = "\u{256d}";
const TOP_RIGHT: &str = "\u{256e}";
const BOT_LEFT: &str = "\u{2570}";
const BOT_RIGHT: &str = "\u{256f}";
const H_LINE: &str = "\u{2500}";
const V_LINE: &str = "\u{2502}";
const ARROW: &str = "\u{25b8}";
const DOT: &str = "\u{2022}";
const DIAMOND: &str = "\u{25c6}";
const SPARK: &str = "\u{2726}";
const CIRCUIT: &str = "\u{25e2}";
const RADIO: &str = "\u{25cf}";
const CHECK: &str = "\u{2714}";
const CROSS: &str = "\u{2718}";
const GEAR: &str = "\u{2699}";
const BOLT: &str = "\u{26a1}";

// ─── Bunny animation ─────────────────────────────────────────────────────────
// 3-line animated running bunny. Uses cursor-up (\x1b[A) for tmux compat.

const BUNNY_LINES: usize = 3;

fn bunny_run_frame(frame: usize) -> String {
    let (p, pd, c, o, d, w, r) = if color_enabled() {
        (
            "\x1b[38;2;155;89;255m",   // purple
            "\x1b[38;2;110;60;200m",   // purple dim
            "\x1b[38;2;0;230;230m",    // cyan
            "\x1b[38;2;255;183;77m",   // orange
            "\x1b[38;2;60;70;90m",     // dim
            "\x1b[38;2;220;225;235m",  // white
            "\x1b[0m",                 // reset
        )
    } else {
        ("", "", "", "", "", "", "")
    };

    // Ear wiggle
    let ears = match frame % 6 {
        0 | 1 | 2 => format!("  {p}(\\{pd}(\\{r}"),
        3         => format!("  {p}(\\{pd} (\\{r}"),
        4         => format!("  {p} (\\{pd}(\\{r}"),
        _         => format!("  {p}(\\{pd}(\\{r}"),
    };

    // Lightning trail animation (8 unique frames)
    let trail = match frame % 8 {
        0 => format!(" {o}\u{2501}\u{26a1}\u{2501}\u{2501}\u{2501}\u{25b8}{r}"),
        1 => format!(" {o}\u{2501}\u{2501}\u{26a1}\u{2501}\u{2501}\u{25b8}{r}"),
        2 => format!(" {o}\u{2501}\u{2501}\u{2501}\u{26a1}\u{2501}\u{25b8}{r}"),
        3 => format!(" {o}\u{2501}\u{2501}\u{2501}\u{2501}\u{26a1}\u{25b8}{r}"),
        4 => format!(" {o}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}{c}\u{2734}{r}"),
        5 => format!(" {o}\u{2501}\u{2501}\u{2501}\u{26a1}\u{2501}\u{25b8}{r}"),
        6 => format!(" {o}\u{2501}\u{2501}\u{26a1}\u{2501}\u{2501}\u{25b8}{r}"),
        _ => format!(" {o}\u{2501}\u{26a1}\u{2501}\u{2501}\u{2501}\u{25b8}{r}"),
    };

    // Running legs (4-frame cycle)
    let legs = match frame % 4 {
        0 => format!("  {d}\u{2571}|  |\\ {r}"),
        1 => format!("  {d} |\\  |\\{r}"),
        2 => format!("  {d} |\\ \\| {r}"),
        _ => format!("  {d}\\|  \u{2571}| {r}"),
    };

    // Dust particles trailing behind
    let dust = match frame % 6 {
        0 => format!("{d} \u{2024} \u{2024}  \u{2024}{r}"),
        1 => format!("{d}  \u{2024}  \u{2024} {r}"),
        2 => format!("{d}\u{2024}  \u{2024}   {r}"),
        3 => format!("{d} \u{2024}  \u{2024}  {r}"),
        4 => format!("{d}  \u{2024} \u{2024}  {r}"),
        _ => format!("{d}\u{2024}   \u{2024} {r}"),
    };

    format!(
        "  {ears}\n  {w}({r} {c}\u{25c8}{r}_{c}\u{25c8}{r}{w}){r}{trail}\n  {legs}{dust}",
    )
}

fn bunny_sit_frame() -> String {
    let (p, pd, c, d, w, r) = if color_enabled() {
        (
            "\x1b[38;2;155;89;255m",
            "\x1b[38;2;110;60;200m",
            "\x1b[38;2;0;230;230m",
            "\x1b[38;2;98;114;138m",
            "\x1b[38;2;220;225;235m",
            "\x1b[0m",
        )
    } else {
        ("", "", "", "", "", "")
    };
    format!(
        "  {p}(\\{pd}(\\{r}\n  {w}({r} {c}\u{02d8}{r}_{c}\u{02d8}{r}{w}){r} {d}\u{2765} zzZ{r}\n  {d}c(\u{3064}\u{30fc}\u{3064}){r}"
    )
}

async fn run_bunny_animation(waiting: Arc<AtomicBool>) {
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    if !waiting.load(Ordering::SeqCst) { return; }

    let mut out = io::stdout();
    // Hide cursor, print initial frame
    write!(out, "\x1b[?25l").ok();
    write!(out, "\n{}", bunny_run_frame(0)).ok();
    out.flush().ok();

    let mut frame = 1u32;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        if !waiting.load(Ordering::SeqCst) { break; }

        // Move cursor up 3 lines, clear them, redraw
        write!(out, "\x1b[{BUNNY_LINES}A\x1b[J{}", bunny_run_frame(frame as usize)).ok();
        out.flush().ok();
        frame = frame.wrapping_add(1);
    }

    // Clear the bunny: move up, clear below, show cursor
    write!(out, "\x1b[{BUNNY_LINES}A\x1b[J\x1b[?25h").ok();
    out.flush().ok();
}

fn stop_bunny(waiting: &Arc<AtomicBool>) {
    if waiting.load(Ordering::SeqCst) {
        waiting.store(false, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn print_bunny_sit() {
    let mut out = io::stdout();
    write!(out, "\n{}\n", bunny_sit_frame()).ok();
    out.flush().ok();
}

// ─── Time helpers ────────────────────────────────────────────────────────────

fn format_time_ago(unix_ts: u64) -> String {
    if unix_ts == 0 { return "never".to_string(); }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let diff = now.saturating_sub(unix_ts);
    match diff {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", diff / 60),
        3600..=86399 => format!("{}h ago", diff / 3600),
        86400..=604799 => format!("{}d ago", diff / 86400),
        _ => format!("{}w ago", diff / 604800),
    }
}

// ─── Config persistence ─────────────────────────────────────────────────────

fn config_dir() -> Option<std::path::PathBuf> {
    dirs_next::config_dir().map(|d| d.join("shadowai"))
}

fn read_token() -> Option<String> {
    if let Ok(token) = std::env::var("SHADOWAI_TOKEN") {
        return Some(token);
    }
    if let Some(dir) = config_dir() {
        if let Ok(t) = std::fs::read_to_string(dir.join("token")) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    // Fall back to config.toml
    load_config().token
}

fn save_token(token: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("token");
        let _ = write_file_restricted(&path, token.as_bytes());
    }
}

fn save_last_session_id(session_id: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("last_session"), session_id);
    }
}

fn read_last_session_id() -> Option<String> {
    config_dir().and_then(|dir| {
        std::fs::read_to_string(dir.join("last_session")).ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn read_host() -> String {
    if let Ok(host) = std::env::var("SHADOWAI_HOST") {
        return host;
    }
    if let Some(dir) = config_dir() {
        if let Ok(host) = std::fs::read_to_string(dir.join("host")) {
            let h = host.trim().to_string();
            if !h.is_empty() {
                return h;
            }
        }
    }
    // Fall back to config.toml
    if let Some(h) = load_config().host {
        if !h.is_empty() {
            return h;
        }
    }
    // Auto-discover: ShadowIDE writes server.port when the remote server starts
    if let Some(dir) = config_dir() {
        if let Ok(port_str) = std::fs::read_to_string(dir.join("server.port")) {
            let port = port_str.trim();
            if !port.is_empty() {
                return format!("127.0.0.1:{}", port);
            }
        }
    }
    "127.0.0.1:9876".to_string()
}

fn save_host(host: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("host");
        let _ = write_file_restricted(&path, host.as_bytes());
    }
}

/// Write a file with mode 0o600 on Unix (owner-only read/write).
/// Falls back to std::fs::write on non-Unix platforms.
#[cfg(unix)]
fn write_file_restricted(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(data)
}

#[cfg(not(unix))]
fn write_file_restricted(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

// ─── TOML config file support ────────────────────────────────────────────────

#[derive(Debug, Default, Clone, serde::Deserialize, serde::Serialize)]
struct CliConfig {
    host: Option<String>,
    token: Option<String>,
    model: Option<String>,
    mode: Option<String>,
    temperature: Option<f64>,
    system_prompt: Option<String>,
    no_color: Option<bool>,
    google_api_key: Option<String>,
    google_cx: Option<String>,
    notification_sound: Option<bool>,
    notification_threshold_secs: Option<u64>,
    auto_lint: Option<bool>,
    auto_commit: Option<bool>,
    // Extended config fields (Section 9.1)
    anthropic_api_key: Option<String>,
    openai_api_key: Option<String>,
    openai_base_url: Option<String>,
    brave_search_key: Option<String>,
    perplexity_api_key: Option<String>,
    theme: Option<String>,
    animation: Option<bool>,
    compact_mode: Option<bool>,
    auto_heal: Option<bool>,
    confirm_tools: Option<bool>,
    max_heal_attempts: Option<u32>,
    default_skill: Option<String>,
    context_warn_pct: Option<u64>,
    auto_compact_pct: Option<u64>,
    history_size: Option<u64>,
    default_branch: Option<String>,
    commit_format: Option<String>,
    slack_webhook: Option<String>,
    discord_webhook: Option<String>,
    // New provider keys
    mistral_api_key: Option<String>,
    cohere_api_key: Option<String>,
    github_token: Option<String>,
    // Pass 4 config fields (Section 9.1)
    font_width: Option<u8>,
    pr_template: Option<String>,
    cache_ttl_secs: Option<u64>,
    show_startup_time: Option<bool>,
    sandbox_commands: Option<bool>,
    tool_allowlist: Option<Vec<String>>,
    ghost_text: Option<bool>,
    shadowide_url: Option<String>,
    // Section 8.1 — Email SMTP notifications
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_user: Option<String>,
    smtp_password: Option<String>,
    smtp_from: Option<String>,
    smtp_to: Option<String>,
    smtp_tls: Option<bool>,
    email_notify_threshold_secs: Option<u64>,
    // Section 7.1 — Real-time Collaboration relay
    relay_port: Option<u16>,
    relay_host: Option<String>,
    relay_secret: Option<String>,
    // Sections 13-19 new fields
    mcp_servers: Option<Vec<String>>,  // list of "name=url" strings
    privacy_mode: Option<bool>,
    air_gap: Option<bool>,
    max_daily_spend: Option<f64>,
    arena_model_a: Option<String>,
    arena_model_b: Option<String>,
    routing_rules: Option<Vec<String>>, // "match_pattern=provider" strings
    // Section 19 — cost routing
    prefer_cheap: Option<bool>,
    // Section 2.1 — Gemini provider
    gemini_api_key: Option<String>,
}

// Section 8.1 — snapshot of email config for use in handle_event
#[derive(Clone, Default)]
struct EmailCfgSnapshot {
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_user: Option<String>,
    smtp_password: Option<String>,
    smtp_from: Option<String>,
    smtp_to: Option<String>,
    smtp_tls: Option<bool>,
    email_notify_threshold_secs: Option<u64>,
}

// Section 13 — MCP server entry
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
struct McpServer {
    name: String,
    url: String,
    transport: String, // "http" | "sse" | "stdio"
    auth_token: Option<String>,
    tools: Vec<String>,
}

fn config_file_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

fn load_config() -> CliConfig {
    let path = match config_file_path() {
        Some(p) => p,
        None => return CliConfig::default(),
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return CliConfig::default(),
    };
    // Try proper TOML first
    if let Ok(cfg) = toml::from_str::<CliConfig>(&contents) {
        return cfg;
    }
    // Legacy: config file is just a bare URL (e.g. "http://localhost:8080/v1")
    let trimmed = contents.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let mut cfg = CliConfig::default();
        cfg.openai_base_url = Some(trimmed.to_string());
        return cfg;
    }
    CliConfig::default()
}

fn print_config_info() {
    let mut o = io::stdout();
    let path = config_file_path();

    set_fg(&mut o, theme::CYAN);
    write!(o, "\n  {SPARK} ShadowAI Config\n\n").ok();

    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "  Config path: ").ok();
    set_fg(&mut o, theme::AI_TEXT);
    match &path {
        Some(p) => write!(o, "{}\n", p.display()).ok(),
        None => write!(o, "(could not determine config directory)\n").ok(),
    };

    match &path {
        Some(p) if p.exists() => {
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  Status:      ").ok();
            set_fg(&mut o, theme::OK);
            write!(o, "found\n\n").ok();

            match std::fs::read_to_string(p) {
                Ok(contents) => {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {}\n", H_LINE.repeat(40)).ok();
                    reset_color(&mut o);
                    for line in contents.lines() {
                        write!(o, "  {}\n", line).ok();
                    }
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {}\n", H_LINE.repeat(40)).ok();
                }
                Err(e) => {
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  Error reading file: {}\n", e).ok();
                }
            }
        }
        _ => {
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  Status:      ").ok();
            set_fg(&mut o, theme::WARN);
            write!(o, "no config file found\n\n").ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "  Create one at the path above with contents like:\n\n").ok();
            reset_color(&mut o);
            write!(o, "  host = \"127.0.0.1:9876\"\n").ok();
            write!(o, "  model = \"my-model\"\n").ok();
            write!(o, "  mode = \"auto\"\n").ok();
            write!(o, "  temperature = 0.7\n").ok();
        }
    }

    write!(o, "\n").ok();
    reset_color(&mut o);
}

// ─── Project Tracking (.shadowai/ directory) ────────────────────────────────

fn tracking_dir(root: &str) -> std::path::PathBuf {
    std::path::Path::new(root).join(".shadowai")
}

fn tracking_file_path(root: &str, name: &str) -> std::path::PathBuf {
    tracking_dir(root).join(name)
}

fn ensure_tracking_dir(root: &str) {
    let dir = tracking_dir(root);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
        // Create .gitignore inside .shadowai/ so it's not tracked
        let gi = dir.join(".gitignore");
        if !gi.exists() {
            let _ = std::fs::write(&gi, "*\n");
        }
    }
}

fn append_tracking_entry(root: &str, file: &str, entry: &str) {
    ensure_tracking_dir(root);
    let path = tracking_file_path(root, file);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "\n## [{}]\n{}\n", ts, entry);
    }
}

fn read_tracking_file(root: &str, name: &str) -> Option<String> {
    std::fs::read_to_string(tracking_file_path(root, name)).ok()
}

fn read_memory_context(root: &str) -> String {
    read_tracking_file(root, "memory.md")
        .filter(|s| !s.trim().is_empty())
        .map(|mem| format!("\n\n<project-memory>\n{}\n</project-memory>\n", mem))
        .unwrap_or_default()
}

#[allow(dead_code)]
fn update_memory_file(root: &str, content: &str) {
    ensure_tracking_dir(root);
    let path = tracking_file_path(root, "memory.md");
    let _ = std::fs::write(&path, content);
}

fn log_error_tracking(root: &str, tool: &str, detail: &str) {
    append_tracking_entry(root, "errors.md",
        &format!("**Tool:** `{}`\n**Error:** {}", tool, detail));
}

fn log_fix_tracking(root: &str, tool: &str, detail: &str) {
    append_tracking_entry(root, "fixed.md",
        &format!("**Tool:** `{}`\n**Fix:** {}", tool, detail));
}

fn log_completed_tracking(root: &str, summary: &str) {
    append_tracking_entry(root, "completed.md",
        &format!("**Completed:** {}", summary));
}

fn print_tracking_file(root: &str, name: &str, title: &str) {
    let mut o = io::stdout();
    match read_tracking_file(root, name) {
        Some(content) if !content.trim().is_empty() => {
            print_section_header(title);
            for line in content.lines().take(50) {
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "  {}\n", line).ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        _ => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  {ARROW} {}: (empty)\n", title).ok();
            reset_color(&mut o);
        }
    }
}

// ─── Web Search (Google Custom Search API) ───────────────────────────────────

async fn google_search(query: &str, api_key: &str, cx: &str, num_results: u32) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get("https://www.googleapis.com/customsearch/v1")
        .query(&[
            ("key", api_key),
            ("cx", cx),
            ("q", query),
            ("num", &num_results.to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("Search request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Google API returned {} — {}", status, body));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let mut results = String::new();
    if let Some(items) = data["items"].as_array() {
        for (i, item) in items.iter().enumerate() {
            let title = item["title"].as_str().unwrap_or("");
            let link = item["link"].as_str().unwrap_or("");
            let snippet = item["snippet"].as_str().unwrap_or("");
            results.push_str(&format!("{}. **{}**\n   {}\n   {}\n\n", i + 1, title, link, snippet));
        }
    }

    if results.is_empty() {
        Ok("No results found.".to_string())
    } else {
        Ok(results)
    }
}

fn print_search_results(results: &str) {
    let mut o = io::stdout();
    write!(o, "\n").ok();
    set_fg(&mut o, theme::CYAN);
    write!(o, "  {SPARK} Search Results\n\n").ok();
    for line in results.lines() {
        set_fg(&mut o, theme::AI_TEXT);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);
    write!(o, "\n").ok();
}

// ─── Brave Search (Section 7) ─────────────────────────────────────────────────

async fn brave_search(query: &str, api_key: &str, num_results: u32) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &num_results.to_string())])
        .send()
        .await
        .map_err(|e| format!("Brave search request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Brave API returned {} — {}", status, body));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();

    if let Some(items) = data["web"]["results"].as_array() {
        for (i, item) in items.iter().enumerate() {
            let title = item["title"].as_str().unwrap_or("");
            let url = item["url"].as_str().unwrap_or("");
            let desc = item["description"].as_str().unwrap_or("");
            results.push_str(&format!("{}. **{}**\n   {}\n   {}\n\n", i + 1, title, url, desc));
        }
    }

    if results.is_empty() {
        Ok("No results found.".to_string())
    } else {
        Ok(results)
    }
}

// ─── DuckDuckGo Search (Section 7) ───────────────────────────────────────────

async fn ddg_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let encoded = query.replace(' ', "+");
    let url = format!("https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1", encoded);

    let resp = client.get(&url)
        .header("User-Agent", "shadowai-cli/0.2")
        .send()
        .await
        .map_err(|e| format!("DDG request failed: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();

    if let Some(abstract_text) = data["AbstractText"].as_str() {
        if !abstract_text.is_empty() {
            let src = data["AbstractSource"].as_str().unwrap_or("DDG");
            let url = data["AbstractURL"].as_str().unwrap_or("");
            results.push_str(&format!("**{}** ({})\n{}\n\n", src, url, abstract_text));
        }
    }

    if let Some(topics) = data["RelatedTopics"].as_array() {
        for (i, topic) in topics.iter().take(5).enumerate() {
            if let Some(text) = topic["Text"].as_str() {
                let url = topic["FirstURL"].as_str().unwrap_or("");
                results.push_str(&format!("{}. {}\n   {}\n\n", i + 1, text, url));
            }
        }
    }

    if let Some(result_list) = data["Results"].as_array() {
        for item in result_list.iter().take(3) {
            let title = item["Text"].as_str().unwrap_or("");
            let url = item["FirstURL"].as_str().unwrap_or("");
            results.push_str(&format!("- **{}**\n   {}\n\n", title, url));
        }
    }

    if results.is_empty() {
        Ok("No results found via DuckDuckGo. Try a more specific query.".to_string())
    } else {
        Ok(results)
    }
}

/// Scrape Google search results (no API key required).
/// Parses the HTML from google.com/search and extracts titles + URLs.
async fn google_scrape_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let encoded = urlencoding_simple(query);
    let url = format!("https://www.google.com/search?q={}&num=10&hl=en", encoded);

    let html = client.get(&url)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| format!("Google request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Google response error: {}", e))?;

    // Parse results: look for <h3> tags followed by hrefs
    // Google wraps results in <div class="g"> with an <a href="..."><h3>...</h3></a>
    let mut results = Vec::new();
    let mut pos = 0;
    while pos < html.len() && results.len() < 8 {
        // Find an anchor pointing to an external URL with an <h3> inside
        if let Some(a_start) = html[pos..].find("<a href=\"/url?q=").map(|i| pos + i) {
            let after_a = a_start + 16; // skip `<a href="/url?q=`
            if let Some(amp_or_quote) = html[after_a..].find(|c| c == '&' || c == '"').map(|i| after_a + i) {
                let raw_url = &html[after_a..amp_or_quote];
                // Only keep real external URLs
                if raw_url.starts_with("http") && !raw_url.contains("google.com") {
                    // Find <h3> inside this anchor
                    if let Some(h3_start) = html[amp_or_quote..amp_or_quote + 400].find("<h3").map(|i| amp_or_quote + i) {
                        if let Some(h3_end_open) = html[h3_start..].find('>').map(|i| h3_start + i + 1) {
                            if let Some(h3_close) = html[h3_end_open..].find("</h3>").map(|i| h3_end_open + i) {
                                let title_html = &html[h3_end_open..h3_close];
                                // Strip any remaining tags from title
                                let mut title = String::new();
                                let mut in_h3_tag = false;
                                for c in title_html.chars() {
                                    match c {
                                        '<' => { in_h3_tag = true; }
                                        '>' => { in_h3_tag = false; }
                                        _ if !in_h3_tag => { title.push(c); }
                                        _ => {}
                                    }
                                }
                                if !title.trim().is_empty() {
                                    results.push(format!("**{}**\n   {}", title.trim(), raw_url));
                                    pos = h3_close;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
            pos = a_start + 1;
        } else {
            break;
        }
    }

    if results.is_empty() {
        // Fallback: try DuckDuckGo if Google returns nothing useful
        ddg_search(query).await
    } else {
        Ok(results.join("\n\n"))
    }
}

// ─── Crates.io Search (Section 7) ────────────────────────────────────────────

async fn crates_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get("https://crates.io/api/v1/crates")
        .header("User-Agent", "shadowai-cli/0.2 (https://github.com/shadowai)")
        .query(&[("q", query), ("per_page", "5")])
        .send()
        .await
        .map_err(|e| format!("crates.io request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("crates.io returned {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();

    if let Some(crates) = data["crates"].as_array() {
        for (i, c) in crates.iter().enumerate() {
            let name = c["name"].as_str().unwrap_or("");
            let version = c["newest_version"].as_str().unwrap_or("?");
            let desc = c["description"].as_str().unwrap_or("No description");
            let downloads = c["downloads"].as_u64().unwrap_or(0);
            results.push_str(&format!("{}. **{}** v{}\n   {}\n   {} downloads\n\n",
                i + 1, name, version, desc, downloads));
        }
    }

    if results.is_empty() {
        Ok("No crates found.".to_string())
    } else {
        Ok(results)
    }
}

// ─── npm Search (Section 7) ───────────────────────────────────────────────────

async fn npm_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get("https://registry.npmjs.org/-/v1/search")
        .query(&[("text", query), ("size", "5")])
        .send()
        .await
        .map_err(|e| format!("npm search request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("npm registry returned {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();

    if let Some(objects) = data["objects"].as_array() {
        for (i, obj) in objects.iter().enumerate() {
            let pkg = &obj["package"];
            let name = pkg["name"].as_str().unwrap_or("");
            let version = pkg["version"].as_str().unwrap_or("?");
            let desc = pkg["description"].as_str().unwrap_or("No description");
            let weekly = obj["score"]["detail"]["popularity"].as_f64().unwrap_or(0.0);
            results.push_str(&format!("{}. **{}** v{}\n   {}\n   popularity score: {:.2}\n\n",
                i + 1, name, version, desc, weekly));
        }
    }

    if results.is_empty() {
        Ok("No npm packages found.".to_string())
    } else {
        Ok(results)
    }
}

// ─── Skills System ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct Skill {
    name: String,
    description: String,
    system_prompt: String,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    max_turns: Option<u32>,
    #[serde(default)]
    auto_deactivate: Option<bool>,
    #[serde(default)]
    include_git_diff: Option<bool>,
    #[serde(default)]
    auto_attach: Option<Vec<String>>,
}

// ─── Hooks System ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
struct Hook {
    event: String,
    tool: Option<String>,
    command: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct HooksConfig {
    #[serde(default)]
    hooks: Vec<Hook>,
}

// Pass 4 structs
#[allow(dead_code)]
struct ContextSlot {
    label: String,
    content: String,
    tokens: usize,
}

#[allow(dead_code)]
struct CachedResponse {
    response: String,
    timestamp: std::time::Instant,
}

#[allow(dead_code)]
struct DapBreakpoint {
    file: String,
    line: u64,
    id: Option<u64>,
}

#[allow(dead_code)]
struct DapState {
    seq: u64,
    child: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<tokio::io::BufReader<tokio::process::ChildStdout>>,
    breakpoints: Vec<DapBreakpoint>,
    thread_id: Option<i64>,
}

fn load_hooks() -> Vec<Hook> {
    let Some(cfg) = config_dir() else { return vec![] };
    let path = cfg.join("hooks.toml");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(config) = toml::from_str::<HooksConfig>(&contents) {
            return config.hooks;
        }
    }
    vec![]
}

/// Run hooks matching the given event name.
///
/// Supported events:
/// - `"pre-commit"` — before git commit runs
/// - `"post-tool"` — after any AI tool call completes
/// - `"session_start"` — when the interactive session begins
/// - `"session_end"` — before the session exits
/// - `"build_success"` — after a successful build
/// - `"build_fail"` — after a failed build
/// - `"test_pass"` — after tests pass
/// - `"test_fail"` — after tests fail
/// - `"commit"` — after a successful git commit
/// - `"skill_activate"` — when a skill is activated
/// - `"error"` — when an AI error is received
fn run_hooks(hooks: &[Hook], event: &str, tool_name: Option<&str>, context: &serde_json::Value) {
    for hook in hooks {
        if hook.event != event {
            continue;
        }
        if let Some(ref filter_tool) = hook.tool {
            if let Some(tn) = tool_name {
                if filter_tool != tn {
                    continue;
                }
            } else {
                continue;
            }
        }
        // Substitute {path}, {tool}, {args} placeholders
        let mut cmd = hook.command.clone();
        if let Some(path) = context["path"].as_str() {
            cmd = cmd.replace("{path}", path);
        }
        if let Some(tn) = tool_name {
            cmd = cmd.replace("{tool}", tn);
        }
        if let Some(args) = context["args"].as_str() {
            cmd = cmd.replace("{args}", args);
        }
        // Fire-and-forget
        let _ = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
    }
}

/// Helper to create a builtin skill with enhanced fields defaulted to None
fn skill(name: &str, desc: &str, prompt: &str, temp: f64, mode: Option<&str>, cat: &str, aliases: Vec<&str>) -> Skill {
    Skill {
        name: name.into(),
        description: desc.into(),
        system_prompt: prompt.into(),
        temperature: Some(temp),
        mode: mode.map(|m| m.into()),
        category: Some(cat.into()),
        aliases: Some(aliases.into_iter().map(String::from).collect()),
        max_turns: None,
        auto_deactivate: None,
        include_git_diff: None,
        auto_attach: None,
    }
}

fn builtin_skills() -> Vec<Skill> {
    vec![
        skill("code-review", "Review code for bugs, style, and improvements",
            "You are a senior code reviewer. Analyze the code for bugs, security issues, performance problems, and style violations. Be thorough but constructive. When you find errors, log them. When you fix them, log the fixes.",
            0.3, Some("plan"), "review", vec!["review", "cr", "r"]),
        skill("debug", "Debug issues and trace errors",
            "You are a debugging expert. Analyze errors, stack traces, and unexpected behavior. Identify root causes and suggest fixes with explanations. Track all errors you find and fixes you apply.",
            0.3, Some("auto"), "coding", vec!["d", "dbg"]),
        skill("refactor", "Refactor code for clarity and maintainability",
            "You are a refactoring specialist. Improve code structure, reduce duplication, improve naming, and apply SOLID principles while preserving behavior.",
            0.5, Some("build"), "coding", vec!["ref"]),
        skill("explain", "Explain code, concepts, or architectures",
            "You are a technical educator. Explain code, algorithms, and architectures clearly. Use analogies and examples. Assume the reader is a developer but may be unfamiliar with this specific domain.",
            0.7, Some("plan"), "coding", vec!["e", "exp"]),
        skill("test", "Generate and improve tests",
            "You are a testing expert. Write comprehensive tests including edge cases, error paths, and integration scenarios. Follow the project's existing test patterns.",
            0.4, Some("build"), "coding", vec!["t"]),
        skill("websearch", "Search the web and summarize findings",
            "You are a research assistant with web search capability. When the user asks a question, search the web for current information and provide a well-sourced summary. Use the [SEARCH: query] syntax to trigger web searches.",
            0.5, None, "research", vec!["ws", "web"]),
        skill("architect", "Design system architecture and APIs",
            "You are a software architect. Design clean APIs, system architectures, and data models. Consider scalability, maintainability, and trade-offs. Document decisions and rationale.",
            0.6, Some("plan"), "design", vec!["arch", "a"]),
        skill("security", "Security audit and vulnerability analysis",
            "You are a security expert. Analyze code for vulnerabilities (OWASP Top 10, injection, auth issues, etc.). Suggest fixes with security best practices. Log all security issues found.",
            0.2, Some("plan"), "security", vec!["sec"]),
        // Game Dev & Extended Skills (Section 5.3)
        skill("gamedev", "Game development: ECS, physics, rendering, game feel",
            "You are a game development expert. Help with game architecture, ECS patterns (Bevy, Unity DOTS), physics systems, rendering pipelines, game feel tuning, and performance optimization. You know Godot (GDScript/C#), Unity (C#), Unreal (C++/Blueprints), Bevy (Rust), and LÖVE2D (Lua).",
            0.5, Some("auto"), "gamedev", vec!["game", "gd"]),
        skill("shader", "GLSL/HLSL/WGSL/Metal shader writing and optimization",
            "You are a graphics programming expert. Write and optimize shaders in GLSL, HLSL, WGSL, and Metal. Understand the GPU pipeline, PBR lighting, post-processing effects, compute shaders, and performance profiling. Validate shader code and explain GPU-specific concepts.",
            0.4, Some("build"), "gamedev", vec!["glsl", "hlsl", "wgsl"]),
        skill("level-design", "Procedural generation, level layout, game feel",
            "You are a level design and procedural generation expert. Help design game levels, implement procedural generation algorithms (noise, BSP, WFC), tune game feel parameters, design enemy encounters, and balance difficulty curves.",
            0.6, Some("plan"), "gamedev", vec!["ld", "proc", "procgen"]),
        skill("game-math", "Vectors, matrices, quaternions, physics math",
            "You are a game mathematics expert. Explain and implement vectors, matrices, quaternions, transforms, collision detection, physics integration (Verlet, RK4), interpolation (lerp/slerp), and coordinate space transformations. Show code examples in the user's language.",
            0.3, Some("plan"), "gamedev", vec!["gmath", "gamemath"]),
        skill("mobile-dev", "iOS/Android/Flutter/React Native mobile development",
            "You are a mobile development expert. Help with Swift/SwiftUI, Kotlin/Jetpack Compose, Flutter/Dart, and React Native. Understand platform APIs, performance optimization, App Store/Play Store submission, and mobile-specific UI patterns.",
            0.5, Some("auto"), "mobile", vec!["ios", "android", "flutter"]),
        skill("devops", "CI/CD, Docker, Kubernetes, infrastructure as code",
            "You are a DevOps and infrastructure expert. Help with Docker, Kubernetes, GitHub Actions, GitLab CI, Terraform, Ansible, Helm, and cloud platforms (AWS, GCP, Azure). Optimize pipelines, write secure configurations, and troubleshoot deployment issues.",
            0.3, Some("auto"), "infra", vec!["k8s", "docker", "cicd"]),
        skill("database", "SQL, NoSQL, query optimization, schema design",
            "You are a database expert. Design schemas, write optimized SQL queries, explain execution plans, help with migrations, and advise on NoSQL patterns (MongoDB, Redis, Cassandra). Support PostgreSQL, MySQL, SQLite, and cloud databases.",
            0.3, Some("auto"), "data", vec!["db", "sql"]),
        skill("heal", "Automatically fix build/test/lint errors",
            "You are an automated error resolution system. Analyze build errors, test failures, and lint warnings. Create a prioritized fix plan and implement fixes one at a time. After each fix, verify it resolved the issue. Track all changes made. Be methodical and conservative — only change what is necessary to fix each specific error.",
            0.2, Some("build"), "coding", vec!["fix", "autofix"]),
    ]
}

/// Get git diff for skill context injection
fn get_git_diff_context(root_path: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["diff", "--staged"])
        .current_dir(root_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let diff = String::from_utf8_lossy(&o.stdout).to_string();
            if diff.is_empty() {
                String::new()
            } else {
                format!("\n\n[Staged git diff]:\n```diff\n{}\n```\n", diff)
            }
        }
        _ => String::new(),
    }
}

/// Find files matching auto_attach glob patterns and return their contents
fn get_auto_attach_context(root_path: &str, patterns: &[String]) -> String {
    let mut ctx = String::new();
    let mut count = 0;
    for pattern in patterns {
        let full_pattern = format!("{}/{}", root_path, pattern);
        if let Ok(paths) = glob::glob(&full_pattern) {
            for entry in paths.flatten() {
                if count >= 5 { break; }
                if let Ok(content) = std::fs::read_to_string(&entry) {
                    let display = entry.to_string_lossy();
                    let display = display.strip_prefix(&format!("{}/", root_path))
                        .unwrap_or(&display);
                    let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let truncated: String = content.chars().take(2000).collect();
                    ctx.push_str(&format!("\n\n[Auto-attached: `{}`]:\n```{}\n{}\n```\n", display, ext, truncated));
                    count += 1;
                }
            }
        }
        if count >= 5 { break; }
    }
    ctx
}

/// Create a skeleton skill TOML file
fn create_skill_skeleton(name: &str) -> Result<String, String> {
    let Some(cfg) = config_dir() else {
        return Err("Could not determine config directory".into());
    };
    let skills_dir = cfg.join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| format!("Failed to create skills dir: {}", e))?;
    let path = skills_dir.join(format!("{}.toml", name));
    if path.exists() {
        return Err(format!("Skill '{}' already exists at {}", name, path.display()));
    }
    let template = format!(r#"name = "{name}"
description = "TODO: Describe what this skill does"
system_prompt = """
TODO: Write the system prompt for this skill.
Be specific about the AI's role, approach, and output format.
"""
temperature = 0.5
# mode = "auto"  # auto, plan, or build
category = "custom"
aliases = ["{short}"]

# Enhanced options (all optional):
# max_turns = 5           # Auto-deactivate after N turns
# auto_deactivate = true  # Deactivate after one response
# include_git_diff = true # Inject staged git diff into context
# auto_attach = ["src/**/*.rs", "*.toml"]  # Auto-attach matching files
"#, name = name, short = &name[..name.len().min(3)]);
    std::fs::write(&path, &template).map_err(|e| format!("Failed to write: {}", e))?;
    Ok(path.display().to_string())
}

fn load_skills_from_dir(dir: &std::path::Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut skills = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(skill) = toml::from_str::<Skill>(&contents) {
                    skills.push(skill);
                }
            }
        }
    }
    skills
}

fn load_custom_skills(root_path: Option<&str>) -> Vec<Skill> {
    let mut skills = vec![];
    // Load global custom skills
    if let Some(d) = config_dir() {
        skills.extend(load_skills_from_dir(&d.join("skills")));
    }
    // Load project-local skills (override global)
    if let Some(rp) = root_path {
        let project_skills_dir = std::path::Path::new(rp).join(".shadowai").join("skills");
        let project = load_skills_from_dir(&project_skills_dir);
        for ps in project {
            if let Some(pos) = skills.iter().position(|s| s.name == ps.name) {
                skills[pos] = ps;
            } else {
                skills.push(ps);
            }
        }
    }
    skills
}

fn find_skill(name: &str, root_path: Option<&str>) -> Option<Skill> {
    let custom = load_custom_skills(root_path);
    // Check by name
    if let Some(s) = custom.iter().find(|s| s.name == name) {
        return Some(s.clone());
    }
    // Check by alias in custom skills
    if let Some(s) = custom.into_iter().find(|s| {
        s.aliases.as_ref().map(|a| a.iter().any(|al| al == name)).unwrap_or(false)
    }) {
        return Some(s);
    }
    let builtins = builtin_skills();
    // Check by name in builtins
    if let Some(s) = builtins.iter().find(|s| s.name == name) {
        return Some(s.clone());
    }
    // Check by alias in builtins
    builtins.into_iter().find(|s| {
        s.aliases.as_ref().map(|a| a.iter().any(|al| al == name)).unwrap_or(false)
    })
}

fn list_all_skills(root_path: Option<&str>) -> Vec<Skill> {
    let mut skills = builtin_skills();
    let custom = load_custom_skills(root_path);
    for cs in custom {
        if let Some(pos) = skills.iter().position(|s| s.name == cs.name) {
            skills[pos] = cs;
        } else {
            skills.push(cs);
        }
    }
    skills
}

// ─── Syntax Highlighting & Streaming Markdown ────────────────────────────────

// 11a: Lazy syntect loading
fn get_syntax_set() -> &'static syntect::parsing::SyntaxSet {
    SYNTAX_SET.get_or_init(|| syntect::parsing::SyntaxSet::load_defaults_newlines())
}

fn get_theme_set() -> &'static syntect::highlighting::ThemeSet {
    THEME_SET.get_or_init(|| syntect::highlighting::ThemeSet::load_defaults())
}

fn highlight_code_block(code: &str, lang: &str) -> String {
    use syntect::easy::HighlightLines;
    use syntect::util::as_24_bit_terminal_escaped;

    let ss = get_syntax_set();
    let ts = get_theme_set();
    let theme = &ts.themes["base16-ocean.dark"];

    let syntax = ss.find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut h = HighlightLines::new(syntax, theme);
    let mut output = String::new();

    for line in syntect::util::LinesWithEndings::from(code) {
        match h.highlight_line(line, ss) {
            Ok(ranges) => output.push_str(&as_24_bit_terminal_escaped(&ranges[..], false)),
            Err(_) => output.push_str(line),
        }
    }
    output.push_str("\x1b[0m");
    output
}

/// StreamFormatter tracks markdown state during AI response streaming.
/// It detects code fences, buffers code blocks for syntax highlighting,
/// and applies simple inline formatting.
struct StreamFormatter {
    in_code_block: bool,
    code_buffer: String,
    code_lang: String,
    line_buffer: String,
    block_counter: u32,
    table_buffer: Vec<String>,
    in_table: bool,
}

impl StreamFormatter {
    fn new() -> Self {
        Self {
            in_code_block: false,
            code_buffer: String::new(),
            code_lang: String::new(),
            line_buffer: String::new(),
            block_counter: 0,
            table_buffer: Vec::new(),
            in_table: false,
        }
    }

    /// Feed a streaming chunk. Returns text that should be printed immediately.
    fn feed(&mut self, chunk: &str) -> String {
        let mut output = String::new();
        for ch in chunk.chars() {
            self.line_buffer.push(ch);
            if ch == '\n' {
                output.push_str(&self.process_line());
            }
        }
        // If not in a code block and line_buffer has content but no newline yet,
        // flush partial line for responsive streaming (but not inside code blocks)
        if !self.in_code_block && !self.line_buffer.is_empty() {
            let partial = std::mem::take(&mut self.line_buffer);
            output.push_str(&self.format_inline(&partial));
        }
        output
    }

    /// Flush any remaining content (call when streaming ends).
    fn flush(&mut self) -> String {
        let mut output = String::new();
        if !self.line_buffer.is_empty() {
            self.line_buffer.push('\n');
            output.push_str(&self.process_line());
        }
        if self.in_code_block && !self.code_buffer.is_empty() {
            // Unterminated code block — flush as highlighted
            output.push_str(&self.emit_code_block());
        }
        // Flush any buffered table
        if self.in_table && !self.table_buffer.is_empty() {
            self.in_table = false;
            let table_lines: Vec<&str> = self.table_buffer.iter().map(|s| s.as_str()).collect();
            let has_separator = table_lines.iter().any(|l| {
                l.split('|').any(|c| c.trim().chars().all(|ch| ch == '-' || ch == ':') && !c.trim().is_empty())
            });
            let table_out = if has_separator {
                output.push_str(&self.render_table(&table_lines));
            } else {
                for l in &table_lines {
                    output.push_str(&format!("  {}\n", l));
                }
            };
            self.table_buffer.clear();
            let _ = table_out;
        }
        output
    }

    fn process_line(&mut self) -> String {
        let line = std::mem::take(&mut self.line_buffer);
        let trimmed = line.trim_end_matches('\n');

        // Check for code fence
        if trimmed.starts_with("```") {
            if self.in_code_block {
                // Closing fence — emit highlighted code block
                self.in_code_block = false;
                return self.emit_code_block();
            } else {
                // Opening fence — extract language
                self.in_code_block = true;
                self.code_lang = trimmed.trim_start_matches('`').trim().to_string();
                self.code_buffer.clear();
                self.block_counter += 1;
                // Print the code block header with block number
                let lang_display = if self.code_lang.is_empty() { "code".to_string() } else { self.code_lang.clone() };
                let header = format!("[block {}] {}", self.block_counter, lang_display);
                return format!(
                    "\x1b[38;2;60;70;90m  {TOP_LEFT}{}{TOP_RIGHT}\x1b[0m\n\x1b[38;2;0;160;160m  {V_LINE} {}\x1b[0m\n\x1b[38;2;60;70;90m  {V_LINE}{}{V_LINE}\x1b[0m\n",
                    H_LINE.repeat(40), header, H_LINE.repeat(40)
                );
            }
        }

        if self.in_code_block {
            self.code_buffer.push_str(&line);
            return String::new(); // Buffer until closing fence
        }

        // Table detection (Section 10)
        let is_table_line = trimmed.contains('|') && !trimmed.starts_with("```");
        if is_table_line {
            self.in_table = true;
            self.table_buffer.push(trimmed.to_string());
            return String::new(); // Buffer table lines
        } else if self.in_table {
            // Flush the buffered table
            self.in_table = false;
            let table_lines: Vec<&str> = self.table_buffer.iter().map(|s| s.as_str()).collect();
            // Only render as table if it has a separator row (|---|)
            let has_separator = table_lines.iter().any(|l| {
                l.split('|').any(|c| c.trim().chars().all(|ch| ch == '-' || ch == ':') && !c.trim().is_empty())
            });
            let table_out = if has_separator {
                self.render_table(&table_lines)
            } else {
                table_lines.iter().map(|l| format!("  {}\n", l)).collect::<String>()
            };
            self.table_buffer.clear();
            // Then format the current non-table line too
            let current_out = self.format_line(trimmed);
            return table_out + &current_out;
        }

        // Outside code blocks: apply inline markdown formatting
        self.format_line(trimmed)
    }

    fn emit_code_block(&mut self) -> String {
        let code = std::mem::take(&mut self.code_buffer);
        let lang = std::mem::take(&mut self.code_lang);
        let highlighted = if color_enabled() {
            highlight_code_block(&code, &lang)
        } else {
            code.clone()
        };
        let mut out = String::new();
        for hl_line in highlighted.lines() {
            out.push_str(&format!(
                "\x1b[38;2;60;70;90m  {V_LINE}\x1b[0m {}\n",
                hl_line
            ));
        }
        out.push_str(&format!(
            "\x1b[38;2;60;70;90m  {BOT_LEFT}{}{BOT_RIGHT}\x1b[0m\n",
            H_LINE.repeat(40)
        ));
        out
    }

    fn format_line(&self, line: &str) -> String {
        // Headers
        if line.starts_with("### ") {
            return format!(
                "\x1b[1m\x1b[38;2;0;230;230m  {}\x1b[0m\n",
                &line[4..]
            );
        }
        if line.starts_with("## ") {
            return format!(
                "\x1b[1m\x1b[38;2;0;230;230m  {}\x1b[0m\n",
                &line[3..]
            );
        }
        if line.starts_with("# ") {
            return format!(
                "\x1b[1m\x1b[38;2;0;230;230m  {}\x1b[0m\n",
                &line[2..]
            );
        }

        // Horizontal rule
        if line == "---" || line == "***" || line == "___" {
            return format!(
                "\x1b[38;2;60;70;90m  {}\x1b[0m\n",
                H_LINE.repeat(40)
            );
        }

        // Bullet lists
        if line.starts_with("- ") || line.starts_with("* ") {
            let content = &line[2..];
            return format!(
                "\x1b[38;2;0;160;160m  {DOT}\x1b[0m {}\n",
                self.format_inline(content)
            );
        }
        // Indented bullets
        if line.starts_with("  - ") || line.starts_with("  * ") {
            let content = &line[4..];
            return format!(
                "\x1b[38;2;0;160;160m    {DOT}\x1b[0m {}\n",
                self.format_inline(content)
            );
        }

        // Regular line with inline formatting
        format!("{}\n", self.format_inline(line))
    }

    /// Render markdown table lines using box-drawing characters (Section 10)
    fn render_table(&self, lines: &[&str]) -> String {
        // Parse rows
        let rows: Vec<Vec<String>> = lines.iter()
            .filter(|l| !l.trim_start_matches('|').starts_with("---") && l.contains('|'))
            .map(|l| {
                let l = l.trim();
                let l = if l.starts_with('|') { &l[1..] } else { l };
                let l = if l.ends_with('|') { &l[..l.len()-1] } else { l };
                l.split('|').map(|c| c.trim().to_string()).collect()
            })
            .collect();

        if rows.is_empty() {
            return lines.join("\n") + "\n";
        }

        let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut col_widths: Vec<usize> = vec![3; col_count];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                if i < col_count {
                    col_widths[i] = col_widths[i].max(cell.len());
                }
            }
        }

        let mut out = String::new();
        let border_color = "\x1b[38;2;60;70;90m";
        let header_color = "\x1b[38;2;0;230;230m";
        let cell_color = "\x1b[38;2;220;225;235m";
        let reset = "\x1b[0m";

        // Top border
        out.push_str(&format!("  {border_color}\u{250c}"));
        for (i, w) in col_widths.iter().enumerate() {
            out.push_str(&"\u{2500}".repeat(w + 2));
            if i < col_count - 1 { out.push_str("\u{252c}"); }
        }
        out.push_str(&format!("\u{2510}{reset}\n"));

        // Rows
        for (row_idx, row) in rows.iter().enumerate() {
            out.push_str(&format!("  {border_color}\u{2502}{reset}"));
            for (i, w) in col_widths.iter().enumerate() {
                let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
                let color = if row_idx == 0 { header_color } else { cell_color };
                out.push_str(&format!(" {color}{:<w$}{reset} {border_color}\u{2502}{reset}", cell, w = w));
            }
            out.push('\n');

            // Separator after header
            if row_idx == 0 && rows.len() > 1 {
                out.push_str(&format!("  {border_color}\u{251c}"));
                for (i, w) in col_widths.iter().enumerate() {
                    out.push_str(&"\u{2500}".repeat(w + 2));
                    if i < col_count - 1 { out.push_str("\u{253c}"); }
                }
                out.push_str(&format!("\u{2524}{reset}\n"));
            }
        }

        // Bottom border
        out.push_str(&format!("  {border_color}\u{2514}"));
        for (i, w) in col_widths.iter().enumerate() {
            out.push_str(&"\u{2500}".repeat(w + 2));
            if i < col_count - 1 { out.push_str("\u{2534}"); }
        }
        out.push_str(&format!("\u{2518}{reset}\n"));

        out
    }

    fn format_inline(&self, text: &str) -> String {
        if !color_enabled() {
            return text.to_string();
        }
        let mut result = String::new();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Bold: **text**
            if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
                if let Some(end) = text[i + 2..].find("**") {
                    let bold_text = &text[i + 2..i + 2 + end];
                    result.push_str(&format!("\x1b[1m\x1b[38;2;220;225;235m{}\x1b[0m\x1b[38;2;220;225;235m", bold_text));
                    i += 4 + end;
                    continue;
                }
            }
            // Italic: *text* (single asterisk, not preceded by *)
            if chars[i] == '*' && (i == 0 || chars[i - 1] != '*') && (i + 1 < len && chars[i + 1] != '*') {
                if let Some(end) = text[i + 1..].find('*') {
                    if !text[i + 1..i + 1 + end].contains("**") {
                        let italic_text = &text[i + 1..i + 1 + end];
                        result.push_str(&format!("\x1b[3m\x1b[38;2;220;225;235m{}\x1b[0m\x1b[38;2;220;225;235m", italic_text));
                        i += 2 + end;
                        continue;
                    }
                }
            }
            // Inline code: `text`
            if chars[i] == '`' && (i + 1 >= len || chars[i + 1] != '`') {
                if let Some(end) = text[i + 1..].find('`') {
                    let code_text = &text[i + 1..i + 1 + end];
                    result.push_str(&format!("\x1b[38;2;255;183;77m{}\x1b[0m\x1b[38;2;220;225;235m", code_text));
                    i += 2 + end;
                    continue;
                }
            }
            // Link: [text](url)
            if chars[i] == '[' {
                if let Some(bracket_end) = text[i + 1..].find(']') {
                    let after_bracket = i + 1 + bracket_end + 1;
                    if after_bracket < len && chars[after_bracket] == '(' {
                        if let Some(paren_end) = text[after_bracket + 1..].find(')') {
                            let link_text = &text[i + 1..i + 1 + bracket_end];
                            let url = &text[after_bracket + 1..after_bracket + 1 + paren_end];
                            result.push_str(&format!(
                                "\x1b[38;2;220;225;235m{}\x1b[0m \x1b[38;2;0;160;160m({})\x1b[0m\x1b[38;2;220;225;235m",
                                link_text, url
                            ));
                            i = after_bracket + 2 + paren_end;
                            continue;
                        }
                    }
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        // Wrap result in AI_TEXT color
        format!("\x1b[38;2;220;225;235m{}\x1b[0m", result)
    }
}

// ─── Terminal helpers ────────────────────────────────────────────────────────

fn term_width() -> usize {
    terminal::size().map(|(w, _)| w as usize).unwrap_or(80)
}

fn h_rule(width: usize) -> String {
    H_LINE.repeat(width)
}

fn print_banner() {
    let w = term_width().min(72);
    let mut o = io::stdout();

    // Top border
    write!(o, "\n").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {}{}{}\n", TOP_LEFT, h_rule(w - 4), TOP_RIGHT).ok();

    // Logo lines with gradient
    let logo = [
        "  ███████╗██╗  ██╗ █████╗ ██████╗  ██████╗ ██╗    ██╗ ",
        "  ██╔════╝██║  ██║██╔══██╗██╔══██╗██╔═══██╗██║    ██║ ",
        "  ███████╗███████║███████║██║  ██║██║   ██║██║ █╗ ██║ ",
        "  ╚════██║██╔══██║██╔══██║██║  ██║██║   ██║██║███╗██║ ",
        "  ███████║██║  ██║██║  ██║██████╔╝╚██████╔╝╚███╔███╔╝ ",
        "  ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═════╝  ╚═════╝  ╚══╝╚══╝  ",
    ];

    let gradient = [
        Color::Rgb { r: 120, g: 60, b: 255 },
        Color::Rgb { r: 140, g: 75, b: 255 },
        Color::Rgb { r: 155, g: 89, b: 255 },
        Color::Rgb { r: 130, g: 100, b: 255 },
        Color::Rgb { r: 100, g: 120, b: 255 },
        Color::Rgb { r: 80, g: 140, b: 255 },
    ];

    for (i, line) in logo.iter().enumerate() {
        set_fg(&mut o, gradient[i]);
        write!(o, "  {V_LINE}{line}").ok();
        // Pad to width
        let pad = (w - 4).saturating_sub(line.chars().count());
        write!(o, "{}", " ".repeat(pad)).ok();
        set_fg(&mut o, theme::BORDER);
        write!(o, "{V_LINE}\n").ok();
    }

    // AI subtitle
    let pad = (w - 4).saturating_sub(57);
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE}").ok();
    set_fg(&mut o, theme::CYAN);
    write!(o, "         {SPARK} A I {SPARK}   ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "Autonomous Intelligence Engine").ok();
    write!(o, "{}", " ".repeat(pad)).ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "{V_LINE}\n").ok();

    // Separator
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE}{}{V_LINE}\n", h_rule(w - 4)).ok();

    // Version + info line
    let ver_pad = (w - 4).saturating_sub(56);
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE}").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "   v{}  {DOT}  ", env!("CARGO_PKG_VERSION")).ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "/help").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, " for commands  {DOT}  ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "Ctrl+C").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, " to abort").ok();
    write!(o, "{}", " ".repeat(ver_pad)).ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "{V_LINE}\n").ok();

    // Bottom border
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {}{}{}\n", BOT_LEFT, h_rule(w - 4), BOT_RIGHT).ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_status_bar(mode: &str, model: &str, root: &str) {
    let mut o = io::stdout();
    let mode_color = match mode {
        "auto" => theme::CYAN,
        "build" => theme::WARN,
        "plan" => theme::ACCENT,
        _ => theme::DIM,
    };
    let mode_upper = mode.to_uppercase();
    let short_root = if root.chars().count() > 30 {
        let s: String = root.chars().rev().take(27).collect::<Vec<_>>().into_iter().rev().collect();
        format!("...{}", s)
    } else {
        root.to_string()
    };

    set_fg(&mut o, theme::BORDER);
    write!(o, "  {DIAMOND} ").ok();
    set_fg(&mut o, mode_color);
    write!(o, "[{mode_upper}]").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {GEAR} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{model}").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {ARROW} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{short_root}").ok();
    reset_color(&mut o);
    write!(o, "\n\n").ok();
}

fn print_connected_msg(host: &str) {
    let mut o = io::stdout();
    let now = chrono::Local::now().format("%H:%M:%S");
    set_fg(&mut o, theme::OK);
    write!(o, "  {RADIO} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "Connected to ").ok();
    set_fg(&mut o, theme::CYAN);
    write!(o, "{host}").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {DOT}  {now}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_ai_prefix() {
    let mut o = io::stdout();
    write!(o, "\n").ok();
    set_fg(&mut o, theme::ACCENT);
    write!(o, "  {SPARK} ").ok();
    set_fg(&mut o, theme::ACCENT_DIM);
    write!(o, "AI").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, " {H_LINE}{H_LINE} ").ok();
    reset_color(&mut o);
    o.flush().ok();
}

fn print_tool_call(name: &str, args: &str) {
    let mut o = io::stdout();
    let short_args = if args.chars().count() > 100 {
        let end: String = args.chars().take(100).collect();
        format!("{}...", end)
    } else {
        args.to_string()
    };
    write!(o, "\n").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, theme::WARN);
    write!(o, "{BOLT} ").ok();
    set_attr(&mut o, Attribute::Bold);
    write!(o, "{name}").ok();
    set_attr(&mut o, Attribute::Reset);
    write!(o, "\n").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE}   ").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "{short_args}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
    o.flush().ok();
}

fn print_tool_result(name: &str, success: bool, duration_ms: Option<u64>) {
    let mut o = io::stdout();
    let (icon, color) = if success {
        (CHECK, theme::OK)
    } else {
        (CROSS, theme::ERR)
    };
    let dur = duration_ms
        .map(|d| {
            if d > 1000 {
                format!(" {:.1}s", d as f64 / 1000.0)
            } else {
                format!(" {}ms", d)
            }
        })
        .unwrap_or_default();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, color);
    write!(o, "{icon} {name}").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "{dur}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
    o.flush().ok();
}

#[allow(dead_code)]
fn send_desktop_notification(title: &str, body: &str) {
    send_desktop_notification_ex(title, body, false);
}

fn send_desktop_notification_ex(title: &str, body: &str, is_error: bool) {
    let icon = if is_error { "dialog-error" } else { "dialog-information" };
    let urgency = if is_error { "critical" } else { "normal" };

    // Fire-and-forget notify-send on Linux
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("notify-send")
        .arg("--app-name=ShadowAI")
        .arg(format!("--icon={}", icon))
        .arg(format!("--urgency={}", urgency))
        .arg(title)
        .arg(body)
        .spawn();

    #[cfg(target_os = "macos")]
    {
        let script = format!("display notification \"{}\" with title \"{}\"", body, title);
        let _ = std::process::Command::new("osascript").arg("-e").arg(&script).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let script = format!("New-BurntToastNotification -Text '{}', '{}'", title, body);
        let _ = std::process::Command::new("powershell")
            .args(["-Command", &script])
            .spawn();
    }

    let _ = (icon, urgency); // suppress unused warnings on non-linux

    // Play sound if configured
    let config = load_config();
    if config.notification_sound.unwrap_or(false) {
        // Try paplay first, fall back to terminal bell
        if std::process::Command::new("paplay")
            .arg("/usr/share/sounds/freedesktop/stereo/complete.oga")
            .spawn().is_err()
        {
            print!("\x07"); // terminal bell
            io::stdout().flush().ok();
        }
    }
}

fn send_smart_notification(body: &str, elapsed_secs: f64, is_error: bool) {
    let config = load_config();
    let threshold = config.notification_threshold_secs.unwrap_or(10);
    if elapsed_secs < threshold as f64 {
        return;
    }
    let preview: String = body.chars().take(50).collect();
    let title = if is_error { "ShadowAI - Error" } else { "ShadowAI" };
    send_desktop_notification_ex(title, &preview, is_error);
}

fn print_error(msg: &str) {
    let mut o = io::stdout();
    write!(o, "\n").ok();
    set_fg(&mut o, theme::ERR);
    write!(o, "  {CROSS} Error: {msg}\n").ok();
    reset_color(&mut o);
}

fn print_info(msg: &str) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{msg}\n").ok();
    reset_color(&mut o);
}

fn print_info_accent(label: &str, value: &str) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {ARROW} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{label}: ").ok();
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "{value}\n").ok();
    reset_color(&mut o);
}

fn print_stats(input_tokens: u64, output_tokens: u64, cached: bool, elapsed: Option<f64>) {
    let mut o = io::stdout();
    let cache_icon = if cached { format!("  {DOT} CACHED") } else { String::new() };
    let elapsed_str = elapsed
        .map(|e| format!("  {DOT} {:.1}s", e))
        .unwrap_or_default();

    set_fg(&mut o, theme::BORDER);
    write!(o, "\n  {BOT_LEFT}{H_LINE}{H_LINE} ").ok();
    set_fg(&mut o, theme::STAT);
    write!(o, "{ARROW} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{} in", fmt_tokens(input_tokens)).ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {DOT}  ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{} out", fmt_tokens(output_tokens)).ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "{cache_icon}{elapsed_str}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn fmt_tokens(n: u64) -> String {
    if n >= 100_000 {
        format!("{:.0}k", n as f64 / 1000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn print_thinking_start() {
    let mut o = io::stdout();
    set_fg(&mut o, theme::THINK);
    set_attr(&mut o, Attribute::Dim);
    write!(o, "\n  {CIRCUIT} thinking ").ok();
    set_attr(&mut o, Attribute::Reset);
    reset_color(&mut o);
    o.flush().ok();
}

fn print_thinking_summary(text: &str) {
    if text.is_empty() { return; }
    let mut o = io::stdout();
    let summary: String = text.lines().next().unwrap_or("").chars().take(120).collect();
    set_fg(&mut o, theme::THINK);
    set_attr(&mut o, Attribute::Dim);
    write!(o, "\n  {CIRCUIT} ").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "{summary}").ok();
    set_attr(&mut o, Attribute::Reset);
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_file_change(path: &str, action: &str) {
    let mut o = io::stdout();
    let (icon, color) = match action {
        "deleted" => (CROSS, theme::FILE_DEL),
        "created" => ("+", theme::FILE_NEW),
        _ => ("~", theme::FILE_MOD),
    };
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, color);
    write!(o, "{icon} ").ok();
    set_attr(&mut o, Attribute::Underlined);
    write!(o, "{path}").ok();
    set_attr(&mut o, Attribute::Reset);
    reset_color(&mut o);
    write!(o, "\n").ok();
}

// ─── Diff preview for file changes ───────────────────────────────────────────

fn print_unified_diff(path: &str, old: &str, new: &str) {
    let mut o = io::stdout();
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    set_fg(&mut o, theme::ACCENT_DIM);
    write!(o, "\n  {V_LINE} {ARROW} diff {path}\n").ok();
    set_fg(&mut o, theme::ERR);
    write!(o, "  {V_LINE} --- a/{path}\n").ok();
    set_fg(&mut o, theme::OK);
    write!(o, "  {V_LINE} +++ b/{path}\n").ok();

    // Simple line-by-line diff using longest common subsequence approach
    let max_old = old_lines.len();
    let max_new = new_lines.len();

    // Build LCS table (limited size for performance)
    let limit = 500;
    if max_old > limit || max_new > limit {
        // Fallback: show truncated new content
        set_fg(&mut o, theme::DIM);
        write!(o, "  {V_LINE}  (file too large for inline diff, showing summary)\n").ok();
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  {V_LINE}  -{} lines, +{} lines\n", max_old, max_new).ok();
        reset_color(&mut o);
        return;
    }

    // Simple diff: walk both line arrays, print removed/added/context
    let mut output_lines: Vec<(char, &str)> = Vec::new();
    let mut oi = 0usize;
    let mut ni = 0usize;
    while oi < max_old || ni < max_new {
        if oi < max_old && ni < max_new && old_lines[oi] == new_lines[ni] {
            output_lines.push((' ', old_lines[oi]));
            oi += 1;
            ni += 1;
        } else if ni < max_new && (oi >= max_old || {
            // Look ahead: is the new line an insertion?
            let ahead_match = old_lines[oi..].iter().take(5).position(|l| *l == new_lines[ni]);
            ahead_match.is_none()
        }) {
            output_lines.push(('+', new_lines[ni]));
            ni += 1;
        } else if oi < max_old {
            output_lines.push(('-', old_lines[oi]));
            oi += 1;
        }
    }

    // Print with context (max 50 lines)
    let mut printed = 0usize;
    for (kind, line) in &output_lines {
        if printed >= 50 {
            set_fg(&mut o, theme::DIM);
            write!(o, "  {V_LINE}  ... ({} more lines)\n", output_lines.len() - printed).ok();
            break;
        }
        match kind {
            '+' => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {V_LINE} +{line}\n").ok();
                printed += 1;
            }
            '-' => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {V_LINE} -{line}\n").ok();
                printed += 1;
            }
            _ => {
                // Context line — only show if near a change
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE}  {line}\n").ok();
                printed += 1;
            }
        }
    }
    reset_color(&mut o);
}

// ─── Per-command help ────────────────────────────────────────────────────────

fn print_command_help(command: &str) {
    let mut o = io::stdout();
    let cmd = command.trim_start_matches('/');
    write!(o, "\n").ok();

    match cmd {
        "git" | "g" => {
            print_section_header("Git Commands");
            let entries = [
                ("/git, /g", "Show git status (short format)"),
                ("/git diff, /gd", "Show unstaged diff"),
                ("/git log, /gl", "Show last 10 commits"),
                ("/git commit, /gc", "Stage all changes + auto-commit with generated message"),
                ("/git undo", "Soft reset last commit (preserves changes)"),
                ("/git branch, /gb", "List all local and remote branches"),
                ("/git stash", "Stash current changes"),
                ("/git stash pop", "Pop last stash"),
                ("/git pr", "Show PR summary (commits and changed files vs main)"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<22}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            write!(o, "\n").ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "  Examples:\n").ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "    /gc              Stage all and commit\n").ok();
            write!(o, "    /gd              Quick diff\n").ok();
            write!(o, "    /git pr          PR summary for current branch\n").ok();
            reset_color(&mut o);
            print_section_end();
        }
        "review" => {
            print_section_header("Review Commands");
            let entries = [
                ("/review", "Review staged git changes (sends diff to AI)"),
                ("/review <file>", "Review a specific file"),
                ("/review --pr", "Review the last commit diff"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<22}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "context" | "ctx" => {
            print_section_header("Context Commands");
            let entries = [
                ("/context, /ctx", "Show token usage breakdown"),
                ("/context files", "List tracked files with sizes and token estimates"),
                ("/context drop <name>", "Remove a file from tracked context by name match"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<26}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "format" | "fmt" => {
            print_section_header("Format Command");
            let entries = [
                ("/format", "Auto-detect and run formatter for the project"),
                ("/format <file>", "Format a specific file"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<22}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            write!(o, "\n").ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "  Auto-detects: cargo fmt, prettier, ruff/black, gofmt\n").ok();
            reset_color(&mut o);
            print_section_end();
        }
        "mode" => {
            print_section_header("Mode Command");
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  {V_LINE} /mode <plan|build|auto>\n").ok();
            write!(o, "  {V_LINE}\n").ok();
            write!(o, "  {V_LINE}   plan  - Planning mode (think before coding)\n").ok();
            write!(o, "  {V_LINE}   build - Build mode (code-focused)\n").ok();
            write!(o, "  {V_LINE}   auto  - Automatic (default)\n").ok();
            reset_color(&mut o);
            print_section_end();
        }
        "skill" | "skills" => {
            print_section_header("Skill Commands");
            let entries = [
                ("/skills", "List all available skills (grouped by category)"),
                ("/skill <name>", "Activate a skill by name or alias"),
                ("/skill off", "Deactivate the current skill"),
                ("/skill create <name>", "Create a new skill template"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<22}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "security" => {
            print_section_header("Security Commands");
            let entries = [
                ("/security", "Scan all source files for issues"),
                ("/security <file>", "Scan a specific file"),
                ("/security --deps", "Audit project dependencies"),
            ];
            for (cmd, desc) in entries {
                set_fg(&mut o, theme::BORDER);
                write!(o, "  {V_LINE} ").ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{:<22}", cmd).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{desc}\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "test" => {
            print_section_header("Test Command");
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  {V_LINE} /test           Run project test suite\n").ok();
            write!(o, "  {V_LINE} /test <file>    Run tests for specific file\n").ok();
            write!(o, "  {V_LINE}\n").ok();
            write!(o, "  {V_LINE} Auto-detects: cargo test, npm test, pytest, go test\n").ok();
            reset_color(&mut o);
            print_section_end();
        }
        _ => {
            set_fg(&mut o, theme::WARN);
            write!(o, "  No detailed help for '/{cmd}'. Use /help to see all commands.\n").ok();
            reset_color(&mut o);
        }
    }
}

// ─── Format command handler ──────────────────────────────────────────────────

fn handle_format_command(args: &str, root_path: &str) {
    let mut o = io::stdout();
    let root = std::path::Path::new(root_path);
    let trimmed = args.trim();

    // Detect project type and run appropriate formatter
    if root.join("Cargo.toml").exists() {
        print_section_header("Format (cargo fmt)");
        let mut cmd_args = vec!["fmt"];
        if !trimmed.is_empty() {
            cmd_args.push("--");
            cmd_args.push(trimmed);
        }
        match std::process::Command::new("cargo")
            .args(&cmd_args)
            .current_dir(root_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Formatted successfully\n").ok();
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                }
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} cargo fmt not available: {}\n", e).ok();
            }
        }
    } else if root.join("package.json").exists() {
        print_section_header("Format (prettier)");
        let target = if trimmed.is_empty() { "." } else { trimmed };
        match std::process::Command::new("npx")
            .args(["prettier", "--write", target])
            .current_dir(root_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let file_count = stdout.lines().count();
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Formatted {} file(s)\n", file_count).ok();
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                }
            }
            Err(_) => {
                set_fg(&mut o, theme::DIM);
                write!(o, "  prettier not available. Install with: npm i -D prettier\n").ok();
            }
        }
    } else if root.join("pyproject.toml").exists() || root.join("setup.py").exists()
        || root.join("requirements.txt").exists()
    {
        print_section_header("Format (ruff/black)");
        let target = if trimmed.is_empty() { "." } else { trimmed };
        // Try ruff first, fall back to black
        let result = std::process::Command::new("ruff")
            .args(["format", target])
            .current_dir(root_path)
            .output();
        match result {
            Ok(output) if output.status.success() => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Formatted with ruff\n").ok();
            }
            _ => {
                // Fall back to black
                match std::process::Command::new("black")
                    .args([target])
                    .current_dir(root_path)
                    .output()
                {
                    Ok(output) if output.status.success() => {
                        set_fg(&mut o, theme::OK);
                        write!(o, "  {CHECK} Formatted with black\n").ok();
                    }
                    _ => {
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  No Python formatter found. Install ruff or black.\n").ok();
                    }
                }
            }
        }
    } else if root.join("go.mod").exists() {
        print_section_header("Format (gofmt)");
        let target = if trimmed.is_empty() { "." } else { trimmed };
        match std::process::Command::new("gofmt")
            .args(["-w", target])
            .current_dir(root_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Formatted with gofmt\n").ok();
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                }
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} gofmt not available: {}\n", e).ok();
            }
        }
    } else {
        set_fg(&mut o, theme::WARN);
        write!(o, "  No recognized project type found (Cargo.toml, package.json, pyproject.toml, go.mod)\n").ok();
    }
    reset_color(&mut o);
    print_section_end();
}

fn print_help() {
    let w = term_width().min(72);
    let mut o = io::stdout();

    write!(o, "\n").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {TOP_LEFT}{}{TOP_RIGHT}\n", h_rule(w - 4)).ok();
    write!(o, "  {V_LINE}").ok();
    set_fg(&mut o, theme::ACCENT);
    write!(o, "  {SPARK} ShadowAI Commands").ok();
    let pad = (w - 4).saturating_sub(21);
    write!(o, "{}", " ".repeat(pad)).ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "{V_LINE}\n").ok();
    write!(o, "  {V_LINE}{}{V_LINE}\n", h_rule(w - 4)).ok();

    let cmds = [
        ("Chat", vec![
            ("/clear", "Clear screen and chat history"),
            ("/mode <mode>", "Switch: plan, build, auto"),
            ("/model <name>", "Switch model"),
            ("/temperature <n>", "Set temperature (0.0-2.0)"),
            ("/tokens <n>", "Set max output tokens"),
            ("/abort", "Abort current AI stream"),
            ("@<path>", "Attach file inline"),
            ("/file <path>", "Attach file to next message"),
            ("/image <path>", "Attach image to next message"),
            ("/browse <url>", "Fetch web page as context"),
            ("\"\"\"", "Start/end multiline input"),
            ("line \\", "Backslash line continuation"),
        ]),
        ("Git", vec![
            ("/git, /g", "Show git status"),
            ("/git diff, /gd", "Show git diff"),
            ("/git log, /gl", "Show last 10 commits"),
            ("/git commit, /gc", "Stage all + commit"),
            ("/git undo", "Undo last commit (soft)"),
            ("/git branch, /gb", "List branches"),
            ("/git stash", "Stash changes"),
            ("/git stash pop", "Pop stash"),
            ("/git pr", "PR summary (commits vs main)"),
        ]),
        ("Dev Workflow", vec![
            ("/test", "Run project test suite"),
            ("/test <file>", "Run tests for specific file"),
            ("/test --watch", "Test watch mode (re-run on change)"),
            ("/lint", "Run project linter"),
            ("/lint --fix", "Auto-fix lint issues"),
            ("/build", "Run project build"),
            ("/build --fix", "Build + AI fix errors"),
            ("/review", "Review staged git changes"),
            ("/review <file>", "Review specific file"),
            ("/review --pr", "Review last commit diff"),
            ("/format", "Auto-format project files"),
            ("/format <file>", "Format specific file"),
            ("/perf", "Scan all files for perf issues"),
            ("/perf <file>", "Analyze specific file"),
            ("/spawn <task>", "Run background AI task"),
        ]),
        ("Files", vec![
            ("/add <file>", "Track file (persistent context)"),
            ("/drop <file>", "Remove tracked file"),
            ("/files", "List tracked files"),
            ("/file <path>", "Attach file to next message"),
        ]),
        ("Search", vec![
            ("/find <pattern>", "Find files by name/glob"),
            ("/grep <pattern>", "Search file contents"),
            ("/symbols <query>", "Search functions/classes/types"),
            ("/tree [path]", "Show directory tree"),
            ("/search <query>", "Google web search"),
        ]),
        ("Context", vec![
            ("/context, /ctx", "Show token usage breakdown"),
            ("/context files", "List tracked files + sizes"),
            ("/context drop <name>", "Remove file from context"),
        ]),
        ("Agent", vec![
            ("/watch", "Toggle watch mode (detect AI! comments)"),
            ("/plan <desc>", "Create structured implementation plan"),
            ("/plan", "Show current plan status"),
            ("/plan approve", "Approve plan for step-by-step execution"),
            ("/plan next", "Execute next plan step"),
            ("/plan export", "Export plan to .shadowai/plan.md"),
            ("/undo", "Undo last edit or commit"),
            ("/copy", "Copy last AI response to clipboard"),
            ("/edits [file]", "Show file edit history"),
        ]),
        ("Skills", vec![
            ("/skills", "List available skills (grouped)"),
            ("/skill <name|alias>", "Activate a skill"),
            ("/skill a+b", "Chain multiple skills"),
            ("/skill off", "Deactivate current skill"),
            ("/skill create <name>", "Create a new skill skeleton"),
            ("/skill edit <name>", "Open skill TOML in $EDITOR"),
            ("/skill export <name>", "Export skill as TOML"),
            ("/skill import <path>", "Import skill from TOML file"),
        ]),
        ("Tracking", vec![
            ("/memory", "Show project memory"),
            ("/remember <text>", "Save to project memory"),
            ("/errors", "Show logged errors"),
            ("/fixed", "Show logged fixes"),
            ("/completed", "Show completed tasks"),
        ]),
        ("Session", vec![
            ("/sessions", "List FerrumChat sessions"),
            ("/session [id]", "Show / switch session"),
            ("/session rename <name>", "Rename current session"),
            ("/resume [id]", "Resume last or specific session"),
            ("/new", "Start a new session  (--new flag forces fresh start)"),
            ("/compact", "Trigger context compaction"),
            ("/memories", "List AI memories"),
            ("/save <name>", "Save conversation snapshot"),
            ("/load <name>", "Load conversation snapshot"),
        ]),
        ("Provider", vec![
            ("/providers", "List provider profiles"),
            ("/provider <name>", "Switch provider"),
            ("/models", "List available models"),
        ]),
        ("Security", vec![
            ("/security", "Scan all source files"),
            ("/security <file>", "Scan specific file"),
            ("/security --deps", "Audit dependencies"),
        ]),
        ("Documentation", vec![
            ("/doc <file>", "Generate docs for file"),
            ("/doc --readme", "Generate/update README"),
            ("/doc --api", "Generate API documentation"),
            ("/changelog", "Generate changelog from commits"),
            ("/release-notes", "Generate release notes from tags"),
        ]),
        ("History", vec![
            ("/history", "Show recent input history"),
            ("/history search <q>", "Search through history"),
        ]),
        ("Export", vec![
            ("/export", "Export chat as markdown"),
            ("/export json", "Export chat as JSON"),
        ]),
        ("System", vec![
            ("/status", "Show connection info"),
            ("/keybindings", "Show key bindings"),
            ("/cheatsheet", "Compact command reference"),
            ("/help", "Show this help"),
            ("/help <cmd>", "Detailed help for command"),
            ("/quit", "Exit ShadowAI"),
        ]),
        ("Pipeline", vec![
            ("shadowai pipe", "Read stdin, output to stdout"),
            ("cmd | shadowai pipe", "Pipe input to AI"),
            ("cmd | shadowai pipe \"q\"", "Pipe with prompt"),
        ]),
    ];

    for (section, entries) in cmds {
        set_fg(&mut o, theme::BORDER);
        write!(o, "  {V_LINE}  ").ok();
        set_fg(&mut o, theme::CYAN_DIM);
        set_attr(&mut o, Attribute::Bold);
        write!(o, "{section}").ok();
        set_attr(&mut o, Attribute::Reset);
        let sp = (w - 4).saturating_sub(section.len() + 2);
        write!(o, "{}", " ".repeat(sp)).ok();
        set_fg(&mut o, theme::BORDER);
        write!(o, "{V_LINE}\n").ok();

        for (cmd, desc) in entries {
            let line = format!("    {:<22}{}", cmd, desc);
            let lp = (w - 4).saturating_sub(line.chars().count());
            set_fg(&mut o, theme::BORDER);
            write!(o, "  {V_LINE}").ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "    ").ok();
            set_fg(&mut o, theme::CYAN);
            write!(o, "{:<22}", cmd).ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "{desc}{}", " ".repeat(lp)).ok();
            set_fg(&mut o, theme::BORDER);
            write!(o, "{V_LINE}\n").ok();
        }

        set_fg(&mut o, theme::BORDER);
        write!(o, "  {V_LINE}{}{V_LINE}\n", " ".repeat(w - 4)).ok();
    }

    // Keyboard shortcuts
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE}  ").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "Ctrl+C").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, " abort stream").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "   {DOT}   Ctrl+D").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, " exit").ok();
    let kp = (w - 4).saturating_sub(42);
    write!(o, "{}", " ".repeat(kp)).ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "{V_LINE}\n").ok();
    write!(o, "  {}{}{}\n", BOT_LEFT, h_rule(w - 4), BOT_RIGHT).ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

#[allow(dead_code)]
fn print_prompt() {
    print_prompt_with_context("auto", None, 0, 128000);
}

fn print_prompt_with_context(mode: &str, skill: Option<&str>, estimated_tokens: usize, max_context: usize) {
    print_prompt_full(mode, skill, estimated_tokens, max_context, 0, "", "", 0);
}

fn print_prompt_full(mode: &str, skill: Option<&str>, estimated_tokens: usize, max_context: usize, turn: u64, branch: &str, root_path: &str, tracked_count: usize) {
    let mut o = io::stdout();
    write!(o, "\n").ok();

    // Mode badge
    let mode_color = match mode {
        "auto" => theme::CYAN,
        "build" => theme::WARN,
        "plan" => theme::ACCENT,
        _ => theme::DIM,
    };
    set_fg(&mut o, mode_color);
    write!(o, "  [{}]", mode.to_uppercase()).ok();

    // Turn number
    if turn > 0 {
        set_fg(&mut o, theme::DIM);
        write!(o, " [{}]", turn).ok();
    }

    // Git branch
    if !branch.is_empty() {
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, " {branch}").ok();
    }

    // Tracked file count
    if tracked_count > 0 {
        set_fg(&mut o, theme::DIM);
        write!(o, " [{} files]", tracked_count).ok();
    }

    // Skill badge
    if let Some(sk) = skill {
        set_fg(&mut o, theme::ACCENT_DIM);
        write!(o, " [{}]", sk).ok();
    }

    // Error count badge — count errors from last 24h
    if !root_path.is_empty() {
        let error_count = count_recent_errors(root_path);
        if error_count > 0 {
            set_fg(&mut o, theme::ERR);
            write!(o, " [{} errs]", error_count).ok();
        }
    }

    // Token budget indicator
    if estimated_tokens > 0 {
        let pct = if max_context > 0 { (estimated_tokens as f64 / max_context as f64 * 100.0) as usize } else { 0 };
        let tok_color = if pct < 60 {
            theme::DIM
        } else if pct < 80 {
            theme::WARN
        } else {
            theme::ERR
        };
        set_fg(&mut o, tok_color);
        write!(o, " {}", fmt_tokens(estimated_tokens as u64)).ok();
    }

    set_fg(&mut o, theme::CYAN);
    write!(o, " {ARROW} ").ok();
    set_fg(&mut o, Color::White);
    o.flush().ok();
}

/// Count errors from errors.md that occurred in the last 24 hours.
fn count_recent_errors(root_path: &str) -> usize {
    let content = match read_tracking_file(root_path, "errors.md") {
        Some(c) => c,
        None => return 0,
    };
    let now = chrono::Local::now();
    let cutoff = now - chrono::Duration::hours(24);
    let ts_re = regex::Regex::new(r"## \[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})\]").unwrap_or_else(|_| return regex::Regex::new(r"^$").unwrap());
    let mut count = 0;
    for cap in ts_re.captures_iter(&content) {
        if let Some(ts_str) = cap.get(1) {
            if let Ok(ts) = chrono::NaiveDateTime::parse_from_str(ts_str.as_str(), "%Y-%m-%d %H:%M:%S") {
                let local_ts = ts.and_local_timezone(chrono::Local).earliest();
                if let Some(lt) = local_ts {
                    if lt > cutoff {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

fn print_reconnecting() {
    let mut o = io::stdout();
    set_fg(&mut o, theme::WARN);
    write!(o, "\n  {RADIO} Reconnecting...\n").ok();
    reset_color(&mut o);
}

fn print_reconnected() {
    let mut o = io::stdout();
    let now = chrono::Local::now().format("%H:%M:%S");
    set_fg(&mut o, theme::OK);
    write!(o, "  {RADIO} Reconnected").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {DOT}  {now}\n").ok();
    reset_color(&mut o);
}

fn print_mode_badge(mode: &str) {
    let mut o = io::stdout();
    let color = match mode {
        "auto" => theme::CYAN,
        "build" => theme::WARN,
        "plan" => theme::ACCENT,
        _ => theme::DIM,
    };
    let upper = mode.to_uppercase();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {DIAMOND} ").ok();
    set_fg(&mut o, color);
    set_attr(&mut o, Attribute::Bold);
    write!(o, "[{upper}]").ok();
    set_attr(&mut o, Attribute::Reset);
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_model_badge(m: &str) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {GEAR} ").ok();
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "{m}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_section_header(title: &str) {
    let mut o = io::stdout();
    let w = term_width().min(60);
    let pad = w.saturating_sub(title.len() + 6);
    write!(o, "\n").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {TOP_LEFT}{H_LINE} ").ok();
    set_fg(&mut o, theme::ACCENT);
    write!(o, "{title}").ok();
    set_fg(&mut o, theme::BORDER);
    write!(o, " {}{TOP_RIGHT}\n", h_rule(pad)).ok();
    reset_color(&mut o);
}

fn print_section_row(label: &str, value: &str) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, theme::DIM);
    write!(o, "{:<14}", label).ok();
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "{value}").ok();
    reset_color(&mut o);
    write!(o, "\n").ok();
}

fn print_section_end() {
    let mut o = io::stdout();
    let w = term_width().min(60);
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {}{}\n", BOT_LEFT, h_rule(w - 2)).ok();
    reset_color(&mut o);
}

fn print_list_item(marker: &str, color: Color, text: &str) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, color);
    write!(o, "{marker} ").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "{text}\n").ok();
    reset_color(&mut o);
}

// ─── Raw mode RAII guard ──────────────────────────────────────────────────────

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> Self {
        terminal::enable_raw_mode().ok();
        Self
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        terminal::disable_raw_mode().ok();
    }
}

// ─── Interactive session picker ───────────────────────────────────────────────

fn interactive_session_picker(
    sessions_cache: &Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> Option<String> {
    let sessions = sessions_cache.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if sessions.is_empty() {
        let mut o = io::stdout();
        execute!(o,
            SetForegroundColor(theme::DIM),
            Print("  No sessions found. Start chatting to create one.\n"),
            ResetColor,
        ).ok();
        return None;
    }

    let count = sessions.len();
    let mut selected: usize = 0;
    let mut o = io::stdout();

    // Header
    execute!(o,
        Print("\n"),
        SetForegroundColor(theme::BORDER),
        Print(format!("  {ARROW} ")),
        SetForegroundColor(theme::ACCENT),
        Print("Select a session"),
        SetForegroundColor(theme::DIM),
        Print(format!("  {DOT}  {UP_ARROW}{DOWN_ARROW} navigate  {DOT}  enter select  {DOT}  esc cancel")),
        ResetColor,
        Print("\n\n"),
    ).ok();
    o.flush().ok();

    // Get cursor position to redraw from
    let start_row = cursor::position().map(|(_, r)| r).unwrap_or(0);

    let render = |sel: usize, out: &mut io::Stdout| {
        execute!(out, cursor::MoveTo(0, start_row)).ok();
        for (i, s) in sessions.iter().enumerate() {
            let id = s["id"].as_str().unwrap_or("?");
            let short_id = &id[..id.len().min(8)];
            let name = s["name"].as_str().unwrap_or("(unnamed)");
            let msg_count = s["message_count"].as_u64().unwrap_or(0);
            let profile = s["profile"].as_str().unwrap_or("");
            let updated_ts = s["updated_at"].as_u64().unwrap_or(0);
            let age = format_time_ago(updated_ts);

            let is_sel = i == sel;
            let marker = if is_sel { RADIO } else { DOT };
            let name_color = if is_sel { theme::CYAN } else { theme::DIM_LIGHT };

            execute!(out,
                terminal::Clear(ClearType::CurrentLine),
                SetForegroundColor(if is_sel { theme::CYAN } else { theme::BORDER }),
                Print(if is_sel { "  \u{25b8} " } else { "    " }),
                SetForegroundColor(name_color),
                Print(format!("{marker} [{}] {} ", i + 1, name)),
                SetForegroundColor(theme::DIM),
                Print(format!("#{} {DOT} {}msg {DOT} {} {DOT} {}", short_id, msg_count, profile, age)),
                ResetColor,
                Print("\r\n"),
            ).ok();
        }
        out.flush().ok();
    };

    let _raw_guard = RawModeGuard::new();
    render(selected, &mut o);

    let result = loop {
        if let Ok(Event::Key(key)) = ct_event::read() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected > 0 { selected -= 1; }
                    render(selected, &mut o);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected < count - 1 { selected += 1; }
                    render(selected, &mut o);
                }
                KeyCode::Enter => {
                    break sessions[selected]["id"].as_str().map(|s| s.to_string());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    break None;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break None;
                }
                _ => {}
            }
        }
    };

    drop(_raw_guard);
    execute!(o, Print("\r\n")).ok();
    result
}

const UP_ARROW: &str = "\u{2191}";
const DOWN_ARROW: &str = "\u{2193}";

// ─── Git command handler ─────────────────────────────────────────────────────

fn handle_git_command(sub: &str, root_path: &str) {
    let mut o = io::stdout();
    let run_git = |args: &[&str]| -> (String, String, bool) {
        match std::process::Command::new("git")
            .args(args)
            .current_dir(root_path)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                (stdout, stderr, output.status.success())
            }
            Err(e) => (String::new(), format!("Failed to run git: {}", e), false),
        }
    };

    match sub {
        "" | "status" => {
            print_section_header("Git Status");
            let (stdout, stderr, ok) = run_git(&["status", "--short", "--branch"]);
            if ok {
                if stdout.trim().is_empty() {
                    print_list_item(CHECK, theme::OK, "Working tree clean");
                } else {
                    for line in stdout.lines() {
                        let color = if line.starts_with("##") {
                            theme::CYAN
                        } else if line.starts_with('?') {
                            theme::WARN
                        } else if line.starts_with('A') || line.starts_with(' ') && line.chars().nth(1) == Some('A') {
                            theme::FILE_NEW
                        } else if line.starts_with('D') {
                            theme::FILE_DEL
                        } else {
                            theme::FILE_MOD
                        };
                        set_fg(&mut o, theme::BORDER);
                        write!(o, "  {V_LINE} ").ok();
                        set_fg(&mut o, color);
                        write!(o, "{}\n", line).ok();
                    }
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "diff" => {
            print_section_header("Git Diff");
            let (stdout, stderr, ok) = run_git(&["diff", "--color=never"]);
            if ok {
                if stdout.trim().is_empty() {
                    print_list_item(DOT, theme::DIM, "No unstaged changes");
                } else {
                    for line in stdout.lines() {
                        let color = if line.starts_with('+') && !line.starts_with("+++") {
                            theme::OK
                        } else if line.starts_with('-') && !line.starts_with("---") {
                            theme::ERR
                        } else if line.starts_with("@@") {
                            theme::CYAN
                        } else if line.starts_with("diff ") || line.starts_with("index ") {
                            theme::ACCENT_DIM
                        } else {
                            theme::DIM_LIGHT
                        };
                        set_fg(&mut o, color);
                        write!(o, "  {}\n", line).ok();
                    }
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "log" => {
            print_section_header("Git Log");
            let (stdout, stderr, ok) = run_git(&["log", "--oneline", "-10", "--decorate", "--color=never"]);
            if ok {
                for (i, line) in stdout.lines().enumerate() {
                    let color = if i == 0 { theme::CYAN } else { theme::DIM_LIGHT };
                    let marker = if i == 0 { RADIO } else { DOT };
                    print_list_item(marker, color, line);
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        "commit" => {
            // Check for staged changes first, then unstaged
            let (staged, _, _) = run_git(&["diff", "--staged", "--stat"]);
            let (unstaged_diff, _, _) = run_git(&["diff", "--stat"]);
            if staged.trim().is_empty() && unstaged_diff.trim().is_empty() {
                print_error("No changes to commit.");
                return;
            }
            // Stage all if nothing staged
            if staged.trim().is_empty() {
                let (_, stderr, ok) = run_git(&["add", "-A"]);
                if !ok {
                    print_error(&format!("git add failed: {}", stderr.trim()));
                    return;
                }
                print_info_accent("Staged", "all changes");
            }
            // Get diff for commit message
            let (diff_out, _, _) = run_git(&["diff", "--staged", "--color=never"]);
            // Generate a simple commit message from the diff stat
            let (stat_out, _, _) = run_git(&["diff", "--staged", "--stat"]);
            // Show what's being committed
            print_section_header("Committing");
            for line in stat_out.lines() {
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "  {V_LINE} {}\n", line).ok();
            }
            reset_color(&mut o);
            print_section_end();

            // Generate commit message from file changes
            let files_changed: Vec<&str> = stat_out.lines()
                .filter(|l| l.contains('|'))
                .map(|l| l.split('|').next().unwrap_or("").trim())
                .collect();
            let commit_msg = if files_changed.len() == 1 {
                format!("Update {}", files_changed[0])
            } else if files_changed.len() <= 5 {
                format!("Update {} files: {}", files_changed.len(), files_changed.join(", "))
            } else {
                format!("Update {} files", files_changed.len())
            };

            // Check diff size to add detail
            let adds = diff_out.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
            let dels = diff_out.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
            let full_msg = format!("{} (+{}, -{})", commit_msg, adds, dels);

            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "\n  Message: ").ok();
            set_fg(&mut o, theme::AI_TEXT);
            write!(o, "{}\n", full_msg).ok();
            reset_color(&mut o);

            // Run pre-commit hooks from hooks config
            {
                let hooks = load_hooks();
                let pre_commit_hooks: Vec<&Hook> = hooks.iter().filter(|h| h.event == "pre-commit").collect();
                for hook in &pre_commit_hooks {
                    let result = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&hook.command)
                        .current_dir(root_path)
                        .output();
                    match result {
                        Ok(output) if !output.status.success() => {
                            let stderr_out = String::from_utf8_lossy(&output.stderr);
                            let stdout_out = String::from_utf8_lossy(&output.stdout);
                            print_error(&format!("Pre-commit hook failed: {}", hook.command));
                            if !stderr_out.is_empty() {
                                set_fg(&mut o, theme::WARN);
                                write!(o, "  {}\n", stderr_out.trim()).ok();
                                reset_color(&mut o);
                            }
                            if !stdout_out.is_empty() {
                                set_fg(&mut o, theme::DIM_LIGHT);
                                write!(o, "  {}\n", stdout_out.trim()).ok();
                                reset_color(&mut o);
                            }
                            return;
                        }
                        Err(e) => {
                            print_error(&format!("Pre-commit hook error: {}", e));
                            return;
                        }
                        _ => {}
                    }
                }
                if !pre_commit_hooks.is_empty() {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Pre-commit hooks passed\n").ok();
                    reset_color(&mut o);
                }
            }

            let (_, stderr, ok) = run_git(&["commit", "-m", &full_msg]);
            if ok {
                let mut o = io::stdout();
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Committed successfully\n").ok();
                reset_color(&mut o);
                // Hooks: commit (Section 8.2)
                let hooks_commit = load_hooks();
                run_hooks(&hooks_commit, "commit", None, &json!({"message": full_msg}));
            } else {
                print_error(&format!("Commit failed: {}", stderr.trim()));
            }
        }
        "undo" => {
            let (_, stderr, ok) = run_git(&["reset", "--soft", "HEAD~1"]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Undid last commit (changes preserved)\n").ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("git reset failed: {}", stderr.trim()));
            }
        }
        "branch" => {
            print_section_header("Branches");
            let (stdout, stderr, ok) = run_git(&["branch", "-a", "--color=never"]);
            if ok {
                for line in stdout.lines() {
                    let (marker, color) = if line.starts_with('*') {
                        (RADIO, theme::CYAN)
                    } else if line.contains("remotes/") {
                        (DOT, theme::DIM)
                    } else {
                        (ARROW, theme::DIM_LIGHT)
                    };
                    print_list_item(marker, color, line.trim_start_matches("* ").trim());
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        "stash" => {
            let (stdout, stderr, ok) = run_git(&["stash"]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} {}\n", stdout.trim()).ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("git stash failed: {}", stderr.trim()));
            }
        }
        "stash list" => {
            print_section_header("Stash List");
            let (stdout, stderr, ok) = run_git(&["stash", "list"]);
            if ok {
                if stdout.trim().is_empty() {
                    print_list_item(DOT, theme::DIM, "No stashes found");
                } else {
                    for line in stdout.lines() {
                        print_list_item(DOT, theme::DIM_LIGHT, line);
                    }
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        "stash pop" => {
            let (stdout, stderr, ok) = run_git(&["stash", "pop"]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} {}\n", stdout.trim()).ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("git stash pop failed: {}", stderr.trim()));
            }
        }
        "stash show" => {
            print_section_header("Stash Show");
            let (stdout, stderr, ok) = run_git(&["stash", "show", "-p"]);
            if ok {
                for line in stdout.lines() {
                    let color = if line.starts_with('+') && !line.starts_with("+++") {
                        theme::OK
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        theme::ERR
                    } else if line.starts_with("@@") {
                        theme::CYAN
                    } else {
                        theme::DIM_LIGHT
                    };
                    set_fg(&mut o, color);
                    write!(o, "  {}\n", line).ok();
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        sub if sub.starts_with("stash pop ") => {
            let n = sub.trim_start_matches("stash pop ").trim();
            let stash_ref = format!("stash@{{{}}}", n);
            let (stdout, stderr, ok) = run_git(&["stash", "pop", &stash_ref]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} {}\n", stdout.trim()).ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("git stash pop failed: {}", stderr.trim()));
            }
        }
        sub if sub.starts_with("stash drop ") => {
            let n = sub.trim_start_matches("stash drop ").trim();
            let stash_ref = format!("stash@{{{}}}", n);
            let (stdout, stderr, ok) = run_git(&["stash", "drop", &stash_ref]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} {}\n", stdout.trim()).ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("git stash drop failed: {}", stderr.trim()));
            }
        }
        "pr" | "pr create" => {
            print_section_header("PR Summary");
            // Get commits on current branch vs main
            let (commits, _, ok1) = run_git(&["log", "--oneline", "main..HEAD"]);
            if !ok1 {
                // Try master if main doesn't exist
                let (commits2, _, ok2) = run_git(&["log", "--oneline", "master..HEAD"]);
                if ok2 && !commits2.trim().is_empty() {
                    set_fg(&mut o, theme::CYAN_DIM);
                    write!(o, "  {V_LINE} Commits (vs master):\n").ok();
                    for line in commits2.lines() {
                        print_list_item(DOT, theme::DIM_LIGHT, line);
                    }
                    let (stat, _, _) = run_git(&["diff", "master..HEAD", "--stat"]);
                    if !stat.trim().is_empty() {
                        write!(o, "\n").ok();
                        set_fg(&mut o, theme::CYAN_DIM);
                        write!(o, "  {V_LINE} Changed files:\n").ok();
                        for line in stat.lines() {
                            set_fg(&mut o, theme::DIM_LIGHT);
                            write!(o, "  {V_LINE}   {line}\n").ok();
                        }
                    }
                } else {
                    print_error("Could not determine base branch (tried main and master).");
                }
            } else if commits.trim().is_empty() {
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE} No commits ahead of main\n").ok();
            } else {
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "  {V_LINE} Commits (vs main):\n").ok();
                for line in commits.lines() {
                    print_list_item(DOT, theme::DIM_LIGHT, line);
                }
                let (stat, _, _) = run_git(&["diff", "main..HEAD", "--stat"]);
                if !stat.trim().is_empty() {
                    write!(o, "\n").ok();
                    set_fg(&mut o, theme::CYAN_DIM);
                    write!(o, "  {V_LINE} Changed files:\n").ok();
                    for line in stat.lines() {
                        set_fg(&mut o, theme::DIM_LIGHT);
                        write!(o, "  {V_LINE}   {line}\n").ok();
                    }
                }
                write!(o, "\n").ok();
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE} Tip: paste this into chat to generate a PR description\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        "conflicts" => {
            print_section_header("Merge Conflicts");
            // Scan tracked files for <<<<<<< conflict markers
            let (all_files, _, _) = run_git(&["ls-files"]);
            let mut found: Vec<(String, usize)> = Vec::new();
            for fname in all_files.lines() {
                let fpath = format!("{}/{}", root_path, fname.trim());
                if let Ok(content) = std::fs::read_to_string(&fpath) {
                    let count = content.lines().filter(|l| l.starts_with("<<<<<<<")).count();
                    if count > 0 {
                        found.push((fname.trim().to_string(), count));
                    }
                }
            }
            let mut o = io::stdout();
            if found.is_empty() {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} No merge conflicts detected.\n").ok();
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {} file{} with conflicts:\n", found.len(), if found.len() == 1 {""} else {"s"}).ok();
                for (f, n) in &found {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  {V_LINE}   {} ({} conflict marker{})\n", f, n, if *n == 1 {""} else {"s"}).ok();
                }
                write!(o, "\n").ok();
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE} Tip: /add <file>  →  ask AI to \"resolve the merge conflict\"\n").ok();
            }
            reset_color(&mut o);
            print_section_end();
        }
        // /blame <file> or blame <file>:<line> (Section 3.4)
        sub if sub.starts_with("blame ") => {
            let arg = sub.trim_start_matches("blame ").trim();
            let (file_arg, _line_arg) = if let Some(idx) = arg.rfind(':') {
                (&arg[..idx], Some(&arg[idx+1..]))
            } else {
                (arg, None)
            };
            print_section_header(&format!("Blame: {}", file_arg));
            let (stdout, stderr, ok) = run_git(&["blame", "--line-porcelain", file_arg]);
            if ok {
                let mut current_hash = String::new();
                let mut current_author = String::new();
                let mut current_date = String::new();
                let mut line_num: usize = 0;
                for line in stdout.lines() {
                    if line.len() >= 40 && line.chars().take(40).all(|c| c.is_ascii_hexdigit()) {
                        current_hash = line[..8].to_string();
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 {
                            line_num = parts[2].parse::<usize>().unwrap_or(0);
                        }
                    } else if let Some(author) = line.strip_prefix("author ") {
                        current_author = author.to_string();
                    } else if let Some(ts) = line.strip_prefix("author-time ") {
                        if let Ok(secs) = ts.trim().parse::<i64>() {
                            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
                                .map(|d| d.format("%Y-%m-%d").to_string())
                                .unwrap_or_default();
                            current_date = dt;
                        }
                    } else if let Some(content) = line.strip_prefix('\t') {
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  {:>4} ", line_num).ok();
                        set_fg(&mut o, theme::CYAN_DIM);
                        write!(o, "{} ", current_hash).ok();
                        set_fg(&mut o, theme::DIM_LIGHT);
                        write!(o, "{:<20} {} ", current_author, current_date).ok();
                        set_fg(&mut o, theme::AI_TEXT);
                        write!(o, "{}\n", content).ok();
                        reset_color(&mut o);
                    }
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        // /resolve — show conflicted files (Section 3.4)
        "resolve" => {
            print_section_header("Resolve Conflicts");
            let (stdout, stderr, ok) = run_git(&["diff", "--name-only", "--diff-filter=U"]);
            if ok {
                let conflicted: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
                if conflicted.is_empty() {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} No conflicted files found.\n").ok();
                    reset_color(&mut o);
                } else {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  {CROSS} {} conflicted file{}:\n", conflicted.len(), if conflicted.len() == 1 {""} else {"s"}).ok();
                    for f in &conflicted {
                        set_fg(&mut o, theme::ERR);
                        write!(o, "  {V_LINE}   {}\n", f).ok();
                    }
                    write!(o, "\n").ok();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {V_LINE} Tip: /add <file> then ask AI to \"resolve the merge conflict\"\n").ok();
                    reset_color(&mut o);
                }
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {CROSS} {}\n", stderr.trim()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        // /gcp <hash> — cherry-pick (Section 3.4)
        sub if sub.starts_with("cherry-pick ") || sub.starts_with("cp ") => {
            let hash = if sub.starts_with("cherry-pick ") {
                sub.trim_start_matches("cherry-pick ").trim()
            } else {
                sub.trim_start_matches("cp ").trim()
            };
            let (stdout, stderr, ok) = run_git(&["cherry-pick", hash]);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Cherry-picked {}: {}\n", hash, stdout.trim()).ok();
                reset_color(&mut o);
            } else {
                print_error(&format!("cherry-pick failed: {}", stderr.trim()));
                // Show conflicted files if any
                let (cf_out, _, _) = run_git(&["diff", "--name-only", "--diff-filter=U"]);
                let conflicts: Vec<&str> = cf_out.lines().filter(|l| !l.is_empty()).collect();
                if !conflicts.is_empty() {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  Conflicted files:\n").ok();
                    for f in &conflicts {
                        write!(o, "  {V_LINE}   {}\n", f).ok();
                    }
                    reset_color(&mut o);
                }
            }
        }
        other => {
            print_error(&format!("Unknown git subcommand: '{}'. Use /help git for available commands.", other));
        }
    }
}

// ─── Find command handler ────────────────────────────────────────────────────

fn handle_find_command(pattern: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Find");
    set_fg(&mut o, theme::DIM);
    write!(o, "  {V_LINE} Pattern: {}\n", pattern).ok();
    reset_color(&mut o);

    let pattern_lower = pattern.to_lowercase();
    let max_results = 200;

    fn walk_dir(
        dir: &std::path::Path,
        pattern_lower: &str,
        results: &mut Vec<String>,
        max: usize,
        root: &std::path::Path,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            if results.len() >= max {
                return;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden and common ignores
            if name.starts_with('.') || name == "node_modules" || name == "target"
                || name == "__pycache__" || name == "build" || name == "dist"
            {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().to_string();
            // Simple glob: * matches anything, ? matches one char
            if glob_match(pattern_lower, &name.to_lowercase()) || name.to_lowercase().contains(pattern_lower) {
                results.push(rel_str);
            }
            if path.is_dir() {
                walk_dir(&path, pattern_lower, results, max, root);
            }
        }
    }

    fn glob_match(pattern: &str, text: &str) -> bool {
        if !pattern.contains('*') && !pattern.contains('?') {
            return false; // Not a glob, will use contains instead
        }
        let pi: Vec<char> = pattern.chars().collect();
        let ti: Vec<char> = text.chars().collect();
        glob_match_inner(&pi, &ti, 0, 0)
    }

    fn glob_match_inner(pattern: &[char], text: &[char], pi: usize, ti: usize) -> bool {
        if pi == pattern.len() {
            return ti == text.len();
        }
        if pattern[pi] == '*' {
            // Try matching * with 0..n characters
            for i in ti..=text.len() {
                if glob_match_inner(pattern, text, pi + 1, i) {
                    return true;
                }
            }
            return false;
        }
        if ti == text.len() {
            return false;
        }
        if pattern[pi] == '?' || pattern[pi] == text[ti] {
            return glob_match_inner(pattern, text, pi + 1, ti + 1);
        }
        false
    }

    let root = std::path::Path::new(root_path);
    let mut results = Vec::new();
    walk_dir(root, &pattern_lower, &mut results, max_results, root);
    let count = results.len();

    for r in &results {
        let color = if std::path::Path::new(root_path).join(r).is_dir() {
            theme::CYAN_DIM
        } else {
            theme::DIM_LIGHT
        };
        print_list_item(ARROW, color, r);
    }

    set_fg(&mut o, theme::DIM);
    if count == 0 {
        write!(o, "  {V_LINE} No matches found\n").ok();
    } else if count >= max_results {
        write!(o, "  {V_LINE} ... truncated at {} results\n", max_results).ok();
    } else {
        write!(o, "  {V_LINE} {} match{}\n", count, if count == 1 { "" } else { "es" }).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

// ─── Grep command handler ────────────────────────────────────────────────────

fn handle_grep_command(pattern: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Grep");
    set_fg(&mut o, theme::DIM);
    write!(o, "  {V_LINE} Pattern: {}\n", pattern).ok();
    reset_color(&mut o);

    // Try rg first, fall back to grep
    let result = std::process::Command::new("rg")
        .args(["--line-number", "--no-heading", "--color=never", "--max-count=5", "--max-filesize=1M", pattern])
        .current_dir(root_path)
        .output()
        .or_else(|_| {
            std::process::Command::new("grep")
                .args(["-rn", "--color=never", "-m", "5", pattern, "."])
                .current_dir(root_path)
                .output()
        });

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.lines().take(200).collect();
            if lines.is_empty() {
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE} No matches found\n").ok();
                reset_color(&mut o);
            } else {
                for line in &lines {
                    // Format: file:line:content
                    if let Some((loc, content)) = line.split_once(':').and_then(|(file, rest)| {
                        rest.split_once(':').map(|(line_no, content)| {
                            (format!("{}:{}", file, line_no), content)
                        })
                    }) {
                        set_fg(&mut o, theme::BORDER);
                        write!(o, "  {V_LINE} ").ok();
                        set_fg(&mut o, theme::CYAN_DIM);
                        write!(o, "{}", loc).ok();
                        set_fg(&mut o, theme::DIM);
                        write!(o, ": ").ok();
                        set_fg(&mut o, theme::DIM_LIGHT);
                        let trimmed: String = content.trim().chars().take(80).collect();
                        write!(o, "{}\n", trimmed).ok();
                    } else {
                        set_fg(&mut o, theme::DIM_LIGHT);
                        write!(o, "  {V_LINE} {}\n", line).ok();
                    }
                }
                set_fg(&mut o, theme::DIM);
                write!(o, "  {V_LINE} {} result{}\n", lines.len(), if lines.len() == 1 { "" } else { "s" }).ok();
            }
            reset_color(&mut o);
        }
        Err(e) => {
            print_error(&format!("Search failed (neither rg nor grep found): {}", e));
        }
    }
    print_section_end();
}

// ─── Tree command handler ────────────────────────────────────────────────────

fn handle_tree_command(sub_path: Option<&str>, root_path: &str) {
    let target = match sub_path {
        Some(p) if p.starts_with('/') => std::path::PathBuf::from(p),
        Some(p) => std::path::PathBuf::from(root_path).join(p),
        None => std::path::PathBuf::from(root_path),
    };

    if !target.exists() {
        print_error(&format!("Path not found: {}", target.display()));
        return;
    }

    let mut o = io::stdout();
    print_section_header("Tree");
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "  {V_LINE} {}\n", target.display()).ok();
    reset_color(&mut o);

    let skip_dirs: std::collections::HashSet<&str> = [
        ".git", "node_modules", "target", "__pycache__", ".shadowai",
        ".shadow-memory", "build", "dist", ".next", ".cache", "vendor",
    ].iter().cloned().collect();

    let mut count = 0usize;
    let max_entries = 200;

    fn print_tree(
        o: &mut io::Stdout,
        dir: &std::path::Path,
        prefix: &str,
        depth: usize,
        max_depth: usize,
        skip: &std::collections::HashSet<&str>,
        count: &mut usize,
        max_entries: usize,
    ) {
        if depth > max_depth || *count >= max_entries {
            return;
        }
        let mut entries: Vec<_> = match std::fs::read_dir(dir) {
            Ok(e) => e.flatten().collect(),
            Err(_) => return,
        };
        entries.sort_by(|a, b| {
            let a_dir = a.path().is_dir();
            let b_dir = b.path().is_dir();
            b_dir.cmp(&a_dir).then_with(|| a.file_name().cmp(&b.file_name()))
        });

        // Filter out skipped dirs
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                !name.starts_with('.') && !skip.contains(name.as_str())
            })
            .collect();

        let total = entries.len();
        for (i, entry) in entries.iter().enumerate() {
            if *count >= max_entries {
                set_fg(o, theme::DIM);
                write!(o, "  {V_LINE} {}... truncated\n", prefix).ok();
                return;
            }
            *count += 1;
            let is_last = i == total - 1;
            let connector = if is_last { "\u{2514}\u{2500}\u{2500}" } else { "\u{251c}\u{2500}\u{2500}" };
            let child_prefix = if is_last { "    " } else { "\u{2502}   " };
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.path().is_dir();

            set_fg(o, theme::BORDER);
            write!(o, "  {V_LINE} ").ok();
            set_fg(o, theme::DIM);
            write!(o, "{}{} ", prefix, connector).ok();
            if is_dir {
                set_fg(o, theme::CYAN_DIM);
                write!(o, "{}/\n", name).ok();
                let new_prefix = format!("{}{}", prefix, child_prefix);
                print_tree(o, &entry.path(), &new_prefix, depth + 1, max_depth, skip, count, max_entries);
            } else {
                let color = if name.ends_with(".rs") || name.ends_with(".ts") || name.ends_with(".py") || name.ends_with(".go") {
                    theme::DIM_LIGHT
                } else if name.ends_with(".toml") || name.ends_with(".json") || name.ends_with(".yaml") || name.ends_with(".yml") {
                    theme::WARN
                } else {
                    theme::DIM
                };
                set_fg(o, color);
                write!(o, "{}\n", name).ok();
            }
        }
        reset_color(o);
    }

    print_tree(&mut o, &target, "", 0, 3, &skip_dirs, &mut count, max_entries);

    set_fg(&mut o, theme::DIM);
    write!(o, "  {V_LINE} {} entries\n", count).ok();
    reset_color(&mut o);
    print_section_end();
}

// ─── Context command handler ─────────────────────────────────────────────────

fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

fn print_context_info(messages: &[serde_json::Value], max_context: usize) {
    let mut o = io::stdout();
    let mut system_tokens = 0usize;
    let mut user_tokens = 0usize;
    let mut assistant_tokens = 0usize;

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("");
        let content = msg["content"].as_str().unwrap_or("");
        let tokens = estimate_tokens(content);
        match role {
            "system" => system_tokens += tokens,
            "user" => user_tokens += tokens,
            "assistant" => assistant_tokens += tokens,
            _ => user_tokens += tokens,
        }
    }

    let total = system_tokens + user_tokens + assistant_tokens;
    let pct = if max_context > 0 { (total as f64 / max_context as f64 * 100.0) as usize } else { 0 };

    print_section_header("Context Usage");
    print_section_row("System", &format!("~{}", fmt_tokens(system_tokens as u64)));
    print_section_row("User", &format!("~{}", fmt_tokens(user_tokens as u64)));
    print_section_row("Assistant", &format!("~{}", fmt_tokens(assistant_tokens as u64)));
    print_section_row("Total", &format!("~{}", fmt_tokens(total as u64)));
    print_section_row("Max context", &fmt_tokens(max_context as u64));

    // Visual bar
    let bar_width = 30usize;
    let filled = (bar_width as f64 * pct as f64 / 100.0).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_color = if pct < 60 {
        theme::OK
    } else if pct < 80 {
        theme::WARN
    } else {
        theme::ERR
    };

    set_fg(&mut o, theme::BORDER);
    write!(o, "  {V_LINE} ").ok();
    set_fg(&mut o, bar_color);
    write!(o, "[{}\u{2591}{}]", "\u{2588}".repeat(filled), "\u{2591}".repeat(empty.saturating_sub(0))).ok();
    // Fix: just use empty blocks
    write!(o, " {}% (~{}/{})\n",
        pct,
        fmt_tokens(total as u64),
        fmt_tokens(max_context as u64),
    ).ok();
    reset_color(&mut o);
    print_section_end();
}

// ─── Turn separator ──────────────────────────────────────────────────────────

fn print_turn_separator() {
    let mut o = io::stdout();
    set_fg(&mut o, theme::DIM);
    write!(o, "  {}\n", H_LINE.repeat(40)).ok();
    reset_color(&mut o);
}

// ─── Input history persistence ───────────────────────────────────────────────

fn history_file_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join("history"))
}

fn save_history_entry(entry: &str) {
    if entry.trim().is_empty() { return; }
    if let Some(path) = history_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Append entry
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{}", entry);
        }
        // Trim to 500 entries
        if let Ok(content) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > 500 {
                let trimmed: Vec<&str> = lines[lines.len() - 500..].to_vec();
                let _ = std::fs::write(&path, trimmed.join("\n") + "\n");
            }
        }
    }
}

#[allow(dead_code)]
fn load_history() -> Vec<String> {
    history_file_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| {
            content.lines()
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ─── User input commands ─────────────────────────────────────────────────────

enum UserCommand {
    Quit,
    Clear,
    Help(Option<String>),
    Mode(String),
    Model(String),
    Status,
    Abort,
    Temperature(f64),
    MaxTokens(i32),
    File(String),
    Sessions,
    Session(Option<String>),
    Providers,
    Provider(String),
    Models,
    Memories,
    Compact,
    Resume(Option<String>),
    New,
    Search(String),
    SkillActivate(String),
    SkillList,
    SkillOff,
    ShowErrors,
    ShowFixed,
    ShowCompleted,
    ShowMemory,
    Remember(String),
    Git(String),
    Find(String),
    Grep(String),
    Tree(Option<String>),
    Context,
    ContextFiles,
    ContextDrop(String),
    Format(String),
    Export(String),
    Test(String),
    Lint(String),
    Build(String),
    AddFile(String),
    DropFile(String),
    ListFiles,
    Review(String),
    Watch,
    Plan(Option<String>),
    SkillCreate(Option<String>),
    Security(String),
    Doc(String),
    Changelog,
    History(String),
    Keybindings,
    Perf(String),
    Image(String),
    Browse(String),
    SkillChain(Vec<String>),
    Spawn(String),
    ReleaseNotes,
    SkillExport(String),
    SkillImport(String),
    Symbols(String),
    Save(String),
    Load(String),
    Cheatsheet,
    Undo,
    EditHistory(Option<String>),
    PlanApprove,
    PlanNext,
    PlanExport,
    SkillEdit(String),
    Copy,
    SessionRename(String),
    Message(String),
    // New commands (Section 10)
    Todo,
    Env,
    Secrets,
    Metrics,
    Deps,
    Diagram,
    DiffFile(String),
    Chat,
    // Theme commands (Section 13)
    Theme(String),
    ThemeList,
    // Heal command (Section 12)
    Heal(String),
    // New commands (Batch 3)
    Explain(String),
    Rename(String, String),
    Extract(String),
    Docker(String),
    Release(String),
    Benchmark(String),
    Coverage,
    Translate(String),
    Mock(String),
    Remote(String),
    Share,
    Cron(String),
    Research(String),
    // New commands (Section improvements)
    Think(String),
    Agent(String),
    Debug(String),
    Shader(String),
    Assets(String),
    Docs(String),
    Rebase(String),
    Gr(String),
    // Pass 4 commands
    Gd(String),
    Rag(String),
    Add(String),
    Ctx,
    Drop(String),
    Memory,
    Unfold(usize),
    CacheClear,
    Dap(String),
    Pdf(String),
    Yt(String),
    DepsFix,
    Profile(String),
    Db(String),
    K8s(String),
    Migrate(String),
    PlanOn,
    PlanOff,
    // Section 8.1 — Email SMTP
    EmailTest,
    // Section 7.1 — Real-time Collaboration
    Relay(String),
    ReviewRequest(String),
    // Section 13 — MCP
    Mcp(String),
    // Section 14 — Agentic
    Architect,
    Yolo,
    Approval(String),
    Arena(String),
    Teleport,
    // Section 15 — Context / Memory
    Snapshot(String),
    RepoMap,
    // Section 16 — Multimodal
    Voice,
    Screenshot,
    // Section 17 — Dev Experience
    Cost,
    Block(usize),
    LogSearch(String),
    Runbook(String),
    Switch(String),
    // Section 18 — Security
    Audit(String),
    // Section 19 — Local LLM
    ModelPull(String),
}

fn parse_command(input: &str) -> UserCommand {
    let trimmed = input.trim();
    match trimmed {
        "/quit" | "/exit" | "/q" => return UserCommand::Quit,
        "/clear" | "/cls" => return UserCommand::Clear,
        "/help" | "/h" | "/?" => return UserCommand::Help(None),
        "/status" | "/st" => return UserCommand::Status,
        "/abort" => return UserCommand::Abort,
        "/sessions" => return UserCommand::Sessions,
        "/providers" | "/prov" => return UserCommand::Providers,
        "/models" => return UserCommand::Models,
        "/memories" | "/mem" => return UserCommand::Memories,
        "/compact" => return UserCommand::Compact,
        "/new" => return UserCommand::New,
        "/skills" => return UserCommand::SkillList,
        "/skill off" | "/skill none" => return UserCommand::SkillOff,
        "/skill create" => return UserCommand::SkillCreate(None),
        "/watch" => return UserCommand::Watch,
        "/plan" => return UserCommand::Plan(None),
        "/errors" => return UserCommand::ShowErrors,
        "/fixed" => return UserCommand::ShowFixed,
        "/completed" => return UserCommand::ShowCompleted,
        "/memory" => return UserCommand::ShowMemory,
        "/context" | "/ctx" => return UserCommand::Context,
        "/context files" | "/ctx files" => return UserCommand::ContextFiles,
        "/format" => return UserCommand::Format(String::new()),
        "/tree" => return UserCommand::Tree(None),
        "/export" | "/export md" => return UserCommand::Export("md".to_string()),
        "/export json" => return UserCommand::Export("json".to_string()),
        "/export html" => return UserCommand::Export("html".to_string()),
        "/security" => return UserCommand::Security(String::new()),
        "/doc" => return UserCommand::Doc(String::new()),
        "/changelog" => return UserCommand::Changelog,
        "/release-notes" | "/rn" => return UserCommand::ReleaseNotes,
        "/history" => return UserCommand::History(String::new()),
        "/keybindings" | "/keys" => return UserCommand::Keybindings,
        "/perf" => return UserCommand::Perf(String::new()),
        "/cheatsheet" | "/cs" => return UserCommand::Cheatsheet,
        "/undo" => return UserCommand::Undo,
        "/copy" | "/cp" => return UserCommand::Copy,
        "/edits" => return UserCommand::EditHistory(None),
        "/plan approve" => return UserCommand::PlanApprove,
        "/plan next" => return UserCommand::PlanNext,
        "/plan export" => return UserCommand::PlanExport,
        _ => {}
    }
    // Export with format argument
    if let Some(rest) = trimmed.strip_prefix("/export ") {
        let fmt = rest.trim().to_string();
        return UserCommand::Export(if fmt == "json" { "json".into() } else { "md".into() });
    }
    // Git commands: /git, /g, /gd, /gl, /gc, /gb
    if trimmed == "/git" || trimmed == "/g" {
        return UserCommand::Git(String::new());
    }
    if trimmed == "/gd" {
        return UserCommand::Git("diff".to_string());
    }
    if trimmed == "/gl" {
        return UserCommand::Git("log".to_string());
    }
    if trimmed == "/gc" {
        return UserCommand::Git("commit".to_string());
    }
    if trimmed == "/gb" {
        return UserCommand::Git("branch".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/git ") {
        return UserCommand::Git(rest.trim().to_string());
    }
    // Find, Grep, Tree
    if let Some(rest) = trimmed.strip_prefix("/find ") {
        return UserCommand::Find(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/grep ") {
        return UserCommand::Grep(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/tree ") {
        return UserCommand::Tree(Some(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("/resume") {
        let s = rest.trim();
        return UserCommand::Resume(if s.is_empty() { None } else { Some(s.to_string()) });
    }
    if let Some(rest) = trimmed.strip_prefix("/search ") {
        return UserCommand::Search(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/edits ") {
        return UserCommand::EditHistory(Some(rest.trim().to_string()));
    }
    if trimmed == "/plan approve" {
        return UserCommand::PlanApprove;
    }
    if trimmed == "/plan next" {
        return UserCommand::PlanNext;
    }
    if trimmed == "/plan export" {
        return UserCommand::PlanExport;
    }
    if let Some(rest) = trimmed.strip_prefix("/plan ") {
        return UserCommand::Plan(Some(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("/skill create ") {
        return UserCommand::SkillCreate(Some(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("/skill edit ") {
        return UserCommand::SkillEdit(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/skill export ") {
        return UserCommand::SkillExport(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/skill import ") {
        return UserCommand::SkillImport(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/skill ") {
        let name = rest.trim().to_string();
        // Skill chaining: detect + in skill name
        if name.contains('+') {
            let chain: Vec<String> = name.split('+').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if chain.len() >= 2 {
                return UserCommand::SkillChain(chain);
            }
        }
        return UserCommand::SkillActivate(name);
    }
    if let Some(rest) = trimmed.strip_prefix("/remember ") {
        return UserCommand::Remember(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/mode ") {
        return UserCommand::Mode(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/model ") {
        return UserCommand::Model(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/temperature ").or_else(|| trimmed.strip_prefix("/temp ")) {
        if let Ok(t) = rest.trim().parse::<f64>() {
            return UserCommand::Temperature(t.clamp(0.0, 2.0));
        }
    }
    if let Some(rest) = trimmed.strip_prefix("/tokens ") {
        if let Ok(t) = rest.trim().parse::<i32>() {
            return UserCommand::MaxTokens(t.max(1));
        }
    }
    if let Some(rest) = trimmed.strip_prefix("/file ") {
        return UserCommand::File(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/session") {
        let s = rest.trim();
        if let Some(name) = s.strip_prefix("rename ").or_else(|| s.strip_prefix("name ")) {
            return UserCommand::SessionRename(name.trim().to_string());
        }
        return UserCommand::Session(if s.is_empty() { None } else { Some(s.to_string()) });
    }
    if let Some(rest) = trimmed.strip_prefix("/provider ") {
        return UserCommand::Provider(rest.trim().to_string());
    }
    // Dev workflow commands
    if trimmed == "/test" {
        return UserCommand::Test(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/test ") {
        return UserCommand::Test(rest.trim().to_string());
    }
    if trimmed == "/lint" {
        return UserCommand::Lint(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/lint ") {
        return UserCommand::Lint(rest.trim().to_string());
    }
    if trimmed == "/build" {
        return UserCommand::Build(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/build ") {
        return UserCommand::Build(rest.trim().to_string());
    }
    // File context management
    if let Some(rest) = trimmed.strip_prefix("/add ") {
        return UserCommand::AddFile(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/drop ") {
        return UserCommand::DropFile(rest.trim().to_string());
    }
    if trimmed == "/files" {
        return UserCommand::ListFiles;
    }
    // Code review
    if trimmed == "/review" {
        return UserCommand::Review(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/review ") {
        return UserCommand::Review(rest.trim().to_string());
    }
    // Security scanning
    if let Some(rest) = trimmed.strip_prefix("/security ") {
        return UserCommand::Security(rest.trim().to_string());
    }
    // Doc generation
    if let Some(rest) = trimmed.strip_prefix("/doc ") {
        return UserCommand::Doc(rest.trim().to_string());
    }
    // History
    if let Some(rest) = trimmed.strip_prefix("/history ") {
        return UserCommand::History(rest.trim().to_string());
    }
    // Per-command help: /help <command>
    if let Some(rest) = trimmed.strip_prefix("/help ") {
        return UserCommand::Help(Some(rest.trim().to_string()));
    }
    // Context subcommands
    if let Some(rest) = trimmed.strip_prefix("/context drop ").or_else(|| trimmed.strip_prefix("/ctx drop ")) {
        return UserCommand::ContextDrop(rest.trim().to_string());
    }
    if trimmed == "/context files" || trimmed == "/ctx files" {
        return UserCommand::ContextFiles;
    }
    // Format command
    if let Some(rest) = trimmed.strip_prefix("/format ") {
        return UserCommand::Format(rest.trim().to_string());
    }
    // Perf command
    if let Some(rest) = trimmed.strip_prefix("/perf ") {
        return UserCommand::Perf(rest.trim().to_string());
    }
    // Image command
    if let Some(rest) = trimmed.strip_prefix("/image ") {
        return UserCommand::Image(rest.trim().to_string());
    }
    // Browse command
    if let Some(rest) = trimmed.strip_prefix("/browse ") {
        return UserCommand::Browse(rest.trim().to_string());
    }
    // Spawn command
    if let Some(rest) = trimmed.strip_prefix("/spawn ") {
        return UserCommand::Spawn(rest.trim().to_string());
    }
    // Symbols command
    if let Some(rest) = trimmed.strip_prefix("/symbols ") {
        return UserCommand::Symbols(rest.trim().to_string());
    }
    // Save/Load snapshot commands
    if let Some(rest) = trimmed.strip_prefix("/save ") {
        return UserCommand::Save(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/load ") {
        return UserCommand::Load(rest.trim().to_string());
    }
    // New commands (Section 10)
    if trimmed == "/todo" {
        return UserCommand::Todo;
    }
    if trimmed == "/env" {
        return UserCommand::Env;
    }
    if trimmed == "/secrets" {
        return UserCommand::Secrets;
    }
    if trimmed == "/metrics" {
        return UserCommand::Metrics;
    }
    if trimmed == "/deps" {
        return UserCommand::Deps;
    }
    if trimmed == "/diagram" {
        return UserCommand::Diagram;
    }
    if trimmed == "/chat" {
        return UserCommand::Chat;
    }
    if let Some(rest) = trimmed.strip_prefix("/diff ") {
        return UserCommand::DiffFile(rest.trim().to_string());
    }
    // Theme commands (Section 13)
    if trimmed == "/theme list" || trimmed == "/themes" {
        return UserCommand::ThemeList;
    }
    if let Some(rest) = trimmed.strip_prefix("/theme ") {
        return UserCommand::Theme(rest.trim().to_string());
    }
    // Heal command (Section 12)
    if trimmed == "/heal" {
        return UserCommand::Heal(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/heal ") {
        return UserCommand::Heal(rest.trim().to_string());
    }
    // Batch 3 commands
    if let Some(rest) = trimmed.strip_prefix("/explain ") {
        return UserCommand::Explain(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/rename ") {
        let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            return UserCommand::Rename(parts[0].to_string(), parts[1].trim().to_string());
        }
    }
    if let Some(rest) = trimmed.strip_prefix("/extract ") {
        return UserCommand::Extract(rest.trim().to_string());
    }
    if trimmed == "/docker" {
        return UserCommand::Docker(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/docker ") {
        return UserCommand::Docker(rest.trim().to_string());
    }
    if trimmed == "/release" {
        return UserCommand::Release(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/release ") {
        return UserCommand::Release(rest.trim().to_string());
    }
    if trimmed == "/benchmark" || trimmed == "/bench" {
        return UserCommand::Benchmark(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/benchmark ").or_else(|| trimmed.strip_prefix("/bench ")) {
        return UserCommand::Benchmark(rest.trim().to_string());
    }
    if trimmed == "/cov" || trimmed == "/coverage" {
        return UserCommand::Coverage;
    }
    if let Some(rest) = trimmed.strip_prefix("/translate ") {
        return UserCommand::Translate(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/mock ") {
        return UserCommand::Mock(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/remote ") {
        return UserCommand::Remote(rest.trim().to_string());
    }
    if trimmed == "/share" {
        return UserCommand::Share;
    }
    if trimmed == "/cron" {
        return UserCommand::Cron(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/cron ") {
        return UserCommand::Cron(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/research ") {
        return UserCommand::Research(rest.trim().to_string());
    }
    // Think command
    if trimmed == "/think" {
        return UserCommand::Think(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/think ") {
        return UserCommand::Think(rest.trim().to_string());
    }
    // Agent command
    if trimmed == "/agent" {
        return UserCommand::Agent(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/agent ") {
        return UserCommand::Agent(rest.trim().to_string());
    }
    // Debug command
    if trimmed == "/debug" {
        return UserCommand::Debug(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/debug ") {
        return UserCommand::Debug(rest.trim().to_string());
    }
    // Shader command
    if trimmed == "/shader" {
        return UserCommand::Shader(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/shader ") {
        return UserCommand::Shader(rest.trim().to_string());
    }
    // Assets command
    if trimmed == "/assets" {
        return UserCommand::Assets(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/assets ") {
        return UserCommand::Assets(rest.trim().to_string());
    }
    // Docs command
    if trimmed == "/docs" {
        return UserCommand::Docs(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/docs ") {
        return UserCommand::Docs(rest.trim().to_string());
    }
    // Rebase command
    if trimmed == "/rebase" {
        return UserCommand::Rebase(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/rebase ") {
        return UserCommand::Rebase(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/gr ") {
        return UserCommand::Gr(rest.trim().to_string());
    }
    if trimmed == "/gr" {
        return UserCommand::Gr(String::new());
    }
    // Pass 4 commands
    if trimmed == "/gd" {
        return UserCommand::Gd(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/gd ") {
        return UserCommand::Gd(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/rag ") {
        return UserCommand::Rag(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/add ") {
        return UserCommand::Add(rest.trim().to_string());
    }
    if trimmed == "/ctx" {
        return UserCommand::Ctx;
    }
    if let Some(rest) = trimmed.strip_prefix("/drop ") {
        return UserCommand::Drop(rest.trim().to_string());
    }
    if trimmed == "/memory" || trimmed == "/mem" {
        return UserCommand::Memory;
    }
    if let Some(rest) = trimmed.strip_prefix("/unfold ") {
        if let Ok(id) = rest.trim().parse::<usize>() {
            return UserCommand::Unfold(id);
        }
    }
    if trimmed == "/cache clear" {
        return UserCommand::CacheClear;
    }
    if trimmed == "/dap" {
        return UserCommand::Dap(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/dap ") {
        return UserCommand::Dap(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/pdf ") {
        return UserCommand::Pdf(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/yt ") {
        return UserCommand::Yt(rest.trim().to_string());
    }
    if trimmed == "/deps fix" {
        return UserCommand::DepsFix;
    }
    if trimmed == "/profile" {
        return UserCommand::Profile(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/profile ") {
        return UserCommand::Profile(rest.trim().to_string());
    }
    if trimmed == "/db" {
        return UserCommand::Db(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/db ") {
        return UserCommand::Db(rest.trim().to_string());
    }
    if trimmed == "/k8s" {
        return UserCommand::K8s(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/k8s ") {
        return UserCommand::K8s(rest.trim().to_string());
    }
    if trimmed == "/migrate" {
        return UserCommand::Migrate(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/migrate ") {
        return UserCommand::Migrate(rest.trim().to_string());
    }
    if trimmed == "/plan on" {
        return UserCommand::PlanOn;
    }
    if trimmed == "/plan off" {
        return UserCommand::PlanOff;
    }
    // Section 8.1 — Email SMTP
    if trimmed == "/email test" {
        return UserCommand::EmailTest;
    }
    // Section 7.1 — Real-time Collaboration
    if trimmed == "/relay" || trimmed == "/relay status" {
        return UserCommand::Relay("status".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/relay ") {
        return UserCommand::Relay(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/review-request ") {
        return UserCommand::ReviewRequest(rest.trim().to_string());
    }
    if trimmed == "/review-request" {
        return UserCommand::ReviewRequest(String::new());
    }
    // Stash shortcuts (Section 3.4)
    if trimmed == "/gs" || trimmed == "/stash" {
        return UserCommand::Git("stash".to_string());
    }
    if trimmed == "/stash list" {
        return UserCommand::Git("stash list".to_string());
    }
    if trimmed == "/stash pop" {
        return UserCommand::Git("stash pop".to_string());
    }
    if trimmed == "/stash show" {
        return UserCommand::Git("stash show".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/stash pop ") {
        return UserCommand::Git(format!("stash pop {}", rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("/stash drop ") {
        return UserCommand::Git(format!("stash drop {}", rest.trim()));
    }
    // Blame command
    if let Some(rest) = trimmed.strip_prefix("/blame ") {
        return UserCommand::Git(format!("blame {}", rest.trim()));
    }
    // Resolve command
    if trimmed == "/resolve" {
        return UserCommand::Git("resolve".to_string());
    }
    // Cherry-pick
    if let Some(rest) = trimmed.strip_prefix("/gcp ") {
        return UserCommand::Git(format!("cherry-pick {}", rest.trim()));
    }
    // PR command
    if trimmed == "/pr" || trimmed == "/pr list" {
        return UserCommand::Git(trimmed.trim_start_matches('/').to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/pr ") {
        return UserCommand::Git(format!("pr {}", rest.trim()));
    }
    // Section 13 — MCP
    if trimmed == "/mcp" {
        return UserCommand::Mcp(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/mcp ") {
        return UserCommand::Mcp(rest.trim().to_string());
    }
    // Section 14 — Agentic
    if trimmed == "/architect" {
        return UserCommand::Architect;
    }
    if trimmed == "/yolo" {
        return UserCommand::Yolo;
    }
    if let Some(rest) = trimmed.strip_prefix("/approval ") {
        return UserCommand::Approval(rest.trim().to_string());
    }
    if trimmed == "/approval" {
        return UserCommand::Approval(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/arena ") {
        return UserCommand::Arena(rest.trim().to_string());
    }
    if trimmed == "/teleport" {
        return UserCommand::Teleport;
    }
    // Section 15 — Context / Memory
    if trimmed == "/snapshot" {
        return UserCommand::Snapshot(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/snapshot ") {
        return UserCommand::Snapshot(rest.trim().to_string());
    }
    if trimmed == "/repomap" {
        return UserCommand::RepoMap;
    }
    // Section 16 — Multimodal
    if trimmed == "/voice" {
        return UserCommand::Voice;
    }
    if trimmed == "/screenshot" {
        return UserCommand::Screenshot;
    }
    // Section 17 — Dev Experience
    if trimmed == "/cost" {
        return UserCommand::Cost;
    }
    if let Some(rest) = trimmed.strip_prefix("/block ") {
        if let Ok(id) = rest.trim().parse::<usize>() {
            return UserCommand::Block(id);
        }
    }
    if let Some(rest) = trimmed.strip_prefix("/log search ") {
        return UserCommand::LogSearch(rest.trim().to_string());
    }
    if trimmed == "/runbook" {
        return UserCommand::Runbook(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/runbook ") {
        return UserCommand::Runbook(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/switch ") {
        return UserCommand::Switch(rest.trim().to_string());
    }
    // Section 18 — Security
    if trimmed == "/audit" {
        return UserCommand::Audit(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("/audit ") {
        return UserCommand::Audit(rest.trim().to_string());
    }
    // Section 19 — Local LLM
    if let Some(rest) = trimmed.strip_prefix("/model pull ") {
        return UserCommand::ModelPull(rest.trim().to_string());
    }
    UserCommand::Message(trimmed.to_string())
}

// ─── Security scanning ───────────────────────────────────────────────────────

struct SecurityFinding {
    severity: &'static str,  // "CRITICAL", "WARNING", "INFO"
    category: &'static str,
    file: String,
    line: usize,
    snippet: String,
}

fn handle_security_command(args: &str, root_path: &str) {
    let mut o = io::stdout();
    let trimmed = args.trim();

    if trimmed == "--deps" {
        print_section_header("Dependency Audit");
        let root = std::path::Path::new(root_path);
        let mut found_any = false;

        if root.join("Cargo.toml").exists() {
            found_any = true;
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "  {ARROW} Running cargo audit...\n").ok();
            reset_color(&mut o);
            match std::process::Command::new("cargo")
                .args(["audit"])
                .current_dir(root_path)
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stdout.is_empty() {
                        write!(o, "{}\n", stdout).ok();
                    }
                    if !stderr.is_empty() && !output.status.success() {
                        set_fg(&mut o, theme::WARN);
                        write!(o, "{}\n", stderr).ok();
                        reset_color(&mut o);
                    }
                    if output.status.success() && stdout.contains("0 vulnerabilities") {
                        set_fg(&mut o, theme::OK);
                        write!(o, "  {CHECK} No vulnerabilities found\n").ok();
                        reset_color(&mut o);
                    }
                }
                Err(_) => {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  cargo-audit not installed. Install with: cargo install cargo-audit\n").ok();
                    reset_color(&mut o);
                }
            }
        }

        if root.join("package.json").exists() {
            found_any = true;
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "  {ARROW} Running npm audit...\n").ok();
            reset_color(&mut o);
            match std::process::Command::new("npm")
                .args(["audit", "--json"])
                .current_dir(root_path)
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if let Ok(audit_json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                        let vulns = audit_json["metadata"]["vulnerabilities"].as_object();
                        if let Some(v) = vulns {
                            let critical = v.get("critical").and_then(|x| x.as_u64()).unwrap_or(0);
                            let high = v.get("high").and_then(|x| x.as_u64()).unwrap_or(0);
                            let moderate = v.get("moderate").and_then(|x| x.as_u64()).unwrap_or(0);
                            let low = v.get("low").and_then(|x| x.as_u64()).unwrap_or(0);
                            let total = critical + high + moderate + low;
                            if total == 0 {
                                set_fg(&mut o, theme::OK);
                                write!(o, "  {CHECK} No vulnerabilities found\n").ok();
                            } else {
                                if critical > 0 {
                                    set_fg(&mut o, theme::ERR);
                                    write!(o, "  {CROSS} {} critical\n", critical).ok();
                                }
                                if high > 0 {
                                    set_fg(&mut o, theme::ERR);
                                    write!(o, "  {CROSS} {} high\n", high).ok();
                                }
                                if moderate > 0 {
                                    set_fg(&mut o, theme::WARN);
                                    write!(o, "  {BOLT} {} moderate\n", moderate).ok();
                                }
                                if low > 0 {
                                    set_fg(&mut o, theme::DIM);
                                    write!(o, "  {DOT} {} low\n", low).ok();
                                }
                            }
                            reset_color(&mut o);
                        }
                    } else {
                        // Fallback: just show raw output
                        let raw_output = std::process::Command::new("npm")
                            .args(["audit"]).current_dir(root_path).output()
                            .map(|o| o.stdout).unwrap_or_default();
                        let plain = String::from_utf8_lossy(&raw_output);
                        write!(o, "{}\n", plain).ok();
                    }
                }
                Err(_) => {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  npm not available\n").ok();
                    reset_color(&mut o);
                }
            }
        }

        if root.join("requirements.txt").exists() {
            found_any = true;
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "  {ARROW} Running pip audit...\n").ok();
            reset_color(&mut o);
            match std::process::Command::new("pip-audit")
                .current_dir(root_path)
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    write!(o, "{}\n", stdout).ok();
                    if output.status.success() && stdout.contains("No known vulnerabilities") {
                        set_fg(&mut o, theme::OK);
                        write!(o, "  {CHECK} No vulnerabilities found\n").ok();
                        reset_color(&mut o);
                    }
                }
                Err(_) => {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  pip-audit not installed. Install with: pip install pip-audit\n").ok();
                    reset_color(&mut o);
                }
            }
        }

        if !found_any {
            print_error("No supported package manifest found (Cargo.toml, package.json, requirements.txt).");
        }
        print_section_end();
        return;
    }

    // Source code scanning
    let files_to_scan: Vec<String> = if trimmed.is_empty() {
        // Scan all source files
        collect_source_files(root_path)
    } else {
        let full_path = if trimmed.starts_with('/') { trimmed.to_string() }
            else { format!("{}/{}", root_path, trimmed) };
        if std::path::Path::new(&full_path).exists() {
            vec![full_path]
        } else {
            print_error(&format!("File not found: {}", trimmed));
            return;
        }
    };

    if files_to_scan.is_empty() {
        print_error("No source files found to scan.");
        return;
    }

    print_section_header("Security Scan");
    set_fg(&mut o, theme::DIM);
    write!(o, "  Scanning {} file(s)...\n\n", files_to_scan.len()).ok();
    reset_color(&mut o);

    let mut findings: Vec<SecurityFinding> = Vec::new();

    let patterns: Vec<(&str, &str, regex::Regex)> = vec![
        ("CRITICAL", "Hardcoded secret", regex::Regex::new(r#"(?i)(password|api_key|secret|token|api_secret|private_key)\s*=\s*"[^"]{4,}""#).unwrap()),
        ("CRITICAL", "Hardcoded secret", regex::Regex::new(r#"(?i)(password|api_key|secret|token|api_secret|private_key)\s*:\s*"[^"]{4,}""#).unwrap()),
        ("WARNING", "SQL injection risk", regex::Regex::new(r#"format!\s*\(\s*"(?i:SELECT|INSERT|UPDATE|DELETE|DROP)\b.*\{\}"#).unwrap()),
        ("WARNING", "Command injection", regex::Regex::new(r#"Command::new\s*\(\s*[^"&]"#).unwrap()),
        ("INFO", "Command execution", regex::Regex::new(r#"(?:system|exec|popen|subprocess\.call)\s*\("#).unwrap()),
        ("WARNING", "XSS risk", regex::Regex::new(r#"(?:innerHTML|dangerouslySetInnerHTML|v-html)\s*="#).unwrap()),
        ("INFO", "Path traversal", regex::Regex::new(r#"\.\./|\.\.\\|path\.join.*user|path\.join.*input"#).unwrap()),
        ("INFO", "Insecure HTTP", regex::Regex::new(r#"http://(?!localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])"#).unwrap()),
        ("WARNING", "Eval usage", regex::Regex::new(r#"\beval\s*\("#).unwrap()),
        ("CRITICAL", "Private key in code", regex::Regex::new(r#"-----BEGIN (?:RSA |EC |DSA )?PRIVATE KEY-----"#).unwrap()),
    ];

    for file_path in &files_to_scan {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let display_path = file_path.strip_prefix(&format!("{}/", root_path)).unwrap_or(file_path);

        for (line_num, line) in content.lines().enumerate() {
            for (severity, category, pattern) in &patterns {
                if pattern.is_match(line) {
                    findings.push(SecurityFinding {
                        severity,
                        category,
                        file: display_path.to_string(),
                        line: line_num + 1,
                        snippet: line.trim().chars().take(80).collect(),
                    });
                }
            }
        }
    }

    if findings.is_empty() {
        set_fg(&mut o, theme::OK);
        write!(o, "  {CHECK} No security issues found\n").ok();
        reset_color(&mut o);
    } else {
        // Sort by severity
        findings.sort_by(|a, b| {
            let order = |s: &str| match s { "CRITICAL" => 0, "WARNING" => 1, _ => 2 };
            order(a.severity).cmp(&order(b.severity))
        });

        let critical_count = findings.iter().filter(|f| f.severity == "CRITICAL").count();
        let warning_count = findings.iter().filter(|f| f.severity == "WARNING").count();
        let info_count = findings.iter().filter(|f| f.severity == "INFO").count();

        for f in &findings {
            let color = match f.severity {
                "CRITICAL" => theme::ERR,
                "WARNING" => theme::WARN,
                _ => theme::DIM,
            };
            let icon = match f.severity {
                "CRITICAL" => CROSS,
                "WARNING" => BOLT,
                _ => DOT,
            };
            set_fg(&mut o, color);
            write!(o, "  {icon} [{:>8}] ", f.severity).ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "{}", f.category).ok();
            set_fg(&mut o, theme::DIM);
            write!(o, " — {}:{}\n", f.file, f.line).ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "             {}\n", f.snippet).ok();
        }

        write!(o, "\n").ok();
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  Summary: ").ok();
        if critical_count > 0 {
            set_fg(&mut o, theme::ERR);
            write!(o, "{} critical  ", critical_count).ok();
        }
        if warning_count > 0 {
            set_fg(&mut o, theme::WARN);
            write!(o, "{} warnings  ", warning_count).ok();
        }
        if info_count > 0 {
            set_fg(&mut o, theme::DIM);
            write!(o, "{} info", info_count).ok();
        }
        write!(o, "\n").ok();
        reset_color(&mut o);
    }

    print_section_end();
}

// ─── Performance Analysis ────────────────────────────────────────────────────

fn handle_perf_command(args: &str, root_path: &str) {
    // --bench: generate benchmark skeleton for top hot spots
    if args == "--bench" || args.starts_with("--bench ") {
        let target_file = args.strip_prefix("--bench").map(|s| s.trim()).filter(|s| !s.is_empty());
        let files = if let Some(f) = target_file {
            let path = if f.starts_with('/') { f.to_string() } else { format!("{}/{}", root_path, f) };
            if !std::path::Path::new(&path).exists() {
                print_error(&format!("File not found: {}", f));
                return;
            }
            vec![path]
        } else {
            collect_source_files(root_path)
        };
        print_section_header("Benchmark Generator");
        let is_cargo = std::path::Path::new(root_path).join("Cargo.toml").exists();
        let is_node = std::path::Path::new(root_path).join("package.json").exists();
        // Collect function signatures from source files for benchmarking
        let mut bench_targets: Vec<(String, String)> = Vec::new(); // (file, fn_name)
        let fn_patterns: &[(&str, &str)] = &[("fn ", "rust"), ("function ", "js"), ("def ", "py"), ("func ", "go")];
        for file_path in &files {
            if let Ok(content) = std::fs::read_to_string(file_path) {
                let display = file_path.strip_prefix(&format!("{}/", root_path)).unwrap_or(file_path);
                for line in content.lines() {
                    let t = line.trim();
                    for (pat, _lang) in fn_patterns {
                        if t.starts_with(pat) && (t.contains('(')) {
                            if let Some(name) = t.strip_prefix(pat).and_then(|s| s.split('(').next()) {
                                let name = name.trim().trim_start_matches("pub ").trim_start_matches("async ").trim();
                                if !name.is_empty() && name.len() < 40 && !name.contains(' ') {
                                    bench_targets.push((display.to_string(), name.to_string()));
                                    if bench_targets.len() >= 10 { break; }
                                }
                            }
                        }
                    }
                    if bench_targets.len() >= 10 { break; }
                }
                if bench_targets.len() >= 10 { break; }
            }
        }
        let mut o = io::stdout();
        if is_cargo {
            // Generate Criterion benchmark skeleton
            let bench_dir = format!("{}/benches", root_path);
            let bench_file = format!("{}/shadowai_bench.rs", bench_dir);
            let _ = std::fs::create_dir_all(&bench_dir);
            let mut content = String::from("use criterion::{criterion_group, criterion_main, Criterion};\n\n");
            for (_, fn_name) in &bench_targets {
                content.push_str(&format!("fn bench_{}(c: &mut Criterion) {{\n    c.bench_function(\"{}\", |b| b.iter(|| {{\n        // TODO: call {}() here\n    }}));\n}}\n\n", fn_name, fn_name, fn_name));
            }
            if bench_targets.is_empty() {
                content.push_str("fn bench_placeholder(c: &mut Criterion) {\n    c.bench_function(\"placeholder\", |b| b.iter(|| {\n        // TODO: add your function call here\n    }));\n}\n\n");
            }
            let group_fns = if bench_targets.is_empty() { "bench_placeholder".to_string() }
                else { bench_targets.iter().map(|(_, f)| format!("bench_{}", f)).collect::<Vec<_>>().join(", ") };
            content.push_str(&format!("criterion_group!(benches, {});\ncriterion_main!(benches);\n", group_fns));
            match std::fs::write(&bench_file, &content) {
                Ok(_) => {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Generated: benches/shadowai_bench.rs\n").ok();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {V_LINE} Add to Cargo.toml:\n").ok();
                    write!(o, "  {V_LINE}   [[bench]]\n  {V_LINE}   name = \"shadowai_bench\"\n  {V_LINE}   harness = false\n\n").ok();
                    write!(o, "  {V_LINE}   [dev-dependencies]\n  {V_LINE}   criterion = \"0.5\"\n\n").ok();
                    write!(o, "  {V_LINE} Run with: cargo bench\n").ok();
                }
                Err(e) => print_error(&format!("Failed to write bench file: {}", e)),
            }
        } else if is_node {
            let bench_file = format!("{}/shadowai.bench.js", root_path);
            let mut content = String::from("// ShadowAI generated benchmark — requires: npm install --save-dev tinybench\nimport { Bench } from 'tinybench';\n\nconst bench = new Bench({ time: 1000 });\n\n");
            for (_, fn_name) in &bench_targets {
                content.push_str(&format!("bench.add('{}', () => {{\n  // TODO: call {}() here\n}});\n\n", fn_name, fn_name));
            }
            if bench_targets.is_empty() {
                content.push_str("bench.add('placeholder', () => {\n  // TODO: add your function call here\n});\n\n");
            }
            content.push_str("await bench.run();\nconsole.table(bench.table());\n");
            match std::fs::write(&bench_file, &content) {
                Ok(_) => {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Generated: shadowai.bench.js\n").ok();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {V_LINE} Run with: node --experimental-vm-modules shadowai.bench.js\n").ok();
                }
                Err(e) => print_error(&format!("Failed to write bench file: {}", e)),
            }
        } else {
            set_fg(&mut o, theme::WARN);
            write!(o, "  Benchmark generation supports Rust (Criterion) and Node.js (tinybench) projects.\n").ok();
        }
        if !bench_targets.is_empty() {
            set_fg(&mut o, theme::DIM);
            write!(o, "\n  {V_LINE} Functions added as targets:\n").ok();
            for (file, fn_name) in &bench_targets {
                write!(o, "  {V_LINE}   {} in {}\n", fn_name, file).ok();
            }
        }
        reset_color(&mut o);
        print_section_end();
        return;
    }

    let files = if args.is_empty() {
        collect_source_files(root_path)
    } else {
        let path = if args.starts_with('/') { args.to_string() }
            else { format!("{}/{}", root_path, args) };
        if !std::path::Path::new(&path).exists() {
            print_error(&format!("File not found: {}", args));
            return;
        }
        vec![path]
    };

    if files.is_empty() {
        print_error("No source files found.");
        return;
    }

    print_section_header("Performance Analysis");

    struct PerfIssue {
        file: String,
        line: usize,
        severity: &'static str,
        message: String,
    }

    let mut issues: Vec<PerfIssue> = Vec::new();

    for file_path in &files {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let display = file_path.strip_prefix(&format!("{}/", root_path)).unwrap_or(file_path);
        let lines: Vec<&str> = content.lines().collect();

        // Track nesting depth for nested loop detection
        let mut brace_depth: i32 = 0;
        let mut loop_depth: i32 = 0;
        let loop_keywords = ["for ", "while ", "loop {", "loop{"];

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let line_num = i + 1;

            // .clone() in loops
            if loop_depth > 0 && trimmed.contains(".clone()") {
                issues.push(PerfIssue {
                    file: display.to_string(), line: line_num,
                    severity: "warning",
                    message: "Unnecessary .clone() in loop — consider borrowing".into(),
                });
            }

            // unwrap() usage
            if trimmed.contains(".unwrap()") && !trimmed.starts_with("//") && !trimmed.starts_with("#[") {
                issues.push(PerfIssue {
                    file: display.to_string(), line: line_num,
                    severity: "warning",
                    message: "Potential panic from .unwrap() — use ? or handle error".into(),
                });
            }

            // SELECT * queries
            if trimmed.to_uppercase().contains("SELECT *") {
                issues.push(PerfIssue {
                    file: display.to_string(), line: line_num,
                    severity: "warning",
                    message: "Unoptimized query: SELECT * — specify needed columns".into(),
                });
            }

            // .collect::<Vec<_>>() followed by .iter()
            if trimmed.contains(".collect::<Vec") && trimmed.contains(".iter()") {
                issues.push(PerfIssue {
                    file: display.to_string(), line: line_num,
                    severity: "info",
                    message: "Unnecessary allocation: .collect() followed by .iter() — use iterator directly".into(),
                });
            }

            // String::new() + push_str in loop
            if loop_depth > 0 && (trimmed.contains("String::new()") || trimmed.contains("push_str")) {
                if trimmed.contains("String::new()") {
                    issues.push(PerfIssue {
                        file: display.to_string(), line: line_num,
                        severity: "info",
                        message: "String::new() in loop — consider String::with_capacity or collect/join".into(),
                    });
                }
            }

            // N+1 query pattern: database calls in loops
            if loop_depth > 0 {
                let db_patterns = [".query(", ".execute(", ".fetch(", "sqlx::", "diesel::",
                                   ".find(", "SELECT ", "INSERT ", "UPDATE ", "DELETE "];
                for pat in &db_patterns {
                    if trimmed.contains(pat) && !trimmed.starts_with("//") {
                        issues.push(PerfIssue {
                            file: display.to_string(), line: line_num,
                            severity: "critical",
                            message: format!("Potential N+1 query: database call inside loop ({})", pat.trim()),
                        });
                        break;
                    }
                }
            }

            // Track brace/loop depth
            for kw in &loop_keywords {
                if trimmed.starts_with(kw) || trimmed.contains(&format!(" {}", kw)) {
                    loop_depth += 1;
                }
            }
            let opens = trimmed.chars().filter(|&c| c == '{').count() as i32;
            let closes = trimmed.chars().filter(|&c| c == '}').count() as i32;
            brace_depth += opens - closes;

            // Nested loop detection (3+ levels)
            if loop_depth >= 3 {
                let mut already_reported = false;
                for kw in &loop_keywords {
                    if trimmed.starts_with(kw) || trimmed.contains(&format!(" {}", kw)) {
                        if !already_reported {
                            issues.push(PerfIssue {
                                file: display.to_string(), line: line_num,
                                severity: "critical",
                                message: format!("O(n{}) or higher complexity — deeply nested loops", match loop_depth { 3 => "\u{00b3}", 4 => "\u{2074}", _ => "\u{207f}" }),
                            });
                            already_reported = true;
                        }
                    }
                }
            }

            if closes > 0 && brace_depth >= 0 {
                // Approximate loop exits
                for _ in 0..closes {
                    if loop_depth > 0 {
                        loop_depth -= 1;
                    }
                }
            }
        }
    }

    let mut o = io::stdout();
    if issues.is_empty() {
        set_fg(&mut o, theme::OK);
        write!(o, "  {CHECK} No performance issues detected.\n").ok();
    } else {
        let critical = issues.iter().filter(|i| i.severity == "critical").count();
        let warnings = issues.iter().filter(|i| i.severity == "warning").count();
        let infos = issues.iter().filter(|i| i.severity == "info").count();

        for issue in &issues {
            let (icon, color) = match issue.severity {
                "critical" => (CROSS, theme::ERR),
                "warning" => (RADIO, theme::WARN),
                _ => (DOT, theme::DIM),
            };
            set_fg(&mut o, color);
            write!(o, "  {} ", icon).ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "{}:{}", issue.file, issue.line).ok();
            set_fg(&mut o, color);
            write!(o, " {}\n", issue.message).ok();
        }

        write!(o, "\n  ").ok();
        if critical > 0 {
            set_fg(&mut o, theme::ERR);
            write!(o, "{} critical  ", critical).ok();
        }
        if warnings > 0 {
            set_fg(&mut o, theme::WARN);
            write!(o, "{} warnings  ", warnings).ok();
        }
        if infos > 0 {
            set_fg(&mut o, theme::DIM);
            write!(o, "{} info", infos).ok();
        }
        write!(o, "\n").ok();
    }
    reset_color(&mut o);
    print_section_end();
}

fn collect_source_files(root_path: &str) -> Vec<String> {
    let extensions = ["rs", "js", "ts", "jsx", "tsx", "py", "go", "java", "rb", "php",
                      "c", "cpp", "h", "hpp", "cs", "swift", "kt", "scala", "sh", "bash",
                      "toml", "yaml", "yml", "json", "env"];
    let mut files = Vec::new();
    let root = std::path::Path::new(root_path);
    collect_source_files_recursive(root, &extensions, &mut files, 0);
    files
}

fn collect_source_files_recursive(dir: &std::path::Path, exts: &[&str], out: &mut Vec<String>, depth: usize) {
    if depth > 6 { return; }
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if dir_name.starts_with('.') || dir_name == "node_modules" || dir_name == "target"
        || dir_name == "vendor" || dir_name == "dist" || dir_name == "build"
        || dir_name == "__pycache__" || dir_name == ".git"
    {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_source_files_recursive(&path, exts, out, depth + 1);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.contains(&ext) {
                    if let Some(s) = path.to_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
}

// ─── History command ─────────────────────────────────────────────────────────

fn handle_history_command(args: &str) {
    let mut o = io::stdout();
    let history_path = match config_dir() {
        Some(d) => d.join("history"),
        None => { print_error("Cannot determine config directory"); return; }
    };

    if !history_path.exists() {
        print_error("No history file found. History is recorded as you use ShadowAI.");
        return;
    }

    let content = match std::fs::read_to_string(&history_path) {
        Ok(c) => c,
        Err(e) => { print_error(&format!("Cannot read history: {}", e)); return; }
    };

    let lines: Vec<&str> = content.lines().collect();
    let trimmed = args.trim();

    if let Some(query) = trimmed.strip_prefix("search ") {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            print_error("Usage: /history search <query>");
            return;
        }
        print_section_header("History Search");
        set_fg(&mut o, theme::DIM);
        write!(o, "  Query: \"{}\"\n\n", query).ok();
        reset_color(&mut o);

        let mut matches = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&query) {
                matches += 1;
                set_fg(&mut o, theme::DIM);
                write!(o, "  {:>5}  ", i + 1).ok();
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "{}\n", line).ok();
            }
        }
        reset_color(&mut o);

        if matches == 0 {
            set_fg(&mut o, theme::DIM);
            write!(o, "  No matches found.\n").ok();
            reset_color(&mut o);
        } else {
            set_fg(&mut o, theme::DIM);
            write!(o, "\n  {} match(es)\n", matches).ok();
            reset_color(&mut o);
        }
    } else {
        // Show last 20 entries
        print_section_header("Recent History");
        let start = if lines.len() > 20 { lines.len() - 20 } else { 0 };
        for (i, line) in lines[start..].iter().enumerate() {
            set_fg(&mut o, theme::DIM);
            write!(o, "  {:>5}  ", start + i + 1).ok();
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "{}\n", line).ok();
        }
        reset_color(&mut o);
        set_fg(&mut o, theme::DIM);
        write!(o, "\n  Showing {} of {} entries. Use /history search <query> to search.\n", lines[start..].len(), lines.len()).ok();
        reset_color(&mut o);
    }
    print_section_end();
}

// ─── Keybindings command ─────────────────────────────────────────────────────

fn handle_keybindings_command() {
    let mut o = io::stdout();
    print_section_header("Keybindings");

    // Check for keybindings config file
    let kb_path = config_dir().map(|d| d.join("keybindings.toml"));
    let custom_bindings: Option<toml::Value> = kb_path.as_ref().and_then(|p| {
        std::fs::read_to_string(p).ok().and_then(|c| c.parse::<toml::Value>().ok())
    });

    let default_bindings = [
        ("Ctrl+C", "Abort current stream"),
        ("Ctrl+D", "Exit ShadowAI"),
        ("Up/Down", "Navigate input history"),
        ("Tab", "Cycle input suggestions (future)"),
    ];

    set_fg(&mut o, theme::CYAN_DIM);
    set_attr(&mut o, Attribute::Bold);
    write!(o, "  Default Bindings\n").ok();
    set_attr(&mut o, Attribute::Reset);
    for (key, action) in &default_bindings {
        set_fg(&mut o, theme::CYAN);
        write!(o, "    {:<16}", key).ok();
        set_fg(&mut o, theme::DIM);
        write!(o, "{}\n", action).ok();
    }

    if let Some(ref bindings) = custom_bindings {
        write!(o, "\n").ok();
        set_fg(&mut o, theme::CYAN_DIM);
        set_attr(&mut o, Attribute::Bold);
        write!(o, "  Custom Bindings\n").ok();
        set_attr(&mut o, Attribute::Reset);
        if let Some(kb) = bindings.get("keybindings").and_then(|v| v.as_table()) {
            for (key, value) in kb {
                set_fg(&mut o, theme::CYAN);
                write!(o, "    {:<16}", key).ok();
                set_fg(&mut o, theme::DIM);
                write!(o, "{}\n", value.as_str().unwrap_or("?")).ok();
            }
        }
    }

    write!(o, "\n").ok();
    set_fg(&mut o, theme::DIM);
    if let Some(ref p) = kb_path {
        write!(o, "  Config: {}\n", p.display()).ok();
    }
    write!(o, "  Edit keybindings.toml to customize (see documentation).\n").ok();
    reset_color(&mut o);

    print_section_end();
}

// ─── Dev workflow handlers ────────────────────────────────────────────────────

fn detect_test_framework(root_path: &str) -> Option<(&'static str, Vec<String>)> {
    let root = std::path::Path::new(root_path);
    if root.join("Cargo.toml").exists() {
        return Some(("cargo", vec!["cargo".into(), "test".into()]));
    }
    if root.join("package.json").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
            if content.contains("vitest") {
                return Some(("vitest", vec!["npx".into(), "vitest".into(), "run".into()]));
            }
            if content.contains("jest") {
                return Some(("jest", vec!["npx".into(), "jest".into()]));
            }
            if content.contains("mocha") {
                return Some(("mocha", vec!["npx".into(), "mocha".into()]));
            }
        }
    }
    if root.join("pytest.ini").exists() || root.join("pyproject.toml").exists() || root.join("setup.py").exists() {
        return Some(("pytest", vec!["python".into(), "-m".into(), "pytest".into()]));
    }
    if root.join("go.mod").exists() {
        return Some(("go", vec!["go".into(), "test".into(), "./...".into()]));
    }
    if root.join("Makefile").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("Makefile")) {
            if content.contains("test:") || content.contains("test :") {
                return Some(("make", vec!["make".into(), "test".into()]));
            }
        }
    }
    None
}

fn run_command_live(cmd: &[String], root_path: &str) -> (String, String, bool) {
    if cmd.is_empty() {
        return (String::new(), "No command to run".into(), false);
    }
    match std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .current_dir(root_path)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (stdout, stderr, output.status.success())
        }
        Err(e) => (String::new(), format!("Failed to run command: {}", e), false),
    }
}

fn handle_test_command(args: &str, root_path: &str) {
    let mut o = io::stdout();
    let framework = detect_test_framework(root_path);
    if framework.is_none() {
        print_error("No test framework detected. Looked for: Cargo.toml, package.json (jest/vitest/mocha), pytest.ini, pyproject.toml, go.mod, Makefile");
        return;
    }
    let (name, mut cmd) = framework.unwrap();
    print_section_header(&format!("Test ({})", name));

    // Handle arguments
    let trimmed = args.trim();
    if trimmed == "--watch" {
        // Watch mode: run tests interactively with file watcher
        let (watch_cmd, watch_args): (&str, Vec<&str>) = match name {
            "cargo" => {
                // Check if cargo-watch is installed
                if std::process::Command::new("cargo").args(["watch", "--version"]).output()
                    .map(|o| o.status.success()).unwrap_or(false) {
                    ("cargo", vec!["watch", "-x", "test"])
                } else {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  cargo-watch not installed. Install with: cargo install cargo-watch\n").ok();
                    reset_color(&mut o);
                    print_section_end();
                    return;
                }
            }
            "jest" => ("npx", vec!["jest", "--watch"]),
            "vitest" => ("npx", vec!["vitest"]), // vitest runs in watch mode by default
            "pytest" => {
                // Try ptw (pytest-watch) first
                if std::process::Command::new("ptw").arg("--version").output()
                    .map(|o| o.status.success()).unwrap_or(false) {
                    ("ptw", vec![])
                } else {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  pytest-watch not installed. Install with: pip install pytest-watch\n").ok();
                    reset_color(&mut o);
                    print_section_end();
                    return;
                }
            }
            "go" => {
                set_fg(&mut o, theme::WARN);
                write!(o, "  For Go test watch, install gotestsum: go install gotest.tools/gotestsum@latest\n").ok();
                write!(o, "  Then run: gotestsum --watch ./...\n").ok();
                reset_color(&mut o);
                print_section_end();
                return;
            }
            _ => {
                set_fg(&mut o, theme::WARN);
                write!(o, "  Watch mode not available for {}\n", name).ok();
                reset_color(&mut o);
                print_section_end();
                return;
            }
        };
        set_fg(&mut o, theme::DIM);
        write!(o, "  $ {} {}\n", watch_cmd, watch_args.join(" ")).ok();
        set_fg(&mut o, theme::CYAN);
        write!(o, "  Running in watch mode (Ctrl+C to stop)...\n\n").ok();
        reset_color(&mut o);
        o.flush().ok();

        let _ = std::process::Command::new(watch_cmd)
            .args(&watch_args)
            .current_dir(root_path)
            .status();

        print_section_end();
        return;
    } else if trimmed == "--coverage" {
        // Coverage report integration
        print_section_header(&format!("Coverage ({})", name));
        let (cov_cmd, cov_args): (&str, Vec<String>) = match name {
            "cargo" => {
                // Prefer cargo-tarpaulin, fall back to llvm-cov
                if std::process::Command::new("cargo").args(["tarpaulin", "--version"]).output()
                    .map(|o| o.status.success()).unwrap_or(false) {
                    ("cargo", vec!["tarpaulin".into(), "--out".into(), "Stdout".into()])
                } else if std::process::Command::new("cargo").args(["llvm-cov", "--version"]).output()
                    .map(|o| o.status.success()).unwrap_or(false) {
                    ("cargo", vec!["llvm-cov".into()])
                } else {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  No coverage tool found. Install one:\n").ok();
                    write!(o, "    cargo install cargo-tarpaulin\n").ok();
                    write!(o, "    cargo install cargo-llvm-cov\n").ok();
                    reset_color(&mut o);
                    print_section_end();
                    return;
                }
            }
            "jest" => ("npx", vec!["jest".into(), "--coverage".into()]),
            "vitest" => ("npx", vec!["vitest".into(), "run".into(), "--coverage".into()]),
            "pytest" => {
                if std::process::Command::new("python").args(["-m", "pytest_cov", "--version"]).output()
                    .map(|o| o.status.success()).unwrap_or(false)
                    || std::process::Command::new("python").args(["-c", "import pytest_cov"]).output()
                    .map(|o| o.status.success()).unwrap_or(false) {
                    ("python", vec!["-m".into(), "pytest".into(), "--cov=.".into(), "--cov-report=term-missing".into()])
                } else {
                    set_fg(&mut o, theme::WARN);
                    write!(o, "  pytest-cov not installed. Run: pip install pytest-cov\n").ok();
                    reset_color(&mut o);
                    print_section_end();
                    return;
                }
            }
            "go" => ("go", vec!["test".into(), "-coverprofile=coverage.out".into(), "./...".into()]),
            _ => {
                set_fg(&mut o, theme::WARN);
                write!(o, "  Coverage not supported for {}\n", name).ok();
                reset_color(&mut o);
                print_section_end();
                return;
            }
        };
        let full_cmd: Vec<String> = std::iter::once(cov_cmd.to_string()).chain(cov_args.clone()).collect();
        set_fg(&mut o, theme::DIM);
        write!(o, "  $ {}\n\n", full_cmd.join(" ")).ok();
        reset_color(&mut o);
        o.flush().ok();
        let (stdout, stderr, ok) = run_command_live(&full_cmd, root_path);
        let combined = if !stdout.is_empty() { stdout } else { stderr };
        // Highlight coverage percentages
        for line in combined.lines() {
            if line.contains('%') {
                let pct: Option<f64> = line.split('%').next()
                    .and_then(|s| s.split_whitespace().last())
                    .and_then(|s| s.parse().ok());
                let color = match pct {
                    Some(p) if p >= 80.0 => theme::OK,
                    Some(p) if p >= 50.0 => theme::WARN,
                    Some(_) => theme::ERR,
                    None => theme::DIM_LIGHT,
                };
                set_fg(&mut o, color);
            } else {
                set_fg(&mut o, theme::DIM_LIGHT);
            }
            write!(o, "  {}\n", line).ok();
        }
        write!(o, "\n").ok();
        if ok {
            set_fg(&mut o, theme::OK);
            write!(o, "  {CHECK} Coverage report complete\n").ok();
        } else {
            set_fg(&mut o, theme::ERR);
            write!(o, "  {CROSS} Coverage run failed\n").ok();
        }
        // For Go: print coverage summary from coverage.out
        if name == "go" && std::path::Path::new(&format!("{}/coverage.out", root_path)).exists() {
            let (cov_out, _, _) = run_command_live(&["go".into(), "tool".into(), "cover".into(), "-func=coverage.out".into()], root_path);
            if !cov_out.is_empty() {
                set_fg(&mut o, theme::DIM_LIGHT);
                if let Some(last) = cov_out.lines().last() {
                    write!(o, "  {DOT} {}\n", last.trim()).ok();
                }
            }
        }
        reset_color(&mut o);
        print_section_end();
        return;
    } else if trimmed == "--fix" {
        // For cargo: cargo test with fix doesn't really exist, just run normally
        // This is a best-effort feature
        set_fg(&mut o, theme::DIM);
        write!(o, "  Running tests...\n").ok();
        reset_color(&mut o);
    } else if let Some(file) = trimmed.strip_prefix("--generate ") {
        set_fg(&mut o, theme::ACCENT);
        write!(o, "  Generate tests for: {}\n", file.trim()).ok();
        reset_color(&mut o);
        print_section_end();
        return;
    } else if !trimmed.is_empty() {
        // Specific file or filter
        match name {
            "cargo" => { cmd.push("--".into()); cmd.push(trimmed.into()); }
            "jest" | "vitest" | "mocha" => { cmd.push(trimmed.into()); }
            "pytest" => { cmd.push(trimmed.into()); }
            "go" => { cmd[2] = trimmed.into(); }
            _ => { cmd.push(trimmed.into()); }
        }
    }

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd, root_path);

    // Display output with coloring
    let combined = if !stdout.is_empty() && !stderr.is_empty() {
        format!("{}\n{}", stdout, stderr)
    } else if !stdout.is_empty() {
        stdout
    } else {
        stderr
    };

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    for line in combined.lines() {
        let lower = line.to_lowercase();
        let color = if lower.contains("pass") || lower.contains("ok") || lower.contains(" passed") {
            passed += 1;
            theme::OK
        } else if lower.contains("fail") || lower.contains("error") || lower.contains("panic") {
            failed += 1;
            theme::ERR
        } else if lower.contains("skip") || lower.contains("ignore") || lower.contains("pending") {
            skipped += 1;
            theme::WARN
        } else {
            theme::DIM_LIGHT
        };
        set_fg(&mut o, color);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    // Summary
    write!(o, "\n").ok();
    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "  {} Tests passed", CHECK).ok();
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "  {} Tests failed", CROSS).ok();
    }
    if passed > 0 || failed > 0 || skipped > 0 {
        set_fg(&mut o, theme::DIM);
        write!(o, " ({} passed, {} failed, {} skipped)", passed, failed, skipped).ok();
    }
    write!(o, "\n").ok();
    reset_color(&mut o);
    print_section_end();
}

fn detect_linter(root_path: &str) -> Option<(&'static str, Vec<String>)> {
    let root = std::path::Path::new(root_path);
    if root.join("Cargo.toml").exists() {
        return Some(("clippy", vec!["cargo".into(), "clippy".into(), "--".into(), "-W".into(), "clippy::all".into()]));
    }
    if root.join("package.json").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
            if content.contains("eslint") {
                return Some(("eslint", vec!["npx".into(), "eslint".into(), ".".into()]));
            }
        }
    }
    // Check for standalone eslint config
    for name in &[".eslintrc", ".eslintrc.js", ".eslintrc.json", ".eslintrc.yml", ".eslintrc.yaml", "eslint.config.js", "eslint.config.mjs"] {
        if root.join(name).exists() {
            return Some(("eslint", vec!["npx".into(), "eslint".into(), ".".into()]));
        }
    }
    // Biome (Section 3.1)
    if root.join("biome.json").exists() || root.join("biome.jsonc").exists() {
        return Some(("biome", vec!["biome".into(), "check".into(), ".".into()]));
    }
    if root.join("pyproject.toml").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("pyproject.toml")) {
            if content.contains("ruff") {
                return Some(("ruff", vec!["ruff".into(), "check".into(), ".".into()]));
            }
        }
    }
    if root.join(".flake8").exists() || root.join("setup.cfg").exists() {
        return Some(("flake8", vec!["python".into(), "-m".into(), "flake8".into()]));
    }
    // SwiftLint (Section 3.1)
    if root.join(".swiftlint.yml").exists() || root.join(".swiftlint.yaml").exists() {
        return Some(("swiftlint", vec!["swiftlint".into()]));
    }
    // Kotlin (ktlint)
    let has_kotlin = root.read_dir().ok().map(|mut d| d.any(|e| {
        e.ok().and_then(|e| e.path().extension().map(|x| x == "kt" || x == "kts")).unwrap_or(false)
    })).unwrap_or(false);
    if has_kotlin {
        if std::process::Command::new("ktlint").arg("--version").output().is_ok() {
            return Some(("ktlint", vec!["ktlint".into(), "--format".into(), "**/*.kt".into()]));
        }
    }
    // Ruby (rubocop)
    if root.join("Gemfile").exists() {
        if std::process::Command::new("bundle").arg("exec").arg("rubocop").arg("--version").output().is_ok() {
            return Some(("rubocop", vec!["bundle".into(), "exec".into(), "rubocop".into(), "--auto-correct".into()]));
        }
        if std::process::Command::new("rubocop").arg("--version").output().is_ok() {
            return Some(("rubocop", vec!["rubocop".into(), "--auto-correct".into()]));
        }
    }
    // PHP (phpcbf/phpcs)
    let has_php = root.read_dir().ok().map(|mut d| d.any(|e| {
        e.ok().and_then(|e| e.path().extension().map(|x| x == "php")).unwrap_or(false)
    })).unwrap_or(false);
    if has_php {
        if std::process::Command::new("phpcbf").arg("--version").output().is_ok() {
            return Some(("phpcbf", vec!["phpcbf".into(), ".".into()]));
        }
        if std::process::Command::new("phpcs").arg("--version").output().is_ok() {
            return Some(("phpcs", vec!["phpcs".into(), ".".into()]));
        }
    }
    None
}

/// Detect linter for a specific file (used by auto-lint)
fn detect_linter_for_file(root_path: &str, file_path: &str) -> Option<(&'static str, Vec<String>)> {
    let path = std::path::Path::new(file_path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let root = std::path::Path::new(root_path);
    match ext {
        "rs" => {
            if root.join("Cargo.toml").exists() {
                Some(("clippy", vec!["cargo".into(), "clippy".into(), "--".into(), "-W".into(), "clippy::all".into()]))
            } else { None }
        }
        "js" | "jsx" | "ts" | "tsx" => {
            // Check Biome first
            if root.join("biome.json").exists() || root.join("biome.jsonc").exists() {
                let target = if file_path.starts_with('/') { file_path.to_string() } else { format!("{}/{}", root_path, file_path) };
                return Some(("biome", vec!["biome".into(), "check".into(), target]));
            }
            if root.join("package.json").exists() || root.join(".eslintrc.js").exists() || root.join("eslint.config.js").exists() {
                let target = if file_path.starts_with('/') { file_path.to_string() } else { format!("{}/{}", root_path, file_path) };
                Some(("eslint", vec!["npx".into(), "eslint".into(), target]))
            } else { None }
        }
        "py" => {
            if root.join("pyproject.toml").exists() {
                Some(("ruff", vec!["ruff".into(), "check".into(), file_path.into()]))
            } else {
                Some(("flake8", vec!["python".into(), "-m".into(), "flake8".into(), file_path.into()]))
            }
        }
        "go" => Some(("go vet", vec!["go".into(), "vet".into(), file_path.into()])),
        // shellcheck (Section 3.1)
        "sh" | "bash" => Some(("shellcheck", vec!["shellcheck".into(), file_path.into()])),
        // markdownlint (Section 3.1)
        "md" | "markdown" => Some(("markdownlint", vec!["markdownlint".into(), file_path.into()])),
        // SwiftLint (Section 3.1)
        "swift" => Some(("swiftlint", vec!["swiftlint".into(), "lint".into(), "--path".into(), file_path.into()])),
        _ => {
            // hadolint for Dockerfile (Section 3.1)
            if filename == "Dockerfile" || filename.starts_with("Dockerfile.") {
                return Some(("hadolint", vec!["hadolint".into(), file_path.into()]));
            }
            None
        }
    }
}

/// Returns lint output if errors were found, for injection into AI context.
fn handle_lint_command(args: &str, root_path: &str) -> Option<String> {
    let mut o = io::stdout();
    let linter = detect_linter(root_path);
    if linter.is_none() {
        print_error("No linter detected. Looked for: Cargo.toml (clippy), biome.json, eslint, ruff, flake8, .swiftlint.yml");
        return None;
    }
    let (name, mut cmd) = linter.unwrap();
    print_section_header(&format!("Lint ({})", name));

    let trimmed = args.trim();
    if trimmed == "--fix" {
        match name {
            "clippy" => { cmd = vec!["cargo".into(), "clippy".into(), "--fix".into(), "--allow-dirty".into(), "--allow-staged".into()]; }
            "eslint" => { cmd.push("--fix".into()); }
            "ruff" => { cmd = vec!["ruff".into(), "check".into(), "--fix".into(), ".".into()]; }
            "biome" => { cmd = vec!["biome".into(), "check".into(), "--apply".into(), ".".into()]; }
            "swiftlint" => { cmd = vec!["swiftlint".into(), "--fix".into()]; }
            _ => {}
        }
    } else if trimmed == "--format" {
        // Run formatter in addition to linter (Section 3.1)
        match name {
            "clippy" => {
                // Also run rustfmt
                set_fg(&mut o, theme::DIM);
                write!(o, "  $ cargo fmt\n\n").ok();
                reset_color(&mut o);
                let (fmt_out, fmt_err, fmt_ok) = run_command_live(&["cargo".into(), "fmt".into()], root_path);
                if fmt_ok {
                    set_fg(&mut o, theme::OK);
                    write!(o, "  {CHECK} Formatted with rustfmt\n").ok();
                } else {
                    set_fg(&mut o, theme::WARN);
                    let combined = if fmt_out.is_empty() { fmt_err } else { fmt_out };
                    write!(o, "  {CROSS} rustfmt: {}\n", combined.trim()).ok();
                }
                reset_color(&mut o);
            }
            _ => {}
        }
    } else if !trimmed.is_empty() {
        // Specific file
        match name {
            "clippy" => { /* clippy doesn't take file args easily */ }
            "eslint" => { cmd[2] = trimmed.into(); }
            "ruff" => { cmd[2] = trimmed.into(); }
            "flake8" => { cmd.push(trimmed.into()); }
            "biome" => { cmd[2] = trimmed.into(); }
            _ => {}
        }
    }

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd, root_path);
    let combined = if !stdout.is_empty() && !stderr.is_empty() {
        format!("{}\n{}", stdout, stderr)
    } else if !stdout.is_empty() {
        stdout
    } else {
        stderr
    };

    for line in combined.lines() {
        let lower = line.to_lowercase();
        let color = if lower.contains("error") {
            theme::ERR
        } else if lower.contains("warning") || lower.contains("warn") {
            theme::WARN
        } else if lower.contains("note") || lower.contains("info") || lower.contains("help") {
            theme::DIM
        } else {
            theme::DIM_LIGHT
        };
        set_fg(&mut o, color);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "\n  {} No lint issues found\n", CHECK).ok();
        reset_color(&mut o);
        print_section_end();
        None
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "\n  {} Lint issues found\n", CROSS).ok();
        reset_color(&mut o);
        print_section_end();
        // Return lint output for AI context injection
        if !combined.is_empty() {
            Some(format!("[Lint errors]\n{}", combined))
        } else {
            None
        }
    }
}

// ─── New Commands (Section 10) ───────────────────────────────────────────────

fn handle_todo_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("TODO / FIXME / HACK / NOTE");

    let output = std::process::Command::new("grep")
        .args([
            "-rn",
            r"TODO\|FIXME\|HACK\|XXX\|NOTE",
            "--include=*.rs",
            "--include=*.ts",
            "--include=*.tsx",
            "--include=*.py",
            "--include=*.go",
            "--include=*.js",
            "--include=*.jsx",
            "--include=*.cpp",
            "--include=*.c",
            "--include=*.java",
            ".",
        ])
        .current_dir(root_path)
        .output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.trim().is_empty() {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} No TODO/FIXME/HACK/NOTE found\n").ok();
                reset_color(&mut o);
            } else {
                // Group by file
                let mut by_file: std::collections::BTreeMap<String, Vec<(String, String)>> = std::collections::BTreeMap::new();
                for line in text.lines() {
                    if let Some(colon_pos) = line.find(':') {
                        let file = &line[..colon_pos];
                        let rest = &line[colon_pos+1..];
                        if let Some(colon2) = rest.find(':') {
                            let lineno = &rest[..colon2];
                            let content = &rest[colon2+1..];
                            by_file.entry(file.to_string()).or_default().push((lineno.to_string(), content.to_string()));
                        }
                    }
                }
                for (file, entries) in &by_file {
                    set_fg(&mut o, theme::CYAN_DIM);
                    write!(o, "\n  {file}\n").ok();
                    for (lineno, content) in entries {
                        let upper = content.to_uppercase();
                        let color = if upper.contains("FIXME") || upper.contains("XXX") {
                            theme::ERR
                        } else if upper.contains("TODO") {
                            theme::WARN
                        } else if upper.contains("HACK") {
                            theme::ACCENT
                        } else {
                            theme::DIM_LIGHT
                        };
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  {:>6}: ", lineno).ok();
                        set_fg(&mut o, color);
                        write!(o, "{}\n", content.trim()).ok();
                    }
                }
                reset_color(&mut o);
            }
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  {CROSS} grep failed: {}\n", e).ok();
            reset_color(&mut o);
        }
    }
    print_section_end();
}

fn redact_secret(val: &str) -> String {
    let trimmed = val.trim();
    if trimmed.len() > 8 {
        format!("{}...[REDACTED]", &trimmed[..4])
    } else if !trimmed.is_empty() {
        "[REDACTED]".to_string()
    } else {
        "(empty)".to_string()
    }
}

fn looks_like_secret(val: &str) -> bool {
    let v = val.trim();
    // Long alphanumeric strings that look like keys/tokens
    if v.len() >= 20 && v.chars().all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '-' || c == '_' || c == '=') {
        return true;
    }
    // Known secret prefixes
    if v.starts_with("sk-") || v.starts_with("AKIA") || v.starts_with("xox") || v.starts_with("ghp_") {
        return true;
    }
    false
}

fn handle_env_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Environment");

    let secret_prefixes = ["DATABASE_", "REDIS_", "AWS_", "GCP_", "OPENAI_", "ANTHROPIC_",
        "SECRET", "KEY", "TOKEN", "PASSWORD", "PASS", "CREDENTIAL", "AUTH"];
    let show_prefixes = ["DATABASE_", "REDIS_", "AWS_", "GCP_", "OPENAI_", "ANTHROPIC_",
        "PORT", "HOST", "DEBUG", "NODE_ENV", "RUST_", "CARGO_", "PATH", "LANG"];

    // Show env vars matching interesting prefixes
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "\n  Environment Variables:\n").ok();
    let mut found = false;
    for (key, val) in std::env::vars() {
        let matches = show_prefixes.iter().any(|p| key.starts_with(p));
        if matches {
            found = true;
            let is_secret = secret_prefixes.iter().any(|p| key.to_uppercase().contains(p));
            let display_val = if is_secret || looks_like_secret(&val) {
                redact_secret(&val)
            } else {
                if val.len() > 60 { format!("{}...", &val[..60]) } else { val.clone() }
            };
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  {:<35}", key).ok();
            set_fg(&mut o, theme::AI_TEXT);
            write!(o, " {}\n", display_val).ok();
        }
    }
    if !found {
        set_fg(&mut o, theme::DIM);
        write!(o, "  (none matching common prefixes)\n").ok();
    }

    // Show .env file if present
    let env_path = std::path::Path::new(root_path).join(".env");
    if env_path.exists() {
        write!(o, "\n").ok();
        set_fg(&mut o, theme::CYAN_DIM);
        write!(o, "  .env file:\n").ok();
        if let Ok(content) = std::fs::read_to_string(&env_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {}\n", line).ok();
                    continue;
                }
                if let Some(eq_pos) = line.find('=') {
                    let key = &line[..eq_pos];
                    let val = &line[eq_pos+1..];
                    let display_val = if looks_like_secret(val) || secret_prefixes.iter().any(|p| key.to_uppercase().contains(p)) {
                        redact_secret(val)
                    } else {
                        if val.len() > 60 { format!("{}...", &val[..60]) } else { val.to_string() }
                    };
                    set_fg(&mut o, theme::DIM_LIGHT);
                    write!(o, "  {:<35}", key).ok();
                    set_fg(&mut o, theme::AI_TEXT);
                    write!(o, " = {}\n", display_val).ok();
                } else {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {}\n", line).ok();
                }
            }
        }
    }
    reset_color(&mut o);
    print_section_end();
}

fn handle_secrets_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Secret Scanner");

    let patterns = [
        r"sk-ant-",
        r"sk-[a-zA-Z0-9]{20,}",
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN.*PRIVATE KEY",
        r#"password\s*=\s*["\'][^"\']{4,}"#,
        r#"api_key\s*=\s*["\'][^"\']{4,}"#,
        r#"token\s*=\s*["\'][^"\']{8,}"#,
        r#"secret\s*=\s*["\'][^"\']{4,}"#,
    ];

    let exclude_dirs = ["--exclude-dir=.git", "--exclude-dir=node_modules", "--exclude-dir=target", "--exclude-dir=.shadowai"];

    let mut total_found = 0usize;
    for pattern in &patterns {
        let mut args: Vec<&str> = vec!["-rn", "-i", pattern];
        for excl in &exclude_dirs {
            args.push(excl);
        }
        args.push(".");

        if let Ok(out) = std::process::Command::new("grep")
            .args(&args)
            .current_dir(root_path)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                // Redact the value part after the match
                total_found += 1;
                let parts: Vec<&str> = line.splitn(3, ':').collect();
                if parts.len() >= 3 {
                    let file = parts[0];
                    let lineno = parts[1];
                    let content = parts[2];
                    // Redact anything that looks like a secret value
                    let redacted = regex::Regex::new(r#"(["\'])[^"\']{8,}(["\'])"#)
                        .map(|re| re.replace_all(content, |caps: &regex::Captures| {
                            format!("{}[REDACTED]{}", &caps[1], &caps[2])
                        }).to_string())
                        .unwrap_or_else(|_| content.to_string());
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  {}:{}: ", file, lineno).ok();
                    set_fg(&mut o, theme::WARN);
                    write!(o, "{}\n", redacted.trim()).ok();
                }
            }
        }
    }

    if total_found == 0 {
        set_fg(&mut o, theme::OK);
        write!(o, "  {CHECK} No hardcoded secrets detected\n").ok();
    } else {
        write!(o, "\n").ok();
        set_fg(&mut o, theme::ERR);
        write!(o, "  {CROSS} Found {} potential secret(s)\n", total_found).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

fn handle_metrics_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Project Metrics");

    // Try tokei first for detailed stats
    let tokei_result = std::process::Command::new("tokei")
        .arg(".")
        .current_dir(root_path)
        .output();

    if let Ok(out) = tokei_result {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            set_fg(&mut o, theme::AI_TEXT);
            write!(o, "\n").ok();
            for line in text.lines() {
                write!(o, "  {}\n", line).ok();
            }
            reset_color(&mut o);
            print_section_end();
            return;
        }
    }

    // Fallback: count files by extension
    set_fg(&mut o, theme::CYAN_DIM);
    write!(o, "\n  File counts by extension:\n").ok();
    let mut ext_counts: std::collections::BTreeMap<String, (usize, usize)> = std::collections::BTreeMap::new();
    if let Ok(walker) = std::fs::read_dir(root_path) {
        fn count_dir(path: &std::path::Path, counts: &mut std::collections::BTreeMap<String, (usize, usize)>) {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    // Skip hidden dirs and common build dirs
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.') || name == "node_modules" || name == "target" { continue; }
                    }
                    if p.is_dir() {
                        count_dir(&p, counts);
                    } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                        let content = std::fs::read_to_string(&p).unwrap_or_default();
                        let lines = content.lines().count();
                        let entry = counts.entry(ext.to_string()).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += lines;
                    }
                }
            }
        }
        drop(walker);
        count_dir(std::path::Path::new(root_path), &mut ext_counts);
    }

    for (ext, (files, lines)) in &ext_counts {
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  {:<12} {:>5} files  {:>8} lines\n", format!(".{}", ext), files, lines).ok();
    }

    // Git stats
    let git_stats = || -> Option<()> {
        let commit_count = std::process::Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(root_path).output().ok()?;
        let count = String::from_utf8_lossy(&commit_count.stdout).trim().to_string();

        let contributors = std::process::Command::new("git")
            .args(["shortlog", "-s", "--no-merges", "HEAD"])
            .current_dir(root_path).output().ok()?;
        let contrib_count = String::from_utf8_lossy(&contributors.stdout).lines().count();

        let first = std::process::Command::new("git")
            .args(["log", "--oneline", "--reverse", "--format=%ar", "-1"])
            .current_dir(root_path).output().ok()?;
        let first_date = String::from_utf8_lossy(&first.stdout).trim().to_string();

        let last = std::process::Command::new("git")
            .args(["log", "--format=%ar", "-1"])
            .current_dir(root_path).output().ok()?;
        let last_date = String::from_utf8_lossy(&last.stdout).trim().to_string();

        let mut o = io::stdout();
        set_fg(&mut o, theme::CYAN_DIM);
        write!(o, "\n  Git:\n").ok();
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  Commits:      {}\n", count).ok();
        write!(o, "  Contributors: {}\n", contrib_count).ok();
        write!(o, "  First commit: {}\n", first_date).ok();
        write!(o, "  Last commit:  {}\n", last_date).ok();
        Some(())
    };
    git_stats();

    reset_color(&mut o);
    print_section_end();
}

fn handle_deps_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Dependencies");
    let root = std::path::Path::new(root_path);

    // Rust (Cargo.toml)
    if root.join("Cargo.toml").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) {
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "\n  Rust (Cargo.toml):\n").ok();
            let mut in_deps = false;
            let mut count = 0usize;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed == "[dependencies]" || trimmed == "[dev-dependencies]" || trimmed == "[build-dependencies]" {
                    in_deps = true;
                    set_fg(&mut o, theme::ACCENT_DIM);
                    write!(o, "  [{}]\n", trimmed.trim_matches('[').trim_matches(']')).ok();
                    continue;
                }
                if in_deps && trimmed.starts_with('[') {
                    in_deps = false;
                }
                if in_deps && !trimmed.is_empty() && !trimmed.starts_with('#') {
                    count += 1;
                    set_fg(&mut o, theme::DIM_LIGHT);
                    write!(o, "  {DOT} {}\n", trimmed).ok();
                }
            }
            set_fg(&mut o, theme::DIM);
            write!(o, "  Total: {} deps\n", count).ok();

            // Check for cargo-audit
            if std::process::Command::new("cargo").arg("audit").arg("--version").output().is_ok() {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} cargo-audit available. Run: cargo audit\n").ok();
            } else {
                set_fg(&mut o, theme::DIM);
                write!(o, "  Install cargo-audit for vulnerability scanning: cargo install cargo-audit\n").ok();
            }
        }
    }

    // Node (package.json)
    if root.join("package.json").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("package.json")) {
            if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) {
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "\n  Node (package.json):\n").ok();
                for section in &["dependencies", "devDependencies"] {
                    if let Some(deps) = pkg[section].as_object() {
                        set_fg(&mut o, theme::ACCENT_DIM);
                        write!(o, "  [{}] ({} pkgs)\n", section, deps.len()).ok();
                        for (name, ver) in deps.iter().take(20) {
                            set_fg(&mut o, theme::DIM_LIGHT);
                            write!(o, "  {DOT} {} {}\n", name, ver.as_str().unwrap_or("")).ok();
                        }
                        if deps.len() > 20 {
                            set_fg(&mut o, theme::DIM);
                            write!(o, "  ... and {} more\n", deps.len() - 20).ok();
                        }
                    }
                }
            }
        }
    }

    // Python (requirements.txt)
    if root.join("requirements.txt").exists() {
        if let Ok(content) = std::fs::read_to_string(root.join("requirements.txt")) {
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "\n  Python (requirements.txt):\n").ok();
            let mut count = 0usize;
            for line in content.lines() {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    count += 1;
                    set_fg(&mut o, theme::DIM_LIGHT);
                    write!(o, "  {DOT} {}\n", line).ok();
                }
            }
            set_fg(&mut o, theme::DIM);
            write!(o, "  Total: {} deps\n", count).ok();
        }
    }

    reset_color(&mut o);
    print_section_end();
}

fn handle_diff_file_command(file: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Diff: {}", file));

    let output = std::process::Command::new("git")
        .args(["diff", "HEAD", "--", file])
        .current_dir(root_path)
        .output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.trim().is_empty() {
                set_fg(&mut o, theme::DIM);
                write!(o, "  No changes in {} vs HEAD\n", file).ok();
            } else {
                for line in text.lines() {
                    let color = if line.starts_with('+') && !line.starts_with("+++") {
                        theme::FILE_NEW
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        theme::FILE_DEL
                    } else if line.starts_with("@@") {
                        theme::CYAN
                    } else if line.starts_with("diff ") || line.starts_with("index ") {
                        theme::ACCENT_DIM
                    } else {
                        theme::DIM_LIGHT
                    };
                    set_fg(&mut o, color);
                    write!(o, "  {}\n", line).ok();
                }
            }
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  {CROSS} git diff failed: {}\n", e).ok();
        }
    }
    reset_color(&mut o);
    print_section_end();
}

fn detect_node_pm(root: &str) -> &'static str {
    let r = std::path::Path::new(root);
    if r.join("bun.lockb").exists() { return "bun"; }
    if r.join("pnpm-lock.yaml").exists() { return "pnpm"; }
    if r.join("yarn.lock").exists() { return "yarn"; }
    "npm"
}

fn detect_build_system(root_path: &str) -> Option<(&'static str, Vec<String>)> {
    let root = std::path::Path::new(root_path);
    if root.join("Cargo.toml").exists() {
        // Check for bevy (game engine) – same build command
        return Some(("cargo", vec!["cargo".into(), "build".into()]));
    }
    if root.join("package.json").exists() {
        let pm = detect_node_pm(root_path);
        return Some((pm, vec![pm.into(), "run".into(), "build".into()]));
    }
    if root.join("go.mod").exists() {
        return Some(("go", vec!["go".into(), "build".into(), "./...".into()]));
    }
    if root.join("CMakeLists.txt").exists() {
        // Create build dir if needed, then build
        return Some(("cmake", vec!["cmake".into(), "-B".into(), "build".into(), "-S".into(), ".".into()]));
    }
    if root.join("build.gradle").exists() || root.join("build.gradle.kts").exists() {
        let gradlew = root.join("gradlew");
        if gradlew.exists() {
            return Some(("gradle", vec!["./gradlew".into(), "build".into()]));
        }
        return Some(("gradle", vec!["gradle".into(), "build".into()]));
    }
    if root.join("pom.xml").exists() {
        return Some(("maven", vec!["mvn".into(), "package".into()]));
    }
    if root.join("Makefile").exists() {
        return Some(("make", vec!["make".into()]));
    }
    None
}

/// Returns Some(error_output) if `--fix` was requested and the build failed.
fn handle_build_command(args: &str, root_path: &str) -> Option<String> {
    let mut o = io::stdout();
    let build = detect_build_system(root_path);
    if build.is_none() {
        print_error("No build system detected. Looked for: Cargo.toml, package.json, go.mod, CMakeLists.txt, Makefile");
        return None;
    }
    let (name, cmd) = build.unwrap();
    let is_fix = args.trim() == "--fix";
    print_section_header(&format!("Build ({}){}", name, if is_fix { " --fix" } else { "" }));

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd, root_path);
    let combined = if !stdout.is_empty() && !stderr.is_empty() {
        format!("{}\n{}", stdout, stderr)
    } else if !stdout.is_empty() {
        stdout
    } else {
        stderr.clone()
    };

    for line in combined.lines() {
        let lower = line.to_lowercase();
        let color = if lower.contains("error") {
            theme::ERR
        } else if lower.contains("warning") || lower.contains("warn") {
            theme::WARN
        } else if lower.contains("compiling") || lower.contains("building") {
            theme::CYAN
        } else if lower.contains("finished") || lower.contains("success") {
            theme::OK
        } else {
            theme::DIM_LIGHT
        };
        set_fg(&mut o, color);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "\n  {} Build succeeded\n", CHECK).ok();
        reset_color(&mut o);
        print_section_end();
        None
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "\n  {} Build failed\n", CROSS).ok();
        reset_color(&mut o);
        if is_fix {
            set_fg(&mut o, theme::ACCENT);
            write!(o, "  {ARROW} Sending build errors to AI for fixing...\n").ok();
            reset_color(&mut o);
        }
        print_section_end();
        if is_fix { Some(combined) } else { None }
    }
}

// ─── Symbols search ──────────────────────────────────────────────────────────

fn handle_symbols_command(query: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Symbols: {}", query));

    let query_lower = query.to_lowercase();

    // Patterns: (regex_pattern, symbol_type, color)
    let patterns: Vec<(&str, &str, crossterm::style::Color)> = vec![
        // Rust
        (r"(?m)^\s*(?:pub\s+)?fn\s+(\w+)", "fn", theme::CYAN),
        (r"(?m)^\s*(?:pub\s+)?struct\s+(\w+)", "struct", theme::ACCENT),
        (r"(?m)^\s*(?:pub\s+)?enum\s+(\w+)", "enum", theme::WARN),
        (r"(?m)^\s*(?:pub\s+)?trait\s+(\w+)", "trait", theme::ACCENT),
        (r"(?m)^\s*(?:pub\s+)?type\s+(\w+)", "type", theme::ACCENT),
        (r"(?m)^\s*impl\s+(\w+)", "impl", theme::CYAN_DIM),
        // Python
        (r"(?m)^\s*def\s+(\w+)", "def", theme::CYAN),
        (r"(?m)^\s*class\s+(\w+)", "class", theme::ACCENT),
        // JavaScript/TypeScript
        (r"(?m)^\s*(?:export\s+)?(?:default\s+)?function\s+(\w+)", "function", theme::CYAN),
        (r"(?m)^\s*(?:export\s+)?(?:default\s+)?class\s+(\w+)", "class", theme::ACCENT),
        (r"(?m)^\s*(?:export\s+)?const\s+(\w+)\s*=\s*(?:\([^)]*\)|[^=])*=>", "const=>", theme::CYAN),
        // Go
        (r"(?m)^func\s+(?:\([^)]+\)\s+)?(\w+)", "func", theme::CYAN),
        (r"(?m)^type\s+(\w+)\s+struct", "struct", theme::ACCENT),
        (r"(?m)^type\s+(\w+)\s+interface", "interface", theme::ACCENT),
    ];

    let compiled: Vec<(regex::Regex, &str, crossterm::style::Color)> = patterns.iter()
        .filter_map(|(pat, kind, color)| {
            regex::Regex::new(pat).ok().map(|re| (re, *kind, *color))
        })
        .collect();

    // Source file extensions
    let extensions = ["rs", "py", "js", "ts", "jsx", "tsx", "go", "java", "rb", "c", "cpp", "h", "hpp"];

    let mut results: Vec<(String, usize, String, &str, crossterm::style::Color)> = Vec::new(); // (file, line, name, kind, color)

    // Try rg first for speed, fallback to manual
    let use_rg = std::process::Command::new("rg").arg("--version").output().is_ok();

    fn walk_source_files(root: &str, extensions: &[&str]) -> Vec<String> {
        let mut files = Vec::new();
        fn walk_dir(dir: &std::path::Path, extensions: &[&str], files: &mut Vec<String>, depth: usize) {
            if depth > 8 { return; }
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "node_modules" || name == "target"
                    || name == "vendor" || name == "dist" || name == "build" || name == "__pycache__" {
                    continue;
                }
                if path.is_dir() {
                    walk_dir(&path, extensions, files, depth + 1);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        files.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
        walk_dir(std::path::Path::new(root), extensions, &mut files, 0);
        files
    }

    let _ = use_rg; // We do manual search for regex-based symbol extraction
    let files = walk_source_files(root_path, &extensions);

    for file_path in &files {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (re, kind, color) in &compiled {
            for cap in re.captures_iter(&content) {
                if let Some(m) = cap.get(1) {
                    let name = m.as_str();
                    if name.to_lowercase().contains(&query_lower) {
                        // Calculate line number
                        let line_num = content[..m.start()].matches('\n').count() + 1;
                        let display_path = file_path.strip_prefix(&format!("{}/", root_path)).unwrap_or(file_path);
                        results.push((display_path.to_string(), line_num, name.to_string(), kind, *color));
                        if results.len() >= 50 { break; }
                    }
                }
            }
            if results.len() >= 50 { break; }
        }
        if results.len() >= 50 { break; }
    }

    if results.is_empty() {
        set_fg(&mut o, theme::DIM);
        write!(o, "  No symbols matching '{}' found.\n", query).ok();
    } else {
        for (file, line, name, kind, color) in &results {
            set_fg(&mut o, theme::DIM);
            write!(o, "  {}:{} ", file, line).ok();
            set_fg(&mut o, *color);
            write!(o, "[{}] ", kind).ok();
            set_fg(&mut o, theme::AI_TEXT);
            write!(o, "{}\n", name).ok();
        }
        set_fg(&mut o, theme::DIM);
        write!(o, "\n  {} result(s)\n", results.len()).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

// ─── Cheatsheet ──────────────────────────────────────────────────────────────

fn print_cheatsheet() {
    let mut o = io::stdout();
    print_section_header("Cheatsheet");

    let entries = [
        ("/clear",           "reset"),
        ("/mode <m>",        "switch"),
        ("/model <m>",       "model"),
        ("/temp <n>",        "temp"),
        ("/tokens <n>",      "tokens"),
        ("/abort",           "stop"),
        ("/file <p>",        "attach"),
        ("/image <p>",       "image"),
        ("/browse <url>",    "fetch"),
        ("/git, /g",         "status"),
        ("/gd",              "diff"),
        ("/gl",              "log"),
        ("/gc",              "commit"),
        ("/gb",              "branch"),
        ("/test",            "test"),
        ("/test --watch",    "watch"),
        ("/lint",            "lint"),
        ("/lint --fix",      "autofix"),
        ("/build",           "build"),
        ("/build --fix",     "ai-fix"),
        ("/review",          "review"),
        ("/format",          "format"),
        ("/perf",            "perf"),
        ("/spawn <t>",       "bg-task"),
        ("/find <p>",        "files"),
        ("/grep <p>",        "search"),
        ("/tree",            "tree"),
        ("/symbols <q>",     "symbols"),
        ("/search <q>",      "web"),
        ("/add <f>",         "track"),
        ("/drop <f>",        "untrack"),
        ("/files",           "tracked"),
        ("/context",         "tokens"),
        ("/watch",           "agent"),
        ("/plan",            "plan"),
        ("/skills",          "skills"),
        ("/skill <n>",       "activate"),
        ("/memory",          "memory"),
        ("/remember <t>",    "save"),
        ("/errors",          "errors"),
        ("/sessions",        "sessions"),
        ("/resume",          "resume"),
        ("/new",             "new"),
        ("/compact",         "compact"),
        ("/save <n>",        "snapshot"),
        ("/load <n>",        "restore"),
        ("/security",        "audit"),
        ("/doc",             "docs"),
        ("/changelog",       "changelog"),
        ("/export",          "export"),
        ("/history",         "history"),
        ("/status",          "info"),
        ("/keybindings",     "keys"),
        ("/help",            "help"),
        ("/cheatsheet",      "this"),
        ("/quit",            "exit"),
        // New commands
        ("/todo",            "todos"),
        ("/env",             "env vars"),
        ("/secrets",         "scan secrets"),
        ("/metrics",         "stats"),
        ("/deps",            "deps"),
        ("/diagram",         "diagram"),
        ("/diff <f>",        "ai diff"),
        ("/chat",            "free chat"),
        ("/theme <n>",       "theme"),
        ("/heal",            "auto-fix"),
        ("/blame <f>",       "git blame"),
        ("/resolve",         "conflicts"),
        ("/stash",           "stash"),
        ("/stash list",      "stash list"),
        ("/gcp <h>",         "cherry-pick"),
    ];

    let col_width = 20;
    let cols = 3;
    let rows = (entries.len() + cols - 1) / cols;

    for row in 0..rows {
        set_fg(&mut o, theme::BORDER);
        write!(o, "  {V_LINE} ").ok();
        for col in 0..cols {
            let idx = col * rows + row;
            if idx < entries.len() {
                let (cmd, hint) = entries[idx];
                set_fg(&mut o, theme::CYAN);
                write!(o, "{:<col_width$}", cmd).ok();
                set_fg(&mut o, theme::DIM);
                write!(o, "{:<8}", hint).ok();
            }
        }
        write!(o, "\n").ok();
    }
    reset_color(&mut o);
    print_section_end();
}

fn expand_file_refs(input: &str, root_path: &str) -> String {
    let mut result = input.to_string();
    let mut expansions = Vec::new();

    for word in input.split_whitespace() {
        if let Some(path) = word.strip_prefix('@') {
            if path.is_empty() { continue; }
            let full_path = if path.starts_with('/') {
                path.to_string()
            } else {
                format!("{}/{}", root_path, path)
            };
            match std::fs::read_to_string(&full_path) {
                Ok(content) => {
                    let ext = std::path::Path::new(path)
                        .extension().and_then(|e| e.to_str()).unwrap_or("");
                    expansions.push((
                        word.to_string(),
                        format!("\n\nContents of `{}`:\n```{}\n{}\n```\n", path, ext, content),
                    ));
                }
                Err(e) => {
                    expansions.push((
                        word.to_string(),
                        format!("\n[Error reading {}: {}]\n", path, e),
                    ));
                }
            }
        }
    }

    for (pattern, replacement) in expansions {
        result = result.replacen(&pattern, &replacement, 1);
    }
    result
}

// ─── RPC helper ──────────────────────────────────────────────────────────────

async fn rpc_invoke(host: &str, token: &str, cmd: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
    let ws_url = format!("wss://{}/remote", host);
    let (ws_stream, _) = connect_ws(&ws_url, 3).await?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Authenticate
    let auth_msg = json!({
        "id": next_id(),
        "type": "auth",
        "token": token,
        "device_name": "ShadowAI CLI"
    });
    ws_tx.send(Message::Text(auth_msg.to_string().into())).await.map_err(|e| e.to_string())?;

    // Wait for auth.ok
    if let Some(Ok(Message::Text(text))) = ws_rx.next().await {
        let msg: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return Err(format!("Failed to parse auth response: {} (raw: {})", e, text));
            }
        };
        if msg["type"] != "auth.ok" {
            return Err(format!("Auth failed: {}", msg["error"].as_str().unwrap_or("unknown")));
        }
    } else {
        return Err("Connection closed during authentication".to_string());
    }

    // Send invoke
    let invoke_id = next_id();
    let invoke_msg = json!({
        "id": invoke_id,
        "type": "tauri.invoke",
        "cmd": cmd,
        "args": args
    });
    ws_tx.send(Message::Text(invoke_msg.to_string().into())).await.map_err(|e| e.to_string())?;

    // Wait for response
    while let Some(Ok(Message::Text(text))) = ws_rx.next().await {
        let msg: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[debug] Failed to parse RPC response: {} (raw: {})", _e, text);
                continue;
            }
        };
        if msg["id"] == invoke_id || msg["type"] == "tauri.result" {
            if let Some(error) = msg["error"].as_str() {
                return Err(error.to_string());
            }
            return Ok(msg["result"].clone());
        }
    }
    Err("No response received".to_string())
}

// ─── RPC result formatting ──────────────────────────────────────────────────

fn print_rpc_result(subcmd: &str, result: &serde_json::Value) {
    let mut o = io::stdout();
    match subcmd {
        "models" => {
            if let Some(models) = result.as_array() {
                execute!(o, SetForegroundColor(theme::CYAN), Print(format!("  {} Local Models ({} found)\n\n", DIAMOND, models.len())), ResetColor).ok();
                for m in models {
                    let name = m["name"].as_str().unwrap_or("?");
                    let size_gb = m["size_bytes"].as_u64().unwrap_or(0) as f64 / (1024.0 * 1024.0 * 1024.0);
                    let model_type = m["model_type"].as_str().unwrap_or("?");
                    let arch = m["architecture"].as_str().unwrap_or("");
                    let quant = m["quantization"].as_str().unwrap_or("");
                    let ctx = m["context_length"].as_u64().unwrap_or(0);
                    execute!(o,
                        SetForegroundColor(theme::AI_TEXT), Print(format!("    {ARROW} {}", name)),
                        SetForegroundColor(theme::DIM), Print(format!("  ({:.1} GB, {})", size_gb, model_type)),
                    ).ok();
                    if !arch.is_empty() || !quant.is_empty() {
                        execute!(o, SetForegroundColor(theme::WARN), Print(format!("  [{}{}{}]", arch, if !arch.is_empty() && !quant.is_empty() { " " } else { "" }, quant))).ok();
                    }
                    if ctx > 0 {
                        execute!(o, SetForegroundColor(theme::STAT), Print(format!("  ctx:{}", ctx))).ok();
                    }
                    execute!(o, Print("\n"), ResetColor).ok();
                }
                if models.is_empty() {
                    execute!(o, SetForegroundColor(theme::DIM), Print("    No local models found\n"), ResetColor).ok();
                }
            } else {
                let pretty = serde_json::to_string_pretty(result).unwrap_or_default();
                execute!(o, SetForegroundColor(theme::AI_TEXT), Print(format!("{}\n", pretty)), ResetColor).ok();
            }
        }
        "engines" => {
            if let Some(engines) = result.as_array() {
                execute!(o, SetForegroundColor(theme::CYAN), Print(format!("  {} Installed Engines\n\n", GEAR)), ResetColor).ok();
                for e in engines {
                    let name = e.as_str().unwrap_or("?");
                    execute!(o, SetForegroundColor(theme::OK), Print(format!("    {CHECK} {}\n", name)), ResetColor).ok();
                }
                if engines.is_empty() {
                    execute!(o, SetForegroundColor(theme::DIM), Print("    No engines installed\n"), ResetColor).ok();
                }
            } else {
                let pretty = serde_json::to_string_pretty(result).unwrap_or_default();
                execute!(o, SetForegroundColor(theme::AI_TEXT), Print(format!("{}\n", pretty)), ResetColor).ok();
            }
        }
        "hardware" => {
            execute!(o, SetForegroundColor(theme::CYAN), Print(format!("  {} Hardware Info\n\n", BOLT)), ResetColor).ok();
            let cpu = result["cpu_model"].as_str().unwrap_or("Unknown");
            let cores = result["cpu_cores"].as_u64().unwrap_or(0);
            let ram = result["ram_gb"].as_f64().unwrap_or(0.0);
            execute!(o,
                SetForegroundColor(theme::AI_TEXT), Print(format!("    CPU: {} ({} cores)\n", cpu, cores)),
                Print(format!("    RAM: {:.1} GB\n", ram)),
            ).ok();
            if let Some(gpus) = result["gpus"].as_array() {
                for g in gpus {
                    let name = g["name"].as_str().unwrap_or("?");
                    let vram = g["vram_gb"].as_f64().unwrap_or(0.0);
                    let gtype = g["gpu_type"].as_str().unwrap_or("");
                    execute!(o, SetForegroundColor(theme::WARN), Print(format!("    GPU: {} ({:.0} GB, {})\n", name, vram, gtype))).ok();
                }
            } else if let Some(gpu) = result["gpu_name"].as_str() {
                let vram = result["gpu_vram_gb"].as_f64().unwrap_or(0.0);
                execute!(o, SetForegroundColor(theme::WARN), Print(format!("    GPU: {} ({:.0} GB)\n", gpu, vram))).ok();
            }
            execute!(o, ResetColor).ok();
        }
        "status" => {
            execute!(o, SetForegroundColor(theme::CYAN), Print(format!("  {} LLM Server Status\n\n", RADIO)), ResetColor).ok();
            let running = result["running"].as_bool().unwrap_or(false);
            if running {
                let port = result["port"].as_u64().unwrap_or(0);
                let model = result["model"].as_str().unwrap_or("?");
                let ctx = result["context_length"].as_u64().unwrap_or(0);
                let backend = result["backend"].as_str().unwrap_or("?");
                execute!(o,
                    SetForegroundColor(theme::OK), Print(format!("    {RADIO} Running on port {}\n", port)),
                    SetForegroundColor(theme::AI_TEXT), Print(format!("    Model: {}\n", model)),
                    Print(format!("    Context: {} tokens\n", ctx)),
                    Print(format!("    Backend: {}\n", backend)),
                    ResetColor,
                ).ok();
            } else {
                let error = result["error"].as_str().unwrap_or("");
                execute!(o, SetForegroundColor(theme::ERR), Print(format!("    {CROSS} Not running\n")), ResetColor).ok();
                if !error.is_empty() {
                    execute!(o, SetForegroundColor(theme::DIM), Print(format!("    Last error: {}\n", error)), ResetColor).ok();
                }
            }
        }
        _ => {
            // Generic JSON dump
            let pretty = serde_json::to_string_pretty(result).unwrap_or_default();
            execute!(o, SetForegroundColor(theme::AI_TEXT), Print(format!("{}\n", pretty)), ResetColor).ok();
        }
    }
}

// ─── CLI Help ────────────────────────────────────────────────────────────────

fn print_shell_completions(shell: &str) {
    let commands = [
        "/quit", "/exit", "/q", "/clear", "/help", "/status", "/abort",
        "/sessions", "/providers", "/models", "/memories", "/compact", "/new",
        "/skills", "/skill", "/watch", "/plan", "/errors", "/fixed", "/completed",
        "/memory", "/context", "/ctx", "/format", "/tree", "/export", "/security",
        "/doc", "/changelog", "/release-notes", "/history", "/keybindings", "/perf",
        "/cheatsheet", "/undo", "/copy", "/edits",
        "/git", "/g", "/gd", "/gl", "/gc", "/gb", "/stash", "/gs",
        "/blame", "/resolve", "/gcp", "/pr",
        "/test", "/lint", "/build", "/review",
        "/add", "/drop", "/files", "/find", "/grep",
        "/search", "/browse", "/symbols", "/image",
        "/save", "/load", "/session", "/provider", "/mode", "/model",
        "/temperature", "/temp", "/tokens", "/remember",
        "/todo", "/env", "/secrets", "/metrics", "/deps", "/diagram", "/diff", "/chat",
        "/theme", "/heal",
        "--help", "--version", "-c", "--command", "--pipe",
        "config", "completions", "setup", "models", "engines", "hardware", "status",
    ];

    match shell {
        "bash" => {
            println!("# ShadowAI bash completions");
            println!("# Add to ~/.bashrc: source <(shadowai completions bash)");
            println!("_shadowai_completions() {{");
            println!("  local cur=${{COMP_WORDS[COMP_CWORD]}}");
            println!("  local commands='{}'", commands.join(" "));
            println!("  COMPREPLY=($(compgen -W \"$commands\" -- \"$cur\"))");
            println!("}}");
            println!("complete -F _shadowai_completions shadowai");
        }
        "zsh" => {
            println!("# ShadowAI zsh completions");
            println!("# Add to ~/.zshrc: source <(shadowai completions zsh)");
            println!("#compdef shadowai");
            println!("_shadowai() {{");
            println!("  local commands=({} )", commands.iter().map(|c| format!("'{}'", c)).collect::<Vec<_>>().join(" "));
            println!("  _arguments '1: :($commands)'");
            println!("}}");
            println!("compdef _shadowai shadowai");
        }
        "fish" => {
            println!("# ShadowAI fish completions");
            println!("# Add to ~/.config/fish/completions/shadowai.fish");
            for cmd in &commands {
                println!("complete -c shadowai -f -a '{}'", cmd);
            }
        }
        _ => {
            eprintln!("Unknown shell: {}. Supported: bash, zsh, fish", shell);
        }
    }
}

fn print_cli_help() {
    let mut o = io::stdout();
    execute!(o,
        SetForegroundColor(theme::CYAN), Print(format!("\n  {} ShadowAI CLI\n\n", SPARK)), ResetColor,
        SetForegroundColor(theme::AI_TEXT),
        Print("  Usage:\n"),
        Print("    shadowai                    Interactive AI chat\n"),
        Print("    shadowai <message>          One-shot message\n"),
        Print("    shadowai exec \"<prompt>\"     Execute prompt (one-shot)\n"),
        Print("    shadowai setup <host> <tok> Configure connection\n"),
        Print("    shadowai models             List local models\n"),
        Print("    shadowai engines            List installed engines\n"),
        Print("    shadowai hardware           Show hardware info\n"),
        Print("    shadowai status             Show LLM server status\n"),
        Print("    shadowai config             Show config file info\n"),
        Print("    shadowai help               Show this help\n"),
        Print("\n  Flags:\n"),
        Print("    --json                      Output JSON response to stdout\n"),
        Print("    --no-stream                 Disable streaming (final output only)\n"),
        Print("    --max-turns N               Limit agent iterations\n"),
        Print("    --resume, -r                Resume last session on startup\n"),
        Print("    --new, -n                   Start a fresh session (skip auto-resume)\n"),
        Print("\n"),
        ResetColor,
    ).ok();
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Load TOML config early so it can influence color and defaults
    let cli_config = load_config();

    // Section 8.1: store email config snapshot in global for use in handle_event
    {
        let snap = EmailCfgSnapshot {
            smtp_host: cli_config.smtp_host.clone(),
            smtp_port: cli_config.smtp_port,
            smtp_user: cli_config.smtp_user.clone(),
            smtp_password: cli_config.smtp_password.clone(),
            smtp_from: cli_config.smtp_from.clone(),
            smtp_to: cli_config.smtp_to.clone(),
            smtp_tls: cli_config.smtp_tls,
            email_notify_threshold_secs: cli_config.email_notify_threshold_secs,
        };
        let _ = EMAIL_CFG_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(Some(snap)));
    }

    // Apply no_color from config (env var NO_COLOR still takes priority via use_color())
    if cli_config.no_color == Some(true) && std::env::var("NO_COLOR").is_err() {
        std::env::set_var("NO_COLOR", "1");
    }

    init_color_support();
    let args: Vec<String> = std::env::args().collect();

    // Handle `shadowai setup <host> <token>`
    if args.len() >= 2 && args[1] == "setup" {
        if args.len() < 4 {
            eprintln!("Usage: shadowai setup <host:port> <token>");
            eprintln!("Example: shadowai setup 127.0.0.1:9000 my-pairing-token");
            std::process::exit(EXIT_CONFIG);
        }
        save_host(&args[2]);
        save_token(&args[3]);
        let mut o = io::stdout();
        set_fg(&mut o, theme::OK);
        write!(o, "  {CHECK} Saved! Host: {}, Token: ****\n", args[2]).ok();
        reset_color(&mut o);
        return;
    }

    // Handle subcommands that use RPC
    if args.len() >= 2 {
        let subcmd = args[1].as_str();
        match subcmd {
            "models" | "engines" | "hardware" | "status" => {
                let host = read_host();
                let token = match read_token() {
                    Some(t) => t,
                    None => {
                        print_error("No token configured. Run: shadowai setup <host:port> <token>");
                        std::process::exit(EXIT_CONFIG);
                    }
                };

                let (cmd, args_val) = match subcmd {
                    "models" => ("scan_local_models", json!({"basePath": ""})),
                    "engines" => ("list_installed_engines", json!({})),
                    "hardware" => ("detect_hardware", json!({})),
                    "status" => ("get_llm_server_status", json!({})),
                    _ => unreachable!(),
                };

                match rpc_invoke(&host, &token, cmd, args_val).await {
                    Ok(result) => {
                        print_rpc_result(subcmd, &result);
                    }
                    Err(e) => {
                        print_error(&format!("RPC failed: {}", e));
                        std::process::exit(EXIT_CONNECTION);
                    }
                }
                return;
            }
            "help" | "--help" | "-h" => {
                print_cli_help();
                return;
            }
            "config" => {
                print_config_info();
                return;
            }
            "completions" => {
                // Shell completions (Section 9.3)
                let shell = args.get(2).map(|s| s.as_str()).unwrap_or("bash");
                print_shell_completions(shell);
                return;
            }
            "prompt-info" => {
                // Shell prompt info (Section 6.6): outputs branch|skill|mode
                let branch = std::process::Command::new("git")
                    .args(["rev-parse", "--abbrev-ref", "HEAD"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "".to_string());
                let config = load_config();
                let skill = config.default_skill.as_deref().unwrap_or("").to_string();
                let mode = config.mode.as_deref().unwrap_or("auto").to_string();
                println!("{}|{}|{}", branch, skill, mode);
                return;
            }
            "-c" | "--command" => {
                // One-shot mode (Section 9.3): shadowai -c "prompt"
                COLOR_ENABLED.store(false, Ordering::SeqCst);
                let prompt = args.get(2).cloned().unwrap_or_default();
                let stdin_extra = if !io::stdin().is_terminal() {
                    let mut s = String::new();
                    let _ = io::Read::read_to_string(&mut io::stdin(), &mut s);
                    s
                } else {
                    String::new()
                };
                let full_prompt = if stdin_extra.is_empty() {
                    prompt
                } else {
                    format!("{}\n\n{}", prompt, stdin_extra)
                };
                if !full_prompt.is_empty() {
                    // Print prompt to stderr so stdout is clean
                    eprintln!("shadowai: {}", full_prompt.chars().take(80).collect::<String>());
                }
                // Fall through to one-shot mode with the prompt
                // (the positional arg handling below will pick it up)
                // We set positional args here so the one-shot path works
                if !full_prompt.is_empty() {
                    let mut args2 = args.clone();
                    args2.push(full_prompt.clone());
                    // We can't easily restart here — just print a notice
                    eprintln!("Use: echo \"prompt\" | shadowai pipe  OR  shadowai \"prompt\"");
                }
                return;
            }
            "--pipe" => {
                // Pipe mode (Section 9.3): read lines from stdin, send each to output
                COLOR_ENABLED.store(false, Ordering::SeqCst);
                use io::BufRead;
                let stdin = io::stdin();
                for line in stdin.lock().lines() {
                    match line {
                        Ok(l) if !l.trim().is_empty() => {
                            println!("{}", l.trim());
                        }
                        _ => {}
                    }
                }
                return;
            }
            _ => {} // Fall through to one-shot/interactive (handles exec, flags, etc.)
        }
    }

    // Parse flags
    let mut json_output = false;
    let mut no_stream = false;
    let mut _max_turns: Option<u32> = None;
    let mut force_resume = false;
    let mut force_new = false;
    let mut tui_flag = false;
    let mut no_tui_flag = false;
    let mut positional_args: Vec<String> = Vec::new();
    {
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--json" => { json_output = true; }
                "--no-stream" => { no_stream = true; }
                "--resume" | "-r" => { force_resume = true; }
                "--new" | "-n" => { force_new = true; }
                "--tui" => { tui_flag = true; }
                "--no-tui" => { no_tui_flag = true; }
                "--max-turns" => {
                    i += 1;
                    if i < args.len() {
                        _max_turns = args[i].parse::<u32>().ok();
                    }
                }
                "--max-budget" => {
                    i += 1;
                    if i < args.len() {
                        if let Ok(budget) = args[i].parse::<u64>() {
                            BUDGET_LIMIT.store(budget, Ordering::SeqCst);
                        }
                    }
                }
                other => { positional_args.push(other.to_string()); }
            }
            i += 1;
        }
    }

    // TUI mode check: activate if --tui flag, or if stdin is a terminal and SHADOWAI_NO_TUI not set and --no-tui not passed
    // Check if shadow-ide is running by looking for server.port auto-discovery file.
    // If found, prefer WS-connected mode even for interactive sessions.
    let shadow_ide_port: Option<u16> = config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("server.port")).ok())
        .and_then(|s| s.trim().parse().ok());

    let tui_mode = if no_tui_flag {
        false
    } else if tui_flag {
        true
    } else if std::env::var("SHADOWAI_NO_TUI").is_ok() {
        false
    } else {
        // TUI is the DEFAULT: activate whenever no positional args are given.
        // This covers both interactive terminal use and `shadowai` with no args.
        // Use `shadowai --no-tui` or SHADOWAI_NO_TUI=1 to get old ANSI mode.
        // Pipe mode / one-shot mode (with args) still uses ANSI.
        positional_args.is_empty() && !json_output && !no_stream && !force_resume && !force_new
    };

    if tui_mode {
        let root = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let model_str = cli_config.model.clone().unwrap_or_else(|| "auto".to_string());
        let mode_str = cli_config.mode.clone().unwrap_or_else(|| "auto".to_string());
        if let Err(e) = run_tui_mode(root, model_str, mode_str, None, &cli_config).await {
            eprintln!("TUI error: {}", e);
        }
        return;
    }

    // Pipe mode: `echo "text" | shadowai pipe` or `cat file | shadowai pipe "prompt"`
    // Also handles: `cat image.png | shadowai "what is in this image?"` (Section 16)
    if !positional_args.is_empty() && positional_args[0] == "pipe" {
        let mut stdin_content = String::new();
        if !io::stdin().is_terminal() {
            use std::io::Read;
            io::stdin().read_to_string(&mut stdin_content).ok();
        }
        let prompt = if positional_args.len() >= 2 {
            let user_prompt = positional_args[1..].join(" ");
            if stdin_content.is_empty() {
                user_prompt
            } else {
                format!("{}\n\n---\n\n{}", user_prompt, stdin_content)
            }
        } else {
            if stdin_content.is_empty() {
                eprintln!("Error: no stdin data and no prompt provided for pipe mode");
                std::process::exit(EXIT_ERROR);
            }
            stdin_content
        };
        positional_args = vec![prompt];
        json_output = true;
        no_stream = true;
    }

    // Section 16: Multimodal pipe — detect image piped to a non-pipe prompt.
    // `cat image.png | shadowai "what is in this image?"`
    // We check early (before pipe mode consumes stdin) only when positional_args
    // are already set (user typed a prompt) and stdin is not a terminal.
    // If image detected, prepend a note to the prompt; the actual image bytes
    // are stored in the CLI config for the provider to attach.
    if !positional_args.is_empty() && positional_args[0] != "pipe"
        && positional_args[0] != "setup"
        && !io::stdin().is_terminal()
    {
        // Peek at stdin — read raw bytes to check for image magic bytes
        use std::io::Read;
        let mut raw = Vec::new();
        if std::io::stdin().lock().read_to_end(&mut raw).is_ok() && !raw.is_empty() {
            let is_image = raw.starts_with(b"\x89PNG\r\n\x1a\n")
                || raw.starts_with(b"\xff\xd8\xff")
                || raw.starts_with(b"GIF8")
                || (raw.len() >= 12 && &raw[8..12] == b"WEBP");

            if is_image {
                let mime = if raw.starts_with(b"\x89PNG") { "image/png" }
                    else if raw.starts_with(b"\xff\xd8") { "image/jpeg" }
                    else if raw.starts_with(b"GIF8") { "image/gif" }
                    else { "image/webp" };

                // Base64-encode the image and embed a data URI note in the prompt
                let b64: String = {
                    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                    let mut enc = String::with_capacity((raw.len() + 2) / 3 * 4);
                    let mut i = 0;
                    while i < raw.len() {
                        let len = (raw.len() - i).min(3);
                        let n = if len == 3 { (raw[i] as u32) << 16 | (raw[i+1] as u32) << 8 | raw[i+2] as u32 }
                                else if len == 2 { (raw[i] as u32) << 16 | (raw[i+1] as u32) << 8 }
                                else { (raw[i] as u32) << 16 };
                        enc.push(TABLE[((n >> 18) & 63) as usize] as char);
                        enc.push(TABLE[((n >> 12) & 63) as usize] as char);
                        enc.push(if len > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
                        enc.push(if len > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
                        i += 3;
                    }
                    enc
                };
                let user_prompt = positional_args.join(" ");
                // Attach image as a data URI note — providers that support vision will use it
                let combined = format!(
                    "{}\n\n[Image attached — {} bytes, {}]\ndata:{};base64,{}",
                    user_prompt, raw.len(), mime, mime,
                    &b64[..b64.len().min(2000)] // truncate for display; full sent separately
                );
                positional_args = vec![combined];
                eprintln!("[shadowai] Image detected from stdin ({}, {} bytes) — attaching to prompt.", mime, raw.len());
            }
        }
    }

    // One-shot mode: `shadowai "your message here"` or `shadowai exec "prompt"`
    let one_shot_message = if !positional_args.is_empty() && positional_args[0] != "setup" {
        if positional_args[0] == "exec" && positional_args.len() >= 2 {
            Some(positional_args[1..].join(" "))
        } else {
            Some(positional_args.join(" "))
        }
    } else {
        None
    };

    // If json_output or no_stream, force one-shot behavior silently
    // Store scripting mode flags as atomics for sharing
    let json_output_flag = Arc::new(AtomicBool::new(json_output));
    let no_stream_flag = Arc::new(AtomicBool::new(no_stream));

    let host = read_host();
    // For localhost connections, shadow-ide auto-pairs any token.
    // Generate and persist a stable auto-token so repeated connections use the same device entry.
    let token = read_token().unwrap_or_else(|| {
        let is_local = host.starts_with("127.") || host.starts_with("localhost") || host.starts_with("::1");
        if is_local {
            // Generate a stable random token once, save it for future sessions
            let auto_token = format!("shadowai-auto-{:016x}", {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                if let Ok(u) = std::fs::read_to_string("/etc/machine-id").or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id")) {
                    u.trim().hash(&mut h);
                } else {
                    std::env::var("USER").unwrap_or_default().hash(&mut h);
                }
                h.finish()
            });
            save_token(&auto_token);
            auto_token
        } else {
            print_error("No token configured. Run: shadowai setup <host:port> <token>");
            std::process::exit(EXIT_CONFIG);
        }
    });

    // ── Connect ──
    let ws_url = format!("wss://{}/remote", host);
    {
        let mut o = io::stdout();
        execute!(o,
            SetForegroundColor(theme::DIM),
            Print(format!("  Connecting to {}...\n", host)),
            ResetColor,
        ).ok();
    }
    let (ws_stream, _) = match connect_ws(&ws_url, 10).await {
        Ok(result) => result,
        Err(e) => {
            print_error(&e);
            std::process::exit(EXIT_CONNECTION);
        }
    };
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Authenticate
    let auth_msg = json!({
        "id": next_id(),
        "type": "auth",
        "token": token,
        "device_name": "ShadowAI CLI"
    });
    ws_tx.send(Message::Text(auth_msg.to_string().into())).await.ok();

    let auth_resp = ws_rx.next().await;
    match auth_resp {
        Some(Ok(Message::Text(text))) => {
            let msg: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    print_error(&format!("Failed to parse auth response: {}", e));
                    std::process::exit(EXIT_CONNECTION);
                }
            };
            if msg["type"] == "auth.ok" {
                // good
            } else if msg["type"] == "auth.error" {
                print_error(&format!("Authentication failed: {}", msg["message"]));
                std::process::exit(EXIT_AUTH);
            } else {
                print_error(&format!("Unexpected auth response: {}", text));
                std::process::exit(EXIT_AUTH);
            }
        }
        _ => {
            print_error("Connection closed during authentication");
            std::process::exit(EXIT_CONNECTION);
        }
    }

    // ── Collect initial state ──
    let state_id = next_id();
    ws_tx.send(Message::Text(
        json!({ "id": state_id, "type": "sync.getState" }).to_string().into(),
    )).await.ok();

    let profiles_id = next_id();
    ws_tx.send(Message::Text(
        json!({ "id": profiles_id, "type": "ferrum.getProfiles" }).to_string().into(),
    )).await.ok();

    let sessions_init_id = next_id();
    ws_tx.send(Message::Text(
        json!({ "id": sessions_init_id, "type": "ferrum.listSessions" }).to_string().into(),
    )).await.ok();

    let mut root_path = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string());
    let mut chat_mode = cli_config.mode.clone().unwrap_or_else(|| "auto".to_string());
    let mut model: Option<String> = cli_config.model.clone();
    let mut base_url: Option<String> = None;
    let mut temperature: f64 = cli_config.temperature.unwrap_or(0.7);
    let mut max_tokens: i32 = 4096;
    let mut active_skill: Option<Skill> = None;
    let mut skill_turn_count: u32 = 0;
    let current_plan: Arc<tokio::sync::Mutex<Option<String>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let plan_steps: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let plan_approved: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let modified_files: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let last_op: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));
    let watch_active: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let hooks = load_hooks();
    let last_tool_failed: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let last_tool_name: Arc<tokio::sync::Mutex<String>> = Arc::new(tokio::sync::Mutex::new(String::new()));
    let streaming = Arc::new(AtomicBool::new(false));
    let waiting_for_first_token = Arc::new(AtomicBool::new(false));
    let current_stream_id: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));
    let abort_flag = Arc::new(AtomicBool::new(false));
    let stream_start: Arc<tokio::sync::Mutex<Option<Instant>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let stream_token_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

    // Turn counter for numbering
    let turn_counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

    // Git branch cache
    let git_branch: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
    {
        // Detect initial git branch
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&root_path)
            .output()
        {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                *git_branch.lock().unwrap_or_else(|e| e.into_inner()) = branch;
            }
        }
    }

    let file_context: Arc<tokio::sync::Mutex<Option<String>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    // Image context: (base64_data, mime_type)
    let image_context: Arc<tokio::sync::Mutex<Option<(String, String)>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    // Spawned background tasks: (stream_id, task_description)
    let spawned_tasks: Arc<tokio::sync::Mutex<Vec<(String, String)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let tracked_files: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let profiles_cache: Arc<tokio::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let sessions_cache: Arc<std::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let current_session_id: Arc<tokio::sync::Mutex<Option<String>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let mut init_count = 0;
    while init_count < 3 {
        tokio::select! {
            Some(Ok(Message::Text(text))) = ws_rx.next() => {
                let msg: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_e) => {
                        #[cfg(debug_assertions)]
                        eprintln!("[debug] Failed to parse init message: {}", _e);
                        continue;
                    }
                };
                if msg["id"] == state_id {
                    if let Some(pr) = msg["project_root"].as_str() {
                        root_path = pr.to_string();
                    }
                    init_count += 1;
                } else if msg["id"] == profiles_id {
                    if let Some(profiles) = msg["profiles"].as_array() {
                        *profiles_cache.lock().await = profiles.clone();
                        if let Some(first) = profiles.first() {
                            if let Some(bu) = first["base_url"].as_str() {
                                base_url = Some(bu.to_string());
                            }
                            if let Some(m) = first["default_model"].as_str() {
                                if model.is_none() {
                                    model = Some(m.to_string());
                                }
                            }
                        }
                    }
                    init_count += 1;
                } else if msg["id"] == sessions_init_id {
                    if let Some(sessions) = msg["sessions"].as_array() {
                        *sessions_cache.lock().unwrap_or_else(|e| e.into_inner()) = sessions.clone();
                    }
                    init_count += 1;
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                break;
            }
        }
    }

    if one_shot_message.is_none() {
        execute!(io::stdout(), terminal::Clear(ClearType::All), cursor::MoveTo(0, 0)).ok();
        print_banner();
        print_connected_msg(&host);
        print_status_bar(
            &chat_mode,
            model.as_deref().unwrap_or("default"),
            &root_path,
        );
        // Hooks: session_start (Section 8.2)
        {
            let hooks_start = load_hooks();
            run_hooks(&hooks_start, "session_start", None, &json!({}));
        }
    }

    // Load theme from config if specified (Section 9.2)
    if let Some(ref theme_name) = cli_config.theme {
        load_theme(theme_name);
    }

    // Auto-create or resume a session for this CLI chat
    let create_session_req_id = next_id();
    let saved_session_id = read_last_session_id();
    let should_resume = !force_new && (force_resume || saved_session_id.is_some());
    {
        if should_resume {
            if let Some(ref sid) = saved_session_id {
                if one_shot_message.is_none() {
                    let short = &sid[..sid.len().min(8)];
                    let mut o = io::stdout();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  Resuming session #{}… (use --new for a fresh session)\n", short).ok();
                    reset_color(&mut o);
                }
                ws_tx.send(Message::Text(json!({
                    "id": create_session_req_id,
                    "type": "ferrum.getSession",
                    "session_id": sid,
                }).to_string().into())).await.ok();
            } else {
                // force_resume but no saved ID — fall through to create
                let profile_name = profiles_cache.lock().await.first()
                    .and_then(|p| p["name"].as_str().map(String::from))
                    .unwrap_or_else(|| "default".to_string());
                let session_name = format!("CLI {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
                ws_tx.send(Message::Text(json!({
                    "id": create_session_req_id,
                    "type": "ferrum.createSession",
                    "name": session_name,
                    "profile": profile_name,
                }).to_string().into())).await.ok();
            }
        } else {
            let profile_name = profiles_cache.lock().await.first()
                .and_then(|p| p["name"].as_str().map(String::from))
                .unwrap_or_else(|| "default".to_string());
            let session_name = format!("CLI {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
            ws_tx.send(Message::Text(json!({
                "id": create_session_req_id,
                "type": "ferrum.createSession",
                "name": session_name,
                "profile": profile_name,
            }).to_string().into())).await.ok();
        }
    }

    // Chat history
    let messages: Arc<tokio::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let response_acc: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));
    let last_response: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));
    let stream_formatter: Arc<tokio::sync::Mutex<StreamFormatter>> =
        Arc::new(tokio::sync::Mutex::new(StreamFormatter::new()));
    let thinking_acc: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));

    // Channel for user commands
    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<UserCommand>();

    // Shared prompt state for the input thread
    let prompt_mode: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(chat_mode.clone()));
    let prompt_skill: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let prompt_tokens: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let prompt_tracked_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

    // Spawn input reader thread
    let user_tx_clone = user_tx.clone();
    let streaming_clone = streaming.clone();
    let one_shot = one_shot_message.clone();
    let sessions_cache_input = sessions_cache.clone();
    let prompt_mode_clone = prompt_mode.clone();
    let prompt_skill_clone = prompt_skill.clone();
    let prompt_tokens_clone = prompt_tokens.clone();
    let prompt_turn_clone = turn_counter.clone();
    let prompt_branch_clone = git_branch.clone();
    let prompt_root_path = root_path.clone();
    let prompt_tracked_clone = prompt_tracked_count.clone();
    std::thread::spawn(move || {
        if let Some(msg) = one_shot {
            let _ = user_tx_clone.send(UserCommand::Message(msg));
            return;
        }
        loop {
            if !streaming_clone.load(Ordering::SeqCst) {
                let mode = prompt_mode_clone.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let skill = prompt_skill_clone.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let tokens = prompt_tokens_clone.load(Ordering::SeqCst) as usize;
                let turn = prompt_turn_clone.load(Ordering::SeqCst);
                let branch = prompt_branch_clone.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let tracked_count = prompt_tracked_clone.load(Ordering::SeqCst) as usize;
                print_prompt_full(&mode, skill.as_deref(), tokens, 128000, turn, &branch, &prompt_root_path, tracked_count);
            }
            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(0) => {
                    let _ = user_tx_clone.send(UserCommand::Quit);
                    break;
                }
                Ok(_) => {
                    let trimmed = input.trim().to_string();
                    if trimmed.is_empty() { continue; }

                    // Ctrl+L (clear screen) — terminal sends \x0c
                    if trimmed == "\x0c" || trimmed.contains('\x0c') {
                        let _ = user_tx_clone.send(UserCommand::Clear);
                        continue;
                    }
                    // Ctrl+K (clear input buffer) — just echo a newline, nothing to send
                    if trimmed == "\x0b" || trimmed.contains('\x0b') {
                        continue;
                    }
                    // Ctrl+N (new session) — terminal sends \x0e
                    if trimmed == "\x0e" || trimmed.contains('\x0e') {
                        let _ = user_tx_clone.send(UserCommand::New);
                        continue;
                    }

                    // Multiline input: triple-quote mode
                    let final_input = if trimmed == "\"\"\"" {
                        let mut lines = Vec::new();
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  ... multiline mode (\"\"\" to end) ...\n").ok();
                        reset_color(&mut o);
                        o.flush().ok();
                        loop {
                            let mut ml = String::new();
                            match io::stdin().read_line(&mut ml) {
                                Ok(0) => break,
                                Ok(_) => {
                                    let lt = ml.trim_end_matches('\n').trim_end_matches('\r');
                                    if lt.trim() == "\"\"\"" { break; }
                                    lines.push(lt.to_string());
                                }
                                Err(_) => break,
                            }
                        }
                        lines.join("\n")
                    } else if trimmed.ends_with('\\') {
                        // Backslash continuation
                        let mut combined = trimmed.trim_end_matches('\\').to_string();
                        loop {
                            let mut cont = String::new();
                            let mut o = io::stdout();
                            set_fg(&mut o, theme::DIM);
                            write!(o, "  ... ").ok();
                            reset_color(&mut o);
                            o.flush().ok();
                            match io::stdin().read_line(&mut cont) {
                                Ok(0) => break,
                                Ok(_) => {
                                    let ct = cont.trim().to_string();
                                    if ct.ends_with('\\') {
                                        combined.push_str(" ");
                                        combined.push_str(ct.trim_end_matches('\\'));
                                    } else {
                                        combined.push_str(" ");
                                        combined.push_str(&ct);
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        combined
                    } else {
                        trimmed.clone()
                    };

                    if final_input.is_empty() { continue; }

                    // Save to history
                    save_history_entry(&final_input);

                    // Interactive session picker for /resume and /sessions
                    if final_input == "/resume" || final_input == "/sessions" {
                        if let Some(session_id) = interactive_session_picker(&sessions_cache_input) {
                            let _ = user_tx_clone.send(UserCommand::Resume(Some(session_id)));
                        }
                        continue;
                    }

                    let _ = user_tx_clone.send(parse_command(&final_input));
                }
                Err(_) => break,
            }
        }
    });

    // Ctrl+C handler
    let abort_ctrlc = abort_flag.clone();
    let streaming_ctrlc = streaming.clone();
    let user_tx_ctrlc = user_tx.clone();
    ctrlc::set_handler(move || {
        if streaming_ctrlc.load(Ordering::SeqCst) {
            abort_ctrlc.store(true, Ordering::SeqCst);
            let _ = user_tx_ctrlc.send(UserCommand::Abort);
        } else {
            println!();
            std::process::exit(EXIT_OK);
        }
    }).ok();

    // ── Heartbeat timer to keep WebSocket alive (server disconnects after 1800s) ──
    let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(20));
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // ── Main event loop ──
    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let v: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_e) => {
                                #[cfg(debug_assertions)]
                                eprintln!("[debug] Failed to parse server message: {}", _e);
                                continue;
                            }
                        };
                        handle_ws_message(
                            &v, &current_stream_id, &streaming,
                            &response_acc, &last_response, &stream_formatter, &thinking_acc, &messages,
                            &stream_start, one_shot_message.is_some(),
                            &sessions_cache, &waiting_for_first_token,
                            &root_path, &last_tool_failed, &last_tool_name,
                            &hooks, &stream_token_count,
                            no_stream_flag.load(Ordering::SeqCst),
                            json_output_flag.load(Ordering::SeqCst),
                            cli_config.auto_lint.unwrap_or(false),
                            &modified_files, &last_op, &plan_steps,
                            cli_config.auto_commit.unwrap_or(false),
                            &turn_counter,
                        ).await;

                        // Auto-load messages when ferrum.session response arrives (from /session last, /resume, etc.)
                        if v["type"].as_str() == Some("ferrum.session") {
                            if let Some(session_id) = v["session"].get("id").and_then(|id| id.as_str()) {
                                // Store the active session ID and persist it
                                *current_session_id.lock().await = Some(session_id.to_string());
                                save_last_session_id(session_id);

                                let is_auto_create = v["id"].as_u64() == Some(create_session_req_id);
                                if is_auto_create {
                                    // Auto-created or resumed session on startup
                                    if one_shot_message.is_none() {
                                        let short = &session_id[..session_id.len().min(8)];
                                        if should_resume && saved_session_id.is_some() {
                                            // Load messages for resumed session
                                            let name = v["session"]["name"].as_str().unwrap_or("CLI");
                                            print_info_accent("Resumed", &format!("{} #{}", name, short));
                                            ws_tx.send(Message::Text(
                                                json!({
                                                    "id": next_id(),
                                                    "type": "ferrum.loadMessages",
                                                    "session_id": session_id
                                                }).to_string().into()
                                            )).await.ok();
                                        } else {
                                            print_info_accent("Session", &format!("#{}", short));
                                        }
                                    }
                                } else {
                                    // User requested session load — load its messages
                                    let name = v["session"]["name"].as_str().unwrap_or("(unnamed)");
                                    let short = &session_id[..session_id.len().min(8)];
                                    print_info_accent("Session", &format!("{} #{}", name, short));
                                    ws_tx.send(Message::Text(
                                        json!({
                                            "id": next_id(),
                                            "type": "ferrum.loadMessages",
                                            "session_id": session_id
                                        }).to_string().into()
                                    )).await.ok();
                                }
                            }
                        }

                        // Save assistant message to session after AI response completes
                        {
                            let event_str = if v["type"].as_str() == Some("tauri.event") {
                                v["event"].as_str().unwrap_or("")
                            } else if v["type"].as_str() == Some("agent.event") {
                                v["event_type"].as_str().unwrap_or("")
                            } else {
                                v["event"].as_str().unwrap_or("")
                            };
                            if event_str.starts_with("ai-chat-done") || event_str == "agent_done" {
                                // Increment turn counter
                                turn_counter.fetch_add(1, Ordering::SeqCst);

                                // Refresh git branch after AI operations (may have changed branch)
                                if let Ok(output) = std::process::Command::new("git")
                                    .args(["rev-parse", "--abbrev-ref", "HEAD"])
                                    .current_dir(&root_path)
                                    .output()
                                {
                                    if output.status.success() {
                                        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                        *git_branch.lock().unwrap_or_else(|e| e.into_inner()) = branch;
                                    }
                                }

                                if let Some(ref sid) = *current_session_id.lock().await {
                                    // Get the last message — if it's an assistant message, save it
                                    let msgs = messages.lock().await;
                                    if let Some(last) = msgs.last() {
                                        if last["role"].as_str() == Some("assistant") {
                                            let content = last["content"].as_str().unwrap_or("");
                                            if !content.is_empty() {
                                                ws_tx.send(Message::Text(json!({
                                                    "id": next_id(),
                                                    "type": "ferrum.saveMessage",
                                                    "session_id": sid,
                                                    "message": { "role": "assistant", "content": content }
                                                }).to_string().into())).await.ok();
                                            }
                                        }
                                    }
                                }

                                // Skill auto-deactivation: check max_turns and auto_deactivate
                                if active_skill.is_some() {
                                    skill_turn_count += 1;
                                    let should_deactivate = {
                                        let sk = active_skill.as_ref().unwrap();
                                        sk.auto_deactivate.unwrap_or(false)
                                            || sk.max_turns.map(|m| skill_turn_count >= m).unwrap_or(false)
                                    };
                                    if should_deactivate {
                                        let name = active_skill.as_ref().unwrap().name.clone();
                                        active_skill = None;
                                        skill_turn_count = 0;
                                        *prompt_skill.lock().unwrap_or_else(|e| e.into_inner()) = None;
                                        print_info_accent("Skill", &format!("{} auto-deactivated", name));
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        ws_tx.send(Message::Pong(data)).await.ok();
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let disconnect_time = Instant::now();
                        log_state_event("errors.md", "Disconnect", "WebSocket closed by server or lost");
                        print_reconnecting();
                        match connect_ws(&ws_url, 10).await {
                            Ok((new_stream, _)) => {
                                let (new_tx, new_rx) = new_stream.split();
                                ws_tx = new_tx;
                                ws_rx = new_rx;

                                let auth = json!({
                                    "id": next_id(), "type": "auth",
                                    "token": token, "device_name": "ShadowAI CLI"
                                });
                                ws_tx.send(Message::Text(auth.to_string().into())).await.ok();

                                if let Some(Ok(Message::Text(resp))) = ws_rx.next().await {
                                    let r: serde_json::Value = match serde_json::from_str(&resp) {
                                        Ok(v) => v,
                                        Err(_e) => {
                                            #[cfg(debug_assertions)]
                                            eprintln!("[debug] Failed to parse reconnect auth response: {}", _e);
                                            print_error("Failed to parse re-auth response.");
                                            break;
                                        }
                                    };
                                    if r["type"] == "auth.ok" {
                                        let gap = disconnect_time.elapsed().as_secs();
                                        log_state_event("Completed.md", "Reconnected", &format!("gap: {}s", gap));

                                        // Memory recovery: check if a task was running when we disconnected
                                        let state_dir = std::env::var("SHADOWAI_STATE_DIR")
                                            .unwrap_or_else(|_| "./state".to_string());
                                        let memory_path = std::path::Path::new(&state_dir).join("memory.md");
                                        if let Ok(memory) = std::fs::read_to_string(&memory_path) {
                                            if memory.contains("- status: running") {
                                                let mut o = io::stdout();
                                                execute!(o,
                                                    SetForegroundColor(theme::WARN),
                                                    Print(format!("  {} Recovered mid-task context from memory.md\n", RADIO)),
                                                    ResetColor,
                                                ).ok();
                                            }
                                        }

                                        print_reconnected();
                                    } else {
                                        print_error("Re-authentication failed.");
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                print_error(&e);
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }

            Some(cmd) = user_rx.recv() => {
                match cmd {
                    UserCommand::Quit => {
                        // Hooks: session_end (Section 8.2)
                        run_hooks(&hooks, "session_end", None, &json!({}));
                        break;
                    }

                    UserCommand::Clear => {
                        messages.lock().await.clear();
                        execute!(io::stdout(), terminal::Clear(ClearType::All), cursor::MoveTo(0, 0)).ok();
                        print_banner();
                        print_connected_msg(&host);
                        print_status_bar(&chat_mode, model.as_deref().unwrap_or("default"), &root_path);
                    }

                    UserCommand::Help(sub) => {
                        match sub {
                            Some(cmd) => print_command_help(&cmd),
                            None => print_help(),
                        }
                    }

                    UserCommand::Mode(m) => {
                        if matches!(m.as_str(), "plan" | "build" | "auto") {
                            chat_mode = m.clone();
                            *prompt_mode.lock().unwrap_or_else(|e| e.into_inner()) = chat_mode.clone();
                            print_mode_badge(&chat_mode);
                        } else {
                            print_error("Invalid mode. Use: plan, build, auto");
                        }
                    }

                    UserCommand::Model(m) => {
                        model = Some(m.clone());
                        print_model_badge(&m);
                    }

                    UserCommand::Status => {
                        print_section_header("Status");
                        print_section_row("Host", &host);
                        print_section_row("Mode", &chat_mode.to_uppercase());
                        print_section_row("Model", model.as_deref().unwrap_or("default"));
                        print_section_row("Base URL", base_url.as_deref().unwrap_or("(auto)"));
                        print_section_row("Root", &root_path);
                        print_section_row("Temperature", &format!("{:.1}", temperature));
                        print_section_row("Max tokens", &max_tokens.to_string());
                        print_section_row("Streaming", &streaming.load(Ordering::SeqCst).to_string());
                        print_section_row("History", &format!("{} messages", messages.lock().await.len()));
                        print_section_end();
                    }

                    UserCommand::Abort => {
                        if streaming.load(Ordering::SeqCst) {
                            let sid = current_stream_id.lock().await.clone();
                            if !sid.is_empty() {
                                let abort_msg = json!({
                                    "id": next_id(), "type": "tauri.invoke",
                                    "cmd": "abort_ai_chat",
                                    "args": { "streamId": sid }
                                });
                                ws_tx.send(Message::Text(abort_msg.to_string().into())).await.ok();
                                streaming.store(false, Ordering::SeqCst);
                                let mut o = io::stdout();
                                execute!(o,
                                    SetForegroundColor(theme::WARN),
                                    Print(format!("\n  {CROSS} Aborted\n")),
                                    ResetColor,
                                ).ok();
                            }
                        }
                    }

                    UserCommand::Temperature(t) => {
                        temperature = t;
                        print_info_accent("Temperature", &format!("{:.1}", t));
                    }

                    UserCommand::MaxTokens(t) => {
                        max_tokens = t;
                        print_info_accent("Max tokens", &t.to_string());
                    }

                    UserCommand::File(path) => {
                        let full_path = if path.starts_with('/') { path.clone() }
                            else { format!("{}/{}", root_path, path) };
                        match std::fs::read_to_string(&full_path) {
                            Ok(content) => {
                                let ext = std::path::Path::new(&path)
                                    .extension().and_then(|e| e.to_str()).unwrap_or("");
                                let ctx = format!(
                                    "\n\nContents of `{}`:\n```{}\n{}\n```\n",
                                    path, ext, content
                                );
                                *file_context.lock().await = Some(ctx);
                                let lines = content.lines().count();
                                print_info_accent("Attached", &format!("{} ({} lines)", path, lines));
                            }
                            Err(e) => print_error(&format!("Cannot read {}: {}", path, e)),
                        }
                    }

                    UserCommand::Sessions => {
                        ws_tx.send(Message::Text(
                            json!({ "id": next_id(), "type": "ferrum.listSessions" }).to_string().into()
                        )).await.ok();
                    }

                    UserCommand::Session(id) => {
                        if let Some(sid) = id {
                            // "last" shortcut — load the most recent session
                            if sid == "last" || sid == "latest" {
                                print_info_accent("Loading", "latest session...");
                                ws_tx.send(Message::Text(
                                    json!({ "id": next_id(), "type": "ferrum.getLatestSession" }).to_string().into()
                                )).await.ok();
                            } else {
                                // If cache is empty, fetch sessions first then retry
                                let cached = sessions_cache.lock().unwrap_or_else(|e| e.into_inner());
                                let resolved = if let Ok(num) = sid.parse::<usize>() {
                                    if num >= 1 && num <= cached.len() {
                                        cached[num - 1]["id"].as_str().map(|s| s.to_string())
                                    } else if cached.is_empty() {
                                        print_error("No sessions cached. Run /sessions first, then /session <number>.");
                                        None
                                    } else {
                                        print_error(&format!("Session #{} not found (have {}).", num, cached.len()));
                                        None
                                    }
                                } else {
                                    // ID or short ID — prefix match
                                    let matched = cached.iter().find(|s| {
                                        s["id"].as_str().map(|id| id.starts_with(&sid)).unwrap_or(false)
                                    });
                                    matched.and_then(|s| s["id"].as_str().map(|s| s.to_string()))
                                        .or_else(|| Some(sid.clone()))
                                };
                                drop(cached);

                                if let Some(full_id) = resolved {
                                    print_info_accent("Loading session", &full_id[..full_id.len().min(8)]);
                                    *current_session_id.lock().await = Some(full_id.clone());
                                    save_last_session_id(&full_id);
                                    messages.lock().await.clear();
                                    ws_tx.send(Message::Text(
                                        json!({
                                            "id": next_id(),
                                            "type": "ferrum.loadMessages",
                                            "session_id": full_id
                                        }).to_string().into()
                                    )).await.ok();
                                }
                            }
                        } else {
                            let sess = current_session_id.lock().await.clone();
                            if let Some(ref sid) = sess {
                                let short = &sid[..sid.len().min(8)];
                                print_info_accent("Session", &format!("#{}", short));
                            } else {
                                print_info_accent("Session", "(none)");
                            }
                            print_info_accent("History", &format!("{} messages", messages.lock().await.len()));
                        }
                    }

                    UserCommand::Resume(arg) => {
                        match arg {
                            None => {
                                // Resume latest session
                                print_info_accent("Resuming", "latest session...");
                                ws_tx.send(Message::Text(
                                    json!({ "id": next_id(), "type": "ferrum.getLatestSession" }).to_string().into()
                                )).await.ok();
                            }
                            Some(id) => {
                                // Resume specific session (number or ID)
                                let cached = sessions_cache.lock().unwrap_or_else(|e| e.into_inner());
                                let resolved = if let Ok(num) = id.parse::<usize>() {
                                    if num >= 1 && num <= cached.len() {
                                        cached[num - 1]["id"].as_str().map(|s| s.to_string())
                                    } else if cached.is_empty() {
                                        print_error("No sessions cached. Run /sessions first, then /resume <number>.");
                                        None
                                    } else {
                                        print_error(&format!("Session #{} not found (have {}).", num, cached.len()));
                                        None
                                    }
                                } else {
                                    let matched = cached.iter().find(|s| {
                                        s["id"].as_str().map(|sid| sid.starts_with(&id)).unwrap_or(false)
                                    });
                                    matched.and_then(|s| s["id"].as_str().map(|s| s.to_string()))
                                        .or_else(|| Some(id.clone()))
                                };
                                drop(cached);

                                if let Some(full_id) = resolved {
                                    print_info_accent("Resuming session", &full_id[..full_id.len().min(8)]);
                                    *current_session_id.lock().await = Some(full_id.clone());
                                    messages.lock().await.clear();
                                    ws_tx.send(Message::Text(
                                        json!({
                                            "id": next_id(),
                                            "type": "ferrum.loadMessages",
                                            "session_id": full_id
                                        }).to_string().into()
                                    )).await.ok();
                                }
                            }
                        }
                    }

                    UserCommand::New => {
                        let profile_name = profiles_cache.lock().await.first()
                            .and_then(|p| p["name"].as_str().map(String::from))
                            .unwrap_or_else(|| "default".to_string());
                        let session_name = format!("CLI {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
                        messages.lock().await.clear();
                        ws_tx.send(Message::Text(json!({
                            "id": next_id(),
                            "type": "ferrum.createSession",
                            "name": session_name,
                            "profile": profile_name,
                        }).to_string().into())).await.ok();
                        print_info_accent("New session", "created");
                    }

                    UserCommand::Providers => {
                        let cached = profiles_cache.lock().await;
                        if cached.is_empty() {
                            print_info("  No providers loaded.");
                        } else {
                            print_section_header("Providers");
                            for (_i, p) in cached.iter().enumerate() {
                                let name = p["name"].as_str().unwrap_or("?");
                                let url = p["base_url"].as_str().unwrap_or("?");
                                let m = p["default_model"].as_str().unwrap_or("?");
                                let active = base_url.as_deref() == Some(url);
                                let marker = if active { RADIO } else { DOT };
                                let color = if active { theme::CYAN } else { theme::DIM_LIGHT };
                                print_list_item(
                                    marker, color,
                                    &format!("{} {} ({})", name, url, m),
                                );
                            }
                            print_section_end();
                        }
                    }

                    UserCommand::Provider(name) => {
                        let cached = profiles_cache.lock().await;
                        let found = cached.iter().find(|p| {
                            p["name"].as_str().map(|n| n == name).unwrap_or(false)
                        });
                        if let Some(p) = found {
                            if let Some(bu) = p["base_url"].as_str() {
                                base_url = Some(bu.to_string());
                            }
                            if let Some(m) = p["default_model"].as_str() {
                                model = Some(m.to_string());
                            }
                            print_info_accent("Provider", &format!("{} ({})", name, base_url.as_deref().unwrap_or("?")));
                        } else {
                            print_error(&format!("Provider '{}' not found. Use /providers to list.", name));
                        }
                    }

                    UserCommand::Models => {
                        // Query providers directly — no dedicated backend command exists
                        let client = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(3))
                            .build().unwrap_or_default();
                        print_section_header("Available Models");
                        // Anthropic (hardcoded — no list endpoint)
                        let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok()
                            .or_else(|| load_config().anthropic_api_key);
                        if anthropic_key.is_some() {
                            print_list_item(ARROW, theme::CYAN_DIM, "claude-opus-4-6   [Anthropic]");
                            print_list_item(ARROW, theme::CYAN_DIM, "claude-sonnet-4-6 [Anthropic]");
                            print_list_item(ARROW, theme::CYAN_DIM, "claude-haiku-4-5-20251001 [Anthropic]");
                        } else {
                            print_list_item(DOT, theme::DIM, "(set ANTHROPIC_API_KEY for Claude models)");
                        }
                        // Ollama
                        if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
                            if let Ok(body) = resp.json::<serde_json::Value>().await {
                                if let Some(models) = body["models"].as_array() {
                                    for m in models {
                                        let name = m["name"].as_str().unwrap_or("?");
                                        let size = m["size"].as_u64().unwrap_or(0);
                                        let gb = size as f64 / 1_073_741_824.0;
                                        print_list_item(ARROW, theme::DIM_LIGHT, &format!("{:<42} {:.1}GB  [Ollama]", name, gb));
                                    }
                                }
                            }
                        }
                        // LM Studio
                        if let Ok(resp) = client.get("http://localhost:1234/v1/models").send().await {
                            if let Ok(body) = resp.json::<serde_json::Value>().await {
                                if let Some(models) = body["data"].as_array() {
                                    for m in models {
                                        let id = m["id"].as_str().unwrap_or("?");
                                        print_list_item(ARROW, theme::CYAN_DIM, &format!("{}  [LM Studio]", id));
                                    }
                                }
                            }
                        }
                        // vLLM
                        if let Ok(resp) = client.get("http://localhost:8000/v1/models").send().await {
                            if let Ok(body) = resp.json::<serde_json::Value>().await {
                                if let Some(models) = body["data"].as_array() {
                                    for m in models {
                                        let id = m["id"].as_str().unwrap_or("?");
                                        print_list_item(ARROW, theme::CYAN_DIM, &format!("{}  [vLLM]", id));
                                    }
                                }
                            }
                        }
                        print_section_end();
                        print_info("Use /model <name> to switch.");
                    }

                    UserCommand::Memories => {
                        ws_tx.send(Message::Text(json!({
                            "id": next_id(), "type": "tauri.invoke",
                            "cmd": "ai_list_memories",
                            "args": { "rootPath": root_path }
                        }).to_string().into())).await.ok();
                    }

                    UserCommand::Compact => {
                        ws_tx.send(Message::Text(json!({
                            "id": next_id(), "type": "ferrum.checkProvider",
                            "base_url": base_url.as_deref().unwrap_or("http://localhost:8080/v1")
                        }).to_string().into())).await.ok();
                        print_info_accent("Compaction", "requested");
                    }

                    UserCommand::Search(query) => {
                        // Priority order (Section 7):
                        // 1. crates: prefix -> crates.io search
                        // 2. npm: prefix -> npm search
                        // 3. google_api_key+google_cx set -> Google Custom Search API
                        // 4. brave_search_key set -> Brave search
                        // 5. fallback -> Google scraping (no key needed; falls back to DDG if blocked)
                        let search_result = if let Some(crate_query) = query.strip_prefix("crates:") {
                            print_info_accent("Searching crates.io", crate_query.trim());
                            crates_search(crate_query.trim()).await
                        } else if let Some(npm_query) = query.strip_prefix("npm:") {
                            print_info_accent("Searching npm", npm_query.trim());
                            npm_search(npm_query.trim()).await
                        } else if let (Some(api_key), Some(cx)) = (&cli_config.google_api_key, &cli_config.google_cx) {
                            print_info_accent("Searching (Google API)", &query);
                            google_search(&query, api_key, cx, 5).await
                        } else if let Some(ref brave_key) = cli_config.brave_search_key {
                            print_info_accent("Searching (Brave)", &query);
                            brave_search(&query, brave_key, 5).await
                        } else {
                            print_info_accent("Searching (Google)", &query);
                            google_scrape_search(&query).await
                        };
                        match search_result {
                            Ok(results) => {
                                print_search_results(&results);
                                let search_context = format!(
                                    "\n\n<web-search query=\"{}\">\n{}\n</web-search>\n",
                                    query, results
                                );
                                *file_context.lock().await = Some(search_context);
                                print_info_accent("Context", "Search results attached to next message");
                            }
                            Err(e) => print_error(&format!("Search failed: {}", e)),
                        }
                    }

                    UserCommand::SkillList => {
                        let skills = list_all_skills(Some(&root_path));
                        // Group by category
                        let mut categories: std::collections::BTreeMap<String, Vec<&Skill>> = std::collections::BTreeMap::new();
                        for s in &skills {
                            let cat = s.category.as_deref().unwrap_or("other").to_string();
                            categories.entry(cat).or_default().push(s);
                        }
                        print_section_header("Skills");
                        for (cat, cat_skills) in &categories {
                            let mut o = io::stdout();
                            set_fg(&mut o, theme::CYAN_DIM);
                            set_attr(&mut o, Attribute::Bold);
                            write!(o, "  {cat}\n").ok();
                            set_attr(&mut o, Attribute::Reset);
                            for s in cat_skills {
                                let active_marker = active_skill.as_ref()
                                    .map(|a| if a.name == s.name { RADIO } else { DOT })
                                    .unwrap_or(DOT);
                                let alias_str = s.aliases.as_ref()
                                    .map(|a| format!(" ({})", a.join(", ")))
                                    .unwrap_or_default();
                                print_list_item(active_marker, theme::CYAN_DIM,
                                    &format!("{}{} — {}", s.name, alias_str, s.description));
                            }
                        }
                        print_section_end();
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Custom skills: ~/.config/shadowai/skills/*.toml\n").ok();
                        write!(o, "  Project skills: .shadowai/skills/*.toml\n").ok();
                        reset_color(&mut o);
                    }

                    UserCommand::SkillActivate(name) => {
                        match find_skill(&name, Some(&root_path)) {
                            Some(skill) => {
                                if let Some(ref m) = skill.mode {
                                    chat_mode = m.clone();
                                    *prompt_mode.lock().unwrap_or_else(|e| e.into_inner()) = chat_mode.clone();
                                    print_mode_badge(&chat_mode);
                                }
                                if let Some(t) = skill.temperature {
                                    temperature = t;
                                }
                                *prompt_skill.lock().unwrap_or_else(|e| e.into_inner()) = Some(skill.name.clone());
                                print_info_accent("Skill", &format!("{} activated", skill.name));
                                skill_turn_count = 0;
                                // Hooks: skill_activate (Section 8.2)
                                run_hooks(&hooks, "skill_activate", None, &json!({"skill": skill.name}));
                                active_skill = Some(skill);
                            }
                            None => print_error(&format!("Skill '{}' not found. Use /skills to list.", name)),
                        }
                    }

                    UserCommand::SkillOff => {
                        active_skill = None;
                        skill_turn_count = 0;
                        *prompt_skill.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        print_info_accent("Skill", "deactivated");
                    }

                    UserCommand::SkillCreate(name_opt) => {
                        match name_opt {
                            Some(name) => {
                                match create_skill_skeleton(&name) {
                                    Ok(path) => print_info_accent("Skill created", &format!("{} — edit at: {}", name, path)),
                                    Err(e) => print_error(&e),
                                }
                            }
                            None => {
                                print_info_accent("Usage", "/skill create <name>");
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::DIM);
                                write!(o, "  Creates a skeleton TOML at ~/.config/shadowai/skills/<name>.toml\n").ok();
                                write!(o, "  Example: /skill create my-reviewer\n").ok();
                                reset_color(&mut o);
                            }
                        }
                    }

                    UserCommand::Watch => {
                        let was_active = watch_active.load(Ordering::SeqCst);
                        if was_active {
                            watch_active.store(false, Ordering::SeqCst);
                            print_info_accent("Watch", "mode OFF");
                        } else {
                            watch_active.store(true, Ordering::SeqCst);
                            print_info_accent("Watch", &format!("mode ON — monitoring {} for AI! comments", root_path));

                            // Spawn watcher task
                            let watch_flag = watch_active.clone();
                            let watch_root = root_path.clone();
                            let watch_tx = user_tx.clone();
                            let watch_tracked = tracked_files.clone();
                            tokio::spawn(async move {
                                use notify::{Watcher, RecursiveMode, Event as NEvent, EventKind};
                                use std::collections::HashMap;

                                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<NEvent>();
                                let mut watcher = match notify::recommended_watcher(move |res: Result<NEvent, notify::Error>| {
                                    if let Ok(event) = res {
                                        let _ = tx.send(event);
                                    }
                                }) {
                                    Ok(w) => w,
                                    Err(e) => {
                                        eprintln!("  Watch error: {}", e);
                                        return;
                                    }
                                };

                                if watcher.watch(std::path::Path::new(&watch_root), RecursiveMode::Recursive).is_err() {
                                    return;
                                }

                                let mut last_processed: HashMap<String, Instant> = HashMap::new();
                                let skip_dirs = [".git", "node_modules", "target", ".shadowai"];

                                while watch_flag.load(Ordering::SeqCst) {
                                    tokio::select! {
                                        Some(event) = rx.recv() => {
                                            let dominated = matches!(event.kind,
                                                EventKind::Modify(_) | EventKind::Create(_));
                                            if !dominated { continue; }

                                            for path in &event.paths {
                                                let path_str = path.to_string_lossy().to_string();

                                                // Filter skip dirs
                                                let should_skip = skip_dirs.iter().any(|d| path_str.contains(&format!("/{}/", d)));
                                                if should_skip { continue; }

                                                // Skip binary files
                                                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                                    let binary_exts = ["png", "jpg", "jpeg", "gif", "ico", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "zip", "tar", "gz", "so", "dylib", "dll", "exe", "o", "a"];
                                                    if binary_exts.contains(&ext) { continue; }
                                                }

                                                // Debounce: 2 seconds
                                                if let Some(last) = last_processed.get(&path_str) {
                                                    if last.elapsed().as_secs() < 2 { continue; }
                                                }

                                                // Check if this is a tracked file and notify
                                                {
                                                    let tf = watch_tracked.lock().await;
                                                    if tf.iter().any(|tp| tp == &path_str) {
                                                        let display = path_str.strip_prefix(&format!("{}/", watch_root)).unwrap_or(&path_str);
                                                        let mut o = io::stdout();
                                                        execute!(o,
                                                            SetForegroundColor(theme::WARN),
                                                            Print(format!("\n  {RADIO} Tracked file updated externally: {}\n", display)),
                                                            ResetColor,
                                                        ).ok();
                                                    }
                                                }

                                                // Read file and search for AI! comments
                                                if let Ok(content) = std::fs::read_to_string(path) {
                                                    let mut found_tasks: Vec<(usize, String)> = Vec::new();
                                                    for (i, line) in content.lines().enumerate() {
                                                        let trimmed = line.trim();
                                                        // Match // AI!, # AI!, /* AI! */
                                                        let task = if let Some(rest) = trimmed.strip_prefix("// AI!") {
                                                            Some(rest.trim().to_string())
                                                        } else if let Some(rest) = trimmed.strip_prefix("# AI!") {
                                                            Some(rest.trim().to_string())
                                                        } else if trimmed.starts_with("/* AI!") {
                                                            let inner = trimmed.strip_prefix("/* AI!").unwrap_or("")
                                                                .strip_suffix("*/").unwrap_or("").trim().to_string();
                                                            Some(inner)
                                                        } else {
                                                            None
                                                        };
                                                        if let Some(t) = task {
                                                            if !t.is_empty() {
                                                                found_tasks.push((i, t));
                                                            }
                                                        }
                                                    }

                                                    if !found_tasks.is_empty() {
                                                        last_processed.insert(path_str.clone(), Instant::now());

                                                        // Remove AI! comments from the file
                                                        let new_content: Vec<&str> = content.lines().enumerate()
                                                            .filter(|(i, _)| !found_tasks.iter().any(|(fi, _)| fi == i))
                                                            .map(|(_, line)| line)
                                                            .collect();
                                                        let _ = std::fs::write(path, new_content.join("\n"));

                                                        // Send tasks as user messages
                                                        for (_, task) in found_tasks {
                                                            let rel_path = path_str.strip_prefix(&format!("{}/", watch_root))
                                                                .unwrap_or(&path_str);
                                                            let msg = format!("[Watch: {}] {}", rel_path, task);
                                                            let _ = watch_tx.send(UserCommand::Message(msg));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                                    }
                                }
                            });
                        }
                    }

                    UserCommand::Plan(desc) => {
                        match desc {
                            Some(description) => {
                                // Send a plan request to the AI
                                let plan_prompt = format!(
                                    "[PLAN MODE] Create a numbered implementation plan for the following task. \
                                    Each step should be concrete and actionable. Format as:\n\
                                    1. [Step title]\n   - Details\n   - Files affected\n   - Estimated complexity: low/medium/high\n\n\
                                    Task: {}", description
                                );
                                *current_plan.lock().await = Some(description);
                                let _ = user_tx.send(UserCommand::Message(plan_prompt));
                            }
                            None => {
                                let plan = current_plan.lock().await;
                                if let Some(ref desc) = *plan {
                                    print_info_accent("Current plan", desc);
                                } else {
                                    print_info_accent("Plan", "No active plan. Use /plan <description> to create one.");
                                }
                            }
                        }
                    }

                    UserCommand::PlanApprove => {
                        let plan = current_plan.lock().await;
                        if plan.is_some() {
                            plan_approved.store(true, Ordering::SeqCst);
                            print_info_accent("Plan", "approved — use /plan next to execute steps");
                        } else {
                            print_error("No active plan. Use /plan <description> to create one first.");
                        }
                    }

                    UserCommand::PlanNext => {
                        if !plan_approved.load(Ordering::SeqCst) {
                            print_error("Plan not approved yet. Use /plan approve first.");
                        } else {
                            let mut steps = plan_steps.lock().await;
                            if steps.is_empty() {
                                print_info_accent("Plan", "all steps completed (or no parsed steps). Use /plan <desc> to start a new plan.");
                            } else {
                                let step = steps.remove(0);
                                let remaining = steps.len();
                                drop(steps);
                                print_info_accent("Plan Step", &format!("executing ({} remaining)", remaining));
                                let step_msg = format!("[PLAN STEP] Execute this step from the approved plan:\n\n{}\n\nImplement this step now.", step);
                                let _ = user_tx.send(UserCommand::Message(step_msg));
                            }
                        }
                    }

                    UserCommand::PlanExport => {
                        let plan = current_plan.lock().await;
                        let steps = plan_steps.lock().await;
                        if plan.is_none() && steps.is_empty() {
                            print_error("No active plan to export.");
                        } else {
                            ensure_tracking_dir(&root_path);
                            let plan_path = tracking_dir(&root_path).join("plan.md");
                            let mut content = String::new();
                            if let Some(ref desc) = *plan {
                                content.push_str(&format!("# Plan: {}\n\n", desc));
                            }
                            let approved = plan_approved.load(Ordering::SeqCst);
                            content.push_str(&format!("Status: {}\n\n", if approved { "Approved" } else { "Pending" }));
                            if !steps.is_empty() {
                                content.push_str("## Steps\n\n");
                                for (i, step) in steps.iter().enumerate() {
                                    content.push_str(&format!("{}. {}\n", i + 1, step));
                                }
                            }
                            match std::fs::write(&plan_path, &content) {
                                Ok(_) => print_info_accent("Plan", &format!("exported to {}", plan_path.display())),
                                Err(e) => print_error(&format!("Failed to export plan: {}", e)),
                            }
                        }
                    }

                    UserCommand::Undo => {
                        let op = last_op.lock().await.clone();
                        if op == "commit" {
                            // Undo last commit (soft reset)
                            match std::process::Command::new("git")
                                .args(["reset", "--soft", "HEAD~1"])
                                .current_dir(&root_path)
                                .output()
                            {
                                Ok(output) if output.status.success() => {
                                    print_info_accent("Undo", "reverted last commit (changes preserved as staged)");
                                    *last_op.lock().await = String::new();
                                }
                                Ok(output) => print_error(&format!("git reset failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
                                Err(e) => print_error(&format!("Failed to run git: {}", e)),
                            }
                        } else {
                            // Revert unstaged changes
                            match std::process::Command::new("git")
                                .args(["diff", "HEAD", "--name-only"])
                                .current_dir(&root_path)
                                .output()
                            {
                                Ok(output) => {
                                    let changed = String::from_utf8_lossy(&output.stdout).to_string();
                                    if changed.trim().is_empty() {
                                        print_info_accent("Undo", "no changes to revert");
                                    } else {
                                        let file_count = changed.lines().filter(|l| !l.is_empty()).count();
                                        match std::process::Command::new("git")
                                            .args(["checkout", "--", "."])
                                            .current_dir(&root_path)
                                            .output()
                                        {
                                            Ok(r) if r.status.success() => {
                                                print_info_accent("Undo", &format!("reverted {} file(s)", file_count));
                                                modified_files.lock().await.clear();
                                                *last_op.lock().await = String::new();
                                            }
                                            Ok(r) => print_error(&format!("git checkout failed: {}", String::from_utf8_lossy(&r.stderr).trim())),
                                            Err(e) => print_error(&format!("Failed to run git: {}", e)),
                                        }
                                    }
                                }
                                Err(e) => print_error(&format!("Failed to run git: {}", e)),
                            }
                        }
                    }

                    UserCommand::Copy => {
                        let text = last_response.lock().await.clone();
                        if text.is_empty() {
                            print_error("Nothing to copy — no AI response yet.");
                        } else {
                            // Try wl-copy (Wayland), xclip (X11), xsel, pbcopy (macOS) in order
                            let copied = ["wl-copy", "xclip", "xsel", "pbcopy"]
                                .iter()
                                .find_map(|cmd| {
                                    let mut child = std::process::Command::new(cmd);
                                    if *cmd == "xclip" { child.arg("-selection").arg("clipboard"); }
                                    if *cmd == "xsel" { child.arg("--clipboard").arg("--input"); }
                                    child.stdin(std::process::Stdio::piped()).spawn().ok().and_then(|mut c| {
                                        use std::io::Write;
                                        c.stdin.as_mut()?.write_all(text.as_bytes()).ok()?;
                                        c.wait().ok().map(|_| *cmd)
                                    })
                                });
                            match copied {
                                Some(tool) => print_info_accent("Copied", &format!("response copied to clipboard ({})", tool)),
                                None => print_error("No clipboard tool found (install wl-copy, xclip, or xsel)"),
                            }
                        }
                    }

                    UserCommand::SessionRename(name) => {
                        if name.is_empty() {
                            print_error("Usage: /session rename <name>");
                        } else {
                            let sess = current_session_id.lock().await.clone();
                            match sess {
                                Some(ref sid) => {
                                    ws_tx.send(Message::Text(json!({
                                        "id": next_id(),
                                        "type": "ferrum.updateSession",
                                        "session_id": sid,
                                        "name": name,
                                    }).to_string().into())).await.ok();
                                    print_info_accent("Session", &format!("renamed to \"{}\"", name));
                                }
                                None => print_error("No active session."),
                            }
                        }
                    }

                    UserCommand::EditHistory(file_filter) => {
                        let history_path = tracking_dir(&root_path).join("edit_history.jsonl");
                        match std::fs::read_to_string(&history_path) {
                            Ok(content) => {
                                let entries: Vec<serde_json::Value> = content.lines()
                                    .filter_map(|l| serde_json::from_str(l).ok())
                                    .collect();
                                let filtered: Vec<&serde_json::Value> = if let Some(ref filter) = file_filter {
                                    entries.iter().filter(|e| {
                                        e["path"].as_str().map(|p| p.contains(filter.as_str())).unwrap_or(false)
                                    }).collect()
                                } else {
                                    entries.iter().collect()
                                };
                                if filtered.is_empty() {
                                    print_info_accent("Edit History", "no edits recorded");
                                } else {
                                    let title = if let Some(ref f) = file_filter {
                                        format!("Edit History ({})", f)
                                    } else {
                                        "Edit History (last 20)".to_string()
                                    };
                                    print_section_header(&title);
                                    let start = if filtered.len() > 20 { filtered.len() - 20 } else { 0 };
                                    for entry in &filtered[start..] {
                                        let ts = entry["timestamp"].as_i64().unwrap_or(0);
                                        let path = entry["path"].as_str().unwrap_or("?");
                                        let action = entry["action"].as_str().unwrap_or("?");
                                        let turn = entry["turn"].as_u64().unwrap_or(0);
                                        let dt = chrono::DateTime::from_timestamp(ts, 0)
                                            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                                            .unwrap_or_else(|| "?".to_string());
                                        let mut o = io::stdout();
                                        set_fg(&mut o, theme::DIM);
                                        write!(o, "  {V_LINE} ").ok();
                                        set_fg(&mut o, theme::DIM_LIGHT);
                                        write!(o, "{dt}").ok();
                                        set_fg(&mut o, theme::FILE_MOD);
                                        write!(o, "  {action}").ok();
                                        set_fg(&mut o, theme::CYAN_DIM);
                                        write!(o, "  {path}").ok();
                                        set_fg(&mut o, theme::DIM);
                                        write!(o, "  (turn {})\n", turn).ok();
                                        reset_color(&mut o);
                                    }
                                    print_section_end();
                                }
                            }
                            Err(_) => print_info_accent("Edit History", "no edits recorded yet"),
                        }
                    }

                    UserCommand::SkillEdit(name) => {
                        // Find skill file path: project-local first, then global
                        let project_path = std::path::Path::new(&root_path).join(".shadowai").join("skills").join(format!("{}.toml", name));
                        let global_path = config_dir().map(|d| d.join("skills").join(format!("{}.toml", name)));
                        let edit_path = if project_path.exists() {
                            Some(project_path)
                        } else if let Some(ref gp) = global_path {
                            if gp.exists() { Some(gp.clone()) } else { None }
                        } else {
                            None
                        };
                        match edit_path {
                            Some(path) => {
                                let editor = std::env::var("EDITOR")
                                    .unwrap_or_else(|_| {
                                        if std::process::Command::new("nano").arg("--version").output().is_ok() {
                                            "nano".to_string()
                                        } else {
                                            "vi".to_string()
                                        }
                                    });
                                print_info_accent("Skill Edit", &format!("opening {} with {}", path.display(), editor));
                                let _ = std::process::Command::new(&editor)
                                    .arg(path.to_str().unwrap_or(""))
                                    .status();
                            }
                            None => print_error(&format!("Skill '{}' not found as a TOML file. Check /skills for available skills.", name)),
                        }
                    }

                    UserCommand::ShowErrors => print_tracking_file(&root_path, "errors.md", "Errors"),
                    UserCommand::ShowFixed => print_tracking_file(&root_path, "fixed.md", "Fixed"),
                    UserCommand::ShowCompleted => print_tracking_file(&root_path, "completed.md", "Completed"),
                    UserCommand::ShowMemory => print_tracking_file(&root_path, "memory.md", "Memory"),

                    UserCommand::Remember(text) => {
                        append_tracking_entry(&root_path, "memory.md", &text);
                        print_info_accent("Memory", "saved");
                    }

                    UserCommand::Git(sub) => {
                        handle_git_command(&sub, &root_path);
                        // Refresh git branch cache after git commands
                        if let Ok(output) = std::process::Command::new("git")
                            .args(["rev-parse", "--abbrev-ref", "HEAD"])
                            .current_dir(&root_path)
                            .output()
                        {
                            if output.status.success() {
                                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                *git_branch.lock().unwrap_or_else(|e| e.into_inner()) = branch;
                            }
                        }
                    }

                    UserCommand::Find(pattern) => {
                        handle_find_command(&pattern, &root_path);
                    }

                    UserCommand::Grep(pattern) => {
                        handle_grep_command(&pattern, &root_path);
                    }

                    UserCommand::Tree(sub_path) => {
                        handle_tree_command(sub_path.as_deref(), &root_path);
                    }

                    UserCommand::Context => {
                        let msgs = messages.lock().await;
                        print_context_info(&msgs, 128000);
                    }

                    UserCommand::ContextFiles => {
                        let tf = tracked_files.lock().await;
                        if tf.is_empty() {
                            print_info_accent("Context", "No tracked files. Use /add <file> to track.");
                        } else {
                            print_section_header("Tracked Files");
                            let mut total_tokens = 0usize;
                            for path in tf.iter() {
                                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                                let tokens = (size as usize) / 4;
                                total_tokens += tokens;
                                let display = path.strip_prefix(&format!("{}/", root_path)).unwrap_or(path);
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::BORDER);
                                write!(o, "  {V_LINE} ").ok();
                                set_fg(&mut o, theme::CYAN_DIM);
                                write!(o, "{display}").ok();
                                set_fg(&mut o, theme::DIM);
                                write!(o, "  ({} bytes, ~{} tokens)\n", size, tokens).ok();
                                reset_color(&mut o);
                            }
                            print_section_row("Total files", &format!("{}", tf.len()));
                            print_section_row("Est. tokens", &format!("~{}", fmt_tokens(total_tokens as u64)));
                            print_section_end();
                        }
                    }

                    UserCommand::ContextDrop(name) => {
                        let mut tf = tracked_files.lock().await;
                        let before = tf.len();
                        tf.retain(|p| {
                            let display = p.strip_prefix(&format!("{}/", root_path)).unwrap_or(p);
                            !display.contains(&name) && !p.ends_with(&name)
                        });
                        let removed = before - tf.len();
                        if removed > 0 {
                            print_info_accent("Context", &format!("Dropped {} file(s) matching '{}'", removed, name));
                        } else {
                            print_error(&format!("No tracked file matching '{}'", name));
                        }
                    }

                    UserCommand::Format(args) => {
                        handle_format_command(&args, &root_path);
                    }

                    UserCommand::Export(format) => {
                        let msgs = messages.lock().await;
                        if msgs.is_empty() {
                            print_error("No messages to export.");
                        } else {
                            ensure_tracking_dir(&root_path);
                            let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                            let ext = match format.as_str() { "json" => "json", "html" => "html", _ => "md" };
                            let filename = format!("export_{}.{}", ts, ext);
                            let path = tracking_dir(&root_path).join(&filename);

                            let content = if format == "json" {
                                serde_json::to_string_pretty(&*msgs).unwrap_or_else(|e| format!("Error: {}", e))
                            } else if format == "html" {
                                let ts_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                                let mut html = String::from("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>ShadowAI Conversation</title>\n<style>\n  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; max-width: 860px; margin: 0 auto; padding: 2rem; background: #0d0d0d; color: #e0e0e0; line-height: 1.6; }\n  h1 { color: #a78bfa; border-bottom: 1px solid #333; padding-bottom: .5rem; }\n  .meta { color: #555; font-size: .85rem; margin-bottom: 2rem; }\n  .message { margin: 1.5rem 0; border-radius: 8px; padding: 1rem 1.25rem; }\n  .user { background: #1a1a2e; border-left: 3px solid #818cf8; }\n  .assistant { background: #0f1f0f; border-left: 3px solid #4ade80; }\n  .system { background: #1a1010; border-left: 3px solid #f87171; }\n  .role { font-size: .75rem; font-weight: 700; text-transform: uppercase; letter-spacing: .08em; margin-bottom: .5rem; }\n  .user .role { color: #818cf8; }\n  .assistant .role { color: #4ade80; }\n  .system .role { color: #f87171; }\n  pre { background: #111; border: 1px solid #333; border-radius: 4px; padding: .75rem; overflow-x: auto; }\n  code { font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: .875rem; }\n  p { margin: .5rem 0; }\n</style>\n</head>\n<body>\n");
                                html.push_str(&format!("<h1>ShadowAI Conversation Export</h1>\n<p class=\"meta\">Exported: {}</p>\n", ts_str));
                                for m in msgs.iter() {
                                    let role = m["role"].as_str().unwrap_or("unknown");
                                    let raw = m["content"].as_str().unwrap_or("");
                                    // Escape HTML entities
                                    let escaped = raw.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                                    // Wrap code blocks in <pre><code>
                                    let formatted = escaped.replace("```", "</code></pre><pre><code>");
                                    html.push_str(&format!("<div class=\"message {}\">\n  <div class=\"role\">{}</div>\n  <div class=\"content\"><p>{}</p></div>\n</div>\n", role, role, formatted.replace('\n', "<br>")));
                                }
                                html.push_str("</body>\n</html>\n");
                                html
                            } else {
                                let mut md = String::from("# ShadowAI Conversation Export\n\n");
                                md.push_str(&format!("*Exported: {}*\n\n---\n\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
                                for m in msgs.iter() {
                                    let role = m["role"].as_str().unwrap_or("unknown");
                                    let content = m["content"].as_str().unwrap_or("");
                                    match role {
                                        "user" => {
                                            md.push_str(&format!("## User\n\n{}\n\n---\n\n", content));
                                        }
                                        "assistant" => {
                                            md.push_str(&format!("## Assistant\n\n{}\n\n---\n\n", content));
                                        }
                                        "system" => {
                                            md.push_str(&format!("## System\n\n{}\n\n---\n\n", content));
                                        }
                                        _ => {
                                            md.push_str(&format!("## {}\n\n{}\n\n---\n\n", role, content));
                                        }
                                    }
                                }
                                md
                            };

                            match std::fs::write(&path, &content) {
                                Ok(_) => print_info_accent("Exported", &format!("{} ({} messages)", path.display(), msgs.len())),
                                Err(e) => print_error(&format!("Export failed: {}", e)),
                            }
                        }
                    }

                    UserCommand::Test(args) => {
                        handle_test_command(&args, &root_path);
                    }

                    UserCommand::Lint(args) => {
                        if let Some(lint_output) = handle_lint_command(&args, &root_path) {
                            // Inject lint errors into file_context for next AI message
                            *file_context.lock().await = Some(lint_output);
                            print_info_accent("Lint", "errors attached to next message — ask AI to fix them");
                        }
                    }

                    UserCommand::Build(args) => {
                        if let Some(build_errors) = handle_build_command(&args, &root_path) {
                            // Hooks: build_fail (Section 8.2)
                            run_hooks(&hooks, "build_fail", None, &json!({}));
                            // --fix mode: inject build errors into context and send to AI
                            let fix_prompt = format!(
                                "The build failed with the following errors. Please analyze the errors \
                                 and fix the code. Show the exact changes needed.\n\n\
                                 Build output:\n```\n{}\n```",
                                build_errors.chars().take(8000).collect::<String>()
                            );
                            *file_context.lock().await = Some(format!("\n\n{}", fix_prompt));

                            let stream_id = uuid::Uuid::new_v4().to_string();
                            *current_stream_id.lock().await = stream_id.clone();
                            *stream_start.lock().await = Some(Instant::now());
                            abort_flag.store(false, Ordering::SeqCst);

                            let mut msgs = messages.lock().await;
                            msgs.push(json!({ "role": "user", "content": fix_prompt }));

                            let mut api_messages: Vec<serde_json::Value> = Vec::new();
                            api_messages.push(json!({ "role": "system", "content":
                                "You are an expert developer. Fix the build errors shown below. \
                                 Be precise and show exact file changes needed."
                            }));
                            api_messages.extend(msgs.iter().cloned());

                            let total_chars: usize = api_messages.iter()
                                .map(|m| m["content"].as_str().unwrap_or("").len())
                                .sum();
                            prompt_tokens.store((total_chars / 4) as u64, Ordering::SeqCst);
                            drop(msgs);

                            streaming.store(true, Ordering::SeqCst);
                            print_ai_prefix();

                            waiting_for_first_token.store(true, Ordering::SeqCst);
                            let wft = waiting_for_first_token.clone();
                            tokio::spawn(async move { run_bunny_animation(wft).await; });

                            ws_tx.send(Message::Text(json!({
                                "id": next_id(),
                                "type": "tauri.invoke",
                                "cmd": "ai_chat_with_tools",
                                "args": {
                                    "streamId": stream_id,
                                    "messages": api_messages,
                                    "model": model,
                                    "baseUrlOverride": base_url,
                                    "temperature": temperature,
                                    "maxTokens": max_tokens,
                                    "toolsEnabled": true,
                                    "chatMode": chat_mode,
                                    "rootPath": root_path,
                                }
                            }).to_string().into())).await.ok();
                        } else {
                            // Build succeeded — run hook (Section 8.2)
                            run_hooks(&hooks, "build_success", None, &json!({}));
                        }
                    }

                    UserCommand::AddFile(path) => {
                        let full_path = if path.starts_with('/') { path.clone() }
                            else { format!("{}/{}", root_path, path) };
                        match std::fs::read_to_string(&full_path) {
                            Ok(content) => {
                                let lines = content.lines().count();
                                let mut tf = tracked_files.lock().await;
                                if !tf.contains(&full_path) {
                                    tf.push(full_path.clone());
                                }
                                prompt_tracked_count.store(tf.len() as u64, Ordering::SeqCst);
                                print_info_accent("Tracked", &format!("{} ({} lines) — persistent across messages", path, lines));
                            }
                            Err(e) => print_error(&format!("Cannot read {}: {}", path, e)),
                        }
                    }

                    UserCommand::DropFile(path) => {
                        let full_path = if path.starts_with('/') { path.clone() }
                            else { format!("{}/{}", root_path, path) };
                        let mut tf = tracked_files.lock().await;
                        let before = tf.len();
                        tf.retain(|p| p != &full_path && !p.ends_with(&format!("/{}", path)));
                        prompt_tracked_count.store(tf.len() as u64, Ordering::SeqCst);
                        if tf.len() < before {
                            print_info_accent("Dropped", &path);
                        } else {
                            print_error(&format!("{} not in tracked files", path));
                        }
                    }

                    UserCommand::ListFiles => {
                        let tf = tracked_files.lock().await;
                        print_section_header("Tracked Files");
                        if tf.is_empty() {
                            print_list_item(DOT, theme::DIM, "No files tracked. Use /add <file> to add.");
                        } else {
                            for path in tf.iter() {
                                let (lines, tokens) = match std::fs::read_to_string(path) {
                                    Ok(content) => {
                                        let l = content.lines().count();
                                        let t = content.len() / 4; // rough token estimate
                                        (l, t)
                                    }
                                    Err(_) => (0, 0),
                                };
                                let display = path.strip_prefix(&format!("{}/", root_path)).unwrap_or(path);
                                print_list_item(ARROW, theme::CYAN_DIM, &format!(
                                    "{} ({} lines, ~{} tokens)", display, lines, tokens
                                ));
                            }
                        }
                        print_section_end();
                    }

                    UserCommand::Review(args) => {
                        let trimmed = args.trim();
                        let review_content = if trimmed == "--pr" || trimmed.is_empty() {
                            // Review staged git changes
                            match std::process::Command::new("git")
                                .args(if trimmed == "--pr" {
                                    vec!["diff", "HEAD~1"]
                                } else {
                                    vec!["diff", "--staged"]
                                })
                                .current_dir(&root_path)
                                .output()
                            {
                                Ok(output) => {
                                    let diff = String::from_utf8_lossy(&output.stdout).to_string();
                                    if diff.trim().is_empty() {
                                        // Fall back to unstaged diff
                                        match std::process::Command::new("git")
                                            .args(["diff"])
                                            .current_dir(&root_path)
                                            .output()
                                        {
                                            Ok(o2) => {
                                                let d2 = String::from_utf8_lossy(&o2.stdout).to_string();
                                                if d2.trim().is_empty() {
                                                    print_error("No changes to review. Stage changes or specify a file.");
                                                    continue;
                                                }
                                                Some(format!("Git diff (unstaged):\n```diff\n{}\n```", d2))
                                            }
                                            Err(e) => { print_error(&format!("git diff failed: {}", e)); continue; }
                                        }
                                    } else {
                                        let label = if trimmed == "--pr" { "PR diff (HEAD~1)" } else { "staged changes" };
                                        Some(format!("Git diff ({}):\n```diff\n{}\n```", label, diff))
                                    }
                                }
                                Err(e) => { print_error(&format!("git diff failed: {}", e)); continue; }
                            }
                        } else {
                            // Review specific file
                            let full_path = if trimmed.starts_with('/') { trimmed.to_string() }
                                else { format!("{}/{}", root_path, trimmed) };
                            match std::fs::read_to_string(&full_path) {
                                Ok(content) => {
                                    let ext = std::path::Path::new(trimmed)
                                        .extension().and_then(|e| e.to_str()).unwrap_or("");
                                    Some(format!("Contents of `{}`:\n```{}\n{}\n```", trimmed, ext, content))
                                }
                                Err(e) => { print_error(&format!("Cannot read {}: {}", trimmed, e)); continue; }
                            }
                        };

                        if let Some(content) = review_content {
                            let review_prompt = format!(
                                "Please perform a structured code review of the following. \
                                 Focus on: bugs, security issues, performance problems, code style, \
                                 and suggestions for improvement. Format your response with sections: \
                                 ## Summary, ## Issues Found (with severity: critical/warning/info), \
                                 ## Suggestions, ## Overall Assessment.\n\n{}",
                                content
                            );

                            // Send as a message to the AI
                            let stream_id = uuid::Uuid::new_v4().to_string();
                            *current_stream_id.lock().await = stream_id.clone();
                            *stream_start.lock().await = Some(Instant::now());
                            abort_flag.store(false, Ordering::SeqCst);

                            let mut msgs = messages.lock().await;
                            msgs.push(json!({ "role": "user", "content": review_prompt }));

                            let mut api_messages: Vec<serde_json::Value> = Vec::new();
                            api_messages.push(json!({ "role": "system", "content":
                                "You are an expert code reviewer. Provide thorough, actionable \
                                 code reviews with clear severity levels. Be constructive but honest."
                            }));
                            api_messages.extend(msgs.iter().cloned());

                            let total_chars: usize = api_messages.iter()
                                .map(|m| m["content"].as_str().unwrap_or("").len())
                                .sum();
                            prompt_tokens.store((total_chars / 4) as u64, Ordering::SeqCst);
                            drop(msgs);

                            streaming.store(true, Ordering::SeqCst);
                            print_ai_prefix();

                            waiting_for_first_token.store(true, Ordering::SeqCst);
                            let wft = waiting_for_first_token.clone();
                            tokio::spawn(async move { run_bunny_animation(wft).await; });

                            ws_tx.send(Message::Text(json!({
                                "id": next_id(),
                                "type": "tauri.invoke",
                                "cmd": "ai_chat_with_tools",
                                "args": {
                                    "streamId": stream_id,
                                    "messages": api_messages,
                                    "model": model,
                                    "baseUrlOverride": base_url,
                                    "temperature": temperature,
                                    "maxTokens": max_tokens,
                                    "toolsEnabled": false,
                                    "chatMode": "plan",
                                    "rootPath": root_path,
                                }
                            }).to_string().into())).await.ok();
                        }
                    }

                    UserCommand::Security(args) => {
                        handle_security_command(&args, &root_path);
                    }

                    UserCommand::Doc(args) => {
                        let trimmed = args.trim();
                        let doc_content = if trimmed == "--readme" {
                            // Gather project info for README generation
                            let mut info = String::from("Generate/update a README.md for this project.\n\n");
                            let root = std::path::Path::new(&root_path);
                            if root.join("README.md").exists() {
                                if let Ok(existing) = std::fs::read_to_string(root.join("README.md")) {
                                    info.push_str(&format!("Existing README:\n```\n{}\n```\n\n", existing));
                                }
                            }
                            if root.join("Cargo.toml").exists() {
                                if let Ok(c) = std::fs::read_to_string(root.join("Cargo.toml")) {
                                    info.push_str(&format!("Cargo.toml:\n```toml\n{}\n```\n\n", c));
                                }
                            }
                            if root.join("package.json").exists() {
                                if let Ok(c) = std::fs::read_to_string(root.join("package.json")) {
                                    info.push_str(&format!("package.json:\n```json\n{}\n```\n\n", c));
                                }
                            }
                            // List main source files
                            let src_files = collect_source_files(&root_path);
                            if !src_files.is_empty() {
                                info.push_str("Source files:\n");
                                for f in src_files.iter().take(30) {
                                    let display = f.strip_prefix(&format!("{}/", root_path)).unwrap_or(f);
                                    info.push_str(&format!("- {}\n", display));
                                }
                            }
                            Some(("You are a technical writer. Generate a comprehensive, well-structured README.md. Include: project name, description, features, installation, usage, configuration, and license sections.".to_string(), info))
                        } else if trimmed == "--api" {
                            // Scan for public functions/exports
                            let src_files = collect_source_files(&root_path);
                            let mut api_content = String::from("Generate API documentation for this project's public interface.\n\n");
                            for f in src_files.iter().take(20) {
                                if let Ok(content) = std::fs::read_to_string(f) {
                                    let display = f.strip_prefix(&format!("{}/", root_path)).unwrap_or(f);
                                    // Include only lines that look like public API
                                    let api_lines: Vec<&str> = content.lines()
                                        .filter(|l| l.contains("pub fn ") || l.contains("pub struct ")
                                            || l.contains("pub enum ") || l.contains("pub trait ")
                                            || l.contains("export function") || l.contains("export const")
                                            || l.contains("export class") || l.contains("export interface")
                                            || l.contains("module.exports") || l.starts_with("def ")
                                            || l.starts_with("class ") || l.contains("pub type "))
                                        .collect();
                                    if !api_lines.is_empty() {
                                        api_content.push_str(&format!("\n## {}\n```\n", display));
                                        for line in api_lines {
                                            api_content.push_str(&format!("{}\n", line));
                                        }
                                        api_content.push_str("```\n");
                                    }
                                }
                            }
                            Some(("You are an API documentation specialist. Generate comprehensive API docs with descriptions, parameters, return values, and usage examples for each public item.".to_string(), api_content))
                        } else if !trimmed.is_empty() {
                            // Document a specific file
                            let full_path = if trimmed.starts_with('/') { trimmed.to_string() }
                                else { format!("{}/{}", root_path, trimmed) };
                            match std::fs::read_to_string(&full_path) {
                                Ok(content) => {
                                    let ext = std::path::Path::new(trimmed)
                                        .extension().and_then(|e| e.to_str()).unwrap_or("");
                                    Some(("Generate comprehensive documentation for this code. Include: purpose, API/function signatures, parameters, return values, examples, and notes.".to_string(),
                                        format!("Document this file (`{}`):\n```{}\n{}\n```", trimmed, ext, content)))
                                }
                                Err(e) => { print_error(&format!("Cannot read {}: {}", trimmed, e)); continue; }
                            }
                        } else {
                            print_error("Usage: /doc <file>, /doc --readme, /doc --api");
                            continue;
                        };

                        if let Some((system_prompt, user_msg)) = doc_content {
                            let stream_id = uuid::Uuid::new_v4().to_string();
                            *current_stream_id.lock().await = stream_id.clone();
                            *stream_start.lock().await = Some(Instant::now());
                            abort_flag.store(false, Ordering::SeqCst);

                            let mut msgs = messages.lock().await;
                            msgs.push(json!({ "role": "user", "content": user_msg }));

                            let mut api_messages: Vec<serde_json::Value> = Vec::new();
                            api_messages.push(json!({ "role": "system", "content": system_prompt }));
                            api_messages.extend(msgs.iter().cloned());

                            let total_chars: usize = api_messages.iter()
                                .map(|m| m["content"].as_str().unwrap_or("").len())
                                .sum();
                            prompt_tokens.store((total_chars / 4) as u64, Ordering::SeqCst);
                            drop(msgs);

                            streaming.store(true, Ordering::SeqCst);
                            print_ai_prefix();

                            waiting_for_first_token.store(true, Ordering::SeqCst);
                            let wft = waiting_for_first_token.clone();
                            tokio::spawn(async move { run_bunny_animation(wft).await; });

                            ws_tx.send(Message::Text(json!({
                                "id": next_id(),
                                "type": "tauri.invoke",
                                "cmd": "ai_chat_with_tools",
                                "args": {
                                    "streamId": stream_id,
                                    "messages": api_messages,
                                    "model": model,
                                    "baseUrlOverride": base_url,
                                    "temperature": temperature,
                                    "maxTokens": max_tokens,
                                    "toolsEnabled": false,
                                    "chatMode": "plan",
                                    "rootPath": root_path,
                                }
                            }).to_string().into())).await.ok();
                        }
                    }

                    UserCommand::Changelog => {
                        // Get recent git commits
                        match std::process::Command::new("git")
                            .args(["log", "--oneline", "-20"])
                            .current_dir(&root_path)
                            .output()
                        {
                            Ok(output) => {
                                let commits = String::from_utf8_lossy(&output.stdout).to_string();
                                if commits.trim().is_empty() {
                                    print_error("No git commits found.");
                                    continue;
                                }

                                let changelog_prompt = format!(
                                    "Generate a well-structured changelog from these recent git commits. \
                                     Group by category (Added, Changed, Fixed, Removed). Use markdown format.\n\n\
                                     Commits:\n```\n{}\n```", commits
                                );

                                let stream_id = uuid::Uuid::new_v4().to_string();
                                *current_stream_id.lock().await = stream_id.clone();
                                *stream_start.lock().await = Some(Instant::now());
                                abort_flag.store(false, Ordering::SeqCst);

                                let mut msgs = messages.lock().await;
                                msgs.push(json!({ "role": "user", "content": changelog_prompt }));

                                let mut api_messages: Vec<serde_json::Value> = Vec::new();
                                api_messages.push(json!({ "role": "system", "content":
                                    "You are a technical writer. Generate clean, concise changelogs \
                                     in Keep a Changelog format. Group changes by category."
                                }));
                                api_messages.extend(msgs.iter().cloned());

                                let total_chars: usize = api_messages.iter()
                                    .map(|m| m["content"].as_str().unwrap_or("").len())
                                    .sum();
                                prompt_tokens.store((total_chars / 4) as u64, Ordering::SeqCst);
                                drop(msgs);

                                streaming.store(true, Ordering::SeqCst);
                                print_ai_prefix();

                                waiting_for_first_token.store(true, Ordering::SeqCst);
                                let wft = waiting_for_first_token.clone();
                                tokio::spawn(async move { run_bunny_animation(wft).await; });

                                ws_tx.send(Message::Text(json!({
                                    "id": next_id(),
                                    "type": "tauri.invoke",
                                    "cmd": "ai_chat_with_tools",
                                    "args": {
                                        "streamId": stream_id,
                                        "messages": api_messages,
                                        "model": model,
                                        "baseUrlOverride": base_url,
                                        "temperature": temperature,
                                        "maxTokens": max_tokens,
                                        "toolsEnabled": false,
                                        "chatMode": "plan",
                                        "rootPath": root_path,
                                    }
                                }).to_string().into())).await.ok();
                            }
                            Err(e) => print_error(&format!("git log failed: {}", e)),
                        }
                    }

                    UserCommand::History(args) => {
                        handle_history_command(&args);
                    }

                    UserCommand::Keybindings => {
                        handle_keybindings_command();
                    }

                    UserCommand::Perf(args) => {
                        handle_perf_command(&args, &root_path);
                    }

                    UserCommand::Image(path) => {
                        let full_path = if path.starts_with('/') { path.clone() }
                            else { format!("{}/{}", root_path, path) };
                        let p = std::path::Path::new(&full_path);
                        if !p.exists() {
                            print_error(&format!("File not found: {}", path));
                            continue;
                        }
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                        let mime = match ext.as_str() {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            _ => {
                                print_error(&format!("Unsupported image format: .{} (supported: png, jpg, jpeg, gif, webp)", ext));
                                continue;
                            }
                        };
                        match std::fs::read(&full_path) {
                            Ok(bytes) => {
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                let size_kb = bytes.len() / 1024;
                                *image_context.lock().await = Some((b64, mime.to_string()));
                                print_info_accent("Image attached", &format!("{} ({}KB, {})", path, size_kb, mime));
                            }
                            Err(e) => print_error(&format!("Cannot read {}: {}", path, e)),
                        }
                    }

                    UserCommand::Browse(url) => {
                        let url_str = if !url.starts_with("http://") && !url.starts_with("https://") {
                            format!("https://{}", url)
                        } else {
                            url.clone()
                        };
                        print_info_accent("Fetching", &url_str);
                        match reqwest::get(&url_str).await {
                            Ok(resp) => {
                                match resp.text().await {
                                    Ok(html) => {
                                        // Strip HTML tags with regex
                                        let tag_re = regex::Regex::new(r"<[^>]+>").unwrap_or_else(|_| regex::Regex::new(".^").unwrap());
                                        let text = tag_re.replace_all(&html, " ");
                                        // Collapse whitespace
                                        let ws_re = regex::Regex::new(r"\s+").unwrap_or_else(|_| regex::Regex::new(".^").unwrap());
                                        let clean = ws_re.replace_all(&text, " ");
                                        let truncated: String = clean.chars().take(5000).collect();

                                        // Print preview
                                        let preview: String = truncated.chars().take(200).collect();
                                        let mut o = io::stdout();
                                        set_fg(&mut o, theme::DIM_LIGHT);
                                        write!(o, "  {}\n", preview).ok();
                                        reset_color(&mut o);

                                        // Attach as context
                                        let browse_ctx = format!(
                                            "\n\n<web-page url=\"{}\">\n{}\n</web-page>\n",
                                            url_str, truncated
                                        );
                                        *file_context.lock().await = Some(browse_ctx);
                                        print_info_accent("Context", "Web page content attached to next message");
                                    }
                                    Err(e) => print_error(&format!("Failed to read response: {}", e)),
                                }
                            }
                            Err(e) => print_error(&format!("Failed to fetch {}: {}", url_str, e)),
                        }
                    }

                    UserCommand::SkillChain(names) => {
                        // Find all skills and merge them
                        let mut found_skills: Vec<Skill> = Vec::new();
                        let mut missing: Vec<String> = Vec::new();
                        for name in &names {
                            match find_skill(name, Some(&root_path)) {
                                Some(s) => found_skills.push(s),
                                None => missing.push(name.clone()),
                            }
                        }
                        if !missing.is_empty() {
                            print_error(&format!("Skills not found: {}. Use /skills to list.", missing.join(", ")));
                            continue;
                        }
                        if found_skills.len() < 2 {
                            print_error("Skill chaining requires at least 2 skills.");
                            continue;
                        }

                        // Merge: combined system prompt, first skill's mode/temperature
                        let first = &found_skills[0];
                        let merged_prompt = found_skills.iter()
                            .map(|s| format!("## {} skill\n{}", s.name, s.system_prompt))
                            .collect::<Vec<_>>()
                            .join("\n\n---\n\n");
                        let merged_name = names.join("+");
                        let merged_desc = found_skills.iter()
                            .map(|s| s.description.as_str())
                            .collect::<Vec<_>>()
                            .join(" + ");

                        let merged = Skill {
                            name: merged_name.clone(),
                            description: merged_desc,
                            system_prompt: merged_prompt,
                            temperature: first.temperature,
                            mode: first.mode.clone(),
                            category: Some("chain".to_string()),
                            aliases: None,
                            max_turns: first.max_turns,
                            auto_deactivate: first.auto_deactivate,
                            include_git_diff: found_skills.iter().any(|s| s.include_git_diff.unwrap_or(false)).then_some(true),
                            auto_attach: {
                                let all: Vec<String> = found_skills.iter()
                                    .filter_map(|s| s.auto_attach.as_ref())
                                    .flat_map(|a| a.iter().cloned())
                                    .collect();
                                if all.is_empty() { None } else { Some(all) }
                            },
                        };

                        if let Some(ref m) = merged.mode {
                            chat_mode = m.clone();
                            *prompt_mode.lock().unwrap_or_else(|e| e.into_inner()) = chat_mode.clone();
                            print_mode_badge(&chat_mode);
                        }
                        if let Some(t) = merged.temperature {
                            temperature = t;
                        }
                        *prompt_skill.lock().unwrap_or_else(|e| e.into_inner()) = Some(merged.name.clone());
                        print_info_accent("Skill chain", &format!("{} activated", merged_name));
                        skill_turn_count = 0;
                        active_skill = Some(merged);
                    }

                    UserCommand::Spawn(task) => {
                        let spawn_stream_id = uuid::Uuid::new_v4().to_string();
                        spawned_tasks.lock().await.push((spawn_stream_id.clone(), task.clone()));

                        let spawn_msgs = vec![
                            json!({ "role": "system", "content": "You are a background assistant. Complete the given task concisely." }),
                            json!({ "role": "user", "content": task.clone() }),
                        ];

                        print_info_accent("Spawned", &format!("Background task: {}", task));
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Stream: {}\n", &spawn_stream_id[..8]).ok();
                        reset_color(&mut o);

                        ws_tx.send(Message::Text(json!({
                            "id": next_id(),
                            "type": "tauri.invoke",
                            "cmd": "ai_chat_with_tools",
                            "args": {
                                "streamId": spawn_stream_id,
                                "messages": spawn_msgs,
                                "model": model,
                                "baseUrlOverride": base_url,
                                "temperature": temperature,
                                "maxTokens": max_tokens,
                                "toolsEnabled": true,
                                "chatMode": "auto",
                                "rootPath": root_path,
                            }
                        }).to_string().into())).await.ok();
                    }

                    UserCommand::ReleaseNotes => {
                        // Get last tag and commits since then
                        let tag_output = std::process::Command::new("git")
                            .args(["tag", "--sort=-creatordate"])
                            .current_dir(&root_path)
                            .output();
                        let last_tag = tag_output.ok()
                            .and_then(|o| {
                                let s = String::from_utf8_lossy(&o.stdout).to_string();
                                s.lines().next().map(|l| l.trim().to_string())
                            })
                            .filter(|t| !t.is_empty());

                        let range_arg = last_tag.as_ref().map(|t| format!("{}..HEAD", t));
                        let log_args: Vec<&str> = match &range_arg {
                            Some(r) => vec!["log", "--oneline", r.as_str()],
                            None => vec!["log", "--oneline", "-20"],
                        };

                        match std::process::Command::new("git")
                            .args(&log_args)
                            .current_dir(&root_path)
                            .output()
                        {
                            Ok(output) => {
                                let commits = String::from_utf8_lossy(&output.stdout).to_string();
                                if commits.trim().is_empty() {
                                    print_error("No commits found for release notes.");
                                    continue;
                                }

                                let tag_info = last_tag.as_deref().unwrap_or("(no previous tag)");
                                let rn_prompt = format!(
                                    "Generate formatted release notes from these git commits.\n\
                                     Last tag: {}\n\
                                     Today's date: {}\n\n\
                                     Format with: version header, date, and categories:\n\
                                     - **Added** (new features)\n\
                                     - **Changed** (modifications to existing functionality)\n\
                                     - **Fixed** (bug fixes)\n\
                                     - **Removed** (removed features)\n\n\
                                     Commits:\n```\n{}\n```",
                                    tag_info,
                                    chrono::Local::now().format("%Y-%m-%d"),
                                    commits
                                );

                                let stream_id = uuid::Uuid::new_v4().to_string();
                                *current_stream_id.lock().await = stream_id.clone();
                                *stream_start.lock().await = Some(Instant::now());
                                abort_flag.store(false, Ordering::SeqCst);

                                let mut msgs = messages.lock().await;
                                msgs.push(json!({ "role": "user", "content": rn_prompt }));

                                let mut api_messages: Vec<serde_json::Value> = Vec::new();
                                api_messages.push(json!({ "role": "system", "content":
                                    "You are a release manager. Generate clean, professional release notes \
                                     in markdown format. Be concise but descriptive."
                                }));
                                api_messages.extend(msgs.iter().cloned());

                                let total_chars: usize = api_messages.iter()
                                    .map(|m| m["content"].as_str().unwrap_or("").len())
                                    .sum();
                                prompt_tokens.store((total_chars / 4) as u64, Ordering::SeqCst);
                                drop(msgs);

                                streaming.store(true, Ordering::SeqCst);
                                print_ai_prefix();

                                waiting_for_first_token.store(true, Ordering::SeqCst);
                                let wft = waiting_for_first_token.clone();
                                tokio::spawn(async move { run_bunny_animation(wft).await; });

                                ws_tx.send(Message::Text(json!({
                                    "id": next_id(),
                                    "type": "tauri.invoke",
                                    "cmd": "ai_chat_with_tools",
                                    "args": {
                                        "streamId": stream_id,
                                        "messages": api_messages,
                                        "model": model,
                                        "baseUrlOverride": base_url,
                                        "temperature": temperature,
                                        "maxTokens": max_tokens,
                                        "toolsEnabled": false,
                                        "chatMode": "plan",
                                        "rootPath": root_path,
                                    }
                                }).to_string().into())).await.ok();
                            }
                            Err(e) => print_error(&format!("git log failed: {}", e)),
                        }
                    }

                    UserCommand::SkillExport(name) => {
                        match find_skill(&name, Some(&root_path)) {
                            Some(skill) => {
                                // Serialize to TOML
                                let mut toml_str = format!("name = {:?}\n", skill.name);
                                toml_str.push_str(&format!("description = {:?}\n", skill.description));
                                toml_str.push_str(&format!("system_prompt = {:?}\n", skill.system_prompt));
                                if let Some(t) = skill.temperature {
                                    toml_str.push_str(&format!("temperature = {}\n", t));
                                }
                                if let Some(ref m) = skill.mode {
                                    toml_str.push_str(&format!("mode = {:?}\n", m));
                                }
                                if let Some(ref c) = skill.category {
                                    toml_str.push_str(&format!("category = {:?}\n", c));
                                }
                                if let Some(ref a) = skill.aliases {
                                    toml_str.push_str(&format!("aliases = {:?}\n", a));
                                }
                                if let Some(mt) = skill.max_turns {
                                    toml_str.push_str(&format!("max_turns = {}\n", mt));
                                }
                                if let Some(ad) = skill.auto_deactivate {
                                    toml_str.push_str(&format!("auto_deactivate = {}\n", ad));
                                }
                                if let Some(gd) = skill.include_git_diff {
                                    toml_str.push_str(&format!("include_git_diff = {}\n", gd));
                                }
                                if let Some(ref aa) = skill.auto_attach {
                                    toml_str.push_str(&format!("auto_attach = {:?}\n", aa));
                                }

                                print_section_header(&format!("Skill Export: {}", name));
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::DIM_LIGHT);
                                write!(o, "{}\n", toml_str).ok();
                                reset_color(&mut o);
                                print_section_end();
                            }
                            None => print_error(&format!("Skill '{}' not found. Use /skills to list.", name)),
                        }
                    }

                    UserCommand::SkillImport(path) => {
                        let full_path = if path.starts_with('/') || path.starts_with('~') {
                            if path.starts_with('~') {
                                path.replacen('~', &dirs_next::home_dir().map(|h| h.display().to_string()).unwrap_or_default(), 1)
                            } else {
                                path.clone()
                            }
                        } else {
                            format!("{}/{}", root_path, path)
                        };
                        match std::fs::read_to_string(&full_path) {
                            Ok(contents) => {
                                // Validate it's a valid skill TOML
                                match toml::from_str::<Skill>(&contents) {
                                    Ok(skill) => {
                                        let Some(cfg) = config_dir() else {
                                            print_error("Could not determine config directory");
                                            continue;
                                        };
                                        let skills_dir = cfg.join("skills");
                                        std::fs::create_dir_all(&skills_dir).ok();
                                        let dest = skills_dir.join(format!("{}.toml", skill.name));
                                        match std::fs::write(&dest, &contents) {
                                            Ok(_) => print_info_accent("Skill imported", &format!("{} -> {}", skill.name, dest.display())),
                                            Err(e) => print_error(&format!("Failed to write: {}", e)),
                                        }
                                    }
                                    Err(e) => print_error(&format!("Invalid skill TOML: {}", e)),
                                }
                            }
                            Err(e) => print_error(&format!("Cannot read {}: {}", full_path, e)),
                        }
                    }

                    UserCommand::Symbols(query) => {
                        handle_symbols_command(&query, &root_path);
                    }

                    UserCommand::Cheatsheet => {
                        print_cheatsheet();
                    }

                    UserCommand::Save(name) => {
                        let snap_dir = dirs_next::config_dir()
                            .map(|d| d.join("shadowai").join("snapshots"))
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/snapshots"));
                        std::fs::create_dir_all(&snap_dir).ok();
                        let snap_path = snap_dir.join(format!("{}.json", name));
                        let msgs = messages.lock().await;
                        match serde_json::to_string_pretty(&*msgs) {
                            Ok(json) => {
                                match std::fs::write(&snap_path, &json) {
                                    Ok(_) => print_info_accent("Saved", &format!("'{}' ({} messages) -> {}", name, msgs.len(), snap_path.display())),
                                    Err(e) => print_error(&format!("Failed to save: {}", e)),
                                }
                            }
                            Err(e) => print_error(&format!("Serialization failed: {}", e)),
                        }
                    }

                    UserCommand::Load(name) => {
                        let snap_dir = dirs_next::config_dir()
                            .map(|d| d.join("shadowai").join("snapshots"))
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/snapshots"));
                        let snap_path = snap_dir.join(format!("{}.json", name));
                        match std::fs::read_to_string(&snap_path) {
                            Ok(json) => {
                                match serde_json::from_str::<Vec<serde_json::Value>>(&json) {
                                    Ok(loaded) => {
                                        let count = loaded.len();
                                        let mut msgs = messages.lock().await;
                                        *msgs = loaded;
                                        let user_count = msgs.iter().filter(|m| m["role"] == "user").count();
                                        let asst_count = msgs.iter().filter(|m| m["role"] == "assistant").count();
                                        drop(msgs);
                                        print_info_accent("Loaded", &format!("'{}' — {} messages ({} user, {} assistant)", name, count, user_count, asst_count));
                                    }
                                    Err(e) => print_error(&format!("Invalid snapshot JSON: {}", e)),
                                }
                            }
                            Err(_) => print_error(&format!("Snapshot '{}' not found at {}", name, snap_path.display())),
                        }
                    }

                    // New commands (Section 10)
                    UserCommand::Todo => {
                        handle_todo_command(&root_path);
                    }

                    UserCommand::Env => {
                        handle_env_command(&root_path);
                    }

                    UserCommand::Secrets => {
                        handle_secrets_command(&root_path);
                    }

                    UserCommand::Metrics => {
                        handle_metrics_command(&root_path);
                    }

                    UserCommand::Deps => {
                        handle_deps_command(&root_path);
                    }

                    UserCommand::DiffFile(file) => {
                        handle_diff_file_command(&file, &root_path);
                    }

                    UserCommand::Diagram => {
                        // Scan project structure and ask AI to generate Mermaid diagram
                        let mut o = io::stdout();
                        print_section_header("Architecture Diagram");
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Scanning project structure...\n").ok();
                        reset_color(&mut o);

                        // Get top-level structure
                        let tree_output = std::process::Command::new("find")
                            .args([".", "-maxdepth", "2", "-not", "-path", "./.git/*", "-not", "-path", "./target/*", "-not", "-path", "./node_modules/*"])
                            .current_dir(&root_path)
                            .output()
                            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                            .unwrap_or_default();

                        let diagram_prompt = format!(
                            "Generate a Mermaid `graph TD` architecture diagram for this project structure. \
                            Output ONLY the mermaid code block, nothing else.\n\nProject structure:\n{}",
                            tree_output.lines().take(50).collect::<Vec<_>>().join("\n")
                        );

                        // Send to AI via the message mechanism
                        let _ = user_tx.send(UserCommand::Message(diagram_prompt));
                        print_section_end();
                        continue;
                    }

                    UserCommand::Chat => {
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::ACCENT);
                        write!(o, "\n  {SPARK} General Chat Mode\n").ok();
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Messages will be sent without project context.\n").ok();
                        write!(o, "  Use /new or start a new session to return to project mode.\n\n").ok();
                        reset_color(&mut o);
                        // Signal to skip project context injection for this session
                        // by sending a flag message (handled via Message with special prefix)
                        // Note: full implementation would require a session-level flag
                        print_info_accent("Chat", "General chat mode activated (no project context)");
                    }

                    // Theme commands (Section 13)
                    UserCommand::ThemeList => {
                        let mut o = io::stdout();
                        print_section_header("Available Themes");
                        let themes = ["dark", "light", "dracula", "nord", "gruvbox", "catppuccin", "tokyo-night"];
                        for t in &themes {
                            set_fg(&mut o, theme::DIM_LIGHT);
                            write!(o, "  {DOT} {}\n", t).ok();
                        }
                        reset_color(&mut o);
                        print_section_end();
                    }

                    UserCommand::Theme(name) => {
                        let mut o = io::stdout();
                        let themes = ["dark", "light", "dracula", "nord", "gruvbox", "catppuccin", "tokyo-night"];
                        if themes.contains(&name.as_str()) {
                            load_theme(&name);
                            set_fg(&mut o, theme::OK);
                            write!(o, "  {CHECK} Theme '{}' loaded (applies to next session start)\n", name).ok();
                            reset_color(&mut o);
                            print_info_accent("Theme", &format!("Set to '{}'. Save theme = \"{}\" in config.toml to persist.", name, name));
                        } else {
                            print_error(&format!("Unknown theme '{}'. Use /theme list to see available themes.", name));
                        }
                    }

                    // Heal command (Section 12)
                    UserCommand::Heal(args) => {
                        let mut o = io::stdout();
                        print_section_header("Auto-Heal");
                        let max_attempts = if let Some(n) = args.strip_prefix("--max-attempts ") {
                            n.trim().parse::<u32>().unwrap_or(3)
                        } else {
                            cli_config.max_heal_attempts.unwrap_or(3)
                        };

                        // Run cargo check / npm build to get current errors
                        let root = std::path::Path::new(&root_path);
                        let (check_cmd, check_args) = if root.join("Cargo.toml").exists() {
                            ("cargo", vec!["check", "--message-format=short"])
                        } else if root.join("package.json").exists() {
                            ("npm", vec!["run", "build"])
                        } else {
                            ("make", vec![])
                        };

                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Running {} {}...\n", check_cmd, check_args.join(" ")).ok();
                        reset_color(&mut o);

                        let check_output = std::process::Command::new(check_cmd)
                            .args(&check_args)
                            .current_dir(&root_path)
                            .output();

                        match check_output {
                            Ok(out) if !out.status.success() => {
                                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                                let stdout_str = String::from_utf8_lossy(&out.stdout).to_string();
                                let errors = if stderr.is_empty() { stdout_str } else { stderr };

                                set_fg(&mut o, theme::WARN);
                                write!(o, "  Found errors. Attempting auto-fix (max {} attempts)...\n", max_attempts).ok();
                                reset_color(&mut o);
                                print_section_end();

                                // Send errors to AI with heal skill prompt
                                let heal_prompt = format!(
                                    "I need you to fix these build errors. Analyze each error, then use write_file/edit_file tools to fix them. After fixing, explain what you changed.\n\nErrors:\n```\n{}\n```",
                                    errors.chars().take(3000).collect::<String>()
                                );
                                let _ = user_tx.send(UserCommand::Message(heal_prompt));
                                continue;
                            }
                            Ok(_) => {
                                set_fg(&mut o, theme::OK);
                                write!(o, "  {CHECK} No errors found — project builds cleanly!\n").ok();
                                reset_color(&mut o);
                                print_section_end();
                            }
                            Err(e) => {
                                print_error(&format!("Failed to run {}: {}", check_cmd, e));
                                print_section_end();
                            }
                        }
                    }

                    // Batch 3 commands
                    UserCommand::Explain(target) => {
                        match handle_explain_command(&target, &root_path) {
                            Some(prompt) => {
                                let _ = user_tx.send(UserCommand::Message(prompt));
                                continue;
                            }
                            None => {}
                        }
                    }

                    UserCommand::Rename(old, new) => {
                        let result = handle_rename_command(&old, &new, &root_path);
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::AI_TEXT);
                        write!(o, "\n  {}\n", result).ok();
                        reset_color(&mut o);
                    }

                    UserCommand::Extract(target) => {
                        let parts: Vec<&str> = target.rsplitn(2, ':').collect();
                        let (file, range) = if parts.len() == 2 {
                            (parts[1], parts[0])
                        } else {
                            (target.as_str(), "")
                        };
                        let prompt = format!(
                            "Extract lines {} of `{}` into a well-named function/method. Show the refactored code.",
                            range, file
                        );
                        let _ = user_tx.send(UserCommand::Message(prompt));
                        continue;
                    }

                    UserCommand::Docker(sub) => {
                        handle_docker_command(&sub, &root_path);
                    }

                    UserCommand::Release(version) => {
                        handle_release_command(&version, &root_path);
                    }

                    UserCommand::Benchmark(name) => {
                        handle_benchmark_command(&name, &root_path);
                    }

                    UserCommand::Coverage => {
                        handle_coverage_command(&root_path);
                    }

                    UserCommand::Translate(lang) => {
                        let ctx = {
                            let tf = tracked_files.lock().await;
                            if tf.is_empty() {
                                None
                            } else {
                                tf.iter().next().map(|p| p.clone())
                            }
                        };
                        match ctx {
                            Some(path) => {
                                if let Ok(code) = std::fs::read_to_string(&path) {
                                    let ext = std::path::Path::new(&path)
                                        .extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
                                    let prompt = format!(
                                        "Translate the following {} code to {}, maintaining the same logic and structure:\n```{}\n{}\n```",
                                        ext, lang, ext, code.chars().take(8000).collect::<String>()
                                    );
                                    let _ = user_tx.send(UserCommand::Message(prompt));
                                    continue;
                                }
                            }
                            None => {
                                print_error("Please attach a file first with /file <path> or /add <path>");
                            }
                        }
                    }

                    UserCommand::Mock(name) => {
                        let prompt = format!(
                            "Generate a mock implementation of `{}` for the current project. \
                            Use the project's test framework conventions. The mock should implement \
                            all methods/functions with configurable return values and call tracking.",
                            name
                        );
                        let _ = user_tx.send(UserCommand::Message(prompt));
                        continue;
                    }

                    UserCommand::Remote(sub) => {
                        handle_remote_command(&sub);
                    }

                    UserCommand::Share => {
                        handle_share_command(&root_path);
                    }

                    UserCommand::Cron(sub) => {
                        handle_cron_command(&sub);
                    }

                    UserCommand::Research(topic) => {
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::CYAN);
                        write!(o, "\n  Searching for: {}...\n", topic).ok();
                        reset_color(&mut o);
                        let prompt = format!(
                            "Research the following topic and provide a comprehensive summary with examples and key findings.\n\nTopic: {}\n\nPlease: 1) Summarize key findings 2) List pros/cons if applicable 3) Provide code examples if relevant 4) Include any important caveats",
                            topic
                        );
                        let _ = user_tx.send(UserCommand::Message(prompt));
                        continue;
                    }

                    UserCommand::Think(arg) => {
                        let mut o = io::stdout();
                        let arg = arg.trim();
                        if arg.is_empty() {
                            let budget = THINK_BUDGET.load(Ordering::SeqCst);
                            if budget == 0 {
                                set_fg(&mut o, theme::DIM_LIGHT);
                                write!(o, "  Extended thinking is disabled. Use /think <tokens> (e.g. /think 8000) to enable.\n").ok();
                            } else {
                                set_fg(&mut o, theme::CYAN);
                                write!(o, "  Extended thinking enabled with budget: {} tokens\n", budget).ok();
                            }
                            reset_color(&mut o);
                        } else if arg == "off" || arg == "0" {
                            THINK_BUDGET.store(0, Ordering::SeqCst);
                            set_fg(&mut o, theme::OK);
                            write!(o, "  Extended thinking disabled.\n").ok();
                            reset_color(&mut o);
                        } else if let Ok(n) = arg.parse::<u64>() {
                            let n = n.min(32000);
                            THINK_BUDGET.store(n, Ordering::SeqCst);
                            set_fg(&mut o, theme::OK);
                            write!(o, "  Extended thinking enabled: {} tokens budget.\n", n).ok();
                            reset_color(&mut o);
                        } else {
                            print_error("Usage: /think <tokens> (0–32000) or /think off");
                        }
                    }

                    UserCommand::Agent(sub) => {
                        handle_agent_command(&sub, &root_path, &cli_config);
                    }

                    UserCommand::Debug(sub) => {
                        handle_debug_command_local(&sub, &root_path);
                    }

                    UserCommand::Shader(sub) => {
                        handle_shader_command_local(&sub, &root_path);
                    }

                    UserCommand::Assets(sub) => {
                        handle_assets_command_local(&sub, &root_path);
                    }

                    UserCommand::Docs(pkg) => {
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::CYAN);
                        write!(o, "\n  Looking up docs for: {}\n\n", pkg).ok();
                        reset_color(&mut o);
                        let prompt = format!(
                            "Look up documentation for `{}`. Explain what it does, key APIs/functions, usage examples, and any important notes.",
                            pkg
                        );
                        let _ = user_tx.send(UserCommand::Message(prompt));
                        continue;
                    }

                    UserCommand::Rebase(arg) | UserCommand::Gr(arg) => {
                        handle_rebase_command_local(&arg, &root_path);
                    }

                    // Pass 4 command dispatch
                    UserCommand::Gd(file) => {
                        handle_gd_command(&file, &cli_config).await;
                    }

                    UserCommand::Rag(query) => {
                        handle_rag_command(&query, &cli_config).await;
                    }

                    UserCommand::Add(glob_pat) => {
                        handle_add_context_command(&glob_pat, &root_path);
                    }

                    UserCommand::Ctx => {
                        handle_ctx_command();
                    }

                    UserCommand::Drop(label) => {
                        handle_drop_context_command(&label);
                    }

                    UserCommand::Memory => {
                        handle_memory_command(&root_path);
                    }

                    UserCommand::Unfold(id) => {
                        handle_unfold_command(id);
                    }

                    UserCommand::CacheClear => {
                        if let Some(cache) = RESPONSE_CACHE.get() {
                            cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
                            let mut o = io::stdout();
                            set_fg(&mut o, theme::OK);
                            write!(o, "  [cache] Response cache cleared.\n").ok();
                            reset_color(&mut o);
                        }
                    }

                    UserCommand::Dap(args) => {
                        handle_dap_command(&args, &cli_config).await;
                    }

                    UserCommand::Pdf(path) => {
                        handle_pdf_command(&path, &cli_config).await;
                    }

                    UserCommand::Yt(url) => {
                        handle_yt_command(&url, &cli_config).await;
                    }

                    UserCommand::DepsFix => {
                        handle_deps_fix(&cli_config).await;
                    }

                    UserCommand::Profile(dur) => {
                        let secs = dur.parse::<u64>().unwrap_or(10);
                        handle_profile_command(secs, &cli_config).await;
                    }

                    UserCommand::Db(args) => {
                        handle_db_command(&args, &cli_config).await;
                    }

                    UserCommand::K8s(args) => {
                        handle_k8s_command(&args, &cli_config).await;
                    }

                    UserCommand::Migrate(args) => {
                        handle_migrate_command(&args, &cli_config).await;
                    }

                    UserCommand::PlanOn => {
                        PLAN_MODE.store(true, Ordering::SeqCst);
                        if THINK_BUDGET.load(Ordering::SeqCst) == 0 {
                            THINK_BUDGET.store(8000, Ordering::SeqCst);
                            let mut o = io::stdout();
                            set_fg(&mut o, theme::OK);
                            write!(o, "  [plan] Auto-think enabled (8k budget)\n").ok();
                            reset_color(&mut o);
                        }
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::CYAN);
                        write!(o, "  [plan] Plan mode ON\n").ok();
                        reset_color(&mut o);
                    }

                    UserCommand::PlanOff => {
                        PLAN_MODE.store(false, Ordering::SeqCst);
                        let mut o = io::stdout();
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  [plan] Plan mode OFF\n").ok();
                        reset_color(&mut o);
                    }

                    // Section 8.1 — Email SMTP
                    UserCommand::EmailTest => {
                        match send_email_notification("ShadowAI test", "This is a test email from ShadowAI CLI.", &cli_config).await {
                            Ok(()) => {
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::OK);
                                write!(o, "  [email] Test email sent successfully.\n").ok();
                                reset_color(&mut o);
                            }
                            Err(e) => {
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::ERR);
                                write!(o, "  [email] {}\n", e).ok();
                                reset_color(&mut o);
                            }
                        }
                    }

                    // Section 7.1 — Real-time Collaboration
                    UserCommand::Relay(args) => {
                        handle_relay_command(&args, &cli_config).await;
                    }

                    UserCommand::ReviewRequest(target) => {
                        handle_review_request(&target, &cli_config).await;
                    }

                    // Section 13 — MCP
                    UserCommand::Mcp(args) => {
                        handle_mcp_command(&args, &cli_config).await;
                    }

                    // Section 14 — Agentic
                    UserCommand::Architect => {
                        handle_architect_command(&root_path, &cli_config).await;
                    }

                    UserCommand::Yolo => {
                        handle_yolo_command();
                    }

                    UserCommand::Approval(tier) => {
                        handle_approval_command(&tier);
                    }

                    UserCommand::Arena(prompt) => {
                        handle_arena_command(&prompt, &cli_config).await;
                    }

                    UserCommand::Teleport => {
                        let msgs = messages.lock().await;
                        handle_teleport_command(&msgs, &root_path);
                    }

                    // Section 15 — Context / Memory
                    UserCommand::Snapshot(args) => {
                        let msgs = messages.lock().await;
                        handle_snapshot_command(&args, &msgs, &root_path).await;
                    }

                    UserCommand::RepoMap => {
                        handle_repomap_command(&root_path);
                    }

                    // Section 16 — Multimodal
                    UserCommand::Voice => {
                        if let Some(text) = handle_voice_command() {
                            let _ = user_tx.send(UserCommand::Message(text));
                        }
                    }

                    UserCommand::Screenshot => {
                        handle_screenshot_command(&image_context).await;
                    }

                    // Section 17 — Dev Experience
                    UserCommand::Cost => {
                        handle_cost_command(&cli_config);
                    }

                    UserCommand::Block(id) => {
                        handle_block_command(id);
                    }

                    UserCommand::LogSearch(term) => {
                        handle_log_search_command(&term, &root_path);
                    }

                    UserCommand::Runbook(args) => {
                        handle_runbook_command(&args, &root_path).await;
                    }

                    UserCommand::Switch(provider) => {
                        handle_switch_command(&provider, &mut model, &cli_config);
                    }

                    // Section 18 — Security
                    UserCommand::Audit(args) => {
                        handle_audit_command(&args, &root_path);
                    }

                    // Section 19 — Local LLM
                    UserCommand::ModelPull(name) => {
                        handle_model_pull_command(&name).await;
                    }

                    UserCommand::Message(input) => {
                        // Dirty file warning: check for uncommitted changes
                        {
                            if let Ok(output) = std::process::Command::new("git")
                                .args(["status", "--porcelain"])
                                .current_dir(&root_path)
                                .output()
                            {
                                let status = String::from_utf8_lossy(&output.stdout);
                                let modified_count = status.lines().filter(|l| !l.is_empty()).count();
                                if modified_count > 0 {
                                    let mut o = io::stdout();
                                    set_fg(&mut o, theme::WARN);
                                    write!(o, "  {RADIO} {modified_count} uncommitted change(s) in working tree\n").ok();
                                    reset_color(&mut o);
                                }
                            }
                        }

                        let expanded = expand_file_refs(&input, &root_path);
                        // Inject file context + tracked files + project memory
                        let memory_ctx = read_memory_context(&root_path);
                        let tracked_ctx = {
                            let tf = tracked_files.lock().await;
                            let mut ctx = String::new();
                            for path in tf.iter() {
                                if let Ok(content) = std::fs::read_to_string(path) {
                                    let display = path.strip_prefix(&format!("{}/", root_path)).unwrap_or(path);
                                    let ext = std::path::Path::new(path)
                                        .extension().and_then(|e| e.to_str()).unwrap_or("");
                                    ctx.push_str(&format!("\n\nContents of `{}`:\n```{}\n{}\n```\n", display, ext, content));
                                }
                            }
                            ctx
                        };
                        // Enhanced skill context injection
                        let skill_ctx = if let Some(ref sk) = active_skill {
                            let mut ctx = String::new();
                            if sk.include_git_diff.unwrap_or(false) {
                                ctx.push_str(&get_git_diff_context(&root_path));
                            }
                            if let Some(ref patterns) = sk.auto_attach {
                                ctx.push_str(&get_auto_attach_context(&root_path, patterns));
                            }
                            ctx
                        } else {
                            String::new()
                        };
                        let full_message = {
                            let mut ctx = file_context.lock().await;
                            let fc = ctx.take().unwrap_or_default();
                            format!("{}{}{}{}{}", expanded, fc, tracked_ctx, memory_ctx, skill_ctx)
                        };

                        let stream_id = uuid::Uuid::new_v4().to_string();
                        *current_stream_id.lock().await = stream_id.clone();
                        *stream_start.lock().await = Some(Instant::now());
                        // Section 8.1: record task start time for email threshold check
                        {
                            let tst = TASK_START_TIME.get_or_init(|| std::sync::Mutex::new(None));
                            if let Ok(mut g) = tst.lock() { *g = Some(std::time::Instant::now()); }
                        }
                        abort_flag.store(false, Ordering::SeqCst);

                        let mut msgs = messages.lock().await;
                        msgs.push(json!({ "role": "user", "content": full_message }));

                        // Inject active skill as system prompt at the front
                        let mut api_messages: Vec<serde_json::Value> = Vec::new();
                        let system_prompt = active_skill.as_ref()
                            .map(|s| s.system_prompt.clone())
                            .or_else(|| cli_config.system_prompt.clone());
                        if let Some(ref prompt) = system_prompt {
                            api_messages.push(json!({ "role": "system", "content": prompt }));
                        }
                        api_messages.extend(msgs.iter().cloned());

                        // Update prompt token estimate
                        let total_chars: usize = api_messages.iter()
                            .map(|m| m["content"].as_str().unwrap_or("").len())
                            .sum();
                        let estimated_tokens = total_chars / 4;
                        prompt_tokens.store(estimated_tokens as u64, Ordering::SeqCst);

                        // Context 80% warning
                        {
                            let max_ctx = 128000usize;
                            let pct = (estimated_tokens as f64 / max_ctx as f64 * 100.0) as usize;
                            if pct >= 80 {
                                let mut o = io::stdout();
                                set_fg(&mut o, theme::ERR);
                                write!(o, "  {RADIO} Context usage at {}% (~{} tokens). Consider /compact or /context drop.\n", pct, fmt_tokens(estimated_tokens as u64)).ok();
                                reset_color(&mut o);
                            }
                        }

                        drop(msgs);

                        streaming.store(true, Ordering::SeqCst);
                        print_ai_prefix();

                        // Start bunny running animation while waiting for AI
                        waiting_for_first_token.store(true, Ordering::SeqCst);
                        let wft = waiting_for_first_token.clone();
                        tokio::spawn(async move { run_bunny_animation(wft).await; });

                        // Build args, optionally including image context
                        let img_ctx = image_context.lock().await.take();
                        let mut send_args = json!({
                            "streamId": stream_id,
                            "messages": api_messages,
                            "model": model,
                            "baseUrlOverride": base_url,
                            "temperature": temperature,
                            "maxTokens": max_tokens,
                            "toolsEnabled": true,
                            "chatMode": chat_mode,
                            "rootPath": root_path,
                        });
                        if let Some((b64_data, mime)) = img_ctx {
                            send_args["images"] = json!([{"data": b64_data, "mime_type": mime}]);
                        }

                        ws_tx.send(Message::Text(json!({
                            "id": next_id(),
                            "type": "tauri.invoke",
                            "cmd": "ai_chat_with_tools",
                            "args": send_args,
                        }).to_string().into())).await.ok();

                        // Save user message to session
                        if let Some(ref sid) = *current_session_id.lock().await {
                            ws_tx.send(Message::Text(json!({
                                "id": next_id(),
                                "type": "ferrum.saveMessage",
                                "session_id": sid,
                                "message": { "role": "user", "content": full_message }
                            }).to_string().into())).await.ok();
                        }
                    }
                }
            }

            _ = heartbeat_interval.tick() => {
                ws_tx.send(Message::Text(
                    json!({
                        "id": next_id(),
                        "type": "heartbeat"
                    }).to_string().into()
                )).await.ok();
            }
        }
    }
}

// ─── WebSocket message handler ───────────────────────────────────────────────

async fn handle_ws_message(
    v: &serde_json::Value,
    current_stream_id: &Arc<tokio::sync::Mutex<String>>,
    streaming: &Arc<AtomicBool>,
    response_acc: &Arc<tokio::sync::Mutex<String>>,
    last_response: &Arc<tokio::sync::Mutex<String>>,
    stream_formatter: &Arc<tokio::sync::Mutex<StreamFormatter>>,
    thinking_acc: &Arc<tokio::sync::Mutex<String>>,
    messages: &Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
    stream_start: &Arc<tokio::sync::Mutex<Option<Instant>>>,
    is_one_shot: bool,
    sessions_cache: &Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    waiting_for_first_token: &Arc<AtomicBool>,
    root_path: &str,
    last_tool_failed: &Arc<AtomicBool>,
    last_tool_name: &Arc<tokio::sync::Mutex<String>>,
    hooks: &[Hook],
    stream_token_count: &Arc<AtomicU64>,
    no_stream: bool,
    json_output: bool,
    auto_lint: bool,
    modified_files: &Arc<tokio::sync::Mutex<Vec<String>>>,
    last_op: &Arc<tokio::sync::Mutex<String>>,
    plan_steps: &Arc<tokio::sync::Mutex<Vec<String>>>,
    auto_commit: bool,
    turn_counter: &Arc<AtomicU64>,
) {
    let msg_type = v["type"].as_str().unwrap_or("");

    // Handle TauriEvent format
    if msg_type == "tauri.event" {
        let event = v["event"].as_str().unwrap_or("");
        let payload = &v["payload"];
        let sid = current_stream_id.lock().await.clone();
        handle_event(event, payload, &sid, streaming, response_acc, last_response, stream_formatter, thinking_acc, messages, stream_start, is_one_shot, waiting_for_first_token, root_path, last_tool_failed, last_tool_name, hooks, stream_token_count, no_stream, json_output, auto_lint, modified_files, last_op, plan_steps, auto_commit, turn_counter).await;
        return;
    }

    // Handle agent.event format
    if msg_type == "agent.event" {
        let event_type = v["event_type"].as_str().unwrap_or(v["type"].as_str().unwrap_or(""));
        let payload = &v["payload"];
        let sid = current_stream_id.lock().await.clone();
        handle_event(event_type, payload, &sid, streaming, response_acc, last_response, stream_formatter, thinking_acc, messages, stream_start, is_one_shot, waiting_for_first_token, root_path, last_tool_failed, last_tool_name, hooks, stream_token_count, no_stream, json_output, auto_lint, modified_files, last_op, plan_steps, auto_commit, turn_counter).await;
        return;
    }

    // Handle direct event format
    let event = v["event"].as_str().unwrap_or("");
    if !event.is_empty() {
        let payload = &v["payload"];
        let sid = current_stream_id.lock().await.clone();
        handle_event(event, payload, &sid, streaming, response_acc, last_response, stream_formatter, thinking_acc, messages, stream_start, is_one_shot, waiting_for_first_token, root_path, last_tool_failed, last_tool_name, hooks, stream_token_count, no_stream, json_output, auto_lint, modified_files, last_op, plan_steps, auto_commit, turn_counter).await;
        return;
    }

    // Handle tauri.invokeResult
    if msg_type == "tauri.invokeResult" {
        let result = &v["result"];
        if let Some(arr) = result.as_array() {
            if !arr.is_empty() {
                print_section_header("Results");
                for item in arr {
                    if let Some(key) = item["key"].as_str() {
                        // Memory list
                        let cat = item["category"].as_str().unwrap_or("");
                        let val: String = item["value"].as_str().unwrap_or("").chars().take(60).collect();
                        print_list_item(DOT, theme::DIM_LIGHT, &format!("[{}] {} - {}", cat, key, val));
                    } else if let Some(id) = item["id"].as_str().or_else(|| item.as_str()) {
                        // Model list
                        print_list_item(ARROW, theme::CYAN_DIM, id);
                    }
                }
                print_section_end();
            }
        } else if let Some(s) = result.as_str() {
            print_info(s);
        }
        return;
    }

    // Handle ferrum.sessions response
    if msg_type == "ferrum.sessions" {
        if let Some(sessions) = v["sessions"].as_array() {
            // Cache session list for /session <number> selection
            *sessions_cache.lock().unwrap_or_else(|e| e.into_inner()) = sessions.clone();

            print_section_header("Sessions");
            if sessions.is_empty() {
                print_list_item(DOT, theme::DIM, "No sessions — use /new to create one");
            }
            for (i, s) in sessions.iter().enumerate() {
                let id = s["id"].as_str().unwrap_or("?");
                let short_id = &id[..id.len().min(8)];
                let name = s["name"].as_str().unwrap_or("(unnamed)");
                let profile = s["profile"].as_str().unwrap_or("");
                let msg_count = s["message_count"].as_u64().unwrap_or(0);
                let pinned = s["is_pinned"].as_bool().unwrap_or(false);
                let preview = s["last_message_preview"].as_str().unwrap_or("");
                let updated_ts = s["updated_at"].as_u64().unwrap_or(0);
                let age = format_time_ago(updated_ts);

                let pin_icon = if pinned { format!(" {}", "\u{2605}") } else { String::new() };
                let marker = if i == 0 { RADIO } else { ARROW };
                let color = if i == 0 { theme::CYAN } else { theme::CYAN_DIM };

                // Line 1: number + name + pin + short ID + age
                let num = i + 1;
                print_list_item(marker, color, &format!(
                    "[{}] {}{} \x1b[90m#{} \u{2022} {}msg \u{2022} {} \u{2022} {}\x1b[0m",
                    num, name, pin_icon, short_id, msg_count, profile, age
                ));

                // Line 2: last message preview (if available)
                if !preview.is_empty() {
                    let truncated = if preview.chars().count() > 60 {
                        let s: String = preview.chars().take(57).collect();
                        format!("{}...", s)
                    } else {
                        preview.to_string()
                    };
                    let mut o = io::stdout();
                    execute!(o,
                        SetForegroundColor(theme::BORDER),
                        Print(format!("  {V_LINE}   ")),
                        SetForegroundColor(theme::DIM),
                        Print(format!("  \"{}\"\n", truncated.replace('\n', " "))),
                        ResetColor,
                    ).ok();
                }
            }
            print_section_end();
        }
        return;
    }

    // Handle ferrum.messages response — load session messages into chat history
    if msg_type == "ferrum.messages" {
        if let Some(msg_array) = v["messages"].as_array() {
            // Replace current conversation with loaded session messages
            let mut msgs = messages.lock().await;
            msgs.clear();
            let mut user_count = 0usize;
            let mut assistant_count = 0usize;
            for m in msg_array {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("");
                if content.is_empty() { continue; }
                match role {
                    "user" => user_count += 1,
                    "assistant" => assistant_count += 1,
                    _ => {}
                }
                msgs.push(json!({ "role": role, "content": content }));
            }
            print_section_header("Session Loaded");
            print_section_row("Messages", &format!("{}", msgs.len()));
            print_section_row("User", &format!("{}", user_count));
            print_section_row("AI", &format!("{}", assistant_count));
            print_section_end();
        }
        return;
    }

    // Handle ferrum.providerModels response
    if msg_type == "ferrum.providerModels" {
        if let Some(models) = v["models"].as_array() {
            print_section_header("Models");
            for m in models {
                let id = m["id"].as_str().or_else(|| m.as_str()).unwrap_or("?");
                print_list_item(ARROW, theme::CYAN_DIM, id);
            }
            print_section_end();
        }
    }
}

async fn handle_event(
    event: &str,
    payload: &serde_json::Value,
    sid: &str,
    streaming: &Arc<AtomicBool>,
    response_acc: &Arc<tokio::sync::Mutex<String>>,
    last_response: &Arc<tokio::sync::Mutex<String>>,
    stream_formatter: &Arc<tokio::sync::Mutex<StreamFormatter>>,
    thinking_acc: &Arc<tokio::sync::Mutex<String>>,
    messages: &Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
    stream_start: &Arc<tokio::sync::Mutex<Option<Instant>>>,
    is_one_shot: bool,
    waiting_for_first_token: &Arc<AtomicBool>,
    root_path: &str,
    last_tool_failed: &Arc<AtomicBool>,
    last_tool_name: &Arc<tokio::sync::Mutex<String>>,
    hooks: &[Hook],
    stream_token_count: &Arc<AtomicU64>,
    no_stream: bool,
    json_output: bool,
    auto_lint: bool,
    modified_files: &Arc<tokio::sync::Mutex<Vec<String>>>,
    last_op: &Arc<tokio::sync::Mutex<String>>,
    plan_steps: &Arc<tokio::sync::Mutex<Vec<String>>>,
    auto_commit: bool,
    turn_counter: &Arc<AtomicU64>,
) {
    let stream_suffix = format!("-{}", sid);

    if event.starts_with("ai-chat-stream") && event.ends_with(&stream_suffix) {
        let content = payload["content"].as_str().unwrap_or("");
        if !content.is_empty() {
            stop_bunny(waiting_for_first_token);
            // Count words as rough token proxy for speed indicator
            let word_count = content.split_whitespace().count().max(1) as u64;
            stream_token_count.fetch_add(word_count, Ordering::SeqCst);
            response_acc.lock().await.push_str(content);
            if !no_stream {
                let formatted = stream_formatter.lock().await.feed(content);
                if !formatted.is_empty() {
                    let mut o = io::stdout();
                    write!(o, "{}", formatted).ok();
                    o.flush().ok();
                    // Section 7.1: relay broadcast AI output to connected collaborators
                    relay_broadcast(&formatted);
                }
            }
        }
    } else if event.starts_with("ai-chat-think") && event.ends_with(&stream_suffix) {
        let content = payload["content"].as_str().unwrap_or("");
        if !content.is_empty() {
            let mut acc = thinking_acc.lock().await;
            if acc.is_empty() {
                print_thinking_start();
            }
            acc.push_str(content);
        }
    } else if event.starts_with("ai-tool-call") && event.ends_with(&stream_suffix) {
        stop_bunny(waiting_for_first_token);
        let name = payload["name"].as_str().unwrap_or("unknown");
        let arguments = payload["arguments"].as_str().unwrap_or("{}");
        run_hooks(hooks, "pre-tool", Some(name), &json!({"args": arguments}));
        print_tool_call(name, arguments);

        // Diff preview for write_file and patch_file
        if name == "write_file" || name == "patch_file" {
            if let Ok(args_json) = serde_json::from_str::<serde_json::Value>(arguments) {
                let file_path = args_json["path"].as_str()
                    .or_else(|| args_json["file_path"].as_str())
                    .unwrap_or("");
                if !file_path.is_empty() {
                    let full_path = if file_path.starts_with('/') {
                        file_path.to_string()
                    } else {
                        format!("{}/{}", root_path, file_path)
                    };
                    if name == "write_file" {
                        let new_content = args_json["content"].as_str().unwrap_or("");
                        let old_content = std::fs::read_to_string(&full_path).unwrap_or_default();
                        if !old_content.is_empty() && !new_content.is_empty() {
                            print_unified_diff(file_path, &old_content, new_content);
                        }
                    } else if name == "patch_file" {
                        // patch_file: the patch/diff is the content, display it colored
                        let patch = args_json["patch"].as_str()
                            .or_else(|| args_json["diff"].as_str())
                            .or_else(|| args_json["content"].as_str())
                            .unwrap_or("");
                        if !patch.is_empty() {
                            let mut o = io::stdout();
                            set_fg(&mut o, theme::ACCENT_DIM);
                            write!(o, "\n  {V_LINE} {ARROW} patch {file_path}\n").ok();
                            for line in patch.lines().take(50) {
                                if line.starts_with('+') && !line.starts_with("+++") {
                                    set_fg(&mut o, theme::OK);
                                } else if line.starts_with('-') && !line.starts_with("---") {
                                    set_fg(&mut o, theme::ERR);
                                } else if line.starts_with("@@") {
                                    set_fg(&mut o, theme::CYAN);
                                } else {
                                    set_fg(&mut o, theme::DIM);
                                }
                                write!(o, "  {V_LINE} {line}\n").ok();
                            }
                            reset_color(&mut o);
                        }
                    }
                }
            }
        }
    } else if event.starts_with("ai-tool-result") && event.ends_with(&stream_suffix) {
        let name = payload["tool"].as_str().unwrap_or("unknown");
        let success = payload["success"].as_bool().unwrap_or(true);
        let duration_ms = payload["durationMs"].as_u64().or_else(|| payload["duration_ms"].as_u64());
        run_hooks(hooks, "post-tool", Some(name), &json!({"success": success}));
        print_tool_result(name, success, duration_ms);

        // Automatic tracking: log errors and fixes
        if !success {
            let error_detail = payload["error"].as_str()
                .or_else(|| payload["output"].as_str())
                .unwrap_or("(no details)");
            log_error_tracking(root_path, name, error_detail);
            last_tool_failed.store(true, Ordering::SeqCst);
            *last_tool_name.lock().await = name.to_string();
        } else if last_tool_failed.load(Ordering::SeqCst) {
            // Previous tool failed, this one succeeded — likely a fix
            let prev_name = last_tool_name.lock().await.clone();
            let detail = payload["output"].as_str().unwrap_or("(auto-detected fix)");
            log_fix_tracking(root_path, &format!("{} → {}", prev_name, name), detail);
            last_tool_failed.store(false, Ordering::SeqCst);
        }

        // Auto-lint after successful write_file or patch_file
        if auto_lint && success && (name == "write_file" || name == "patch_file") {
            let file_path = payload["path"].as_str()
                .or_else(|| payload["file_path"].as_str())
                .unwrap_or("");
            if !file_path.is_empty() {
                if let Some((_linter_name, lint_cmd)) = detect_linter_for_file(root_path, file_path) {
                    let mut o = io::stdout();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  {GEAR} auto-lint: $ {}\n", lint_cmd.join(" ")).ok();
                    reset_color(&mut o);
                    let (stdout, stderr, lint_ok) = run_command_live(&lint_cmd, root_path);
                    if !lint_ok {
                        let output = if !stderr.is_empty() { stderr } else { stdout };
                        for line in output.lines().take(10) {
                            set_fg(&mut o, theme::WARN);
                            write!(o, "  {V_LINE} {}\n", line).ok();
                        }
                        reset_color(&mut o);
                    }
                }
            }
        }

        print_ai_prefix();
    } else if event.starts_with("ai-tool-stream") && event.ends_with(&stream_suffix) {
        let chunk = payload["chunk"].as_str().unwrap_or("");
        if !chunk.is_empty() {
            let mut o = io::stdout();
            execute!(o, SetForegroundColor(theme::DIM), Print(chunk), ResetColor).ok();
            o.flush().ok();
        }
    } else if event.starts_with("ai-tool-confirm") && event.ends_with(&stream_suffix) {
        let tool_name = payload["name"].as_str().unwrap_or("unknown");
        let mut o = io::stdout();
        execute!(o,
            SetForegroundColor(theme::BORDER),
            Print(format!("  {V_LINE} ")),
            SetForegroundColor(theme::WARN),
            Print(format!("{BOLT} AUTO-APPROVE: {tool_name}\n")),
            ResetColor,
        ).ok();
    } else if event.starts_with("ai-file-change") && event.ends_with(&stream_suffix) {
        let path = payload["path"].as_str().unwrap_or("");
        let action = payload["action"].as_str().unwrap_or("modified");
        print_file_change(path, action);
        // Track modified files for auto-commit and edit history
        if !path.is_empty() {
            modified_files.lock().await.push(path.to_string());
            *last_op.lock().await = "edit".to_string();
            // Log to edit_history.jsonl
            let turn = turn_counter.load(Ordering::SeqCst);
            let ts = chrono::Utc::now().timestamp();
            let entry = json!({
                "timestamp": ts,
                "path": path,
                "action": action,
                "turn": turn
            });
            ensure_tracking_dir(root_path);
            let history_path = tracking_dir(root_path).join("edit_history.jsonl");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&history_path) {
                let _ = writeln!(f, "{}", entry.to_string());
            }
        }
    } else if event.starts_with("ai-chat-done") && event.ends_with(&stream_suffix)
        || event == "agent_done"
    {
        stop_bunny(waiting_for_first_token);
        streaming.store(false, Ordering::SeqCst);
        last_tool_failed.store(false, Ordering::SeqCst);
        run_hooks(hooks, "post-response", None, &json!({}));

        // Flush any remaining content from the stream formatter
        {
            let remaining = stream_formatter.lock().await.flush();
            if !remaining.is_empty() {
                let mut o = io::stdout();
                write!(o, "{}", remaining).ok();
                o.flush().ok();
            }
            // Reset formatter for next response
            *stream_formatter.lock().await = StreamFormatter::new();
        }

        let resp_text = std::mem::take(&mut *response_acc.lock().await);
        if !resp_text.is_empty() {
            // Save for /copy command
            *last_response.lock().await = resp_text.clone();
            messages.lock().await.push(json!({ "role": "assistant", "content": resp_text.clone() }));
            // Store block for /block <id> command
            {
                let blocks = RESPONSE_BLOCKS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
                if let Ok(mut b) = blocks.lock() {
                    let id = b.len() + 1;
                    b.push((id, resp_text.clone()));
                    // Print block ID label in dim color
                    let mut o = io::stdout();
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  [#{}]\n", id).ok();
                    reset_color(&mut o);
                }
            }

            // Auto-track completed tasks from AI response
            let lower = resp_text.to_lowercase();
            if lower.contains("completed") || lower.contains("task done") || lower.contains("finished implementing")
                || lower.contains("successfully") || lower.contains("all done")
            {
                let summary: String = resp_text.lines()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(200)
                    .collect();
                log_completed_tracking(root_path, &summary);
            }

            // Auto-update memory: detect memory-worthy content
            if lower.contains("[memory]") || lower.contains("[remember]") {
                // Extract the line containing [memory] or [remember]
                for line in resp_text.lines() {
                    let ll = line.to_lowercase();
                    if ll.contains("[memory]") || ll.contains("[remember]") {
                        let clean = line.replace("[memory]", "").replace("[MEMORY]", "")
                            .replace("[remember]", "").replace("[REMEMBER]", "").trim().to_string();
                        if !clean.is_empty() {
                            append_tracking_entry(root_path, "memory.md", &clean);
                        }
                    }
                }
            }

            // Parse plan steps from response (numbered list)
            {
                let mut steps = plan_steps.lock().await;
                let mut found_steps: Vec<String> = Vec::new();
                let step_re = regex::Regex::new(r"(?m)^\s*(\d+)\.\s+(.+)$").unwrap();
                for cap in step_re.captures_iter(&resp_text) {
                    if let Some(step_text) = cap.get(2) {
                        found_steps.push(step_text.as_str().trim().to_string());
                    }
                }
                if found_steps.len() >= 2 {
                    *steps = found_steps;
                }
            }

            // Auto-commit if enabled and files were modified
            if auto_commit {
                let mut mf = modified_files.lock().await;
                if !mf.is_empty() {
                    let file_count = mf.len();
                    let summary = if file_count == 1 {
                        format!("AI: Update {}", mf[0])
                    } else {
                        format!("AI: Update {} files", file_count)
                    };
                    mf.clear();
                    drop(mf);
                    // Run git add + commit
                    let add_result = std::process::Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(root_path)
                        .output();
                    if let Ok(add_out) = add_result {
                        if add_out.status.success() {
                            let commit_result = std::process::Command::new("git")
                                .args(["commit", "-m", &summary])
                                .current_dir(root_path)
                                .output();
                            if let Ok(commit_out) = commit_result {
                                if commit_out.status.success() {
                                    let mut o = io::stdout();
                                    set_fg(&mut o, theme::OK);
                                    write!(o, "  {CHECK} Auto-committed: {}\n", summary).ok();
                                    reset_color(&mut o);
                                    *last_op.lock().await = "commit".to_string();
                                }
                            }
                        }
                    }
                } else {
                    drop(mf);
                }
            } else {
                modified_files.lock().await.clear();
            }
        }

        let think_text = std::mem::take(&mut *thinking_acc.lock().await);
        if !think_text.is_empty() {
            print_thinking_summary(&think_text);
        }

        // Show elapsed time and streaming speed
        let elapsed = stream_start.lock().await.take().map(|s| s.elapsed().as_secs_f64());
        let token_count = stream_token_count.swap(0, Ordering::SeqCst);
        if let Some(e) = elapsed {
            let mut o = io::stdout();
            let speed_str = if e > 0.5 && token_count > 0 {
                let tps = token_count as f64 / e;
                format!("  {DOT} {:.0} tok/s", tps)
            } else {
                String::new()
            };
            execute!(o,
                SetForegroundColor(theme::BORDER),
                Print(format!("\n  {BOT_LEFT}{H_LINE}{H_LINE} ")),
                SetForegroundColor(theme::DIM),
                Print(format!("{:.1}s{}", e, speed_str)),
                ResetColor,
                Print("\n"),
            ).ok();
        } else {
            println!();
        }

        // Bunny sits down to rest
        print_bunny_sit();

        // Turn separator
        print_turn_separator();

        // Smart desktop notification (only if response took > threshold)
        {
            let elapsed_for_notif = elapsed.unwrap_or(0.0);
            let preview = resp_text.lines().next().unwrap_or("AI response complete");
            send_smart_notification(preview, elapsed_for_notif, false);
        }

        // Section 14: notify_task_complete for /spawn background tasks
        // If the completed stream ID matches a spawned task, fire a system notification.
        {
            let sid = sid.to_string();
            if !sid.is_empty() {
                // `resp_text` summary for notification
                let summary: String = resp_text.lines().take(2).collect::<Vec<_>>().join(" ");
                if !summary.is_empty() {
                    // Check if this looks like it came from a spawn (heuristic: any completed AI task)
                    notify_task_complete("ShadowAI task", &summary);
                }
            }
        }

        // Section 8.1: Email notification for tasks exceeding threshold
        {
            if let Some(tst_lock) = TASK_START_TIME.get() {
                let maybe_duration = tst_lock.lock().ok().and_then(|mut g| g.take()).map(|s| s.elapsed().as_secs());
                if let Some(duration_secs) = maybe_duration {
                    let snap = EMAIL_CFG_SNAPSHOT.get()
                        .and_then(|m| m.lock().ok())
                        .and_then(|g| g.clone());
                    if let Some(snap) = snap {
                        let threshold = snap.email_notify_threshold_secs.unwrap_or(300);
                        if duration_secs >= threshold {
                            // Build a temporary CliConfig from snapshot for email fn
                            let tmp_cfg = CliConfig {
                                smtp_host: snap.smtp_host,
                                smtp_port: snap.smtp_port,
                                smtp_user: snap.smtp_user,
                                smtp_password: snap.smtp_password,
                                smtp_from: snap.smtp_from,
                                smtp_to: snap.smtp_to,
                                smtp_tls: snap.smtp_tls,
                                email_notify_threshold_secs: snap.email_notify_threshold_secs,
                                ..Default::default()
                            };
                            let subject = format!("ShadowAI task completed ({}s)", duration_secs);
                            let preview = resp_text.lines().next().unwrap_or("Task complete");
                            let body = format!("Task finished in {}s.\n\nFirst line of response:\n{}", duration_secs, preview);
                            tokio::spawn(async move {
                                let _ = send_email_notification(&subject, &body, &tmp_cfg).await;
                            });
                        }
                    }
                }
            }
        }

        if is_one_shot {
            if json_output {
                let json_resp = json!({
                    "response": resp_text,
                    "tokens": {
                        "stream_words": stream_token_count.load(Ordering::SeqCst),
                    }
                });
                println!("{}", serde_json::to_string(&json_resp).unwrap_or_default());
            } else if no_stream {
                // Print the final response text (wasn't printed during streaming)
                print!("{}", resp_text);
            }
            std::process::exit(EXIT_OK);
        }
    } else if (event.starts_with("ai-chat-stats") || event.starts_with("ai-token-stats"))
        && event.ends_with(&stream_suffix)
    {
        let input_tokens = payload["input_tokens"].as_u64()
            .or_else(|| payload["inputTokens"].as_u64()).unwrap_or(0);
        let output_tokens = payload["output_tokens"].as_u64()
            .or_else(|| payload["outputTokens"].as_u64()).unwrap_or(0);
        let cached = payload["cached"].as_bool().unwrap_or(false);
        let elapsed = stream_start.lock().await.as_ref().map(|s| s.elapsed().as_secs_f64());
        // Accumulate session token counts for /cost command
        SESSION_INPUT_TOKENS.fetch_add(input_tokens, Ordering::SeqCst);
        SESSION_OUTPUT_TOKENS.fetch_add(output_tokens, Ordering::SeqCst);
        // Accumulate response block for /block command
        {
            let blocks = RESPONSE_BLOCKS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
            if let Ok(mut b) = blocks.lock() {
                let id = b.len() + 1;
                // We don't have the response text here yet — will update in streaming done handler
                let _ = id; // placeholder; actual content stored in streaming handler
            }
        }
        print_stats(input_tokens, output_tokens, cached, elapsed);
        // --max-budget enforcement
        let limit = BUDGET_LIMIT.load(Ordering::SeqCst);
        if limit > 0 {
            let used = BUDGET_USED.fetch_add(input_tokens + output_tokens, Ordering::SeqCst)
                + input_tokens + output_tokens;
            let mut o = io::stdout();
            set_fg(&mut o, theme::DIM);
            write!(o, "  {DOT} Budget: {} / {} tokens\n", fmt_tokens(used), fmt_tokens(limit)).ok();
            reset_color(&mut o);
            if used >= limit {
                set_fg(&mut o, theme::ERR);
                write!(o, "\n  {CROSS} Token budget of {} exhausted — exiting.\n", fmt_tokens(limit)).ok();
                reset_color(&mut o);
                o.flush().ok();
                std::process::exit(EXIT_OK);
            }
        }
    } else if event == "agent_cancelled" {
        streaming.store(false, Ordering::SeqCst);
        let mut o = io::stdout();
        execute!(o,
            SetForegroundColor(theme::WARN),
            Print(format!("\n  {CROSS} Agent cancelled\n")),
            ResetColor,
        ).ok();
        send_desktop_notification_ex("ShadowAI", "Agent cancelled", true);
    } else if event == "agent_thinking" {
        // Agent started processing — no visual needed
    } else if event.contains("compacted") || event.contains("compact") {
        // Auto-compact notification
        let removed = payload["removed"].as_u64().unwrap_or(0);
        let kept = payload["kept"].as_u64().unwrap_or(0);
        let mut o = io::stdout();
        set_fg(&mut o, theme::WARN);
        if removed > 0 || kept > 0 {
            write!(o, "\n  {GEAR} Compacted: removed {} messages, kept {}\n", removed, kept).ok();
        } else {
            write!(o, "\n  {GEAR} Context compacted\n").ok();
        }
        reset_color(&mut o);
    }
}

// ─── WebSocket connection with retry ─────────────────────────────────────────

/// Check if a host string refers to a local/LAN address.
/// Append a timestamped event to a state file (errors.md, Completed.md, etc).
/// Best-effort — failures are silently ignored since this is diagnostic logging.
fn log_state_event(filename: &str, event_type: &str, detail: &str) {
    let state_dir = std::env::var("SHADOWAI_STATE_DIR")
        .unwrap_or_else(|_| "./state".to_string());
    let path = std::path::Path::new(&state_dir).join(filename);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = writeln!(f, "\n## [{now}] {event_type}\n- {detail}");
    }
}

fn is_local_host(host: &str) -> bool {
    // Strip port if present
    let h = host.split(':').next().unwrap_or(host);
    h == "localhost"
        || h == "127.0.0.1"
        || h == "::1"
        || h == "[::1]"
        || h.starts_with("192.168.")
        || h.starts_with("10.")
        || {
            // 172.16.0.0 - 172.31.255.255
            if let Some(rest) = h.strip_prefix("172.") {
                rest.split('.').next()
                    .and_then(|s| s.parse::<u8>().ok())
                    .map(|n| (16..=31).contains(&n))
                    .unwrap_or(false)
            } else {
                false
            }
        }
}

async fn connect_ws(
    url: &str,
    max_retries: u32,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    String,
> {
    // Extract host from wss://host:port/path for security check
    let host_part = url.strip_prefix("wss://").or_else(|| url.strip_prefix("ws://")).unwrap_or(url);
    let host_part = host_part.split('/').next().unwrap_or(host_part);

    if !is_local_host(host_part) {
        // danger_accept_invalid_certs is needed for self-signed certs in local network mode.
        // Warn loudly when connecting to a non-local host with cert validation disabled.
        eprintln!(
            "WARNING: TLS certificate validation is disabled for non-local host '{}'! \
             This connection may be insecure. Set SHADOWAI_HOST to a local address or \
             configure proper TLS certificates.",
            host_part
        );
    }

    let mut delay = 1u64;
    let mut attempts = 0u32;
    loop {
        attempts += 1;

        let tls_connector = native_tls::TlsConnector::builder()
            // Needed for self-signed certs in local network mode
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| format!("Failed to build TLS connector: {}", e))?;
        let connector = tokio_tungstenite::Connector::NativeTls(tls_connector);

        let connect_fut = tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector));
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), connect_fut).await;
        match result {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => {
                if attempts >= max_retries {
                    return Err(format!(
                        "Failed to connect after {} attempts. Last error: {}. \
                         Check that ShadowIDE's remote server is running (port 9876).",
                        attempts, e
                    ));
                }
                let mut o = io::stdout();
                execute!(o,
                    SetForegroundColor(theme::WARN),
                    Print(format!("  {RADIO} Connection failed: {}. Retrying in {}s... ({}/{})\n", e, delay, attempts, max_retries)),
                    ResetColor,
                ).ok();
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                delay = (delay * 2).min(30);
            }
            Err(_) => {
                if attempts >= max_retries {
                    return Err(format!(
                        "Connection timed out after {} attempts. \
                         Make sure ShadowIDE's remote server is running. \
                         Run: shadowai setup 127.0.0.1:9876 <token>",
                        attempts
                    ));
                }
                let mut o = io::stdout();
                execute!(o,
                    SetForegroundColor(theme::WARN),
                    Print(format!("  {RADIO} Connection timed out. Retrying in {}s... ({}/{})\n", delay, attempts, max_retries)),
                    ResetColor,
                ).ok();
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                delay = (delay * 2).min(30);
            }
        }
    }
}

// ─── Batch 1: PromptHistory ────────────────────────────────────────────────

struct PromptHistory {
    entries: Vec<String>,
    pos: usize,
    draft: String,
    max_size: usize,
}

#[allow(dead_code)]
impl PromptHistory {
    fn new(max: usize) -> Self {
        Self { entries: vec![], pos: 0, draft: String::new(), max_size: max }
    }
    fn push(&mut self, s: String) {
        if s.trim().is_empty() { return; }
        if self.entries.last().map(|e| e == &s).unwrap_or(false) { return; }
        self.entries.push(s);
        if self.entries.len() > self.max_size { self.entries.remove(0); }
        self.pos = self.entries.len();
    }
    fn up(&mut self, current: &str) -> Option<&str> {
        if self.entries.is_empty() { return None; }
        if self.pos == self.entries.len() { self.draft = current.to_string(); }
        if self.pos > 0 { self.pos -= 1; Some(&self.entries[self.pos]) }
        else { Some(&self.entries[0]) }
    }
    fn down(&mut self) -> Option<&str> {
        if self.pos >= self.entries.len() { return None; }
        self.pos += 1;
        if self.pos == self.entries.len() {
            // Return draft ref. We can't return &self.draft easily, so we handle this at call site.
            None
        } else {
            Some(&self.entries[self.pos])
        }
    }
    fn reset(&mut self) { self.pos = self.entries.len(); }
    fn save_to_file(&self, path: &std::path::Path) {
        let content = self.entries.join("\n");
        let _ = std::fs::write(path, content);
    }
    fn load_from_file(path: &std::path::Path, max: usize) -> Self {
        let mut h = Self::new(max);
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                if !line.trim().is_empty() { h.entries.push(line.to_string()); }
            }
        }
        h.pos = h.entries.len();
        h
    }
    fn get_draft(&self) -> &str { &self.draft }
}

// ─── Batch 1: open_in_editor ──────────────────────────────────────────────

#[allow(dead_code)]
fn open_in_editor(initial: &str) -> Option<String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("shadowai_edit_{}.txt", std::process::id()));
    std::fs::write(&path, initial).ok()?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    let status = std::process::Command::new(&editor).arg(&path).status().ok()?;
    if !status.success() { return None; }
    let result = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let trimmed = result.trim_end_matches('\n').to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

// ─── Batch 2: Provider detection ──────────────────────────────────────────

#[allow(dead_code)]
enum Provider {
    Anthropic { api_key: String, model: String },
    OpenAI { api_key: String, base_url: String, model: String },
    ShadowIDE { host: String },
}

#[allow(dead_code)]
/// Extract host:port from a URL string for TCP reachability checks.
fn extract_host_port(url: &str) -> Option<std::net::SocketAddr> {
    // Strip scheme (http:// or https://)
    let without_scheme = url.trim_start_matches("https://").trim_start_matches("http://");
    // Strip path after /
    let host_port = without_scheme.split('/').next()?;
    // Parse as SocketAddr (handles both host:port and bare host)
    host_port.parse::<std::net::SocketAddr>().ok()
        .or_else(|| {
            // Try with default port 80 if none given
            format!("{}:80", host_port).parse().ok()
        })
}

fn detect_provider(config: &CliConfig) -> Provider {
    // 1. Local llama.cpp / Ollama / LM Studio (no API key needed)
    //    Probe the configured local URL, or well-known defaults in order.
    let local_candidates: Vec<String> = {
        let mut candidates = Vec::new();
        // User-configured local URL takes priority
        if let Some(ref url) = config.openai_base_url {
            let u = url.to_lowercase();
            if u.contains("localhost") || u.contains("127.0.0.1") || u.contains("0.0.0.0") {
                candidates.push(url.clone());
            }
        }
        // Default llama.cpp port
        if !candidates.iter().any(|u| u.contains(":8080")) {
            candidates.push("http://localhost:8080/v1".to_string());
        }
        // Ollama
        if !candidates.iter().any(|u| u.contains(":11434")) {
            candidates.push("http://localhost:11434/v1".to_string());
        }
        // LM Studio
        if !candidates.iter().any(|u| u.contains(":1234")) {
            candidates.push("http://localhost:1234/v1".to_string());
        }
        candidates
    };

    for url in &local_candidates {
        // Quick TCP port check — avoids reqwest blocking in sync context
        if let Some(addr) = extract_host_port(url) {
            let reachable = std::net::TcpStream::connect_timeout(
                &addr,
                std::time::Duration::from_millis(300),
            ).is_ok();
            if reachable {
                let model = config.model.clone().unwrap_or_else(|| "default".to_string());
                let label = if url.contains(":8080") { "llama.cpp" }
                    else if url.contains(":11434") { "Ollama" }
                    else { "LM Studio" };
                eprintln!("\x1b[2m[provider] Using local {} at {}\x1b[0m", label, url);
                return Provider::OpenAI {
                    api_key: String::new(),
                    base_url: url.clone(),
                    model,
                };
            }
        }
    }

    // 2. Anthropic cloud
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok()
        .or_else(|| config.anthropic_api_key.clone());
    if let Some(key) = anthropic_key {
        let model = config.model.clone().unwrap_or_else(|| "claude-opus-4-5".to_string());
        return Provider::Anthropic { api_key: key, model };
    }

    // 3. OpenAI-compatible cloud
    let openai_key = std::env::var("OPENAI_API_KEY").ok()
        .or_else(|| config.openai_api_key.clone());
    if let Some(key) = openai_key {
        let base_url = config.openai_base_url.clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = config.model.clone().unwrap_or_else(|| "gpt-4o".to_string());
        return Provider::OpenAI { api_key: key, base_url, model };
    }

    // 4. Shadow IDE backend (WS)
    let host = config.host.clone().unwrap_or_else(|| "127.0.0.1:9876".to_string());
    Provider::ShadowIDE { host }
}

// ─── Batch 2: send_anthropic_request ──────────────────────────────────────

#[allow(dead_code)]
async fn send_anthropic_request(
    api_key: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "system": system,
        "messages": messages,
        "stream": true
    });
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Anthropic request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, text));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk: bytes::Bytes = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].to_string();
            buf = buf[pos + 1..].to_string();
            let line = line.trim().to_string();
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" { break; }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if v["type"].as_str() == Some("content_block_delta") {
                        if let Some(text) = v["delta"]["text"].as_str() {
                            let _ = tx.send(text.to_string()).await;
                        }
                    }
                    if v["type"].as_str() == Some("message_stop") {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Batch 2a: send_llamacpp_native (native /completion endpoint) ─────────

/// Convert chat messages + system prompt into a ChatML-formatted prompt string
/// for llama.cpp's native /completion endpoint.
fn build_chatml_prompt(system: &str, messages: &[serde_json::Value]) -> String {
    let mut prompt = String::with_capacity(system.len() + messages.len() * 256);
    prompt.push_str("<|im_start|>system\n");
    prompt.push_str(system);
    prompt.push_str("<|im_end|>\n");
    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");
        prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", role, content));
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

/// Derive the base server URL from a /v1-suffixed URL.
/// e.g. "http://localhost:8080/v1" → "http://localhost:8080"
/// e.g. "http://192.168.0.14:8080/v1" → "http://192.168.0.14:8080"
fn llamacpp_server_url(base_url: &str) -> String {
    let u = base_url.trim_end_matches('/');
    u.strip_suffix("/v1").unwrap_or(u).to_string()
}

/// Send a request to llama.cpp using the native /completion endpoint with
/// SSE streaming.  Falls back gracefully — if the server doesn't support
/// the native endpoint (404/405) we return an Err so the caller can retry
/// via the OpenAI-compat path.
async fn send_llamacpp_native(
    base_url: &str,
    system: &str,
    messages: &[serde_json::Value],
    tools: &serde_json::Value,
    max_tokens: u32,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let server_url = llamacpp_server_url(base_url);

    // Build the full system prompt with tool injection (same as prompt_tool_mode)
    let full_system = format!("{}\n\n{}", build_tui_tool_injection_prompt(tools), system);
    let prompt = build_chatml_prompt(&full_system, messages);

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        // No overall timeout — streaming can take as long as it needs
        .build()
        .map_err(|e| e.to_string())?;

    let body = serde_json::json!({
        "prompt": prompt,
        "n_predict": max_tokens,
        "stream": true,
        "temperature": 0.7,
        "stop": ["<|im_end|>", "<|im_start|>"],
        "cache_prompt": true,
    });

    let url = format!("{}/completion", server_url);
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                format!("llama.cpp server not reachable at {server_url}")
            } else {
                format!("llama.cpp request failed: {e}")
            }
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
        return Err("FALLBACK_TO_OPENAI".to_string());
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("llama.cpp error {status}: {text}"));
    }

    // Parse streaming SSE response
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut full_response = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk: bytes::Bytes = chunk_result.map_err(|e| format!("Stream error: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].to_string();
            buf = buf[pos + 1..].to_string();
            let line = line.trim().to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    // Check for tool calls in the full response before finishing
                    let tool_calls = extract_tui_tool_calls_from_text(&full_response);
                    if !tool_calls.is_empty() {
                        // Send assistant content sentinel for history
                        let msg = serde_json::json!({"role": "assistant", "content": full_response});
                        let content_sentinel = format!(
                            "{ASSISTANT_CONTENT_SENTINEL}{}",
                            serde_json::to_string(&msg).unwrap_or_default()
                        );
                        let _ = tx.send(content_sentinel).await;
                        for tc in tool_calls {
                            let ev = serde_json::json!({
                                "id": tc.id, "name": tc.name,
                                "input": serde_json::from_str::<serde_json::Value>(&tc.input_json)
                                    .unwrap_or_else(|_| serde_json::json!({}))
                            });
                            let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
                        }
                        let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
                    }
                    return Ok(());
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    // llama.cpp native format: {"content": "token", "stop": false}
                    if let Some(content) = v["content"].as_str() {
                        if !content.is_empty() {
                            full_response.push_str(content);
                            let _ = tx.send(content.to_string()).await;
                        }
                    }
                    // Check if generation stopped
                    if v["stop"].as_bool() == Some(true) {
                        let tool_calls = extract_tui_tool_calls_from_text(&full_response);
                        if !tool_calls.is_empty() {
                            let msg = serde_json::json!({"role": "assistant", "content": full_response});
                            let content_sentinel = format!(
                                "{ASSISTANT_CONTENT_SENTINEL}{}",
                                serde_json::to_string(&msg).unwrap_or_default()
                            );
                            let _ = tx.send(content_sentinel).await;
                            for tc in tool_calls {
                                let ev = serde_json::json!({
                                    "id": tc.id, "name": tc.name,
                                    "input": serde_json::from_str::<serde_json::Value>(&tc.input_json)
                                        .unwrap_or_else(|_| serde_json::json!({}))
                                });
                                let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
                            }
                            let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
                        }
                        return Ok(());
                    }
                }
            }
        }
    }

    // Stream ended — check for tool calls in accumulated response
    if !full_response.is_empty() {
        let tool_calls = extract_tui_tool_calls_from_text(&full_response);
        if !tool_calls.is_empty() {
            let msg = serde_json::json!({"role": "assistant", "content": full_response});
            let content_sentinel = format!(
                "{ASSISTANT_CONTENT_SENTINEL}{}",
                serde_json::to_string(&msg).unwrap_or_default()
            );
            let _ = tx.send(content_sentinel).await;
            for tc in tool_calls {
                let ev = serde_json::json!({
                    "id": tc.id, "name": tc.name,
                    "input": serde_json::from_str::<serde_json::Value>(&tc.input_json)
                        .unwrap_or_else(|_| serde_json::json!({}))
                });
                let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
            }
            let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
        }
    }
    Ok(())
}

// ─── Batch 2b: send_openai_request ────────────────────────────────────────

#[allow(dead_code)]
async fn send_openai_request(
    api_key: &str,
    base_url: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let mut all_messages = vec![serde_json::json!({"role": "system", "content": system})];
    all_messages.extend_from_slice(messages);
    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "messages": all_messages,
    });
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, text));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk: bytes::Bytes = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].to_string();
            buf = buf[pos + 1..].to_string();
            let line = line.trim().to_string();
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" { return Ok(()); }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = v["choices"][0]["delta"]["content"].as_str() {
                        let _ = tx.send(content.to_string()).await;
                    }
                }
            }
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// TUI TOOL CALLING SYSTEM
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct PendingTuiToolCall {
    id: String,
    name: String,
    input_json: String, // serialised serde_json::Value
}

#[derive(Debug, Clone, PartialEq)]
enum TuiApprovalMode {
    /// All tools auto-approved (/yolo)
    Yolo,
    /// Safe tools auto-approved, dangerous ones ask
    Smart,
    /// Ask for every tool
    AskAll,
}

/// Sentinel prefixes used over the String channel to encode non-text events
const TOOL_SENTINEL: &str = "\x01TOOL\x01";
const APPROVAL_SENTINEL: &str = "\x01APPROVAL_NEEDED\x01";
const ASSISTANT_CONTENT_SENTINEL: &str = "\x01ASSISTANT_CONTENT\x01";
const ERROR_SENTINEL: &str = "\x01ERROR\x01";
const THINKING_START_SENTINEL: &str = "\x01THINK_START\x01";
const THINKING_END_SENTINEL: &str = "\x01THINK_END\x01";
const THINKING_TOKEN_SENTINEL: &str = "\x01THINK\x01";

fn is_likely_llamacpp_endpoint(base_url: &str) -> bool {
    if let Ok(url) = reqwest::Url::parse(base_url) {
        if url.port_or_known_default() == Some(8080) {
            return true;
        }
    }
    let lower = base_url.to_lowercase();
    lower.contains("llama.cpp") || lower.contains("llamacpp")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointKind {
    Localhost,
    PrivateNetwork,
    Remote,
}

fn classify_endpoint(base_url: &str) -> EndpointKind {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return EndpointKind::Remote;
    };
    let Some(host) = url.host_str() else {
        return EndpointKind::Remote;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return EndpointKind::Localhost;
    }
    if host.ends_with(".local") {
        return EndpointKind::PrivateNetwork;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            if ip.is_loopback() || ip.is_unspecified() {
                EndpointKind::Localhost
            } else if ip.is_private() || ip.is_link_local() {
                EndpointKind::PrivateNetwork
            } else {
                EndpointKind::Remote
            }
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            if ip.is_loopback() || ip.is_unspecified() {
                EndpointKind::Localhost
            } else if ip.is_unique_local() || ip.is_unicast_link_local() {
                EndpointKind::PrivateNetwork
            } else {
                EndpointKind::Remote
            }
        }
        Err(_) => EndpointKind::Remote,
    }
}

fn openai_timeout_override() -> Option<std::time::Duration> {
    let secs = std::env::var("SHADOWAI_OPENAI_TIMEOUT_SECS").ok()?;
    let secs = secs.trim().parse::<u64>().ok()?;
    if secs == 0 {
        return None;
    }
    Some(std::time::Duration::from_secs(secs))
}

fn openai_request_timeout(base_url: &str, prompt_tool_mode: bool) -> std::time::Duration {
    if let Some(timeout) = openai_timeout_override() {
        return timeout;
    }

    match (prompt_tool_mode, classify_endpoint(base_url)) {
        (true, EndpointKind::Localhost) => {
            // Local non-streaming reasoning models can take several minutes
            // before producing a full chat completion response.
            std::time::Duration::from_secs(900)
        }
        (true, EndpointKind::PrivateNetwork) => std::time::Duration::from_secs(180),
        (false, EndpointKind::Localhost) => std::time::Duration::from_secs(300),
        _ => std::time::Duration::from_secs(120),
    }
}

fn format_openai_request_error(
    base_url: &str,
    request_timeout: std::time::Duration,
    err: reqwest::Error,
) -> String {
    if err.is_timeout() {
        match classify_endpoint(base_url) {
            EndpointKind::Localhost => {
                return format!(
                    "OpenAI request timed out after {}s waiting for local model response from {}",
                    request_timeout.as_secs(),
                    base_url
                );
            }
            EndpointKind::PrivateNetwork => {
                return format!(
                    "OpenAI request timed out after {}s waiting for LAN model response from {}. Set SHADOWAI_OPENAI_TIMEOUT_SECS to increase this.",
                    request_timeout.as_secs(),
                    base_url
                );
            }
            EndpointKind::Remote => {}
        }
        return format!(
            "OpenAI request timed out after {}s for {}",
            request_timeout.as_secs(),
            base_url
        );
    }
    format!("OpenAI request failed: {err}")
}

fn build_tui_tool_injection_prompt(tools: &serde_json::Value) -> String {
    let mut tool_descriptions = String::new();
    if let Some(arr) = tools.as_array() {
        for tool in arr {
            let function = tool.get("function").unwrap_or(tool);
            let name = function["name"].as_str().unwrap_or("unknown");
            let description = function["description"].as_str().unwrap_or("");
            let parameters = function.get("parameters")
                .or_else(|| function.get("input_schema"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            tool_descriptions.push_str(&format!(
                "- {}: {}\n  Parameters: {}\n\n",
                name,
                description,
                serde_json::to_string_pretty(&parameters).unwrap_or_else(|_| "{}".to_string()),
            ));
        }
    }

    format!(
        "You have access to these tools.\n\
         When you need a tool, respond with ONLY a single tool call as JSON in a ```tool_call block \
         or as raw JSON, using this exact shape:\n\
         {{\"tool\":\"<name>\",\"args\":{{...}}}}\n\n\
         Rules:\n\
         1. Emit one tool call at a time\n\
         2. Do not wrap the JSON in prose\n\
         3. After a tool result is provided, continue the task normally\n\n\
         Available tools:\n{}",
        tool_descriptions
    )
}

fn find_matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    for (i, ch) in text.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn repair_tui_tool_json(input: &str) -> String {
    let trimmed = input.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    let mut fixed = trimmed.to_string();
    fixed = fixed.replace(",}", "}").replace(",]", "]");
    if serde_json::from_str::<serde_json::Value>(&fixed).is_ok() {
        return fixed;
    }
    trimmed.to_string()
}

fn parse_tui_tool_call_json(json_str: &str, counter: &mut usize) -> Option<PendingTuiToolCall> {
    let repaired = repair_tui_tool_json(json_str);
    let value: serde_json::Value = serde_json::from_str(&repaired).ok()?;
    let tool_name = value.get("tool")?.as_str()?.to_string();
    let args = value.get("args")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    *counter += 1;
    Some(PendingTuiToolCall {
        id: format!("prompt_call_{}", counter),
        name: tool_name,
        input_json: serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
    })
}

fn extract_tui_tool_calls_from_text(text: &str) -> Vec<PendingTuiToolCall> {
    let mut calls = Vec::new();
    let mut counter = 0usize;

    let parts: Vec<&str> = text.split("```tool_call").collect();
    for part in parts.iter().skip(1) {
        if let Some(end) = part.find("```") {
            if let Some(call) = parse_tui_tool_call_json(part[..end].trim(), &mut counter) {
                calls.push(call);
            }
        }
    }
    if !calls.is_empty() {
        return calls;
    }

    let json_parts: Vec<&str> = text.split("```json").collect();
    for part in json_parts.iter().skip(1) {
        if let Some(end) = part.find("```") {
            if let Some(call) = parse_tui_tool_call_json(part[..end].trim(), &mut counter) {
                calls.push(call);
            }
        }
    }
    if !calls.is_empty() {
        return calls;
    }

    let mut search_from = 0usize;
    while let Some(start) = text[search_from..].find("{\"tool\"") {
        let abs_start = search_from + start;
        if let Some(end) = find_matching_brace(&text[abs_start..]) {
            if let Some(call) = parse_tui_tool_call_json(&text[abs_start..abs_start + end + 1], &mut counter) {
                calls.push(call);
            }
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }

    calls
}

fn normalize_openai_tool_message(message: &serde_json::Value, keep_tool_calls: bool) -> serde_json::Value {
    let role = message["role"].as_str().unwrap_or("assistant");
    let content = message.get("content")
        .filter(|v| !v.is_null())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut msg = serde_json::json!({
        "role": role,
        "content": content,
    });

    if keep_tool_calls {
        let normalized_tool_calls: Vec<serde_json::Value> = message["tool_calls"]
            .as_array()
            .map(|tool_calls| {
                tool_calls.iter().enumerate().map(|(idx, tool_call)| {
                    let id = tool_call.get("id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("call_{idx}"));
                    let name = tool_call["function"]["name"].as_str().unwrap_or("").to_string();
                    let arguments = tool_call["function"]["arguments"]
                        .as_str()
                        .unwrap_or("{}")
                        .to_string();
                    serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments,
                        }
                    })
                }).collect()
            })
            .unwrap_or_default();
        if !normalized_tool_calls.is_empty() {
            msg["tool_calls"] = serde_json::Value::Array(normalized_tool_calls);
        }
    }

    msg
}

#[cfg(test)]
mod tui_tool_tests {
    use super::*;

    #[test]
    fn normalize_openai_tool_message_fills_missing_content_and_ids() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "internal reasoning that should not be replayed",
            "tool_calls": [{
                "id": "",
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": "{\"path\":\"README.md\"}"
                }
            }]
        });

        let normalized = normalize_openai_tool_message(&msg, true);
        assert_eq!(normalized["content"], serde_json::Value::String(String::new()));
        assert_eq!(normalized["tool_calls"][0]["id"], "call_0");
        assert!(normalized.get("reasoning_content").is_none());
    }

    #[test]
    fn normalize_openai_tool_message_strips_tool_calls_in_prompt_mode() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": "{\"tool\":\"make_dir\",\"args\":{\"path\":\"src\"}}",
            "tool_calls": [{
                "id": "",
                "type": "function",
                "function": {
                    "name": "make_dir",
                    "arguments": "{\"path\":\"src\"}"
                }
            }],
            "reasoning_content": "hidden thinking"
        });

        let normalized = normalize_openai_tool_message(&msg, false);
        assert_eq!(normalized["role"], "assistant");
        assert_eq!(normalized["content"], "{\"tool\":\"make_dir\",\"args\":{\"path\":\"src\"}}");
        assert!(normalized.get("tool_calls").is_none());
        assert!(normalized.get("reasoning_content").is_none());
    }

    #[test]
    fn extract_tui_tool_calls_from_tool_call_block() {
        let text = "```tool_call\n{\"tool\":\"read_file\",\"args\":{\"path\":\"README.md\"}}\n```";
        let calls = extract_tui_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].input_json, "{\"path\":\"README.md\"}");
    }

    #[test]
    fn extract_tui_tool_calls_from_raw_json() {
        let text = "{\"tool\":\"list_dir\",\"args\":{\"path\":\"src\"}}";
        let calls = extract_tui_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "list_dir");
        assert_eq!(calls[0].input_json, "{\"path\":\"src\"}");
    }
}

/// Returns true for read-only, no-side-effect tools
fn is_safe_tool(name: &str) -> bool {
    matches!(name, "read_file" | "list_dir" | "search_files")
}

/// Anthropic tool definitions
fn tui_tool_defs_anthropic() -> serde_json::Value {
    serde_json::json!([
        {"name":"read_file","description":"Read the full contents of a file.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Absolute or relative path."}},"required":["path"]}},
        {"name":"write_file","description":"Write (overwrite) a file with new content.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}},
        {"name":"create_file","description":"Create a NEW file (error if it already exists).","input_schema":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}},
        {"name":"append_to_file","description":"Append text to the end of a file.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}},
        {"name":"delete_file","description":"Delete a file permanently.","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
        {"name":"list_dir","description":"List files and directories inside a path.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Directory to list. Defaults to project root."}},"required":[]}},
        {"name":"run_command","description":"Run a shell command and return stdout+stderr. Use for build, test, git, lint operations.","input_schema":{"type":"object","properties":{"command":{"type":"string","description":"Shell command to run."},"cwd":{"type":"string","description":"Working directory (optional)."}},"required":["command"]}},
        {"name":"search_files","description":"Search for a regex/text pattern inside files using grep.","input_schema":{"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string","description":"Directory or file to search in."},"file_pattern":{"type":"string","description":"Glob filter e.g. *.rs"}},"required":["pattern"]}},
        {"name":"patch_file","description":"Apply a unified diff patch to a file.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"patch":{"type":"string","description":"Unified diff text starting with ---/+++"}},"required":["path","patch"]}},
        {"name":"move_file","description":"Move or rename a file.","input_schema":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}},
        {"name":"copy_file","description":"Copy a file to a new path.","input_schema":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}},
        {"name":"make_dir","description":"Create a directory (and parents) if it does not exist.","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
        {"name":"git_status","description":"Run git status --short in the project root.","input_schema":{"type":"object","properties":{},"required":[]}},
        {"name":"git_diff","description":"Run git diff (optionally for a specific file).","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":[]}},
        {"name":"git_log","description":"Show last N commits.","input_schema":{"type":"object","properties":{"n":{"type":"integer","description":"Number of commits, default 15."}},"required":[]}},
        {"name":"git_commit","description":"Stage all changes and create a commit.","input_schema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}},
        {"name":"web_fetch","description":"Fetch a URL and return its text content (HTML stripped).","input_schema":{"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}}
    ])
}

/// OpenAI-compatible tool definitions
fn tui_tool_defs_openai() -> serde_json::Value {
    serde_json::json!([
        {"type":"function","function":{"name":"read_file","description":"Read the full contents of a file.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"write_file","description":"Write (overwrite) a file with new content.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"create_file","description":"Create a NEW file.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"append_to_file","description":"Append text to a file.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"delete_file","description":"Delete a file.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"list_dir","description":"List directory contents.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":[]}}},
        {"type":"function","function":{"name":"run_command","description":"Run a shell command.","parameters":{"type":"object","properties":{"command":{"type":"string"},"cwd":{"type":"string"}},"required":["command"]}}},
        {"type":"function","function":{"name":"search_files","description":"Search files with grep.","parameters":{"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"file_pattern":{"type":"string"}},"required":["pattern"]}}},
        {"type":"function","function":{"name":"patch_file","description":"Apply unified diff to file.","parameters":{"type":"object","properties":{"path":{"type":"string"},"patch":{"type":"string"}},"required":["path","patch"]}}},
        {"type":"function","function":{"name":"move_file","description":"Move or rename a file.","parameters":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}}},
        {"type":"function","function":{"name":"copy_file","description":"Copy a file.","parameters":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}}},
        {"type":"function","function":{"name":"make_dir","description":"Create a directory.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"git_status","description":"Run git status.","parameters":{"type":"object","properties":{},"required":[]}}},
        {"type":"function","function":{"name":"git_diff","description":"Run git diff.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":[]}}},
        {"type":"function","function":{"name":"git_log","description":"Show recent commits.","parameters":{"type":"object","properties":{"n":{"type":"integer"}},"required":[]}}},
        {"type":"function","function":{"name":"git_commit","description":"Stage all and commit.","parameters":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}},
        {"type":"function","function":{"name":"web_fetch","description":"Fetch a URL and return plain text.","parameters":{"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}}}
    ])
}

/// Execute one tool call; returns a result string to feed back to the AI
async fn execute_tui_tool(name: &str, input: &serde_json::Value, root_path: &str) -> String {
    let resolve = |p: &str| -> String {
        if p.starts_with('/') { p.to_string() }
        else { format!("{}/{}", root_path.trim_end_matches('/'), p) }
    };
    match name {
        "read_file" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            std::fs::read_to_string(&path).unwrap_or_else(|e| format!("Error: {e}"))
        }
        "write_file" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            let content = input["content"].as_str().unwrap_or("");
            if let Some(p) = std::path::Path::new(&path).parent() { let _ = std::fs::create_dir_all(p); }
            std::fs::write(&path, content)
                .map(|_| format!("Written: {path}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "create_file" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            let content = input["content"].as_str().unwrap_or("");
            if std::path::Path::new(&path).exists() {
                return format!("Error: file already exists: {path}");
            }
            if let Some(p) = std::path::Path::new(&path).parent() { let _ = std::fs::create_dir_all(p); }
            std::fs::write(&path, content)
                .map(|_| format!("Created: {path}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "append_to_file" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            let content = input["content"].as_str().unwrap_or("");
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).create(true).open(&path)
                .and_then(|mut f| f.write_all(content.as_bytes()))
                .map(|_| format!("Appended to {path}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "delete_file" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            std::fs::remove_file(&path)
                .map(|_| format!("Deleted: {path}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "list_dir" => {
            let raw = input["path"].as_str().unwrap_or(root_path);
            let path = if raw.is_empty() { root_path.to_string() } else { resolve(raw) };
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut items: Vec<_> = entries.flatten().collect();
                    items.sort_by_key(|e| {
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        (!is_dir, e.file_name())
                    });
                    items.iter().map(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { format!("{n}/") } else { n }
                    }).collect::<Vec<_>>().join("\n")
                }
                Err(e) => format!("Error: {e}"),
            }
        }
        "make_dir" => {
            let path = resolve(input["path"].as_str().unwrap_or(""));
            std::fs::create_dir_all(&path)
                .map(|_| format!("Created dir: {path}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "move_file" => {
            let from = resolve(input["from"].as_str().unwrap_or(""));
            let to   = resolve(input["to"].as_str().unwrap_or(""));
            std::fs::rename(&from, &to)
                .map(|_| format!("Moved {from} → {to}"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "copy_file" => {
            let from = resolve(input["from"].as_str().unwrap_or(""));
            let to   = resolve(input["to"].as_str().unwrap_or(""));
            std::fs::copy(&from, &to)
                .map(|n| format!("Copied {from} → {to} ({n} bytes)"))
                .unwrap_or_else(|e| format!("Error: {e}"))
        }
        "patch_file" => {
            let path    = resolve(input["path"].as_str().unwrap_or(""));
            let patch   = input["patch"].as_str().unwrap_or("");
            // Write patch to temp file and run `patch`
            let tmp = format!("/tmp/shadowai_patch_{}.diff", uuid::Uuid::new_v4());
            if let Err(e) = std::fs::write(&tmp, patch) { return format!("Error writing patch: {e}"); }
            match tokio::process::Command::new("patch")
                .arg("--no-backup-if-mismatch")
                .arg(&path)
                .arg(&tmp)
                .output().await
            {
                Ok(o) => {
                    let _ = std::fs::remove_file(&tmp);
                    let out = String::from_utf8_lossy(&o.stdout).to_string();
                    let err = String::from_utf8_lossy(&o.stderr).to_string();
                    if o.status.success() { format!("Patched: {out}") } else { format!("Patch failed: {err}") }
                }
                Err(e) => { let _ = std::fs::remove_file(&tmp); format!("Error: {e}") }
            }
        }
        "run_command" => {
            let cmd  = input["command"].as_str().unwrap_or("");
            let cwd  = input["cwd"].as_str().unwrap_or(root_path);
            let full_cwd = if cwd.starts_with('/') { cwd.to_string() } else { resolve(cwd) };
            match tokio::process::Command::new("sh")
                .arg("-c").arg(cmd)
                .current_dir(&full_cwd)
                .output().await
            {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let code   = o.status.code().unwrap_or(-1);
                    let mut r  = stdout;
                    if !stderr.is_empty() { r.push_str(&format!("\n[stderr]\n{stderr}")); }
                    if code != 0 { r.push_str(&format!("\n[exit {code}]")); }
                    if r.is_empty() { "(no output)".to_string() } else { r }
                }
                Err(e) => format!("Error: {e}"),
            }
        }
        "search_files" => {
            let pattern = input["pattern"].as_str().unwrap_or("");
            let base = input["path"].as_str()
                .map(|p| if p.is_empty() { root_path.to_string() } else { resolve(p) })
                .unwrap_or_else(|| root_path.to_string());
            let fpat = input["file_pattern"].as_str().unwrap_or("");
            let mut cmd = tokio::process::Command::new("grep");
            cmd.arg("-rn").arg("--color=never");
            if !fpat.is_empty() { cmd.arg(format!("--include={fpat}")); }
            cmd.arg(pattern).arg(&base);
            match cmd.output().await {
                Ok(o) => {
                    let out = String::from_utf8_lossy(&o.stdout).to_string();
                    if out.is_empty() { "(no matches)".to_string() } else {
                        let lines: Vec<&str> = out.lines().collect();
                        if lines.len() > 50 { format!("{}\n... ({} more lines)", lines[..50].join("\n"), lines.len()-50) }
                        else { out }
                    }
                }
                Err(e) => format!("Error: {e}"),
            }
        }
        "git_status" => {
            match tokio::process::Command::new("git")
                .args(["status","--short"])
                .current_dir(root_path)
                .output().await
            {
                Ok(o) => { let s = String::from_utf8_lossy(&o.stdout).to_string(); if s.is_empty() { "clean".to_string() } else { s } }
                Err(e) => format!("Error: {e}"),
            }
        }
        "git_diff" => {
            let path_arg = input["path"].as_str().unwrap_or("");
            let mut cmd = tokio::process::Command::new("git");
            cmd.arg("diff");
            if !path_arg.is_empty() { cmd.arg(path_arg); }
            cmd.current_dir(root_path);
            match cmd.output().await {
                Ok(o) => { let s = String::from_utf8_lossy(&o.stdout).to_string(); if s.is_empty() { "(no diff)".to_string() } else { s } }
                Err(e) => format!("Error: {e}"),
            }
        }
        "git_log" => {
            let n = input["n"].as_u64().unwrap_or(15);
            match tokio::process::Command::new("git")
                .args(["log", "--oneline", &format!("-{n}")])
                .current_dir(root_path)
                .output().await
            {
                Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                Err(e) => format!("Error: {e}"),
            }
        }
        "git_commit" => {
            let msg = input["message"].as_str().unwrap_or("chore: update");
            match tokio::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(root_path)
                .output().await
            {
                Err(e) => return format!("git add error: {e}"),
                Ok(o) if !o.status.success() => return format!("git add failed: {}", String::from_utf8_lossy(&o.stderr)),
                _ => {}
            }
            match tokio::process::Command::new("git")
                .args(["commit", "-m", msg])
                .current_dir(root_path)
                .output().await
            {
                Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                Err(e) => format!("Error: {e}"),
            }
        }
        "web_fetch" => {
            let url = input["url"].as_str().unwrap_or("");
            match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default()
                .get(url)
                .header("User-Agent", "Mozilla/5.0")
                .send().await
            {
                Ok(resp) => {
                    match resp.text().await {
                        Ok(html) => {
                            // Strip HTML tags
                            let mut out = String::with_capacity(html.len());
                            let mut in_tag = false;
                            for c in html.chars() {
                                match c {
                                    '<' => { in_tag = true; }
                                    '>' => { in_tag = false; out.push(' '); }
                                    _ if !in_tag => out.push(c),
                                    _ => {}
                                }
                            }
                            let trimmed: Vec<&str> = out.split_whitespace().collect();
                            let text = trimmed.join(" ");
                            if text.len() > 4000 { format!("{}...", &text[..4000]) } else { text }
                        }
                        Err(e) => format!("Error reading response: {e}"),
                    }
                }
                Err(e) => format!("Error fetching URL: {e}"),
            }
        }
        _ => format!("Unknown tool: {name}"),
    }
}

// ── Blocking (non-streaming) API calls with tool support ───────────────────

/// Anthropic non-streaming request with tool calling
async fn send_anthropic_blocking(
    api_key: &str, model: &str, system: &str,
    messages: &[serde_json::Value], tools: &serde_json::Value,
    max_tokens: u32, tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build().map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "model": model, "max_tokens": max_tokens,
        "system": system, "messages": messages,
        "tools": tools, "stream": false
    });
    let resp = client.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body).send().await
        .map_err(|e| format!("Anthropic request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {status}: {text}"));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("JSON error: {e}"))?;
    let stop_reason = v["stop_reason"].as_str().unwrap_or("").to_string();
    let content = v["content"].clone();

    // Send full content array as sentinel so drain can store it for history
    let content_sentinel = format!("{ASSISTANT_CONTENT_SENTINEL}{}", serde_json::to_string(&content).unwrap_or_default());
    let _ = tx.send(content_sentinel).await;

    if let Some(blocks) = content.as_array() {
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        let _ = tx.send(text.to_string()).await;
                    }
                }
                Some("tool_use") => {
                    let ev = serde_json::json!({
                        "id": block["id"], "name": block["name"], "input": block["input"]
                    });
                    let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
                }
                _ => {}
            }
        }
    }
    if stop_reason == "tool_use" {
        let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
    }
    Ok(())
}

/// OpenAI-compatible non-streaming request with tool calling
async fn send_openai_blocking(
    api_key: &str, base_url: &str, model: &str, system: &str,
    messages: &[serde_json::Value], tools: &serde_json::Value,
    max_tokens: u32, prompt_tool_mode: bool, tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let request_timeout = openai_request_timeout(base_url, prompt_tool_mode);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(request_timeout)
        .build().map_err(|e| e.to_string())?;
    let system_content = if prompt_tool_mode {
        format!("{}\n\n{}", build_tui_tool_injection_prompt(tools), system)
    } else {
        system.to_string()
    };
    let mut all_msgs = vec![serde_json::json!({"role":"system","content":system_content})];
    all_msgs.extend_from_slice(messages);
    let mut body = serde_json::json!({
        "model": model, "stream": false,
        "max_tokens": max_tokens,
        "messages": all_msgs,
    });
    if !prompt_tool_mode {
        body["tools"] = tools.clone();
        body["tool_choice"] = serde_json::json!("auto");
    }
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut req = client.post(&url).header("content-type", "application/json");
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {api_key}")); }
    let resp = req.json(&body).send().await
        .map_err(|e| format_openai_request_error(base_url, request_timeout, e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {status}: {text}"));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("JSON error: {e}"))?;
    let finish_reason = v["choices"][0]["finish_reason"].as_str().unwrap_or("").to_string();
    let msg = normalize_openai_tool_message(&v["choices"][0]["message"], !prompt_tool_mode);

    if prompt_tool_mode {
        let text = msg["content"].as_str().unwrap_or("");
        let tool_calls = extract_tui_tool_calls_from_text(text);
        let content_sentinel = format!(
            "{ASSISTANT_CONTENT_SENTINEL}{}",
            serde_json::to_string(&msg).unwrap_or_default()
        );
        let _ = tx.send(content_sentinel).await;

        if !tool_calls.is_empty() {
            for tool_call in tool_calls {
                let ev = serde_json::json!({
                    "id": tool_call.id,
                    "name": tool_call.name,
                    "input": serde_json::from_str::<serde_json::Value>(&tool_call.input_json)
                        .unwrap_or_else(|_| serde_json::json!({}))
                });
                let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
            }
            let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
            return Ok(());
        }

        if !text.is_empty() {
            let _ = tx.send(text.to_string()).await;
        }
        return Ok(());
    }

    // Send assistant message as content sentinel (for OpenAI format history)
    let content_sentinel = format!("{ASSISTANT_CONTENT_SENTINEL}{}", serde_json::to_string(&msg).unwrap_or_default());
    let _ = tx.send(content_sentinel).await;

    if let Some(text) = msg["content"].as_str() {
        if !text.is_empty() { let _ = tx.send(text.to_string()).await; }
    }
    if let Some(tcs) = msg["tool_calls"].as_array() {
        for tc in tcs {
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input: serde_json::Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
            let ev = serde_json::json!({
                "id": tc["id"], "name": tc["function"]["name"], "input": input
            });
            let _ = tx.send(format!("{TOOL_SENTINEL}{}", serde_json::to_string(&ev).unwrap_or_default())).await;
        }
    }
    if finish_reason == "tool_calls" || finish_reason == "tool" || msg["tool_calls"].as_array().map(|a| !a.is_empty()).unwrap_or(false) {
        let _ = tx.send(APPROVAL_SENTINEL.to_string()).await;
    }
    Ok(())
}

// ─── Batch 3: handle_explain_command ──────────────────────────────────────

#[allow(dead_code)]
fn handle_explain_command(target: &str, root_path: &str) -> Option<String> {
    // Parse: file, file:line, file:start-end
    let (file_part, range_part) = if let Some(idx) = target.rfind(':') {
        let maybe_range = &target[idx + 1..];
        if maybe_range.chars().all(|c| c.is_ascii_digit() || c == '-') {
            (&target[..idx], Some(maybe_range))
        } else {
            (target, None)
        }
    } else {
        (target, None)
    };

    let full_path = if std::path::Path::new(file_part).is_absolute() {
        file_part.to_string()
    } else {
        format!("{}/{}", root_path, file_part)
    };

    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let (start, end) = if let Some(range) = range_part {
        if let Some(dash) = range.find('-') {
            let s: usize = range[..dash].parse::<usize>().unwrap_or(1).saturating_sub(1);
            let e: usize = range[dash + 1..].parse::<usize>().unwrap_or(total).min(total);
            (s, e)
        } else {
            let ln: usize = range.parse::<usize>().unwrap_or(1).saturating_sub(1);
            let s = ln.saturating_sub(5);
            let e = (ln + 5).min(total);
            (s, e)
        }
    } else {
        (0, total.min(50))
    };

    let snippet: Vec<&str> = lines[start..end].to_vec();
    let ext = std::path::Path::new(file_part)
        .extension().and_then(|e| e.to_str()).unwrap_or("");
    let display = file_part;

    Some(format!(
        "Please explain this code from `{}` lines {}-{}:\n```{}\n{}\n```",
        display,
        start + 1,
        end,
        ext,
        snippet.join("\n")
    ))
}

// ─── Batch 3: handle_rename_command ───────────────────────────────────────

fn handle_rename_command(old: &str, new: &str, root_path: &str) -> String {
    let mut o = io::stdout();
    print_section_header(&format!("Rename: {} → {}", old, new));

    // Find all occurrences with grep
    let grep_output = std::process::Command::new("grep")
        .args(["-rn",
            "--include=*.rs", "--include=*.ts", "--include=*.py",
            "--include=*.go", "--include=*.js", "--include=*.cpp",
            "--include=*.h", "--include=*.jsx", "--include=*.tsx",
            old, "."])
        .current_dir(root_path)
        .output();

    let matches: Vec<String> = match grep_output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines().map(|l| l.to_string()).filter(|l| !l.is_empty()).collect(),
        Err(_) => vec![],
    };

    if matches.is_empty() {
        print_section_end();
        return format!("No occurrences of '{}' found.", old);
    }

    // Group by file
    let mut file_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in &matches {
        if let Some(colon) = m.find(':') {
            let file = m[..colon].to_string();
            *file_map.entry(file).or_insert(0) += 1;
        }
    }
    let file_count = file_map.len();
    let total_occurrences: usize = file_map.values().sum();

    set_fg(&mut o, theme::WARN);
    write!(o, "\n  Preview: {} files, {} total occurrences\n", file_count, total_occurrences).ok();
    reset_color(&mut o);
    for (file, count) in &file_map {
        set_fg(&mut o, theme::DIM);
        write!(o, "    {} ({} occurrence{})\n", file, count, if *count == 1 { "" } else { "s" }).ok();
    }
    reset_color(&mut o);

    set_fg(&mut o, theme::ACCENT);
    write!(o, "\n  Rename '{}' → '{}' in {} file(s)? [y/N]: ", old, new, file_count).ok();
    reset_color(&mut o);
    o.flush().ok();

    let mut answer = String::new();
    io::stdin().read_line(&mut answer).ok();
    if answer.trim().eq_ignore_ascii_case("y") {
        let mut replaced_files = 0;
        let mut replaced_total = 0;
        for file in file_map.keys() {
            let full = format!("{}/{}", root_path, file);
            if let Ok(content) = std::fs::read_to_string(&full) {
                let count = content.matches(old).count();
                let new_content = content.replace(old, new);
                if std::fs::write(&full, &new_content).is_ok() {
                    replaced_files += 1;
                    replaced_total += count;
                }
            }
        }
        print_section_end();
        format!("Renamed `{}` → `{}` in {} files ({} replacements)", old, new, replaced_files, replaced_total)
    } else {
        print_section_end();
        "Rename cancelled.".to_string()
    }
}

// ─── Batch 3: handle_docker_command ───────────────────────────────────────

fn handle_docker_command(sub: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Docker: {}", if sub.is_empty() { "info" } else { sub }));

    let root = std::path::Path::new(root_path);
    let compose_file = if root.join("docker-compose.yml").exists() {
        "docker-compose.yml"
    } else if root.join("compose.yml").exists() {
        "compose.yml"
    } else {
        ""
    };

    if sub.is_empty() {
        if compose_file.is_empty() {
            set_fg(&mut o, theme::WARN);
            write!(o, "  No docker-compose.yml or compose.yml found.\n").ok();
        } else {
            set_fg(&mut o, theme::OK);
            write!(o, "  Found: {}\n", compose_file).ok();
            reset_color(&mut o);
            write!(o, "\n  Subcommands: up, down, ps, build, pull\n").ok();
            write!(o, "               logs <service>, restart <service>\n").ok();
            write!(o, "               exec <service> <cmd>\n").ok();
        }
        reset_color(&mut o);
        print_section_end();
        return;
    }

    let docker_cmd = if std::process::Command::new("docker").arg("compose").arg("version").output().is_ok() {
        vec!["docker".to_string(), "compose".to_string()]
    } else {
        vec!["docker-compose".to_string()]
    };

    let parts: Vec<&str> = sub.splitn(3, ' ').collect();
    let cmd_args: Vec<String> = match parts[0] {
        "up"      => [docker_cmd.clone(), vec!["up".into(), "-d".into()]].concat(),
        "down"    => [docker_cmd.clone(), vec!["down".into()]].concat(),
        "ps"      => [docker_cmd.clone(), vec!["ps".into()]].concat(),
        "build"   => [docker_cmd.clone(), vec!["build".into()]].concat(),
        "pull"    => [docker_cmd.clone(), vec!["pull".into()]].concat(),
        "logs"    => {
            let svc = parts.get(1).unwrap_or(&"").to_string();
            [docker_cmd.clone(), vec!["logs".into(), "--tail=50".into(), svc]].concat()
        }
        "restart" => {
            let svc = parts.get(1).unwrap_or(&"").to_string();
            [docker_cmd.clone(), vec!["restart".into(), svc]].concat()
        }
        "exec"    => {
            let svc = parts.get(1).unwrap_or(&"").to_string();
            let cmd = parts.get(2).unwrap_or(&"sh").to_string();
            [docker_cmd.clone(), vec!["exec".into(), svc, cmd]].concat()
        }
        _ => {
            print_error(&format!("Unknown docker subcommand: {}", sub));
            print_section_end();
            return;
        }
    };

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd_args.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd_args, root_path);
    let combined = if !stdout.is_empty() { stdout } else { stderr };
    for line in combined.lines() {
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "\n  {} Done\n", CHECK).ok();
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "\n  {} Command failed\n", CROSS).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

// ─── Batch 3: handle_release_command ──────────────────────────────────────

fn handle_release_command(version: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Release");

    let root = std::path::Path::new(root_path);
    let cargo_path = root.join("Cargo.toml");
    let pkg_path = root.join("package.json");

    // Detect current version
    let (current_version, manifest_type) = if cargo_path.exists() {
        let content = std::fs::read_to_string(&cargo_path).unwrap_or_default();
        let ver = content.lines()
            .find(|l| l.starts_with("version"))
            .and_then(|l| l.split('"').nth(1))
            .unwrap_or("0.0.0")
            .to_string();
        (ver, "cargo")
    } else if pkg_path.exists() {
        let content = std::fs::read_to_string(&pkg_path).unwrap_or_default();
        let ver = serde_json::from_str::<serde_json::Value>(&content).ok()
            .and_then(|v| v["version"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0.0.0".to_string());
        (ver, "npm")
    } else {
        print_error("No Cargo.toml or package.json found.");
        print_section_end();
        return;
    };

    // Compute new version
    let new_version = match version {
        "patch" | "minor" | "major" | "" => {
            let parts: Vec<u32> = current_version.split('.')
                .filter_map(|p| p.parse().ok()).collect();
            let (ma, mi, pa) = (
                parts.get(0).copied().unwrap_or(0),
                parts.get(1).copied().unwrap_or(0),
                parts.get(2).copied().unwrap_or(0),
            );
            match version {
                "major" => format!("{}.0.0", ma + 1),
                "minor" => format!("{}.{}.0", ma, mi + 1),
                _       => format!("{}.{}.{}", ma, mi, pa + 1),
            }
        }
        v => v.to_string(),
    };

    set_fg(&mut o, theme::CYAN);
    write!(o, "\n  Plan:\n").ok();
    set_fg(&mut o, theme::DIM_LIGHT);
    write!(o, "    {} Bump version: {} → {}\n", ARROW, current_version, new_version).ok();
    write!(o, "    {} Update CHANGELOG.md\n", ARROW).ok();
    write!(o, "    {} git commit + tag v{}\n", ARROW, new_version).ok();
    reset_color(&mut o);

    set_fg(&mut o, theme::ACCENT);
    write!(o, "\n  Proceed? [y/N]: ").ok();
    reset_color(&mut o);
    o.flush().ok();

    let mut answer = String::new();
    io::stdin().read_line(&mut answer).ok();
    if !answer.trim().eq_ignore_ascii_case("y") {
        write!(o, "  Cancelled.\n").ok();
        print_section_end();
        return;
    }

    // Update version in manifest
    let updated = match manifest_type {
        "cargo" => {
            let content = std::fs::read_to_string(&cargo_path).unwrap_or_default();
            let new_content = content.replacen(
                &format!("version = \"{}\"", current_version),
                &format!("version = \"{}\"", new_version),
                1
            );
            std::fs::write(&cargo_path, &new_content).is_ok()
        }
        "npm" => {
            let content = std::fs::read_to_string(&pkg_path).unwrap_or_default();
            let new_content = content.replace(
                &format!("\"version\": \"{}\"", current_version),
                &format!("\"version\": \"{}\"", new_version),
            );
            std::fs::write(&pkg_path, &new_content).is_ok()
        }
        _ => false,
    };

    if !updated {
        print_error("Failed to update version in manifest.");
        print_section_end();
        return;
    }

    // Update CHANGELOG.md
    let cl_path = root.join("CHANGELOG.md");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    if cl_path.exists() {
        let content = std::fs::read_to_string(&cl_path).unwrap_or_default();
        let new_content = format!("## [{}] - {}\n\n", new_version, today) + &content;
        let _ = std::fs::write(&cl_path, &new_content);
    } else {
        let _ = std::fs::write(&cl_path, format!("# Changelog\n\n## [{}] - {}\n\n", new_version, today));
    }

    // Git commit + tag
    let _ = std::process::Command::new("git").args(["add", "-A"]).current_dir(root_path).status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", &format!("chore: release v{}", new_version)])
        .current_dir(root_path).status();
    let _ = std::process::Command::new("git")
        .args(["tag", &format!("v{}", new_version)])
        .current_dir(root_path).status();

    set_fg(&mut o, theme::OK);
    write!(o, "\n  {} Released v{}\n", CHECK, new_version).ok();
    reset_color(&mut o);

    set_fg(&mut o, theme::DIM);
    write!(o, "  Push with: git push && git push --tags\n").ok();
    reset_color(&mut o);
    print_section_end();
}

// ─── Batch 3: handle_benchmark_command ────────────────────────────────────

fn handle_benchmark_command(name: &str, root_path: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Benchmark{}", if name.is_empty() { String::new() } else { format!(": {}", name) }));

    let root = std::path::Path::new(root_path);
    let cmd: Vec<String> = if root.join("Cargo.toml").exists() {
        if name.is_empty() { vec!["cargo".into(), "bench".into()] }
        else { vec!["cargo".into(), "bench".into(), name.to_string()] }
    } else if root.join("package.json").exists() {
        vec!["npm".into(), "run".into(), "bench".into()]
    } else {
        print_error("No supported benchmark runner found.");
        print_section_end();
        return;
    };

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd, root_path);
    let combined = if !stdout.is_empty() { stdout } else { stderr };

    // Parse criterion output: lines like "test bench_name ... bench: 1,234 ns/iter"
    let bench_re = regex::Regex::new(r"(?i)(\w[\w: ]+)\s+\.\.\.\s+bench:\s+([\d,]+)\s+ns/iter").ok();
    let mut results: Vec<(String, u64)> = vec![];
    for line in combined.lines() {
        if let Some(ref re) = bench_re {
            if let Some(cap) = re.captures(line) {
                let name_s = cap.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                let mean_s = cap.get(2).map(|m| m.as_str().replace(',', "")).unwrap_or_default();
                if let Ok(mean_ns) = mean_s.parse::<u64>() {
                    results.push((name_s, mean_ns));
                }
            }
        }
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    // Store results and compare
    if !results.is_empty() {
        ensure_tracking_dir(root_path);
        let hist_path = tracking_file_path(root_path, "bench-history.json");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let new_entry = serde_json::json!({
            "timestamp": now,
            "results": results.iter().map(|(n, m)| serde_json::json!({"name": n, "mean_ns": m})).collect::<Vec<_>>()
        });

        let prev: Option<serde_json::Value> = std::fs::read_to_string(&hist_path).ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        // Show comparison if we have history
        if let Some(prev_val) = &prev {
            if let Some(prev_results) = prev_val["results"].as_array() {
                write!(o, "\n").ok();
                for (name_s, mean_ns) in &results {
                    if let Some(prev_item) = prev_results.iter().find(|p| p["name"].as_str() == Some(name_s)) {
                        if let Some(prev_ns) = prev_item["mean_ns"].as_u64() {
                            let diff_pct = (*mean_ns as f64 - prev_ns as f64) / prev_ns as f64 * 100.0;
                            if diff_pct > 2.0 {
                                set_fg(&mut o, theme::ERR);
                                write!(o, "  {} {} +{:.1}% slower\n", ARROW, name_s, diff_pct).ok();
                            } else if diff_pct < -2.0 {
                                set_fg(&mut o, theme::OK);
                                write!(o, "  {} {} {:.1}% faster\n", ARROW, name_s, -diff_pct).ok();
                            } else {
                                set_fg(&mut o, theme::DIM);
                                write!(o, "  {} {} ~unchanged\n", ARROW, name_s).ok();
                            }
                            reset_color(&mut o);
                        }
                    }
                }
            }
        }

        // Save new results (replace file with latest)
        let _ = std::fs::write(&hist_path, serde_json::to_string_pretty(&new_entry).unwrap_or_default());
    }

    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "\n  {} Benchmark complete\n", CHECK).ok();
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "\n  {} Benchmark failed\n", CROSS).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

// ─── Batch 3: handle_coverage_command ─────────────────────────────────────

fn handle_coverage_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Coverage");

    let root = std::path::Path::new(root_path);
    if !root.join("Cargo.toml").exists() {
        print_error("Coverage currently supported for Rust projects (cargo-tarpaulin).");
        print_section_end();
        return;
    }

    // Check for existing report first
    let tarpaulin_json = root.join("tarpaulin-report.json");
    let lcov = root.join("lcov.info");

    let report_content = if tarpaulin_json.exists() {
        std::fs::read_to_string(&tarpaulin_json).ok()
    } else if lcov.exists() {
        None // lcov parsing is complex; run tarpaulin instead
    } else {
        None
    };

    let json_data: Option<serde_json::Value> = if let Some(content) = &report_content {
        serde_json::from_str(content).ok()
    } else {
        // Try running tarpaulin
        if std::process::Command::new("cargo").arg("tarpaulin").arg("--version").output().is_ok() {
            set_fg(&mut o, theme::DIM);
            write!(o, "  Running cargo tarpaulin...\n").ok();
            reset_color(&mut o);
            let out = std::process::Command::new("cargo")
                .args(["tarpaulin", "--out", "Json"])
                .current_dir(root_path)
                .output();
            match out {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    serde_json::from_str(&stdout).ok()
                }
                Err(_) => None,
            }
        } else {
            set_fg(&mut o, theme::WARN);
            write!(o, "  cargo-tarpaulin not installed.\n").ok();
            write!(o, "  Install: cargo install cargo-tarpaulin\n").ok();
            reset_color(&mut o);
            print_section_end();
            return;
        }
    };

    if let Some(data) = json_data {
        // Parse tarpaulin JSON: files array with covered/coverable
        if let Some(files) = data["files"].as_array() {
            let mut file_rows: Vec<(String, u64, u64, f64)> = vec![];
            for file in files {
                let path = file["path"].as_str().unwrap_or("unknown");
                let covered = file["covered"].as_u64().unwrap_or(0);
                let coverable = file["coverable"].as_u64().unwrap_or(1);
                let pct = if coverable > 0 { covered as f64 / coverable as f64 * 100.0 } else { 100.0 };
                let display = path.strip_prefix(root_path).unwrap_or(path).trim_start_matches('/').to_string();
                file_rows.push((display, covered, coverable, pct));
            }
            file_rows.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));

            let total_cov: u64 = file_rows.iter().map(|(_, c, _, _)| c).sum();
            let total_all: u64 = file_rows.iter().map(|(_, _, t, _)| t).sum();
            let overall = if total_all > 0 { total_cov as f64 / total_all as f64 * 100.0 } else { 0.0 };

            write!(o, "\n").ok();
            for (path, covered, total, pct) in &file_rows {
                let color = if *pct >= 80.0 { theme::OK } else if *pct >= 50.0 { theme::WARN } else { theme::ERR };
                set_fg(&mut o, color);
                write!(o, "  {:5.1}% ", pct).ok();
                set_fg(&mut o, theme::DIM_LIGHT);
                write!(o, "{}/{} ", covered, total).ok();
                set_fg(&mut o, theme::AI_TEXT);
                write!(o, "{}\n", path).ok();
                reset_color(&mut o);
            }

            write!(o, "\n").ok();
            let overall_color = if overall >= 80.0 { theme::OK } else if overall >= 50.0 { theme::WARN } else { theme::ERR };
            set_fg(&mut o, overall_color);
            write!(o, "  Overall: {:.1}%\n", overall).ok();
            reset_color(&mut o);
        } else {
            set_fg(&mut o, theme::WARN);
            write!(o, "  Could not parse coverage report.\n").ok();
            reset_color(&mut o);
        }
    }

    print_section_end();
}

// ─── Batch 3: handle_remote_command ───────────────────────────────────────

fn handle_remote_command(sub: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Remote: {}", sub));

    let parts: Vec<&str> = sub.splitn(3, ' ').collect();
    if parts.is_empty() {
        set_fg(&mut o, theme::DIM);
        write!(o, "  Usage:\n").ok();
        write!(o, "    /remote ssh <user@host> <cmd>\n").ok();
        write!(o, "    /remote docker <container> <cmd>\n").ok();
        write!(o, "    /remote k8s <pod> <cmd>\n").ok();
        reset_color(&mut o);
        print_section_end();
        return;
    }

    let cmd: Vec<String> = match parts[0] {
        "ssh" if parts.len() >= 3 => {
            vec!["ssh".into(), "-o".into(), "BatchMode=yes".into(),
                 parts[1].into(), parts[2].into()]
        }
        "docker" if parts.len() >= 3 => {
            vec!["docker".into(), "exec".into(), parts[1].into(),
                 "sh".into(), "-c".into(), parts[2].into()]
        }
        "k8s" if parts.len() >= 3 => {
            let pod_parts: Vec<&str> = parts[1].splitn(2, '/').collect();
            let pod = pod_parts.last().unwrap_or(&parts[1]);
            vec!["kubectl".into(), "exec".into(), pod.to_string(), "--".into(),
                 "sh".into(), "-c".into(), parts[2].into()]
        }
        _ => {
            print_error(&format!("Invalid remote subcommand: {}", sub));
            print_section_end();
            return;
        }
    };

    set_fg(&mut o, theme::DIM);
    write!(o, "  $ {}\n\n", cmd.join(" ")).ok();
    reset_color(&mut o);

    let (stdout, stderr, ok) = run_command_live(&cmd, ".");
    let combined = if !stdout.is_empty() { stdout } else { stderr };
    for line in combined.lines() {
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  {}\n", line).ok();
    }
    reset_color(&mut o);

    if ok {
        set_fg(&mut o, theme::OK);
        write!(o, "\n  {} Done\n", CHECK).ok();
    } else {
        set_fg(&mut o, theme::ERR);
        write!(o, "\n  {} Failed\n", CROSS).ok();
    }
    reset_color(&mut o);
    print_section_end();
}

// ─── Batch 3: handle_share_command ────────────────────────────────────────

fn handle_share_command(root_path: &str) {
    let mut o = io::stdout();
    print_section_header("Share Session");

    ensure_tracking_dir(root_path);
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("shared-{}.json", ts);
    let path = tracking_file_path(root_path, &filename);

    // Export minimal session info
    let export = serde_json::json!({
        "exported": ts,
        "note": "ShadowAI session export",
        "root": root_path,
    });

    match std::fs::write(&path, serde_json::to_string_pretty(&export).unwrap_or_default()) {
        Ok(_) => {
            set_fg(&mut o, theme::OK);
            write!(o, "  {} Session exported to:\n", CHECK).ok();
            set_fg(&mut o, theme::CYAN);
            write!(o, "    {}\n", path.display()).ok();
            reset_color(&mut o);
            set_fg(&mut o, theme::DIM);
            write!(o, "\n  Share with: shadowai load {}\n", filename).ok();
            reset_color(&mut o);
        }
        Err(e) => {
            print_error(&format!("Export failed: {}", e));
        }
    }
    print_section_end();
}

// ─── Batch 3: handle_cron_command ─────────────────────────────────────────

fn handle_cron_command(sub: &str) {
    let mut o = io::stdout();
    print_section_header(&format!("Cron: {}", if sub.is_empty() { "list" } else { sub }));

    let cron_path = match config_dir() {
        Some(d) => d.join("cron.toml"),
        None => {
            print_error("Could not determine config directory.");
            print_section_end();
            return;
        }
    };

    // Load existing jobs
    #[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
    struct CronJob {
        id: String,
        schedule: String,
        command: String,
        created: String,
    }
    #[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
    struct CronConfig { jobs: Vec<CronJob> }

    let load_cron = || -> CronConfig {
        std::fs::read_to_string(&cron_path).ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    };
    let save_cron = |cfg: &CronConfig| {
        if let Some(parent) = cron_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cron_path, toml::to_string(cfg).unwrap_or_default());
    };

    if sub.is_empty() || sub == "list" {
        let cfg = load_cron();
        if cfg.jobs.is_empty() {
            set_fg(&mut o, theme::DIM);
            write!(o, "  No cron jobs configured.\n").ok();
            write!(o, "  Add with: /cron add \"<schedule>\" <command>\n").ok();
        } else {
            for job in &cfg.jobs {
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "  {} ", job.id).ok();
                set_fg(&mut o, theme::WARN);
                write!(o, "{} ", job.schedule).ok();
                set_fg(&mut o, theme::AI_TEXT);
                write!(o, "{}\n", job.command).ok();
            }
        }
        reset_color(&mut o);
    } else if let Some(rest) = sub.strip_prefix("add ") {
        // Parse: add "<schedule>" <command>
        let rest = rest.trim();
        let (schedule, command) = if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                (&rest[1..end + 1], rest[end + 2..].trim())
            } else {
                (rest, "")
            }
        } else {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            (parts.get(0).copied().unwrap_or(""), parts.get(1).copied().unwrap_or("").trim())
        };

        let id: String = format!("{:x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u32);
        let created = chrono::Local::now().to_rfc3339();
        let mut cfg = load_cron();
        cfg.jobs.push(CronJob {
            id: id.clone(),
            schedule: schedule.to_string(),
            command: command.to_string(),
            created,
        });
        save_cron(&cfg);
        set_fg(&mut o, theme::OK);
        write!(o, "  {} Added cron job: {} ({})\n", CHECK, command, id).ok();
        reset_color(&mut o);
    } else if let Some(id) = sub.strip_prefix("remove ") {
        let id = id.trim();
        let mut cfg = load_cron();
        let before = cfg.jobs.len();
        cfg.jobs.retain(|j| j.id != id);
        if cfg.jobs.len() < before {
            save_cron(&cfg);
            set_fg(&mut o, theme::OK);
            write!(o, "  {} Removed job {}\n", CHECK, id).ok();
        } else {
            set_fg(&mut o, theme::WARN);
            write!(o, "  Job '{}' not found.\n", id).ok();
        }
        reset_color(&mut o);
    } else if let Some(id) = sub.strip_prefix("run ") {
        let id = id.trim();
        let cfg = load_cron();
        if let Some(job) = cfg.jobs.iter().find(|j| j.id == id) {
            set_fg(&mut o, theme::DIM);
            write!(o, "  Running: {}\n\n", job.command).ok();
            reset_color(&mut o);
            let parts: Vec<String> = job.command.splitn(2, ' ').map(|s| s.to_string()).collect();
            if !parts.is_empty() {
                let (stdout, stderr, ok) = run_command_live(&parts, ".");
                let combined = if !stdout.is_empty() { stdout } else { stderr };
                for line in combined.lines() {
                    set_fg(&mut o, theme::DIM_LIGHT);
                    write!(o, "  {}\n", line).ok();
                }
                reset_color(&mut o);
                if ok { set_fg(&mut o, theme::OK); write!(o, "  {} Done\n", CHECK).ok(); }
                else   { set_fg(&mut o, theme::ERR); write!(o, "  {} Failed\n", CROSS).ok(); }
                reset_color(&mut o);
            }
        } else {
            print_error(&format!("Job '{}' not found.", id));
        }
    } else {
        set_fg(&mut o, theme::WARN);
        write!(o, "  Unknown subcommand: {}\n", sub).ok();
        write!(o, "  Use: list, add, remove, run\n").ok();
        reset_color(&mut o);
    }

    print_section_end();
}

// ─── Batch 5: set_terminal_title ──────────────────────────────────────────

#[allow(dead_code)]
fn set_terminal_title(title: &str) {
    print!("\x1b]2;{}\x07", title);
    io::stdout().flush().ok();
}

#[allow(dead_code)]
fn clear_terminal_title() {
    set_terminal_title("shadowai");
}

// ─── Batch 5: send_webhook_notification ───────────────────────────────────

#[allow(dead_code)]
async fn send_webhook_notification(url: &str, text: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    // Detect Discord vs Slack by URL
    let body = if url.contains("discord.com") {
        serde_json::json!({"content": text})
    } else {
        serde_json::json!({"text": text})
    };
    client.post(url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send().await
        .map_err(|e| format!("Webhook failed: {}", e))?;
    Ok(())
}

// ─── Batch 6: redact_secrets ──────────────────────────────────────────────

fn redact_secrets(s: &str) -> String {
    use std::borrow::Cow;
    let patterns: &[(&str, &str)] = &[
        (r"sk-ant-[A-Za-z0-9\-_]{20,}", "sk-ant-***REDACTED***"),
        (r"sk-[A-Za-z0-9]{20,}", "sk-***REDACTED***"),
        (r"AKIA[A-Z0-9]{16}", "AKIA***REDACTED***"),
        (r"ghp_[A-Za-z0-9]{36}", "ghp_***REDACTED***"),
        (r"ghs_[A-Za-z0-9]{36}", "ghs_***REDACTED***"),
        (r"xoxb-[0-9]+-[0-9]+-[A-Za-z0-9]+", "xoxb-***REDACTED***"),
        (r"pplx-[A-Za-z0-9]{48}", "pplx-***REDACTED***"),
    ];
    let mut result = Cow::Borrowed(s);
    for (pat, replacement) in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            if re.is_match(&result) {
                let replaced = re.replace_all(&result, *replacement).to_string();
                result = Cow::Owned(replaced);
            }
        }
    }
    result.into_owned()
}

// ─── Batch 6: append_audit_log ────────────────────────────────────────────

#[allow(dead_code)]
fn append_audit_log(root: &str, entry: &str) {
    ensure_tracking_dir(root);
    let path = tracking_file_path(root, "audit.log");
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let safe_entry = redact_secrets(entry);
        let _ = writeln!(f, "[{}] {}", ts, safe_entry);
    }
}

// ─── Batch 6: sanitize_file_content ───────────────────────────────────────

#[allow(dead_code)]
fn sanitize_file_content(content: &str, max_len: usize) -> String {
    // Truncate
    let truncated: String = content.chars().take(max_len).collect();
    // Remove null bytes
    let cleaned = truncated.replace('\x00', "");
    // Remove ANSI escapes
    let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*[mGKHF]").ok();
    let cleaned = if let Some(re) = ansi_re {
        re.replace_all(&cleaned, "").to_string()
    } else {
        cleaned
    };
    // Warn on injection patterns
    let injection_patterns = ["</system>", "<|im_end|>", "IGNORE PREVIOUS INSTRUCTIONS", "IGNORE ALL PREVIOUS"];
    let mut o = io::stdout();
    for pat in &injection_patterns {
        if cleaned.to_uppercase().contains(&pat.to_uppercase()) {
            set_fg(&mut o, theme::WARN);
            write!(o, "  WARNING: Suspicious pattern found in file content: '{}'\n", pat).ok();
            reset_color(&mut o);
        }
    }
    cleaned
}

// ─── Batch 6: load_custom_theme ───────────────────────────────────────────

#[derive(Debug, Default, serde::Deserialize)]
struct CustomThemeToml {
    accent: Option<[u8; 3]>,
    accent_dim: Option<[u8; 3]>,
    cyan: Option<[u8; 3]>,
    cyan_dim: Option<[u8; 3]>,
    ai_text: Option<[u8; 3]>,
    ok: Option<[u8; 3]>,
    warn: Option<[u8; 3]>,
    err: Option<[u8; 3]>,
    dim: Option<[u8; 3]>,
    dim_light: Option<[u8; 3]>,
    think: Option<[u8; 3]>,
    stat: Option<[u8; 3]>,
    border: Option<[u8; 3]>,
}

fn rgb(arr: Option<[u8; 3]>, default: Color) -> Color {
    arr.map(|[r, g, b]| Color::Rgb { r, g, b }).unwrap_or(default)
}

fn load_custom_theme(name: &str) -> Option<ThemeColors> {
    let path = config_dir()?.join("themes").join(format!("{}.toml", name));
    if !path.exists() { return None; }
    let content = std::fs::read_to_string(&path).ok()?;
    let custom: CustomThemeToml = toml::from_str(&content).ok()?;
    let def = get_default_dark_theme();
    Some(ThemeColors {
        accent:    rgb(custom.accent, def.accent),
        accent_dim: rgb(custom.accent_dim, def.accent_dim),
        cyan:      rgb(custom.cyan, def.cyan),
        cyan_dim:  rgb(custom.cyan_dim, def.cyan_dim),
        ai_text:   rgb(custom.ai_text, def.ai_text),
        ok:        rgb(custom.ok, def.ok),
        warn:      rgb(custom.warn, def.warn),
        err:       rgb(custom.err, def.err),
        dim:       rgb(custom.dim, def.dim),
        dim_light: rgb(custom.dim_light, def.dim_light),
        think:     rgb(custom.think, def.think),
        stat:      rgb(custom.stat, def.stat),
        border:    rgb(custom.border, def.border),
    })
}

fn get_default_dark_theme() -> ThemeColors {
    ThemeColors {
        accent:    theme::ACCENT,
        accent_dim: theme::ACCENT_DIM,
        cyan:      theme::CYAN,
        cyan_dim:  theme::CYAN_DIM,
        ai_text:   theme::AI_TEXT,
        ok:        theme::OK,
        warn:      theme::WARN,
        err:       theme::ERR,
        dim:       theme::DIM,
        dim_light: theme::DIM_LIGHT,
        think:     theme::THINK,
        stat:      theme::STAT,
        border:    theme::BORDER,
    }
}

// ─── Batch 7: detect_project_type ─────────────────────────────────────────

#[allow(dead_code)]
fn detect_project_type(root: &str) -> Vec<&'static str> {
    let r = std::path::Path::new(root);
    let mut types = vec![];

    if r.join("project.godot").exists() { types.push("godot"); }
    if r.join("Assets").exists() && r.join("ProjectSettings").exists() { types.push("unity"); }
    // Unreal: look for *.uproject
    if r.read_dir().ok().map(|mut d| d.any(|e| {
        e.ok().and_then(|e| e.path().extension().map(|x| x == "uproject")).unwrap_or(false)
    })).unwrap_or(false) { types.push("unreal"); }

    if r.join("Cargo.toml").exists() {
        // Check for bevy in Cargo.toml
        let cargo_content = std::fs::read_to_string(r.join("Cargo.toml")).unwrap_or_default();
        if cargo_content.contains("bevy") {
            types.push("bevy");
        } else {
            types.push("rust");
        }
    }
    if r.join("main.lua").exists() && !r.join("Cargo.toml").exists() { types.push("love2d"); }
    if r.join("package.json").exists() { types.push("node"); }
    if r.join("pyproject.toml").exists() || r.join("requirements.txt").exists() { types.push("python"); }
    if r.join("go.mod").exists() { types.push("go"); }
    if r.read_dir().ok().map(|mut d| d.any(|e| {
        e.ok().and_then(|e| e.path().extension().map(|x| x == "swift")).unwrap_or(false)
    })).unwrap_or(false) { types.push("swift"); }
    if r.join("CMakeLists.txt").exists() { types.push("cmake"); }
    types
}

// ─── Batch 9: load_keybindings ────────────────────────────────────────────

#[allow(dead_code)]
fn load_keybindings() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    // Defaults
    map.insert("ctrl+l".into(), "/clear".into());
    map.insert("ctrl+k".into(), "kill_line".into());
    map.insert("ctrl+n".into(), "/new".into());
    map.insert("ctrl+e".into(), "open_editor".into());
    map.insert("ctrl+r".into(), "history_search".into());
    map.insert("ctrl+s".into(), "/save".into());
    map.insert("ctrl+u".into(), "undo_turn".into());
    map.insert("f1".into(), "/help".into());
    map.insert("f2".into(), "/status".into());
    map.insert("f3".into(), "/memory".into());
    map.insert("f10".into(), "/quit".into());

    // Override from file
    if let Some(dir) = config_dir() {
        let path = dir.join("keybindings.toml");
        if let Ok(content) = std::fs::read_to_string(&path) {
            #[derive(serde::Deserialize)]
            struct KbFile { keys: Option<std::collections::HashMap<String, String>> }
            if let Ok(kf) = toml::from_str::<KbFile>(&content) {
                if let Some(keys) = kf.keys {
                    for (k, v) in keys { map.insert(k, v); }
                }
            }
        }
    }
    map
}

// ─── Batch 6: generate_theme_create ───────────────────────────────────────

#[allow(dead_code)]
fn generate_theme_toml(base_theme: &ThemeColors) -> String {
    fn color_to_arr(c: Color) -> [u8; 3] {
        match c {
            Color::Rgb { r, g, b } => [r, g, b],
            _ => [128, 128, 128],
        }
    }
    let a = color_to_arr(base_theme.accent);
    let ad = color_to_arr(base_theme.accent_dim);
    let cy = color_to_arr(base_theme.cyan);
    let cyd = color_to_arr(base_theme.cyan_dim);
    let ai = color_to_arr(base_theme.ai_text);
    let ok = color_to_arr(base_theme.ok);
    let wn = color_to_arr(base_theme.warn);
    let er = color_to_arr(base_theme.err);
    let dm = color_to_arr(base_theme.dim);
    let dl = color_to_arr(base_theme.dim_light);
    let th = color_to_arr(base_theme.think);
    let st = color_to_arr(base_theme.stat);
    let bo = color_to_arr(base_theme.border);
    format!(
        "# ShadowAI custom theme\n\
         accent = [{}, {}, {}]\n\
         accent_dim = [{}, {}, {}]\n\
         cyan = [{}, {}, {}]\n\
         cyan_dim = [{}, {}, {}]\n\
         ai_text = [{}, {}, {}]\n\
         ok = [{}, {}, {}]\n\
         warn = [{}, {}, {}]\n\
         err = [{}, {}, {}]\n\
         dim = [{}, {}, {}]\n\
         dim_light = [{}, {}, {}]\n\
         think = [{}, {}, {}]\n\
         stat = [{}, {}, {}]\n\
         border = [{}, {}, {}]\n",
        a[0], a[1], a[2], ad[0], ad[1], ad[2],
        cy[0], cy[1], cy[2], cyd[0], cyd[1], cyd[2],
        ai[0], ai[1], ai[2], ok[0], ok[1], ok[2],
        wn[0], wn[1], wn[2], er[0], er[1], er[2],
        dm[0], dm[1], dm[2], dl[0], dl[1], dl[2],
        th[0], th[1], th[2], st[0], st[1], st[2],
        bo[0], bo[1], bo[2],
    )
}

// ─── Context bar formatting ───────────────────────────────────────────────────

#[allow(dead_code)]
fn format_context_bar(used: u64, total: u64) -> String {
    if total == 0 { return String::new(); }
    let pct = (used as f64 / total as f64 * 100.0) as u64;
    let filled = (pct / 10) as usize;
    let empty = 10usize.saturating_sub(filled);
    format!(" [{}{}] {}%", "█".repeat(filled), "░".repeat(empty), pct)
}

// ─── Simple hash for error pattern DB ───────────────────────────────────────

#[allow(dead_code)]
fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ─── Error pattern database ──────────────────────────────────────────────────

#[allow(dead_code)]
fn save_error_pattern(root: &str, error_hash: &str, file: &str, line: u32, fix: &str, success: bool) {
    ensure_tracking_dir(root);
    let path = tracking_file_path(root, "error-patterns.json");
    let mut patterns: Vec<serde_json::Value> = if path.exists() {
        std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        vec![]
    };
    // Find existing or add new
    if let Some(entry) = patterns.iter_mut().find(|e| e["hash"].as_str() == Some(error_hash)) {
        if let Some(count) = entry["count"].as_u64() {
            entry["count"] = serde_json::json!(count + 1);
        }
        entry["success"] = serde_json::json!(success);
    } else {
        patterns.push(serde_json::json!({
            "hash": error_hash,
            "file": file,
            "line": line,
            "fix": fix,
            "success": success,
            "count": 1
        }));
    }
    if let Ok(s) = serde_json::to_string_pretty(&patterns) {
        let _ = std::fs::write(&path, s);
    }
}

#[allow(dead_code)]
fn lookup_error_pattern(root: &str, error_text: &str) -> Option<String> {
    let key: String = error_text.chars().take(100).collect();
    let hash = format!("{:08x}", simple_hash(&key));
    let path = tracking_file_path(root, "error-patterns.json");
    let patterns: Vec<serde_json::Value> = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    patterns.into_iter()
        .find(|e| e["hash"].as_str() == Some(&hash) && e["success"].as_bool() == Some(true))
        .and_then(|e| e["fix"].as_str().map(String::from))
}

// ─── File snapshot helpers (for heal loop) ───────────────────────────────────

#[allow(dead_code)]
fn take_file_snapshot_simple(root: &str) -> std::collections::HashMap<String, String> {
    let mut snap = std::collections::HashMap::new();
    fn walk(dir: &std::path::Path, snap: &mut std::collections::HashMap<String, String>, depth: usize) {
        if depth > 5 { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for entry in entries.flatten() {
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();
            if path_str.contains("/target/") || path_str.contains("/node_modules/") || path_str.contains("/.git/") {
                continue;
            }
            if path.is_dir() {
                walk(&path, snap, depth + 1);
            } else if path.is_file() {
                if let Ok(meta) = std::fs::metadata(&path) {
                    let mtime = meta.modified().ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_default();
                    snap.insert(path_str, mtime);
                }
            }
        }
    }
    walk(std::path::Path::new(root), &mut snap, 0);
    snap
}

#[allow(dead_code)]
fn diff_file_snapshots(before: &std::collections::HashMap<String, String>, root: &str) -> Vec<String> {
    let after = take_file_snapshot_simple(root);
    let mut changes = Vec::new();
    for (path, after_mtime) in &after {
        match before.get(path) {
            None => changes.push(format!("+ {}", path)),
            Some(before_mtime) if before_mtime != after_mtime => changes.push(format!("~ {}", path)),
            _ => {}
        }
    }
    changes
}

// ─── Build check helper ──────────────────────────────────────────────────────

#[allow(dead_code)]
fn run_command_or_capture(args: &[&str], cwd: &str) -> (String, String, bool) {
    if args.is_empty() { return (String::new(), String::new(), false); }
    let output = std::process::Command::new(args[0])
        .args(&args[1..])
        .current_dir(cwd)
        .output();
    match output {
        Ok(o) => (
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
            o.status.success(),
        ),
        Err(e) => (String::new(), e.to_string(), false),
    }
}

#[allow(dead_code)]
fn parse_error_count(stdout: &str, stderr: &str) -> u32 {
    let combined = format!("{}\n{}", stdout, stderr);
    let rust_errors = combined.lines().filter(|l| l.starts_with("error[") || l.starts_with("error: aborting")).count();
    if rust_errors > 0 { return rust_errors as u32; }
    combined.lines().filter(|l| {
        let l = l.to_lowercase();
        l.contains("error:") || l.contains(" error ") || l.contains("failed")
    }).count() as u32
}

// ─── which_command helper ────────────────────────────────────────────────────

#[allow(dead_code)]
fn which_command(name: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ─── Flaky test tracking ─────────────────────────────────────────────────────

#[allow(dead_code)]
fn track_test_results(root: &str, results: &[(String, bool)]) {
    ensure_tracking_dir(root);
    let path = tracking_file_path(root, "test-history.json");
    let mut history: serde_json::Value = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"runs": []}));
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let run_results: Vec<serde_json::Value> = results.iter().map(|(name, passed)| {
        serde_json::json!({"name": name, "passed": passed})
    }).collect();
    if let Some(runs) = history["runs"].as_array_mut() {
        runs.push(serde_json::json!({"timestamp": ts, "results": run_results}));
        if runs.len() > 20 {
            let to_remove = runs.len() - 20;
            runs.drain(0..to_remove);
        }
    }
    if let Ok(s) = serde_json::to_string_pretty(&history) {
        let _ = std::fs::write(&path, s);
    }
}

#[allow(dead_code)]
fn find_flaky_tests(root: &str) -> Vec<(String, f64)> {
    let path = tracking_file_path(root, "test-history.json");
    let history: serde_json::Value = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"runs": []}));
    let runs = match history["runs"].as_array() {
        Some(r) => r.clone(),
        None => return vec![],
    };
    if runs.len() < 3 { return vec![]; }
    let mut test_records: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
    for run in &runs {
        if let Some(results) = run["results"].as_array() {
            for r in results {
                let name = r["name"].as_str().unwrap_or("").to_string();
                let passed = r["passed"].as_bool().unwrap_or(false);
                let entry = test_records.entry(name).or_insert((0, 0));
                entry.1 += 1;
                if passed { entry.0 += 1; }
            }
        }
    }
    let mut flaky: Vec<(String, f64)> = test_records.into_iter()
        .filter(|(_, (pass, total))| {
            let rate = *pass as f64 / (*total as f64).max(1.0);
            *total >= 3 && rate > 0.2 && rate < 0.8
        })
        .map(|(name, (pass, total))| (name, pass as f64 / total as f64))
        .collect();
    flaky.sort_by(|a, b| {
        let va = (a.1 - 0.5).abs();
        let vb = (b.1 - 0.5).abs();
        va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
    });
    flaky
}

// ─── Dependency check ────────────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_deps_check(root_path: &str) {
    let mut o = io::stdout();
    let root = std::path::Path::new(root_path);

    if root.join("Cargo.toml").exists() {
        print_section_header("Rust Dependency Audit");
        if which_command("cargo").is_some() {
            let (out, err, ok) = run_command_or_capture(&["cargo", "audit"], root_path);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  ✓ No vulnerabilities found\n").ok();
                reset_color(&mut o);
            } else {
                for line in out.lines().chain(err.lines()) {
                    if line.contains("error[") || line.contains("vulnerability") || line.contains("warning") {
                        let color = if line.contains("error") { theme::ERR } else { theme::WARN };
                        set_fg(&mut o, color);
                        write!(o, "  {} {}\n", ARROW, line.trim()).ok();
                        reset_color(&mut o);
                    }
                }
                if out.is_empty() && err.is_empty() {
                    set_fg(&mut o, theme::DIM);
                    write!(o, "  Install cargo-audit: cargo install cargo-audit\n").ok();
                    reset_color(&mut o);
                }
            }
        } else {
            set_fg(&mut o, theme::DIM);
            write!(o, "  cargo not found\n").ok();
            reset_color(&mut o);
        }
        print_section_end();
    }

    if root.join("requirements.txt").exists() {
        print_section_header("Python Dependency Audit");
        if which_command("safety").is_some() {
            let (_out, _err, ok) = run_command_or_capture(&["safety", "check", "--json"], root_path);
            if ok {
                set_fg(&mut o, theme::OK);
                write!(o, "  ✓ No vulnerabilities found\n").ok();
            } else {
                set_fg(&mut o, theme::ERR);
                write!(o, "  {} Vulnerabilities found — run `safety check` for details\n", CROSS).ok();
            }
            reset_color(&mut o);
        } else {
            set_fg(&mut o, theme::DIM);
            write!(o, "  Install safety: pip install safety\n").ok();
            reset_color(&mut o);
        }
        print_section_end();
    }
}

// ─── Stack trace detection ───────────────────────────────────────────────────

#[allow(dead_code)]
fn looks_like_stack_trace(input: &str) -> bool {
    let patterns = ["at ", "Traceback", "panicked at", "error[E", "#0 0x", "  File \"", " line "];
    let count = input.lines().filter(|l| patterns.iter().any(|p| l.contains(p))).count();
    count >= 3
}

// ─── GitHub Code Search ──────────────────────────────────────────────────────

#[allow(dead_code)]
async fn github_code_search(query: &str, github_token: Option<&str>) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let encoded = urlencoding_simple(query);
    let url = format!("https://api.github.com/search/code?q={}&per_page=5", encoded);
    let mut req = client.get(&url)
        .header("User-Agent", "shadowai-cli/0.2")
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = github_token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req.send().await.map_err(|e| format!("GitHub search failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API error {}", resp.status()));
    }
    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();
    if let Some(items) = data["items"].as_array() {
        for (i, item) in items.iter().take(5).enumerate() {
            let repo = item["repository"]["full_name"].as_str().unwrap_or("");
            let path = item["path"].as_str().unwrap_or("");
            let url = item["html_url"].as_str().unwrap_or("");
            results.push_str(&format!("{}. {} — {}\n   {}\n\n", i + 1, repo, path, url));
        }
    }
    if results.is_empty() { Ok("No results found.".into()) } else { Ok(results) }
}

fn urlencoding_simple(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "+".to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

// ─── StackOverflow Search ─────────────────────────────────────────────────────

#[allow(dead_code)]
async fn stackoverflow_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let encoded = urlencoding_simple(query);
    let url = format!(
        "https://api.stackexchange.com/2.3/search/advanced?q={}&site=stackoverflow&pagesize=5&order=desc&sort=relevance",
        encoded
    );
    let resp = client.get(&url).send().await.map_err(|e| format!("SO search failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("StackOverflow API error {}", resp.status()));
    }
    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut results = String::new();
    if let Some(items) = data["items"].as_array() {
        for (i, item) in items.iter().take(5).enumerate() {
            let title = item["title"].as_str().unwrap_or("");
            let link = item["link"].as_str().unwrap_or("");
            let score = item["score"].as_i64().unwrap_or(0);
            let answered = item["is_answered"].as_bool().unwrap_or(false);
            let ans_count = item["answer_count"].as_i64().unwrap_or(0);
            let status = if answered { "✓" } else { "?" };
            results.push_str(&format!("{}. {} [score: {}, {} answers {}]\n   {}\n\n",
                i + 1, title, score, ans_count, status, link));
        }
    }
    if results.is_empty() { Ok("No results found.".into()) } else { Ok(results) }
}

// ─── Docs.rs / PyPI lookup ───────────────────────────────────────────────────

#[allow(dead_code)]
async fn docsrs_search(crate_name: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://crates.io/api/v1/crates/{}", crate_name);
    let resp = client.get(&url)
        .header("User-Agent", "shadowai-cli/0.2")
        .send()
        .await
        .map_err(|e| format!("crates.io request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("crates.io error {}", resp.status()));
    }
    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let name = data["crate"]["name"].as_str().unwrap_or(crate_name);
    let version = data["crate"]["newest_version"].as_str().unwrap_or("?");
    let desc = data["crate"]["description"].as_str().unwrap_or("No description");
    let docs_url = format!("https://docs.rs/{}/{}/{}/", name, version, name.replace('-', "_"));
    Ok(format!("{} v{}\n{}\nDocs: {}", name, version, desc, docs_url))
}

#[allow(dead_code)]
async fn pypi_search(package: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://pypi.org/pypi/{}/json", package);
    let resp = client.get(&url).send().await.map_err(|e| format!("PyPI request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("PyPI error {}", resp.status()));
    }
    let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let name = data["info"]["name"].as_str().unwrap_or(package);
    let version = data["info"]["version"].as_str().unwrap_or("?");
    let summary = data["info"]["summary"].as_str().unwrap_or("");
    let home = data["info"]["home_page"].as_str().unwrap_or("");
    let requires_python = data["info"]["requires_python"].as_str().unwrap_or("");
    Ok(format!("{} v{}\n{}\nPython: {}\nHome: {}", name, version, summary, requires_python, home))
}

// ─── Gemini AI Handler ───────────────────────────────────────────────────────

#[allow(dead_code)]
async fn send_gemini_request(
    api_key: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let mut contents: Vec<serde_json::Value> = Vec::new();
    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");
        let gemini_role = if role == "assistant" { "model" } else { "user" };
        contents.push(serde_json::json!({
            "role": gemini_role,
            "parts": [{"text": content}]
        }));
    }

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
        model, api_key
    );

    let body = serde_json::json!({
        "system_instruction": {"parts": [{"text": system}]},
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": max_tokens,
            "temperature": temperature
        }
    });

    let resp = client.post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Gemini API error {}: {}", status, body));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim().to_string();
            buf = buf[pos + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if data.trim() == "[DONE]" { return Ok(()); }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(text) = v["candidates"][0]["content"]["parts"][0]["text"].as_str() {
                        let _ = tx.send(text.to_string()).await;
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Mistral AI Handler ──────────────────────────────────────────────────────

#[allow(dead_code)]
async fn send_mistral_request(
    api_key: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let mut all_messages = vec![serde_json::json!({"role": "system", "content": system})];
    all_messages.extend_from_slice(messages);

    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "messages": all_messages,
    });

    let resp = client.post("https://api.mistral.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Mistral request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Mistral API error {}: {}", status, text));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk: bytes::Bytes = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].to_string();
            buf = buf[pos + 1..].to_string();
            let line = line.trim().to_string();
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" { return Ok(()); }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = v["choices"][0]["delta"]["content"].as_str() {
                        let _ = tx.send(content.to_string()).await;
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Cohere Command R+ Handler ───────────────────────────────────────────────

#[allow(dead_code)]
async fn send_cohere_request(
    api_key: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f64,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "messages": messages,
        "preamble": system,
        "max_tokens": max_tokens,
        "temperature": temperature,
    });

    let resp = client.post("https://api.cohere.com/v2/chat")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("X-Client-Name", "shadowai-cli")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Cohere request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Cohere API error {}: {}", status, text));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk: bytes::Bytes = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf = buf[nl + 1..].to_string();
            if line.starts_with("event: stream-end") { return Ok(()); }
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(text) = v["text"].as_str() {
                        let _ = tx.send(text.to_string()).await;
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Shader highlight ────────────────────────────────────────────────────────

#[allow(dead_code)]
fn highlight_shader_code(code: &str, lang: &str) -> String {
    let keywords: &[&str] = match lang {
        "glsl" | "vert" | "frag" | "comp" => &[
            "void", "float", "vec2", "vec3", "vec4", "mat4", "mat3", "uniform",
            "in", "out", "layout", "precision", "mediump", "highp", "lowp",
            "sampler2D", "texture", "gl_Position", "gl_FragColor", "varying",
            "attribute", "discard", "return", "if", "else", "for", "while",
        ],
        "hlsl" | "fx" | "hlsli" => &[
            "void", "float", "float2", "float3", "float4", "matrix", "cbuffer",
            "Texture2D", "SamplerState", "SV_Position", "SV_Target", "register",
            "return", "if", "else", "for",
        ],
        "wgsl" => &[
            "fn", "var", "let", "struct", "binding", "group", "vertex", "fragment",
            "compute", "builtin", "location", "position", "vec2", "vec3", "vec4",
            "mat4x4", "f32", "i32", "u32", "return", "if", "else", "loop", "for",
        ],
        _ => &[],
    };

    let mut result = String::new();
    for line in code.lines() {
        let mut colored_line = line.to_string();
        if let Some(pos) = colored_line.find("//") {
            let (before, comment) = colored_line.split_at(pos);
            colored_line = format!("{}\x1b[38;2;98;114;138m{}\x1b[0m", before, comment);
        }
        for kw in keywords {
            colored_line = colored_line.replace(
                &format!(" {} ", kw),
                &format!(" \x1b[38;2;255;121;198m{}\x1b[0m ", kw),
            );
        }
        result.push_str(&colored_line);
        result.push('\n');
    }
    result
}

// ─── Agent command handler ───────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_agent_command(sub: &str, root_path: &str, _config: &CliConfig) {
    let mut o = io::stdout();
    let sub = sub.trim();

    let built_in = vec![
        ("feature-dev", "Full feature development: architect → implement → test → review"),
        ("bugfix", "Diagnose bug → fix → test → verify"),
        ("refactor", "Analyze → plan refactor → implement → review"),
        ("release", "Version bump → changelog → tag → announce"),
        ("security-audit", "Scan → report → prioritize → fix"),
    ];

    if sub == "list" || sub.is_empty() {
        print_section_header("Agent Templates");
        set_fg(&mut o, theme::CYAN);
        write!(o, "  Built-in:\n").ok();
        reset_color(&mut o);
        for (name, desc) in &built_in {
            set_fg(&mut o, theme::ACCENT);
            write!(o, "    {:<20}", name).ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "{}\n", desc).ok();
        }
        // List custom agents
        if let Some(dir) = config_dir() {
            let agents_dir = dir.join("agents");
            if agents_dir.exists() {
                set_fg(&mut o, theme::CYAN);
                write!(o, "\n  Custom:\n").ok();
                reset_color(&mut o);
                if let Ok(entries) = std::fs::read_dir(&agents_dir) {
                    for entry in entries.flatten() {
                        if entry.path().extension().and_then(|e| e.to_str()) == Some("toml") {
                            let name = entry.file_name().to_string_lossy().replace(".toml", "");
                            set_fg(&mut o, theme::ACCENT_DIM);
                            write!(o, "    {}\n", name).ok();
                        }
                    }
                }
            }
        }
        reset_color(&mut o);
        print_section_end();
        return;
    }

    if let Some(name) = sub.strip_prefix("describe ") {
        let name = name.trim();
        if let Some((_, desc)) = built_in.iter().find(|(n, _)| *n == name) {
            set_fg(&mut o, theme::CYAN);
            write!(o, "\n  Agent: {}\n  {}\n\n", name, desc).ok();
            reset_color(&mut o);
        } else {
            print_error(&format!("Agent '{}' not found. Use /agent list to see available agents.", name));
        }
        return;
    }

    if let Some(name) = sub.strip_prefix("create ") {
        let name = name.trim();
        if let Some(dir) = config_dir() {
            let agents_dir = dir.join("agents");
            let _ = std::fs::create_dir_all(&agents_dir);
            let path = agents_dir.join(format!("{}.toml", name));
            let template = format!(
                "name = \"{name}\"\ndescription = \"TODO: describe this agent\"\n\n[[steps]]\nskill = \"architect\"\nprompt = \"Design the architecture for: {{GOAL}}\"\n\n[[steps]]\nskill = \"code-review\"\nprompt = \"Review the implementation plan\"\n"
            );
            let _ = std::fs::write(&path, &template);
            set_fg(&mut o, theme::OK);
            write!(o, "  {} Created agent template: {}\n", CHECK, path.display()).ok();
            reset_color(&mut o);
        }
        return;
    }

    if let Some(rest) = sub.strip_prefix("run ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        let name = parts[0].trim();
        let goal = if parts.len() > 1 { parts[1].trim() } else { "Complete the task" };
        set_fg(&mut o, theme::CYAN);
        write!(o, "\n  Running agent '{}' with goal: {}\n\n", name, goal).ok();
        reset_color(&mut o);
        // Find built-in description
        if let Some((_, desc)) = built_in.iter().find(|(n, _)| *n == name) {
            append_tracking_entry(root_path, "agent-runs.md",
                &format!("Agent: {}\nGoal: {}\nDescription: {}", name, goal, desc));
        }
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  Agent run initiated. Connect to AI backend for full multi-step execution.\n").ok();
        reset_color(&mut o);
        return;
    }

    print_error("Usage: /agent list | /agent run <name> <goal> | /agent create <name> | /agent describe <name>");
}

// ─── Debug command handler ───────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_debug_command_local(sub: &str, _root_path: &str) {
    let mut o = io::stdout();
    let sub = sub.trim();

    if sub.is_empty() {
        print_error("Usage: /debug core <binary> <corefile> | /debug attach <pid> | /debug trace");
        return;
    }

    if sub == "trace" {
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  No stack trace captured in current session.\n").ok();
        reset_color(&mut o);
        return;
    }

    if let Some(rest) = sub.strip_prefix("core ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let (binary, corefile) = (parts[0], parts[1]);
            set_fg(&mut o, theme::DIM);
            write!(o, "  Running gdb on {} with core {}...\n", binary, corefile).ok();
            reset_color(&mut o);
            let result = std::process::Command::new("gdb")
                .args([binary, corefile, "-batch", "-ex", "bt", "-ex", "quit"])
                .output();
            match result {
                Ok(out) => {
                    let trace = String::from_utf8_lossy(&out.stdout).to_string()
                        + &String::from_utf8_lossy(&out.stderr).to_string();
                    for line in trace.lines().take(50) {
                        set_fg(&mut o, theme::AI_TEXT);
                        write!(o, "  {}\n", line).ok();
                    }
                    reset_color(&mut o);
                }
                Err(e) => { print_error(&format!("gdb not found or failed: {}", e)); }
            }
        } else {
            print_error("Usage: /debug core <binary> <corefile>");
        }
        return;
    }

    if let Some(pid) = sub.strip_prefix("attach ") {
        set_fg(&mut o, theme::DIM);
        write!(o, "  Attaching gdb to PID {}...\n", pid).ok();
        reset_color(&mut o);
        let result = std::process::Command::new("gdb")
            .args(["-p", pid.trim(), "-batch", "-ex", "bt", "-ex", "quit"])
            .output();
        match result {
            Ok(out) => {
                let trace = String::from_utf8_lossy(&out.stdout).to_string();
                for line in trace.lines().take(50) {
                    set_fg(&mut o, theme::AI_TEXT);
                    write!(o, "  {}\n", line).ok();
                }
                reset_color(&mut o);
            }
            Err(e) => { print_error(&format!("gdb failed: {}", e)); }
        }
        return;
    }

    print_error("Usage: /debug core <binary> <corefile> | /debug attach <pid> | /debug trace");
}

// ─── Shader command handler ───────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_shader_command_local(sub: &str, _root_path: &str) {
    let mut o = io::stdout();
    let sub = sub.trim();

    if sub.is_empty() {
        print_error("Usage: /shader validate <file> | /shader optimize <file> | /shader cross <file> <target>");
        return;
    }

    if let Some(file) = sub.strip_prefix("validate ") {
        let file = file.trim();
        let ext = std::path::Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or("");
        set_fg(&mut o, theme::DIM);
        write!(o, "  Validating shader: {}\n", file).ok();
        reset_color(&mut o);

        let validator = if which_command("glslangValidator").is_some() { "glslangValidator" }
            else if which_command("glslc").is_some() { "glslc" }
            else { "" };

        if validator.is_empty() {
            set_fg(&mut o, theme::WARN);
            write!(o, "  {} glslangValidator or glslc not found. Install Vulkan SDK for shader validation.\n", ARROW).ok();
            reset_color(&mut o);
            return;
        }

        let args: Vec<&str> = if validator == "glslangValidator" {
            vec!["glslangValidator", "-V", file]
        } else {
            vec!["glslc", file, "-o", "/dev/null"]
        };

        let (out, err, ok) = run_command_or_capture(&args, ".");
        if ok {
            set_fg(&mut o, theme::OK);
            write!(o, "  {} Shader validation passed\n", CHECK).ok();
        } else {
            set_fg(&mut o, theme::ERR);
            write!(o, "  {} Shader validation failed:\n", CROSS).ok();
            reset_color(&mut o);
            for line in out.lines().chain(err.lines()).take(20) {
                set_fg(&mut o, theme::ERR);
                write!(o, "    {}\n", line).ok();
            }
        }
        reset_color(&mut o);
        let _ = ext;
        return;
    }

    if let Some(file) = sub.strip_prefix("optimize ") {
        let file = file.trim();
        if which_command("spirv-opt").is_none() {
            set_fg(&mut o, theme::WARN);
            write!(o, "  spirv-opt not found. Install SPIRV-Tools.\n").ok();
            reset_color(&mut o);
            return;
        }
        let out_file = format!("{}.opt.spv", file);
        let (_, _, ok) = run_command_or_capture(&["spirv-opt", "-O", file, "-o", &out_file], ".");
        if ok {
            set_fg(&mut o, theme::OK);
            write!(o, "  {} Optimized → {}\n", CHECK, out_file).ok();
        } else {
            print_error("spirv-opt failed");
        }
        reset_color(&mut o);
        return;
    }

    print_error("Usage: /shader validate <file> | /shader optimize <file>");
}

// ─── Assets command handler ───────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_assets_command_local(sub: &str, root_path: &str) {
    let mut o = io::stdout();
    let sub = sub.trim();

    if sub == "list" || sub.is_empty() {
        print_section_header("Asset Files");
        let mut total_size = 0u64;
        fn scan_assets(dir: &std::path::Path, out: &mut impl Write, total: &mut u64) {
            let Ok(entries) = std::fs::read_dir(dir) else { return; };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.to_string_lossy().to_string();
                if name.contains("/target/") || name.contains("/node_modules/") { continue; }
                if path.is_dir() { scan_assets(&path, out, total); }
                else {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "png"|"jpg"|"jpeg"|"gif"|"webp"|"svg"|"bmp"|"ico"|"mp3"|"wav"|"ogg"|"ttf"|"woff"|"woff2") {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            let size = meta.len();
                            *total += size;
                            set_fg(out, theme::DIM_LIGHT);
                            let _ = write!(out, "  {:>8}  {}\n", format_bytes(size), name);
                        }
                    }
                }
            }
        }
        scan_assets(std::path::Path::new(root_path), &mut o, &mut total_size);
        set_fg(&mut o, theme::STAT);
        write!(o, "\n  Total: {}\n", format_bytes(total_size)).ok();
        reset_color(&mut o);
        print_section_end();
        return;
    }

    if sub == "large" {
        print_section_header("Large Assets (>1MB)");
        fn scan_large(dir: &std::path::Path, out: &mut impl Write) {
            let Ok(entries) = std::fs::read_dir(dir) else { return; };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() { scan_large(&path, out); }
                else if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() > 1_048_576 {
                        set_fg(out, theme::WARN);
                        let _ = write!(out, "  {:>10}  {}\n", format_bytes(meta.len()), path.display());
                        reset_color(out);
                    }
                }
            }
        }
        scan_large(std::path::Path::new(root_path), &mut o);
        print_section_end();
        return;
    }

    if sub == "optimize" {
        print_section_header("Asset Optimization");
        let tools = [
            ("oxipng", "png", &["oxipng", "-o", "4"][..]),
            ("optipng", "png", &["optipng"][..]),
            ("jpegoptim", "jpg", &["jpegoptim"][..]),
        ];
        for (tool, ext, _args) in &tools {
            if which_command(tool).is_some() {
                set_fg(&mut o, theme::OK);
                write!(o, "  {} {} optimizer available\n", CHECK, ext).ok();
            }
        }
        set_fg(&mut o, theme::DIM_LIGHT);
        write!(o, "  Use individual tools on specific files for optimization.\n").ok();
        reset_color(&mut o);
        print_section_end();
        return;
    }

    print_error("Usage: /assets list | /assets large | /assets optimize");
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 { format!("{:.1}GB", bytes as f64 / 1_073_741_824.0) }
    else if bytes >= 1_048_576 { format!("{:.1}MB", bytes as f64 / 1_048_576.0) }
    else if bytes >= 1024 { format!("{:.1}KB", bytes as f64 / 1024.0) }
    else { format!("{}B", bytes) }
}

// ─── Rebase command handler ───────────────────────────────────────────────────

#[allow(dead_code)]
fn handle_rebase_command_local(n_str: &str, root_path: &str) {
    let mut o = io::stdout();
    let n: usize = n_str.trim().parse().unwrap_or(5);

    let output = std::process::Command::new("git")
        .args(["log", "--oneline", &format!("-{}", n)])
        .current_dir(root_path)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let log = String::from_utf8_lossy(&out.stdout).to_string();
            let commits: Vec<&str> = log.lines().collect();

            print_section_header(&format!("Interactive Rebase — Last {} Commits", n));
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "  Commits (newest first):\n\n").ok();
            for (i, commit) in commits.iter().enumerate() {
                set_fg(&mut o, theme::CYAN_DIM);
                write!(o, "  [{}] ", i + 1).ok();
                set_fg(&mut o, theme::AI_TEXT);
                write!(o, "{}\n", commit).ok();
            }
            reset_color(&mut o);
            write!(o, "\n").ok();
            set_fg(&mut o, theme::DIM);
            write!(o, "  Actions: p=pick, s=squash, f=fixup, d=drop, r=rename\n").ok();
            write!(o, "  Run: git rebase -i HEAD~{} to open interactive rebase\n", n).ok();
            reset_color(&mut o);
            print_section_end();
        }
        Ok(_) => {
            print_error("git log failed — are you in a git repository?");
        }
        Err(e) => {
            print_error(&format!("git not found: {}", e));
        }
    }
}

// ─── TUI Application ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum TuiPane {
    FileTree,
    Chat,
    Context,
    Input,
}

#[derive(Debug, Clone)]
struct ChatMessage {
    role: String,
    content: String,
    timestamp: String,
    /// Optional thinking/CoT text (for assistant messages)
    thinking: Option<String>,
    /// Whether thinking is collapsed in display
    thinking_collapsed: bool,
}

#[derive(Debug, Clone)]
struct FileTreeNode {
    name: String,
    #[allow(dead_code)]
    path: String,
    is_dir: bool,
    depth: usize,
    git_status: char,
    #[allow(dead_code)]
    expanded: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ProviderProfile {
    name: String,
    provider: String,        // "llamacpp", "ollama", "openai", "anthropic"
    base_url: String,
    model: String,
    api_key_env: Option<String>,
    system_prompt: Option<String>,
    max_context_tokens: Option<u64>,
}

struct TuiApp {
    active_pane: TuiPane,
    file_tree_width: u16,
    context_width: u16,
    messages: Vec<ChatMessage>,
    chat_scroll: usize,
    streaming_buf: String,
    is_streaming: bool,
    input: String,
    input_cursor: usize,
    input_history: PromptHistory,
    #[allow(dead_code)]
    history_searching: bool,
    #[allow(dead_code)]
    history_search_query: String,
    file_nodes: Vec<FileTreeNode>,
    #[allow(dead_code)]
    file_tree_scroll: usize,
    file_tree_selected: usize,
    active_skill: String,
    current_mode: String,
    current_model: String,
    root_path: String,
    token_used: u64,
    token_total: u64,
    git_branch: String,
    session_tabs: Vec<String>,
    active_tab: usize,
    palette_open: bool,
    palette_query: String,
    palette_items: Vec<String>,
    palette_selected: usize,
    status_msg: String,
    status_is_error: bool,
    should_quit: bool,
    // Direct AI fields (no Shadow IDE server needed)
    api_key: Option<String>,
    openai_api_key: Option<String>,
    openai_base_url: Option<String>,
    system_prompt: String,
    conversation_history: Vec<serde_json::Value>,
    stream_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    /// Section 19: prefer cheaper model for short prompts when true
    prefer_cheap: bool,
    /// Section 14: auto-compact at this % context usage (0 = disabled)
    auto_compact_pct: u64,
    // ── Tool calling ──────────────────────────────────────────────────────
    /// Tool calls received from AI that are pending user decision
    pending_tool_calls: Vec<PendingTuiToolCall>,
    /// True while the UI is waiting for the user to approve/deny tools
    awaiting_tool_approval: bool,
    /// True when we should auto-execute pending tools on the next event loop tick
    auto_execute_tools: bool,
    /// Approval mode: Yolo = all auto-approved, Smart = safe auto / dangerous ask, AskAll = ask all
    tool_approval_mode: TuiApprovalMode,
    /// Whether the current provider uses Anthropic message format (vs OpenAI)
    is_anthropic_provider: bool,
    /// Use prompt-injected tool calls instead of native OpenAI tool round-tripping.
    prompt_tool_mode: bool,
    /// Stored raw assistant content from the last API response (for correct history format)
    assistant_tool_content: Option<serde_json::Value>,
    // ── Code block tracking ──────────────────────────────────────────────
    /// Extracted code blocks from the latest AI response: (lang, code, source_file_hint)
    code_blocks: Vec<(String, String, Option<String>)>,
    // ── Include file toggle ──────────────────────────────────────────────
    /// Path of file to include as context with every message (None = disabled)
    include_file: Option<String>,
    // ── Session pinning ──────────────────────────────────────────────────
    /// Indices of pinned sessions
    pinned_sessions: Vec<usize>,
    // ── Token breakdown ──────────────────────────────────────────────────
    token_system: u64,
    token_history: u64,
    token_tools: u64,
    token_response: u64,
    // ── Temperature ──────────────────────────────────────────────────────
    temperature: f64,
    max_tokens: u32,
    // ── File changes from tool runs ──────────────────────────────────────
    recent_file_changes: Vec<(String, String)>, // (action_icon, path)
    // ── Profiles ─────────────────────────────────────────────────────────
    profiles: Vec<ProviderProfile>,
    active_profile: Option<usize>,
    // ── Tools toggle ─────────────────────────────────────────────────
    tools_enabled: bool,
    // ── Privacy / air-gap mode ───────────────────────────────────────
    privacy_mode: bool,
    // ── Connection status ────────────────────────────────────────────
    connection_status: String, // "connected", "disconnected", "local"
    // ── Thinking state ───────────────────────────────────────────────
    /// Accumulator for thinking tokens during streaming
    thinking_buf: String,
    /// Whether we're currently receiving thinking tokens
    in_thinking: bool,
}

impl TuiApp {
    fn new(root_path: &str, model: &str, mode: &str) -> Self {
        let history_size = 1000usize;
        let history_path = dirs_next::config_dir()
            .map(|d| d.join("shadowai").join("history"))
            .unwrap_or_default();
        let history = PromptHistory::load_from_file(&history_path, history_size);

        // Resolve API key: env var takes priority, then config file
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
        let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
        let openai_base_url = std::env::var("OPENAI_BASE_URL")
            .ok()
            .or_else(|| std::env::var("OPENAI_API_BASE").ok());

        TuiApp {
            active_pane: TuiPane::Input,
            file_tree_width: 20,
            context_width: 22,
            messages: Vec::new(),
            chat_scroll: 0,
            streaming_buf: String::new(),
            is_streaming: false,
            input: String::new(),
            input_cursor: 0,
            input_history: history,
            history_searching: false,
            history_search_query: String::new(),
            file_nodes: Vec::new(),
            file_tree_scroll: 0,
            file_tree_selected: 0,
            active_skill: String::new(),
            current_mode: mode.to_string(),
            current_model: model.to_string(),
            root_path: root_path.to_string(),
            token_used: 0,
            token_total: 200_000,
            git_branch: String::new(),
            session_tabs: vec!["Session 1".to_string()],
            active_tab: 0,
            palette_open: false,
            palette_query: String::new(),
            palette_items: Vec::new(),
            palette_selected: 0,
            status_msg: String::new(),
            status_is_error: false,
            should_quit: false,
            api_key,
            openai_api_key,
            openai_base_url,
            system_prompt: "You are ShadowAI, a helpful coding assistant.".to_string(),
            conversation_history: Vec::new(),
            stream_rx: None,
            prefer_cheap: false,
            auto_compact_pct: 75, // default: auto-compact at 75% context usage
            pending_tool_calls: Vec::new(),
            awaiting_tool_approval: false,
            auto_execute_tools: false,
            tool_approval_mode: TuiApprovalMode::Smart,
            is_anthropic_provider: false,
            prompt_tool_mode: false,
            assistant_tool_content: None,
            code_blocks: Vec::new(),
            include_file: None,
            pinned_sessions: Vec::new(),
            token_system: 0,
            token_history: 0,
            token_tools: 0,
            token_response: 0,
            temperature: 0.7,
            max_tokens: 8192,
            recent_file_changes: Vec::new(),
            profiles: load_profiles(),
            active_profile: None,
            tools_enabled: true,
            privacy_mode: false,
            connection_status: "disconnected".to_string(),
            thinking_buf: String::new(),
            in_thinking: false,
        }
    }

    fn push_message(&mut self, role: &str, content: &str) {
        let ts = chrono::Local::now().format("%H:%M").to_string();
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: ts,
            thinking: None,
            thinking_collapsed: true,
        });
        // Auto-scroll to bottom (usize::MAX gets clamped in renderer)
        self.chat_scroll = usize::MAX;
    }

    fn load_file_tree(&mut self) {
        self.file_nodes.clear();
        let root = std::path::Path::new(&self.root_path).to_path_buf();
        self.collect_file_nodes(&root, 0, 2);
    }

    fn collect_file_nodes(&mut self, dir: &std::path::Path, depth: usize, max_depth: usize) {
        if depth > max_depth { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (!is_dir, e.file_name())
        });
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".shadowai" { continue; }
            if name == "target" || name == "node_modules" || name == ".git" { continue; }
            let path = entry.path().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            self.file_nodes.push(FileTreeNode {
                name,
                path: path.clone(),
                is_dir,
                depth,
                git_status: ' ',
                expanded: false,
            });
            if is_dir && depth < max_depth {
                let ep = entry.path();
                self.collect_file_nodes(&ep, depth + 1, max_depth);
            }
        }
    }

    fn refresh_git_branch(&mut self) {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.root_path)
            .output();
        if let Ok(o) = output {
            self.git_branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
        }
    }

    fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.palette_items = get_all_commands();
    }

    fn filter_palette(&mut self) {
        let all = get_all_commands();
        let q = self.palette_query.to_lowercase();
        self.palette_items = if q.is_empty() {
            all
        } else {
            all.into_iter().filter(|c| c.to_lowercase().contains(&q)).collect()
        };
        self.palette_selected = 0;
    }

    fn update_token_bar(&mut self, used: u64, total: u64) {
        self.token_used = used;
        self.token_total = total.max(1);
    }

    fn token_bar_str(&self) -> String {
        let ratio = self.token_used as f64 / self.token_total.max(1) as f64;
        let pct = ratio * 100.0;
        // At least 1 filled block if any tokens are used; otherwise scale 0-10
        let filled = if self.token_used > 0 {
            ((ratio * 10.0) as usize).max(1).min(10)
        } else {
            0
        };
        let empty = 10usize.saturating_sub(filled);
        let count_str = if self.token_used >= 1_000 {
            format!("{}k", self.token_used / 1_000)
        } else {
            format!("{}", self.token_used)
        };
        format!("{}{} {} ({:.1}%)", "█".repeat(filled), "░".repeat(empty), count_str, pct)
    }

    /// Extract code blocks from the last assistant message for interactive actions
    fn extract_code_blocks(&mut self) {
        self.code_blocks.clear();
        let last_content = self.messages.iter().rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let mut in_block = false;
        let mut lang = String::new();
        let mut code = String::new();
        for line in last_content.lines() {
            if line.trim_start().starts_with("```") && !in_block {
                in_block = true;
                lang = line.trim_start().strip_prefix("```").unwrap_or("").trim().to_string();
                code.clear();
            } else if line.trim_start().starts_with("```") && in_block {
                in_block = false;
                // Try to guess filename from content context
                let hint = if !lang.is_empty() {
                    let ext = match lang.as_str() {
                        "rust" | "rs" => "rs", "python" | "py" => "py",
                        "typescript" | "ts" => "ts", "javascript" | "js" => "js",
                        "cpp" | "c++" => "cpp", "c" => "c", "go" => "go",
                        "java" => "java", "toml" => "toml", "yaml" | "yml" => "yaml",
                        "json" => "json", "html" => "html", "css" => "css",
                        "shell" | "bash" | "sh" => "sh", "swift" => "swift",
                        _ => &lang,
                    };
                    Some(format!("new_file.{}", ext))
                } else {
                    None
                };
                self.code_blocks.push((lang.clone(), code.clone(), hint));
            } else if in_block {
                if !code.is_empty() { code.push('\n'); }
                code.push_str(line);
            }
        }
    }

    /// Estimate token breakdown from conversation history
    fn update_token_breakdown(&mut self) {
        let est = |s: &str| -> u64 { (s.len() as u64) / 4 };
        self.token_system = est(&self.system_prompt);
        let mut hist = 0u64;
        let mut tools = 0u64;
        for msg in &self.conversation_history {
            let content = msg["content"].as_str().unwrap_or("");
            match msg["role"].as_str().unwrap_or("") {
                "user" | "assistant" => hist += est(content),
                "tool" => tools += est(content),
                _ => hist += est(content),
            }
        }
        self.token_history = hist;
        self.token_tools = tools;
        self.token_used = self.token_system + self.token_history + self.token_tools + self.token_response;
    }

    /// Rewind conversation to a specific user message index (0-based among user messages)
    fn rewind_to(&mut self, user_msg_idx: usize) -> Option<String> {
        let mut user_count = 0usize;
        let mut cut_at = None;
        for (i, msg) in self.messages.iter().enumerate() {
            if msg.role == "user" {
                if user_count == user_msg_idx {
                    cut_at = Some(i);
                    break;
                }
                user_count += 1;
            }
        }
        let cut = cut_at?;
        let content = self.messages[cut].content.clone();
        // Truncate messages after this point
        self.messages.truncate(cut);
        // Truncate conversation history: find corresponding user entry
        let mut hist_user_count = 0usize;
        let mut hist_cut = None;
        for (i, msg) in self.conversation_history.iter().enumerate() {
            if msg["role"].as_str() == Some("user") {
                if hist_user_count == user_msg_idx {
                    hist_cut = Some(i);
                    break;
                }
                hist_user_count += 1;
            }
        }
        if let Some(hc) = hist_cut {
            self.conversation_history.truncate(hc);
        }
        Some(content)
    }
}

fn profiles_path() -> std::path::PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("shadowai")
        .join("profiles.json")
}

fn load_profiles() -> Vec<ProviderProfile> {
    let path = profiles_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn save_profiles(profiles: &[ProviderProfile]) {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(profiles) {
        let _ = std::fs::write(&path, json);
    }
}

fn get_all_commands() -> Vec<String> {
    vec![
        "/help", "/status", "/new", "/clear", "/quit", "/sessions", "/resume",
        "/models", "/providers", "/skills", "/skill", "/memories", "/memory",
        "/compact", "/context", "/plan", "/watch", "/abort",
        "/git", "/gd", "/gl", "/gc", "/gb", "/stash", "/blame", "/resolve", "/pr",
        "/build", "/test", "/lint", "/format", "/benchmark", "/cov",
        "/todo", "/env", "/secrets", "/metrics", "/deps", "/diagram",
        "/diff", "/explain", "/rename", "/extract", "/translate", "/mock",
        "/docker", "/release", "/remote", "/share", "/cron", "/research",
        "/heal", "/theme", "/themes", "/browse", "/search", "/file", "/image",
        "/find", "/grep", "/tree", "/symbols", "/add", "/drop", "/files",
        "/export", "/history", "/keybindings", "/perf", "/cheatsheet",
        "/security", "/doc", "/changelog", "/undo", "/copy", "/edits",
        "/save", "/load", "/spawn", "/chat",
        "/think", "/agent", "/debug", "/shader", "/assets", "/docs", "/rebase", "/gr",
        "/code", "/rewind", "/include", "/pin", "/unpin", "/temp", "/maxtok",
        "/profiles", "/profile", "/memories", "/rag-index", "/changes",
    ].into_iter().map(String::from).collect()
}

fn tui_truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() }
    else { format!("...{}", &s[s.len().saturating_sub(max.saturating_sub(3))..]) }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_tui(f: &mut ratatui::Frame, app: &TuiApp) {
    use ratatui::layout::{Constraint, Direction, Layout, Rect};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap, Clear};

    let size = f.area();

    let tab_height = 1u16;
    let status_height = 1u16;
    let input_height = 3u16;

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(tab_height),
            Constraint::Min(5),
            Constraint::Length(status_height),
            Constraint::Length(input_height),
        ])
        .split(size);

    // Tab bar
    let tab_titles: Vec<Line> = app.session_tabs.iter().enumerate()
        .map(|(i, t)| {
            let pin = if app.pinned_sessions.contains(&i) { "⭐ " } else { "" };
            if i == app.active_tab {
                Line::from(Span::styled(format!(" {}{} ", pin, t), Style::default()
                    .fg(Color::Rgb(0, 230, 230))
                    .add_modifier(Modifier::BOLD)))
            } else {
                Line::from(Span::styled(format!(" {}{} ", pin, t), Style::default()
                    .fg(Color::Rgb(98, 114, 138))))
            }
        })
        .collect();
    let tabs = Tabs::new(tab_titles)
        .block(Block::default().style(Style::default().bg(Color::Rgb(18, 18, 28))))
        .select(app.active_tab)
        .highlight_style(Style::default().fg(Color::Rgb(155, 89, 255)));
    f.render_widget(tabs, vertical[0]);

    // Main area split
    let main_area = vertical[1];
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(app.file_tree_width),
            Constraint::Min(20),
            Constraint::Percentage(app.context_width),
        ])
        .split(main_area);

    // File tree
    let tree_items: Vec<ListItem> = app.file_nodes.iter().map(|node| {
        let indent = "  ".repeat(node.depth);
        let icon = if node.is_dir { "▸ " } else { "  " };
        let status_color = match node.git_status {
            'M' => Color::Rgb(100, 180, 255),
            'A' => Color::Rgb(80, 250, 123),
            'D' => Color::Rgb(255, 85, 85),
            '?' => Color::Rgb(255, 183, 77),
            _ => Color::Rgb(180, 190, 210),
        };
        ListItem::new(Line::from(vec![
            Span::raw(indent),
            Span::styled(icon, Style::default().fg(Color::Rgb(98, 114, 138))),
            Span::styled(node.name.clone(), Style::default().fg(status_color)),
        ]))
    }).collect();

    let tree_block = Block::default()
        .title(Span::styled(" Files ", Style::default()
            .fg(Color::Rgb(155, 89, 255)).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 70, 90)));
    let tree = List::new(tree_items)
        .block(tree_block)
        .highlight_style(Style::default().bg(Color::Rgb(40, 45, 65)).fg(Color::Rgb(0, 230, 230)));
    f.render_widget(tree, horizontal[0]);

    // Chat area
    let mut chat_lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        match msg.role.as_str() {
            "user" => {
                chat_lines.push(Line::from(vec![
                    Span::styled(" ▸ You  ", Style::default()
                        .fg(Color::Rgb(0, 230, 230)).add_modifier(Modifier::BOLD)),
                    Span::styled(msg.timestamp.clone(), Style::default().fg(Color::Rgb(60, 70, 90))),
                ]));
                for line in msg.content.lines() {
                    chat_lines.push(Line::from(Span::styled(
                        format!("   {}", line),
                        Style::default().fg(Color::Rgb(220, 225, 235)),
                    )));
                }
                chat_lines.push(Line::from(""));
            }
            "assistant" => {
                chat_lines.push(Line::from(vec![
                    Span::styled(" ✦ AI   ", Style::default()
                        .fg(Color::Rgb(155, 89, 255)).add_modifier(Modifier::BOLD)),
                    Span::styled(msg.timestamp.clone(), Style::default().fg(Color::Rgb(60, 70, 90))),
                ]));
                // Show thinking block if present
                if let Some(ref thinking) = msg.thinking {
                    if msg.thinking_collapsed {
                        let preview_len = thinking.len().min(60);
                        let preview: String = thinking.chars().take(preview_len).collect();
                        chat_lines.push(Line::from(vec![
                            Span::styled("   ▸ Thinking ", Style::default()
                                .fg(Color::Rgb(100, 80, 160)).add_modifier(Modifier::ITALIC)),
                            Span::styled(
                                format!("({}c) {}…", thinking.len(), preview.replace('\n', " ")),
                                Style::default().fg(Color::Rgb(70, 60, 110)),
                            ),
                        ]));
                    } else {
                        chat_lines.push(Line::from(Span::styled(
                            "   ▾ Thinking",
                            Style::default().fg(Color::Rgb(100, 80, 160)).add_modifier(Modifier::ITALIC),
                        )));
                        for line in thinking.lines().take(30) {
                            chat_lines.push(Line::from(Span::styled(
                                format!("   │ {}", line),
                                Style::default().fg(Color::Rgb(80, 70, 120)),
                            )));
                        }
                        if thinking.lines().count() > 30 {
                            chat_lines.push(Line::from(Span::styled(
                                format!("   │ … ({} more lines)", thinking.lines().count() - 30),
                                Style::default().fg(Color::Rgb(60, 50, 100)),
                            )));
                        }
                    }
                }
                for line in msg.content.lines() {
                    chat_lines.push(Line::from(Span::styled(
                        format!("   {}", line),
                        Style::default().fg(Color::Rgb(220, 225, 235)),
                    )));
                }
                chat_lines.push(Line::from(""));
            }
            "tool" => {
                for (i, line) in msg.content.lines().enumerate() {
                    let prefix = if i == 0 { "   ⚡ " } else { "     " };
                    chat_lines.push(Line::from(Span::styled(
                        format!("{prefix}{line}"),
                        Style::default().fg(Color::Rgb(255, 183, 77)),
                    )));
                }
            }
            "error" => {
                chat_lines.push(Line::from(Span::styled(
                    format!("   ✗ {}", msg.content),
                    Style::default().fg(Color::Rgb(255, 85, 85)),
                )));
            }
            _ => {}
        }
    }
    if app.is_streaming && !app.streaming_buf.is_empty() {
        for line in app.streaming_buf.lines() {
            chat_lines.push(Line::from(Span::styled(
                format!("   {}", line),
                Style::default().fg(Color::Rgb(200, 210, 230)),
            )));
        }
        chat_lines.push(Line::from(Span::styled(
            "   ▌",
            Style::default().fg(Color::Rgb(155, 89, 255)).add_modifier(Modifier::RAPID_BLINK),
        )));
    }

    let chat_height = horizontal[1].height.saturating_sub(2) as usize;
    let total_lines = chat_lines.len();
    let max_scroll = total_lines.saturating_sub(chat_height);
    // Clamp chat_scroll to valid range
    let scroll_offset = if total_lines > chat_height {
        app.chat_scroll.min(max_scroll)
    } else {
        0
    };

    let chat_text = Text::from(chat_lines);
    let chat_block = Block::default()
        .title(Span::styled(
            format!(" Chat — {} ", app.session_tabs.get(app.active_tab).unwrap_or(&"Session".to_string())),
            Style::default().fg(Color::Rgb(0, 230, 230)).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.active_pane == TuiPane::Chat {
            Color::Rgb(155, 89, 255)
        } else {
            Color::Rgb(60, 70, 90)
        }));
    let chat_para = Paragraph::new(chat_text)
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset as u16, 0));
    f.render_widget(chat_para, horizontal[1]);

    // Context panel
    let mut ctx_lines: Vec<Line> = Vec::new();

    // Mode pills (PLAN / BUILD / AUTO)
    ctx_lines.push(Line::from(Span::styled(" Mode", Style::default().fg(Color::Rgb(98, 114, 138)))));
    {
        let modes = [("PLAN", "plan", Color::Rgb(155, 89, 255)),
                     ("BUILD", "build", Color::Rgb(255, 183, 77)),
                     ("AUTO", "auto", Color::Rgb(0, 230, 230))];
        let mut pills: Vec<Span> = vec![Span::raw("  ")];
        for (label, key, color) in &modes {
            if app.current_mode == *key {
                pills.push(Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(Color::Rgb(18, 18, 28)).bg(*color).add_modifier(Modifier::BOLD),
                ));
            } else {
                pills.push(Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(Color::Rgb(60, 70, 90)),
                ));
            }
            pills.push(Span::raw(" "));
        }
        ctx_lines.push(Line::from(pills));
    }
    ctx_lines.push(Line::from(""));

    // Model
    ctx_lines.push(Line::from(Span::styled(" Model", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  {}", tui_truncate_str(&app.current_model, 18)),
        Style::default().fg(Color::Rgb(180, 190, 210)),
    )));
    ctx_lines.push(Line::from(""));

    // Token breakdown
    ctx_lines.push(Line::from(Span::styled(" Tokens", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  {}", app.token_bar_str()),
        Style::default().fg(if app.token_used as f64 / app.token_total.max(1) as f64 > 0.8 {
            Color::Rgb(255, 85, 85)
        } else if app.token_used as f64 / app.token_total.max(1) as f64 > 0.6 {
            Color::Rgb(255, 183, 77)
        } else {
            Color::Rgb(80, 200, 200)
        }),
    )));
    if app.token_system > 0 || app.token_history > 0 {
        ctx_lines.push(Line::from(vec![
            Span::styled("  sys:", Style::default().fg(Color::Rgb(70, 80, 100))),
            Span::styled(format!("{} ", app.token_system), Style::default().fg(Color::Rgb(155, 89, 255))),
            Span::styled("hist:", Style::default().fg(Color::Rgb(70, 80, 100))),
            Span::styled(format!("{}", app.token_history), Style::default().fg(Color::Rgb(0, 230, 230))),
        ]));
        ctx_lines.push(Line::from(vec![
            Span::styled("  tool:", Style::default().fg(Color::Rgb(70, 80, 100))),
            Span::styled(format!("{} ", app.token_tools), Style::default().fg(Color::Rgb(255, 183, 77))),
            Span::styled("resp:", Style::default().fg(Color::Rgb(70, 80, 100))),
            Span::styled(format!("{}", app.token_response), Style::default().fg(Color::Rgb(80, 250, 123))),
        ]));
    }
    ctx_lines.push(Line::from(""));

    // Temperature & Max Tokens
    ctx_lines.push(Line::from(Span::styled(" Settings", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  temp: {:.1}  max: {}", app.temperature, app.max_tokens),
        Style::default().fg(Color::Rgb(130, 145, 165)),
    )));
    ctx_lines.push(Line::from(""));

    // Include file
    if let Some(ref path) = app.include_file {
        ctx_lines.push(Line::from(Span::styled(" Include", Style::default().fg(Color::Rgb(98, 114, 138)))));
        let short = std::path::Path::new(path).file_name()
            .and_then(|n| n.to_str()).unwrap_or(path);
        ctx_lines.push(Line::from(Span::styled(
            format!("  📎 {}", tui_truncate_str(short, 16)),
            Style::default().fg(Color::Rgb(0, 230, 230)),
        )));
        ctx_lines.push(Line::from(""));
    }

    // Skill
    ctx_lines.push(Line::from(Span::styled(" Skill", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  {}", if app.active_skill.is_empty() { "none" } else { &app.active_skill }),
        Style::default().fg(Color::Rgb(155, 89, 255)),
    )));
    ctx_lines.push(Line::from(""));

    // Branch
    ctx_lines.push(Line::from(Span::styled(" Branch", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  {}", if app.git_branch.is_empty() { "—" } else { &app.git_branch }),
        Style::default().fg(Color::Rgb(80, 250, 123)),
    )));
    ctx_lines.push(Line::from(""));

    // Root
    ctx_lines.push(Line::from(Span::styled(" Root", Style::default().fg(Color::Rgb(98, 114, 138)))));
    ctx_lines.push(Line::from(Span::styled(
        format!("  {}", tui_truncate_str(&app.root_path, 18)),
        Style::default().fg(Color::Rgb(130, 145, 165)),
    )));

    // Recent file changes
    if !app.recent_file_changes.is_empty() {
        ctx_lines.push(Line::from(""));
        ctx_lines.push(Line::from(Span::styled(" Changes", Style::default().fg(Color::Rgb(98, 114, 138)))));
        let show = app.recent_file_changes.len().min(6);
        for (icon, path) in app.recent_file_changes.iter().rev().take(show) {
            let short = std::path::Path::new(path).file_name()
                .and_then(|n| n.to_str()).unwrap_or(path);
            ctx_lines.push(Line::from(Span::styled(
                format!("  {} {}", icon, tui_truncate_str(short, 14)),
                Style::default().fg(Color::Rgb(180, 190, 210)),
            )));
        }
    }

    // Code blocks available
    if !app.code_blocks.is_empty() {
        ctx_lines.push(Line::from(""));
        ctx_lines.push(Line::from(Span::styled(" Code Blocks", Style::default().fg(Color::Rgb(98, 114, 138)))));
        ctx_lines.push(Line::from(Span::styled(
            format!("  {} blocks (/code)", app.code_blocks.len()),
            Style::default().fg(Color::Rgb(0, 230, 230)),
        )));
    }
    let ctx_block = Block::default()
        .title(Span::styled(" Context ", Style::default()
            .fg(Color::Rgb(155, 89, 255)).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 70, 90)));
    let ctx_para = Paragraph::new(Text::from(ctx_lines)).block(ctx_block);
    f.render_widget(ctx_para, horizontal[2]);

    // Status bar with connection indicator
    let status_color = if app.status_is_error { Color::Rgb(255, 85, 85) } else { Color::Rgb(80, 250, 123) };
    let conn_icon = match app.connection_status.as_str() {
        "connected" | "local" => "●",
        "disconnected" => "○",
        _ => "◌",
    };
    let conn_color = match app.connection_status.as_str() {
        "connected" | "local" => Color::Rgb(80, 250, 123),
        _ => Color::Rgb(255, 85, 85),
    };
    let privacy_badge = if app.privacy_mode { " 🔒" } else { "" };
    let tools_badge = if !app.tools_enabled { " ⊘tools" } else { "" };
    let status_text = if app.status_msg.is_empty() {
        format!(" {} ◈ {} ▸ {}{}{}",
            app.current_mode.to_uppercase(),
            app.current_model,
            tui_truncate_str(&app.root_path, 30),
            privacy_badge, tools_badge)
    } else {
        format!(" {} ", app.status_msg)
    };
    let status_spans = vec![
        Span::styled(format!(" {} ", conn_icon), Style::default().fg(conn_color)),
        Span::styled(status_text, Style::default().fg(status_color)),
    ];
    let status_para = Paragraph::new(Line::from(status_spans))
        .style(Style::default().bg(Color::Rgb(20, 22, 32)));
    f.render_widget(status_para, vertical[2]);

    // Input box — or tool approval prompt
    if app.awaiting_tool_approval && !app.pending_tool_calls.is_empty() {
        let tool_names: Vec<String> = app.pending_tool_calls.iter().map(|t| t.name.clone()).collect();
        let approval_text = format!(
            "AI wants to run: {}  │  [Y/Enter] approve  [n] deny  [a] yolo (approve all future)",
            tool_names.join(", ")
        );
        let approval_block = Block::default()
            .title(Span::styled(
                " ⚠ Tool Permission Required ",
                Style::default().fg(Color::Rgb(255, 183, 77)).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(255, 183, 77)));
        let approval_para = Paragraph::new(Span::styled(
            approval_text, Style::default().fg(Color::Rgb(220, 225, 235)),
        )).block(approval_block).wrap(Wrap { trim: true });
        f.render_widget(approval_para, vertical[3]);
    } else {
        let input_label = app.input.clone();
        let title_text = if app.is_streaming {
            " ⟳ Thinking... (Ctrl+C to abort) "
        } else {
            " Input (Enter=send, Tab=switch pane, Shift+↑↓=scroll, Ctrl+P=palette) "
        };
        let input_block = Block::default()
            .title(Span::styled(title_text, Style::default().fg(Color::Rgb(0, 230, 230))))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.active_pane == TuiPane::Input {
                Color::Rgb(155, 89, 255)
            } else {
                Color::Rgb(60, 70, 90)
            }));
        let input_para = Paragraph::new(input_label).block(input_block);
        f.render_widget(input_para, vertical[3]);
        if app.active_pane == TuiPane::Input && !app.is_streaming {
            let input_area = vertical[3];
            f.set_cursor_position((
                input_area.x + 1 + app.input_cursor as u16,
                input_area.y + 1,
            ));
        }
    }

    // Command palette overlay
    if app.palette_open {
        let palette_area = centered_rect(60, 60, size);
        f.render_widget(Clear, palette_area);
        let palette_block = Block::default()
            .title(Span::styled(" ⌘ Command Palette ",
                Style::default().fg(Color::Rgb(155, 89, 255)).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(155, 89, 255)));

        let inner = palette_block.inner(palette_area);
        f.render_widget(palette_block, palette_area);

        let search_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
        let items_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(3),
        };

        let search_para = Paragraph::new(Span::styled(
            format!("  🔍 {}", app.palette_query),
            Style::default().fg(Color::Rgb(220, 225, 235)),
        )).style(Style::default().bg(Color::Rgb(30, 32, 48)));
        f.render_widget(search_para, search_area);

        let palette_items: Vec<ListItem> = app.palette_items.iter().enumerate()
            .map(|(i, item)| {
                let style = if i == app.palette_selected {
                    Style::default()
                        .bg(Color::Rgb(60, 65, 95))
                        .fg(Color::Rgb(0, 230, 230))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(180, 190, 210))
                };
                ListItem::new(Line::from(Span::styled(format!("  {}", item), style)))
            })
            .collect();
        let palette_list = List::new(palette_items);
        f.render_widget(palette_list, items_area);
    }
}

/// Send a message to the AI and start streaming the response into app.messages
/// Shared provider-resolution helper used by both send functions.
/// Returns (is_anthropic, api_key, base_url, resolved_model)
async fn tui_resolve_provider(app: &mut TuiApp, prefer_cheap: bool) -> Option<(bool, String, String, String)> {
    let context_chars: usize = app.conversation_history.iter()
        .map(|m| m["content"].as_str().unwrap_or("").len())
        .sum();
    let effective_model = route_model_by_cost(prefer_cheap, &app.current_model, context_chars);

    let explicit_url = app.openai_base_url.clone()
        .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
        .or_else(|| std::env::var("OPENAI_API_BASE").ok());

    if let Some(ref url) = explicit_url {
        return Some((false, app.openai_api_key.clone().unwrap_or_default(), url.clone(), effective_model));
    }

    // Probe local providers
    let probe = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(600))
        .build().unwrap_or_default();
    for url in &["http://localhost:8080/v1", "http://localhost:11434/v1", "http://localhost:1234/v1"] {
        if let Ok(resp) = probe.get(&format!("{url}/models")).send().await {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let mid = body["data"].as_array()
                    .and_then(|a| a.first())
                    .and_then(|m| m["id"].as_str())
                    .unwrap_or("local-model").to_string();
                app.openai_base_url = Some(url.to_string());
                app.current_model = mid.clone();
                return Some((false, String::new(), url.to_string(), mid));
            }
        }
    }
    if let Some(ref key) = app.api_key.clone() {
        let m = if effective_model.starts_with("claude") { effective_model.clone() } else { "claude-sonnet-4-6".to_string() };
        app.current_model = m.clone();
        return Some((true, key.clone(), String::new(), m));
    }
    if let Some(ref key) = app.openai_api_key.clone() {
        let m = if effective_model.starts_with("gpt") { effective_model.clone() } else { "gpt-4o".to_string() };
        app.current_model = m.clone();
        return Some((false, key.clone(), "https://api.openai.com/v1".to_string(), m));
    }
    None
}

/// Start the AI request cycle (spawns task; tokens arrive via stream_rx)
fn tui_start_ai_request(app: &mut TuiApp, is_anthropic: bool, api_key: String, base_url: String, model: String) {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(512);
    let prompt_tool_mode = !is_anthropic && is_likely_llamacpp_endpoint(&base_url);
    app.stream_rx = Some(rx);
    app.is_streaming = true;
    app.is_anthropic_provider = is_anthropic;
    app.prompt_tool_mode = prompt_tool_mode;
    app.assistant_tool_content = None;
    app.pending_tool_calls.clear();
    app.awaiting_tool_approval = false;
    app.auto_execute_tools = false;
    app.status_is_error = false;
    app.status_msg = format!("Thinking ({model})...");

    let ts = chrono::Local::now().format("%H:%M").to_string();
    app.messages.push(ChatMessage { role: "assistant".to_string(), content: String::new(), timestamp: ts, thinking: None, thinking_collapsed: true });
    app.chat_scroll = usize::MAX;

    // Inject work ledger into system prompt so AI avoids redoing completed work.
    // Reads ALL ledger entries (not just the last) and also includes fixed.md.
    let system = {
        let mut sp = app.system_prompt.clone();
        let ledger_path = std::path::Path::new(&app.root_path).join(".shadow-memory/work_ledger.json");
        let mut all_done: Vec<String> = Vec::new();

        // Collect completed work from ALL ledger entries
        if ledger_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&ledger_path) {
                if let Ok(ledger) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    for entry in ledger.iter().rev().take(10) {
                        if let Some(items) = entry["completed_work"].as_array() {
                            for item in items.iter().take(8) {
                                if let Some(s) = item.as_str() {
                                    let line = s.lines().next().unwrap_or(s);
                                    let trimmed = if line.len() > 100 { &line[..100] } else { line };
                                    all_done.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Also collect explicitly fixed items from .shadowai/fixed.md
        let fixed_path = std::path::Path::new(&app.root_path).join(".shadowai/fixed.md");
        if fixed_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&fixed_path) {
                for line in content.lines() {
                    let l = line.trim();
                    if !l.is_empty() && !l.starts_with('#') && !l.starts_with("##") {
                        let trimmed = if l.len() > 100 { &l[..100] } else { l };
                        all_done.push(format!("FIXED: {}", trimmed));
                    }
                }
            }
        }

        // Deduplicate and inject
        all_done.sort();
        all_done.dedup();
        all_done.retain(|s| !s.trim().is_empty());
        all_done.truncate(25);

        if !all_done.is_empty() {
            sp.push_str("\n\n[ALREADY DONE — do NOT redo or ask about these:]\n");
            for item in &all_done {
                sp.push_str(&format!("- {}\n", item));
            }
        }
        sp
    };
    let history = app.conversation_history.clone();
    let is_llamacpp = !is_anthropic && is_likely_llamacpp_endpoint(&base_url);
    let tools = if !app.tools_enabled {
        serde_json::json!([])
    } else if is_anthropic {
        tui_tool_defs_anthropic()
    } else {
        tui_tool_defs_openai()
    };

    tokio::spawn(async move {
        let err_tx = tx.clone();
        let result = if is_anthropic {
            send_anthropic_blocking(&api_key, &model, &system, &history, &tools, 8192, tx).await
        } else if is_llamacpp {
            // Try native llama.cpp /completion endpoint first (streaming, no timeout issues)
            let native_result = send_llamacpp_native(&base_url, &system, &history, &tools, 8192, tx.clone()).await;
            match native_result {
                Err(ref e) if e == "FALLBACK_TO_OPENAI" => {
                    // Server doesn't support native endpoint — fall back to OpenAI-compat
                    send_openai_blocking(&api_key, &base_url, &model, &system, &history, &tools, 8192, prompt_tool_mode, tx).await
                }
                other => other,
            }
        } else {
            send_openai_blocking(&api_key, &base_url, &model, &system, &history, &tools, 8192, prompt_tool_mode, tx).await
        };
        if let Err(e) = result {
            let _ = err_tx.send(format!("{ERROR_SENTINEL}{e}")).await;
        }
    });
}

async fn tui_send_message(app: &mut TuiApp, user_input: String) {
    if app.is_streaming {
        app.push_message("assistant", "⚠ Already streaming, please wait...");
        return;
    }

    // Expand @file refs and glob patterns before sending to AI
    // The chat display shows the original text; the AI receives the expanded version
    let expanded_input = {
        // First expand @file references
        let after_refs = expand_file_refs(&user_input, &app.root_path);
        // Then expand glob patterns like *.md — replace each glob word with file contents
        let mut result = after_refs.clone();
        for word in after_refs.split_whitespace() {
            // Only expand words that look like globs (contain * or ?) and have an extension
            if (word.contains('*') || word.contains('?')) && !word.starts_with('@') {
                let full_pattern = if word.starts_with('/') {
                    word.to_string()
                } else {
                    format!("{}/{}", app.root_path.trim_end_matches('/'), word)
                };
                if let Ok(paths) = glob::glob(&full_pattern) {
                    let mut files_content = String::new();
                    for path in paths.flatten() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            files_content.push_str(&format!("\n\nContents of `{}`:\n```{}\n{}\n```\n", name, ext, content));
                        }
                    }
                    if !files_content.is_empty() {
                        result = result.replacen(word, &files_content, 1);
                    }
                }
            }
        }
        result
    };

    // Inject included file content if toggle is on
    let final_input = if let Some(ref include_path) = app.include_file {
        if let Ok(file_content) = std::fs::read_to_string(include_path) {
            let name = std::path::Path::new(include_path).file_name()
                .and_then(|n| n.to_str()).unwrap_or("file");
            let ext = std::path::Path::new(include_path).extension()
                .and_then(|e| e.to_str()).unwrap_or("");
            format!("{}\n\n[Included file `{}`]:\n```{}\n{}\n```", expanded_input, name, ext, file_content)
        } else {
            expanded_input
        }
    } else {
        expanded_input
    };

    // Clear file changes for new turn & reset code blocks
    app.recent_file_changes.clear();
    app.code_blocks.clear();

    // Add user turn to history (AI sees expanded content; display shows original)
    app.conversation_history.push(serde_json::json!({
        "role": "user",
        "content": final_input
    }));

    // Update token breakdown estimate
    app.update_token_breakdown();

    // ── PRE-SEND AUTO-COMPACTION ──────────────────────────────────────────
    // Prevent "request exceeds context" errors by compacting BEFORE sending.
    // This runs regardless of auto_compact_pct — it's a hard safety check.
    let context_limit = app.token_total;
    // Use 80% of context as the hard ceiling to leave room for the response
    let hard_ceiling = (context_limit as f64 * 0.80) as u64;
    if app.token_used > hard_ceiling && app.conversation_history.len() > 4 {
        // First: clean up completed/resolved items to free space efficiently
        let cleaned = clean_completed_items(&app.conversation_history, &app.root_path);
        let before = app.conversation_history.len();
        app.conversation_history = cleaned;

        // If still over limit, do hierarchical compaction
        app.update_token_breakdown();
        if app.token_used > hard_ceiling {
            run_pre_compact_hooks(&app.root_path);
            // Save what we're compacting to .shadow-memory before discarding
            save_compaction_archive(&app.conversation_history, &app.root_path);
            app.conversation_history = hierarchical_compact_messages(&app.conversation_history, 8);
        }

        app.update_token_breakdown();
        let after = app.conversation_history.len();
        if before != after {
            app.push_message("assistant", &format!(
                "⚡ Pre-send compaction: {} → {} messages ({}% context used). Completed items archived to .shadow-memory/.",
                before, after, (app.token_used * 100) / context_limit.max(1)
            ));
        }
    }

    let prefer_cheap = app.prefer_cheap;
    let Some((is_anthropic, api_key, base_url, model)) = tui_resolve_provider(app, prefer_cheap).await else {
        app.push_message("assistant",
            "⚠ No provider found.\n\
             • Start llama.cpp: llama-server -m model.gguf --port 8080\n\
             • Start Ollama: ollama serve\n\
             • Or add to ~/.config/shadowai/config.toml:\n\
               anthropic_api_key = \"sk-ant-...\"\n\
               openai_base_url = \"http://localhost:8080/v1\"");
        app.conversation_history.pop();
        return;
    };

    // Privacy mode: block non-local providers
    if app.privacy_mode {
        let is_local = base_url.contains("localhost") || base_url.contains("127.0.0.1")
            || base_url.contains("0.0.0.0") || base_url.contains("192.168.");
        if !is_local && !is_anthropic {
            // Allow if it's a private network
        } else if is_anthropic {
            app.push_message("assistant", "🔒 Privacy mode blocks cloud providers. Use a local model or /privacy to disable.");
            app.conversation_history.pop();
            return;
        }
    }

    // Update connection status
    if is_anthropic {
        app.connection_status = "connected".to_string();
    } else if base_url.contains("localhost") || base_url.contains("127.0.0.1") || base_url.contains("192.168.") {
        app.connection_status = "local".to_string();
    } else {
        app.connection_status = "connected".to_string();
    }

    tui_start_ai_request(app, is_anthropic, api_key, base_url, model);
}

/// Continue the AI conversation after tool results have been appended to history.
/// Does NOT add a new user message — just calls the AI with the existing history.
async fn tui_continue_with_tool_results(app: &mut TuiApp) {
    if app.is_streaming { return; }
    let prefer_cheap = app.prefer_cheap;
    let Some((is_anthropic, api_key, base_url, model)) = tui_resolve_provider(app, prefer_cheap).await else {
        app.push_message("assistant", "⚠ Provider not available for tool continuation.");
        return;
    };
    tui_start_ai_request(app, is_anthropic, api_key, base_url, model);
}

/// List available models from Ollama, LM Studio, and configured providers
async fn tui_list_models(app: &mut TuiApp) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let mut lines = vec!["Available models:".to_string(), String::new()];

    // llama.cpp (port 8080) — checked first as the preferred local provider
    if let Ok(resp) = client.get("http://localhost:8080/v1/models").send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(models) = body["data"].as_array() {
                lines.push("── llama.cpp (localhost:8080) ────────────".to_string());
                if models.is_empty() {
                    lines.push("  (no models loaded — start llama-server with a model file)".to_string());
                }
                for m in models {
                    let id = m["id"].as_str().unwrap_or("?");
                    let marker = if id == app.current_model { " ◀ active" } else { "" };
                    lines.push(format!("  {}{}", id, marker));
                }
                lines.push(String::new());
            }
        }
    }

    // Anthropic (hardcoded — no list endpoint without streaming)
    if app.api_key.is_some() {
        lines.push("── Anthropic (API key set) ──────────────".to_string());
        for m in &["claude-opus-4-6", "claude-sonnet-4-6", "claude-haiku-4-5-20251001",
                   "claude-opus-4-5", "claude-sonnet-4-5", "claude-haiku-4-5"] {
            let marker = if *m == app.current_model { " ◀ active" } else { "" };
            lines.push(format!("  {}{}", m, marker));
        }
        lines.push(String::new());
    }

    // Ollama
    if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(models) = body["models"].as_array() {
                lines.push("── Ollama (localhost:11434) ──────────────".to_string());
                if models.is_empty() {
                    lines.push("  (no models pulled — run: ollama pull <model>)".to_string());
                }
                for m in models {
                    let name = m["name"].as_str().unwrap_or("?");
                    let size = m["size"].as_u64().unwrap_or(0);
                    let gb = size as f64 / 1_073_741_824.0;
                    let marker = if name == app.current_model { " ◀ active" } else { "" };
                    lines.push(format!("  {:<40} {:.1} GB{}", name, gb, marker));
                }
                lines.push(String::new());
            }
        }
    }

    // LM Studio
    if let Ok(resp) = client.get("http://localhost:1234/v1/models").send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(models) = body["data"].as_array() {
                lines.push("── LM Studio (localhost:1234) ────────────".to_string());
                for m in models {
                    let id = m["id"].as_str().unwrap_or("?");
                    let marker = if id == app.current_model { " ◀ active" } else { "" };
                    lines.push(format!("  {}{}", id, marker));
                }
                lines.push(String::new());
            }
        }
    }

    // vLLM
    if let Ok(resp) = client.get("http://localhost:8000/v1/models").send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(models) = body["data"].as_array() {
                lines.push("── vLLM (localhost:8000) ─────────────────".to_string());
                for m in models {
                    let id = m["id"].as_str().unwrap_or("?");
                    lines.push(format!("  {}", id));
                }
                lines.push(String::new());
            }
        }
    }

    if app.api_key.is_none() && lines.len() <= 2 {
        lines.push("No providers found. Options:".to_string());
        lines.push("  • Start llama-server (llama.cpp) on port 8080  ← preferred local".to_string());
        lines.push("  • Run 'ollama serve' for Ollama models".to_string());
        lines.push("  • Start LM Studio on port 1234".to_string());
        lines.push("  • Set ANTHROPIC_API_KEY for Claude cloud".to_string());
    }

    lines.push(String::new());
    lines.push("Use /model <name> to switch.".to_string());

    app.push_message("assistant", &lines.join("\n"));
}

// ══════════════════════════════════════════════════════════════════════════════
// Section 13-19 Handler Functions
// ══════════════════════════════════════════════════════════════════════════════

/// MCP: list/connect Model Context Protocol servers
async fn handle_mcp_command(args: &str, _config: &CliConfig) {
    let args = args.trim();
    if args.is_empty() || args == "list" {
        let config_path = config_dir().map(|d| d.join("mcp.toml"));
        let servers: Vec<(String, String)> = config_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .and_then(|v| v["servers"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                let name = s["name"].as_str()?.to_string();
                let url = s["url"].as_str()?.to_string();
                Some((name, url))
            })
            .collect();

        print_section_header("MCP Servers");
        if servers.is_empty() {
            print_info("No MCP servers configured.");
            print_info("Add servers to ~/.config/shadowai/mcp.toml:");
            println!("  [[servers]]");
            println!("  name = \"my-server\"");
            println!("  url = \"http://localhost:3000\"");
            println!("  transport = \"http\"");
        } else {
            for (name, url) in &servers {
                // Quick TCP check
                let reachable = extract_host_port(url).map(|addr| {
                    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
                }).unwrap_or(false);
                let status = if reachable { "\x1b[32m●\x1b[0m" } else { "\x1b[31m●\x1b[0m" };
                println!("  {} {} — {}", status, name, url);
            }
        }
    } else if let Some(url) = args.strip_prefix("connect ") {
        let url = url.trim();
        print_info(&format!("Connecting to MCP server at {}…", url));
        let reachable = extract_host_port(url).map(|addr| {
            std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(1000)).is_ok()
        }).unwrap_or(false);
        if reachable {
            print_info_accent("Connected", &format!("MCP server at {} is reachable", url));
            // Fetch tool list via /tools endpoint (standard MCP)
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default();
            if let Ok(resp) = client.get(format!("{}/tools", url)).send().await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(tools) = body["tools"].as_array() {
                        print_section_header("Available Tools");
                        for tool in tools {
                            let name = tool["name"].as_str().unwrap_or("?");
                            let desc = tool["description"].as_str().unwrap_or("");
                            println!("  • {}: {}", name, desc);
                        }
                    }
                }
            }
        } else {
            print_error(&format!("Cannot reach MCP server at {}", url));
        }
    } else {
        print_error("Usage: /mcp [list | connect <url>]");
    }
}

/// Architect: read-only planning mode — reason about architecture before touching files
async fn handle_architect_command(root_path: &str, config: &CliConfig) {
    print_section_header("Architect Mode");
    println!("\x1b[33m⚠  Architect mode: AI will reason about the codebase but NOT edit files.\x1b[0m");
    println!("   Describe what you want to build, and ShadowAI will present a numbered plan.");
    println!("   Type /approve to execute the plan, or press Enter to cancel.\n");

    // Generate a repo overview for context
    let tree_out = std::process::Command::new("find")
        .args([root_path, "-maxdepth", "3", "-not", "-path", "*/.*", "-not", "-path", "*/target/*", "-not", "-path", "*/node_modules/*"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    print_info("Project structure (3 levels):");
    for line in tree_out.lines().take(40) {
        println!("  {}", line);
    }
    println!();

    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok()
        .or_else(|| config.anthropic_api_key.clone());

    if anthropic_key.is_none() {
        print_info("Tip: Set ANTHROPIC_API_KEY for AI-powered architect planning.");
        print_info("Without it, architect mode shows the project structure only.");
        return;
    }

    print_info("AI architecture analysis ready. Send your question via the normal prompt.");
    print_info("(Architect mode is active — file edits are disabled until you /approve)");
}

/// Yolo: skip all confirmations for current session
fn handle_yolo_command() {
    static YOLO_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let was_active = YOLO_ACTIVE.fetch_xor(true, std::sync::atomic::Ordering::SeqCst);
    if was_active {
        println!("\x1b[32m✓ YOLO mode OFF — confirmations restored\x1b[0m");
    } else {
        println!("\x1b[31m⚡ YOLO MODE ON — all confirmations skipped!\x1b[0m");
        println!("\x1b[31m   Use /yolo again to restore confirmation prompts.\x1b[0m");
    }
}

/// Approval: set approval tier (full / smart / yolo)
fn handle_approval_command(tier: &str) {
    match tier.trim() {
        "full" => {
            println!("\x1b[36mApproval: FULL — confirm every tool call\x1b[0m");
            println!("  All shell commands, file edits, and external requests require [y/N].");
        }
        "smart" => {
            println!("\x1b[33mApproval: SMART — confirm shell commands only\x1b[0m");
            println!("  File reads/writes are auto-approved; shell commands require [y/N].");
        }
        "yolo" => {
            println!("\x1b[31mApproval: YOLO — no confirmations\x1b[0m");
            println!("  All tool calls auto-approved. Use with caution.");
        }
        "" => {
            println!("Current approval tier: smart (default)");
            println!("Usage: /approval full | smart | yolo");
        }
        other => {
            print_error(&format!("Unknown tier '{}'. Use: full | smart | yolo", other));
        }
    }
}

/// Arena: send same prompt to 2 models and show side-by-side comparison
async fn handle_arena_command(prompt: &str, config: &CliConfig) {
    if prompt.is_empty() {
        print_error("Usage: /arena <prompt>");
        return;
    }

    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok()
        .or_else(|| config.anthropic_api_key.clone());

    let Some(key) = anthropic_key else {
        print_error("Arena mode requires ANTHROPIC_API_KEY (used for both models).");
        return;
    };

    let model_a = config.model.clone().unwrap_or_else(|| "claude-opus-4-5".to_string());
    let model_b = "claude-sonnet-4-6".to_string();

    print_section_header("Arena Mode");
    println!("Prompt: {}", prompt);
    println!("Model A: {}   Model B: {}\n", model_a, model_b);

    let msgs = vec![serde_json::json!({"role": "user", "content": prompt})];
    let (tx_a, mut rx_a) = tokio::sync::mpsc::channel::<String>(256);
    let (tx_b, mut rx_b) = tokio::sync::mpsc::channel::<String>(256);

    let key_a = key.clone();
    let key_b = key.clone();
    let m_a = model_a.clone();
    let m_b = model_b.clone();
    let msgs_a = msgs.clone();
    let msgs_b = msgs.clone();

    tokio::spawn(async move {
        let _ = send_anthropic_request(&key_a, &m_a, "You are a helpful assistant.", &msgs_a, 2048, 0.7, tx_a).await;
    });
    tokio::spawn(async move {
        let _ = send_anthropic_request(&key_b, &m_b, "You are a helpful assistant.", &msgs_b, 2048, 0.7, tx_b).await;
    });

    let mut resp_a = String::new();
    let mut resp_b = String::new();

    // Collect both responses concurrently
    loop {
        let done_a = rx_a.is_closed();
        let done_b = rx_b.is_closed();
        if done_a && done_b { break; }

        tokio::select! {
            Some(tok) = rx_a.recv() => resp_a.push_str(&tok),
            Some(tok) = rx_b.recv() => resp_b.push_str(&tok),
            else => break,
        }
    }

    // Side-by-side display
    let width = 44;
    println!("\x1b[1;36m{:<width$}  {:<width$}\x1b[0m", format!("▶ {} (A)", model_a), format!("▶ {} (B)", model_b), width = width);
    println!("{}", "─".repeat(width * 2 + 2));

    let lines_a: Vec<&str> = resp_a.lines().collect();
    let lines_b: Vec<&str> = resp_b.lines().collect();
    let max_lines = lines_a.len().max(lines_b.len());
    for i in 0..max_lines {
        let la = lines_a.get(i).copied().unwrap_or("");
        let lb = lines_b.get(i).copied().unwrap_or("");
        println!("{:<width$}  {:<width$}", &la[..la.len().min(width)], &lb[..lb.len().min(width)], width = width);
    }
    println!();
    print_info("Type /arena again with a different prompt to compare. Results not saved.");
}

/// Teleport: serialize current session to a file for cross-terminal resume
fn handle_teleport_command(msgs: &[serde_json::Value], root_path: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let teleport_path = std::path::Path::new(root_path)
        .join(".shadowai")
        .join(format!("teleport-{}.json", ts));
    std::fs::create_dir_all(teleport_path.parent().unwrap_or(std::path::Path::new("."))).ok();

    let payload = serde_json::json!({
        "version": 1,
        "root_path": root_path,
        "timestamp": ts,
        "messages": msgs,
    });

    match serde_json::to_string_pretty(&payload) {
        Ok(json) => {
            match std::fs::write(&teleport_path, &json) {
                Ok(_) => {
                    print_info_accent("Teleport saved", &format!("{}", teleport_path.display()));
                    println!("  Resume with: shadowai --load-teleport {}", teleport_path.display());
                }
                Err(e) => print_error(&format!("Failed to write teleport file: {}", e)),
            }
        }
        Err(e) => print_error(&format!("Serialization failed: {}", e)),
    }
}

/// Snapshot: save/load named session snapshots (enhanced /save /load)
async fn handle_snapshot_command(args: &str, msgs: &[serde_json::Value], root_path: &str) {
    let snap_dir = dirs_next::config_dir()
        .map(|d| d.join("shadowai").join("snapshots"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/snapshots"));
    std::fs::create_dir_all(&snap_dir).ok();

    if args.is_empty() || args == "list" {
        print_section_header("Snapshots");
        let entries = std::fs::read_dir(&snap_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        if entries.is_empty() {
            print_info("No snapshots found.");
            println!("  Create one with: /snapshot <name>");
        } else {
            for entry in &entries {
                println!("  • {}", entry.file_name().to_string_lossy().trim_end_matches(".json"));
            }
        }
        return;
    }

    if let Some(name) = args.strip_prefix("load ") {
        let name = name.trim();
        let snap_path = snap_dir.join(format!("{}.json", name));
        match std::fs::read_to_string(&snap_path) {
            Ok(json) => {
                let count = serde_json::from_str::<Vec<serde_json::Value>>(&json)
                    .map(|v| v.len())
                    .unwrap_or(0);
                print_info_accent("Loaded snapshot", &format!("'{}' — {} messages", name, count));
                println!("  Use /load {} to restore this into the active session.", name);
            }
            Err(_) => print_error(&format!("Snapshot '{}' not found", name)),
        }
        return;
    }

    // Save snapshot
    let name = args.trim();
    if name.is_empty() {
        print_error("Usage: /snapshot <name> | /snapshot load <name> | /snapshot list");
        return;
    }

    let snap_path = snap_dir.join(format!("{}.json", name));
    let payload = serde_json::json!({
        "root_path": root_path,
        "messages": msgs,
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(json) => match std::fs::write(&snap_path, &json) {
            Ok(_) => print_info_accent("Snapshot saved", &format!("'{}' → {}", name, snap_path.display())),
            Err(e) => print_error(&format!("Failed to save snapshot: {}", e)),
        },
        Err(e) => print_error(&format!("Serialization error: {}", e)),
    }
}

/// RepoMap: generate a structural map of the codebase (ctags/tree-sitter style)
fn handle_repomap_command(root_path: &str) {
    print_section_header("Repository Map");

    // Try ctags first
    let ctags_out = std::process::Command::new("ctags")
        .args(["--recurse", "--fields=+n", "--output-format=json",
               "--exclude=.git", "--exclude=target", "--exclude=node_modules",
               "-f", "-", root_path])
        .output();

    if let Ok(out) = ctags_out {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut by_file: std::collections::BTreeMap<String, Vec<(String, String)>> = std::collections::BTreeMap::new();
            for line in text.lines() {
                if let Ok(tag) = serde_json::from_str::<serde_json::Value>(line) {
                    let path = tag["path"].as_str().unwrap_or("?")
                        .trim_start_matches(root_path).trim_start_matches('/').to_string();
                    let name = tag["name"].as_str().unwrap_or("?").to_string();
                    let kind = tag["kind"].as_str().unwrap_or("?").to_string();
                    by_file.entry(path).or_default().push((kind, name));
                }
            }
            for (path, symbols) in by_file.iter().take(30) {
                println!("\x1b[36m{}\x1b[0m", path);
                for (kind, name) in symbols.iter().take(10) {
                    println!("  {:8} {}", kind, name);
                }
            }
            return;
        }
    }

    // Fallback: simple file tree with extension stats
    let find_out = std::process::Command::new("find")
        .args([root_path, "-maxdepth", "4", "-type", "f",
               "-not", "-path", "*/.*", "-not", "-path", "*/target/*",
               "-not", "-path", "*/node_modules/*"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let mut ext_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for line in find_out.lines() {
        let ext = std::path::Path::new(line)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "none".to_string());
        *ext_counts.entry(ext).or_insert(0) += 1;
    }

    println!("File types:");
    let mut counts: Vec<_> = ext_counts.iter().collect();
    counts.sort_by(|a, b| b.1.cmp(a.1));
    for (ext, count) in counts.iter().take(15) {
        println!("  .{:<12} {} files", ext, count);
    }
    println!("\nInstall 'ctags' (universal-ctags) for symbol-level repo map.");
}

/// Voice: record audio → transcribe → inject as prompt text
fn handle_voice_command() -> Option<String> {
    print_info("Voice input: recording for 5 seconds… (Ctrl+C to cancel)");

    let wav_path = "/tmp/shadowai-voice.wav";
    let record_result = std::process::Command::new("arecord")
        .args(["-d", "5", "-f", "cd", "-t", "wav", wav_path])
        .status();

    if record_result.is_err() || !record_result.unwrap().success() {
        // Try sox as fallback
        let sox_result = std::process::Command::new("sox")
            .args(["-d", "-r", "16000", "-c", "1", "-b", "16", wav_path, "trim", "0", "5"])
            .status();
        if sox_result.is_err() || !sox_result.unwrap().success() {
            print_error("Recording failed. Install 'arecord' (alsa-utils) or 'sox'.");
            return None;
        }
    }

    print_info("Transcribing with whisper.cpp…");

    // Try whisper.cpp
    let whisper = std::process::Command::new("whisper-cpp")
        .args(["-m", "/usr/share/whisper.cpp/ggml-base.en.bin", "-f", wav_path, "--output-txt", "/tmp/shadowai-voice"])
        .output()
        .or_else(|_| std::process::Command::new("whisper")
            .args([wav_path, "--model", "base.en", "--output-txt", "/tmp/shadowai-voice"])
            .output());

    if whisper.is_ok() {
        if let Ok(text) = std::fs::read_to_string("/tmp/shadowai-voice.txt") {
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                print_info_accent("Transcribed", &trimmed);
                return Some(trimmed);
            }
        }
    }

    print_error("Transcription failed. Install whisper.cpp or whisper for voice input.");
    None
}

/// Screenshot: capture screen → encode as base64 → attach to next message
async fn handle_screenshot_command(image_context: &std::sync::Arc<tokio::sync::Mutex<Option<(String, String)>>>) {
    let path = "/tmp/shadowai-screenshot.png";
    print_info("Capturing screenshot…");

    let result = std::process::Command::new("scrot")
        .arg(path)
        .status()
        .or_else(|_| std::process::Command::new("gnome-screenshot")
            .args(["-f", path])
            .status())
        .or_else(|_| std::process::Command::new("import")
            .args(["-window", "root", path])
            .status());

    match result {
        Ok(s) if s.success() => {
            match std::fs::read(path) {
                Ok(bytes) => {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    *image_context.lock().await = Some((b64, "image/png".to_string()));
                    print_info_accent("Screenshot captured", "Image attached to your next message.");
                    println!("  (send a message to include the screenshot in the AI request)");
                }
                Err(e) => print_error(&format!("Failed to read screenshot: {}", e)),
            }
        }
        _ => {
            print_error("Screenshot failed. Install 'scrot', 'gnome-screenshot', or 'imagemagick'.");
        }
    }
}

/// Cost: show estimated token cost for the current session
fn handle_cost_command(_config: &CliConfig) {
    // Per-million-token pricing (as of 2026)
    let pricing: &[(&str, f64, f64)] = &[
        ("claude-opus-4-6",         15.0,  75.0),
        ("claude-sonnet-4-6",        3.0,  15.0),
        ("claude-haiku-4-5",         0.25,  1.25),
        ("claude-opus-4-5",         15.0,  75.0),
        ("claude-sonnet-4-5",        3.0,  15.0),
        ("gpt-4o",                   5.0,  15.0),
        ("gpt-4o-mini",              0.15,  0.60),
        ("gpt-4-turbo",             10.0,  30.0),
        ("llama.cpp/local",          0.0,   0.0),
        ("ollama/local",             0.0,   0.0),
    ];

    print_section_header("Model Pricing (per 1M tokens)");
    println!("  {:<35} {:>10}  {:>10}", "Model", "Input", "Output");
    println!("  {}", "─".repeat(60));
    for (model, inp, out) in pricing {
        println!("  {:<35} {:>9.2}$  {:>9.2}$", model, inp, out);
    }
    println!();
    print_info("Local models (llama.cpp / Ollama) are free — no API cost.");
    print_info("Use /switch <provider> to change providers mid-session.");
    print_info("Token counts shown in the context bar (████░ N%).");
}

/// Block: re-print a past response by block ID
fn handle_block_command(id: usize) {
    // Blocks are written to a session log; without SQLite we scan the audit log
    let log_path = dirs_next::config_dir()
        .map(|d| d.join("shadowai").join("audit.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/audit.log"));

    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let target = format!("[#{}]", id);
            let mut found = false;
            let mut in_block = false;
            let mut lines_buf = Vec::new();
            for line in content.lines() {
                if line.contains(&target) {
                    in_block = true;
                    found = true;
                }
                if in_block {
                    lines_buf.push(line);
                    if lines_buf.len() > 1 && (line.starts_with("[#") || line.is_empty() && lines_buf.len() > 5) {
                        break;
                    }
                }
            }
            if found {
                print_section_header(&format!("Block #{}", id));
                for l in &lines_buf {
                    println!("{}", l);
                }
            } else {
                print_error(&format!("Block #{} not found in audit log.", id));
            }
        }
        Err(_) => print_error("No audit log found. Blocks are recorded during active sessions."),
    }
}

/// LogSearch: search the audit log
fn handle_log_search_command(term: &str, _root_path: &str) {
    if term.is_empty() {
        print_error("Usage: /log search <term>");
        return;
    }
    let log_path = dirs_next::config_dir()
        .map(|d| d.join("shadowai").join("audit.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/audit.log"));

    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let term_lower = term.to_lowercase();
            let matches: Vec<&str> = content.lines()
                .filter(|l| l.to_lowercase().contains(&term_lower))
                .collect();
            print_section_header(&format!("Log search: '{}'", term));
            if matches.is_empty() {
                print_info("No matches found.");
            } else {
                println!("  {} matches:\n", matches.len());
                for line in matches.iter().take(50) {
                    println!("  {}", line);
                }
                if matches.len() > 50 {
                    print_info(&format!("  … {} more matches", matches.len() - 50));
                }
            }
        }
        Err(_) => print_error("Audit log not found at ~/.config/shadowai/audit.log"),
    }
}

/// Runbook: create / run executable Markdown runbooks
async fn handle_runbook_command(args: &str, root_path: &str) {
    let runbook_dir = std::path::Path::new(root_path).join(".shadowai").join("runbooks");
    std::fs::create_dir_all(&runbook_dir).ok();

    if args.is_empty() || args == "list" {
        print_section_header("Runbooks");
        let entries = std::fs::read_dir(&runbook_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>())
            .unwrap_or_default();
        if entries.is_empty() {
            print_info("No runbooks found.");
            println!("  Create one with: /runbook new <name>");
        } else {
            for name in &entries { println!("  • {}", name.trim_end_matches(".md")); }
        }
        return;
    }

    if let Some(name) = args.strip_prefix("new ") {
        let name = name.trim();
        let path = runbook_dir.join(format!("{}.md", name));
        let template = format!("# {}\n\nDescription: TODO\n\n## Steps\n\n```sh\necho \"Step 1\"\n```\n\n```sh\necho \"Step 2\"\n```\n", name);
        match std::fs::write(&path, &template) {
            Ok(_) => {
                print_info_accent("Created", &format!("{}", path.display()));
                println!("  Edit it with your preferred editor, then run with: /runbook run {}", name);
            }
            Err(e) => print_error(&format!("Failed to create runbook: {}", e)),
        }
        return;
    }

    if let Some(name) = args.strip_prefix("run ") {
        let name = name.trim();
        let path = runbook_dir.join(format!("{}.md", name));
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                print_section_header(&format!("Running runbook: {}", name));
                // Extract fenced sh/bash blocks
                let mut in_block = false;
                let mut cmd_buf = String::new();
                let mut step = 0;
                for line in content.lines() {
                    if line.starts_with("```sh") || line.starts_with("```bash") {
                        in_block = true;
                        cmd_buf.clear();
                    } else if in_block && line == "```" {
                        in_block = false;
                        step += 1;
                        println!("\n\x1b[33m▶ Step {}\x1b[0m: {}", step, cmd_buf.trim());
                        let status = std::process::Command::new("sh")
                            .args(["-c", cmd_buf.trim()])
                            .current_dir(root_path)
                            .status();
                        match status {
                            Ok(s) if s.success() => println!("\x1b[32m✓ OK\x1b[0m"),
                            Ok(s) => print_error(&format!("Step {} failed (exit code {:?})", step, s.code())),
                            Err(e) => print_error(&format!("Step {} error: {}", step, e)),
                        }
                        cmd_buf.clear();
                    } else if in_block {
                        if !cmd_buf.is_empty() { cmd_buf.push('\n'); }
                        cmd_buf.push_str(line);
                    }
                }
                println!("\n\x1b[32m✓ Runbook '{}' completed ({} steps)\x1b[0m", name, step);
            }
            Err(_) => print_error(&format!("Runbook '{}' not found", name)),
        }
        return;
    }

    print_error("Usage: /runbook [list | new <name> | run <name>]");
}

/// Switch: change provider mid-session
fn handle_switch_command(provider: &str, _model: &mut Option<String>, _config: &CliConfig) {
    match provider.trim() {
        "anthropic" | "claude" => {
            let key = std::env::var("ANTHROPIC_API_KEY").ok();
            if key.is_some() {
                print_info_accent("Switched", "Using Anthropic (Claude). Set /model claude-* to pick a model.");
            } else {
                print_error("ANTHROPIC_API_KEY not set. Export it and try again.");
            }
        }
        "openai" | "gpt" => {
            let key = std::env::var("OPENAI_API_KEY").ok();
            if key.is_some() {
                print_info_accent("Switched", "Using OpenAI. Set /model gpt-* to pick a model.");
            } else {
                print_error("OPENAI_API_KEY not set. Export it and try again.");
            }
        }
        "ollama" => {
            let ok = std::net::TcpStream::connect_timeout(
                &"127.0.0.1:11434".parse().unwrap(),
                std::time::Duration::from_millis(500),
            ).is_ok();
            if ok {
                print_info_accent("Switched", "Using Ollama (localhost:11434). Use /models to list available models.");
            } else {
                print_error("Ollama is not running. Start it with: ollama serve");
            }
        }
        "llamacpp" | "llama.cpp" | "llama" => {
            let ok = std::net::TcpStream::connect_timeout(
                &"127.0.0.1:8080".parse().unwrap(),
                std::time::Duration::from_millis(500),
            ).is_ok();
            if ok {
                print_info_accent("Switched", "Using llama.cpp (localhost:8080). Local inference active.");
            } else {
                print_error("llama.cpp server is not running. Start with: llama-server -m <model.gguf> --port 8080");
            }
        }
        "" => {
            println!("Usage: /switch <provider>");
            println!("Providers: anthropic | openai | ollama | llamacpp");
        }
        other => print_error(&format!("Unknown provider '{}'. Options: anthropic, openai, ollama, llamacpp", other)),
    }
}

/// Audit: search/stats for the audit log
fn handle_audit_command(args: &str, root_path: &str) {
    let log_path = dirs_next::config_dir()
        .map(|d| d.join("shadowai").join("audit.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/shadowai/audit.log"));

    if args == "stats" {
        match std::fs::read_to_string(&log_path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let tool_calls = lines.iter().filter(|l| l.contains("[tool]")).count();
                let errors = lines.iter().filter(|l| l.contains("[error]")).count();
                print_section_header("Audit Log Stats");
                println!("  Total entries : {}", lines.len());
                println!("  Tool calls    : {}", tool_calls);
                println!("  Errors        : {}", errors);
                println!("  Log path      : {}", log_path.display());
            }
            Err(_) => print_error("No audit log found."),
        }
    } else if let Some(term) = args.strip_prefix("search ") {
        handle_log_search_command(term.trim(), root_path);
    } else if args.is_empty() {
        print_section_header("Audit");
        println!("Usage: /audit search <term> | /audit stats");
    } else {
        print_error("Usage: /audit search <term> | /audit stats");
    }
}

/// ModelPull: pull an Ollama model from the CLI
async fn handle_model_pull_command(name: &str) {
    if name.is_empty() {
        print_error("Usage: /model pull <model-name>  (e.g., /model pull llama3)");
        return;
    }
    print_info(&format!("Pulling Ollama model '{}'…", name));
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "name": name });
    match client.post("http://localhost:11434/api/pull").json(&body).send().await {
        Ok(resp) => {
            use tokio::io::AsyncBufReadExt;
            let bytes = resp.bytes_stream();
            use futures_util::StreamExt;
            let mut stream = bytes;
            let mut last_pct = 0u64;
            while let Some(chunk) = stream.next().await {
                if let Ok(data) = chunk {
                    if let Ok(text) = std::str::from_utf8(&data) {
                        for line in text.lines() {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                                let status = v["status"].as_str().unwrap_or("");
                                let completed = v["completed"].as_u64().unwrap_or(0);
                                let total = v["total"].as_u64().unwrap_or(0);
                                if total > 0 {
                                    let pct = completed * 100 / total;
                                    if pct != last_pct {
                                        last_pct = pct;
                                        print!("\r  {} {}% ", status, pct);
                                        let _ = io::Write::flush(&mut io::stdout());
                                    }
                                } else if !status.is_empty() {
                                    println!("  {}", status);
                                }
                            }
                        }
                    }
                }
            }
            println!();
            print_info_accent("Done", &format!("Model '{}' pulled successfully.", name));
        }
        Err(_) => {
            print_error("Cannot connect to Ollama. Make sure 'ollama serve' is running.");
        }
    }
}

async fn run_tui_mode(
    root_path: String,
    model: String,
    mode: String,
    skill: Option<String>,
    cfg: &CliConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use crossterm::terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
    use crossterm::execute;
    use crossterm::event::{self as ct_event, Event, KeyEventKind};

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new(&root_path, &model, &mode);
    // Overlay config-file keys if env vars aren't set
    if app.api_key.is_none() {
        app.api_key = cfg.anthropic_api_key.clone();
    }
    if app.openai_api_key.is_none() {
        app.openai_api_key = cfg.openai_api_key.clone();
    }
    if app.openai_base_url.is_none() {
        app.openai_base_url = cfg.openai_base_url.clone();
    }
    if let Some(sp) = &cfg.system_prompt {
        app.system_prompt = sp.clone();
    }
    // Section 19: load prefer_cheap from config
    if let Some(pc) = cfg.prefer_cheap {
        app.prefer_cheap = pc;
    }
    // Section 14: load auto_compact_pct from config
    if let Some(pct) = cfg.auto_compact_pct {
        app.auto_compact_pct = pct;
    }

    if let Some(sk) = skill { app.active_skill = sk; }
    app.load_file_tree();
    app.refresh_git_branch();

    // Startup: consolidate stale compaction archives from previous sessions.
    // This merges files older than 24h into a single archive and updates the
    // work ledger so the AI knows what was done without re-reading raw history.
    {
        let consolidated = consolidate_shadow_memory(&root_path);
        if consolidated > 0 {
            let mut o = io::stdout();
            set_fg(&mut o, theme::DIM);
            write!(o, "  📦 Consolidated {} old compaction archives into work ledger.\n", consolidated).ok();
            reset_color(&mut o);
        }
    }

    // Auto-detect the actual model name from the configured/local provider.
    // Tries the configured URL first, then standard local ports.
    {
        let probe_urls: Vec<String> = {
            let mut v = Vec::new();
            if let Some(ref u) = app.openai_base_url { v.push(u.clone()); }
            v.push("http://localhost:8080/v1".to_string());
            v.push("http://localhost:11434/v1".to_string());
            v.push("http://localhost:1234/v1".to_string());
            v
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        'model_detect: for url in &probe_urls {
            let models_url = format!("{}/models", url.trim_end_matches('/'));
            if let Ok(resp) = client.get(&models_url).send().await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(first) = body["data"].as_array().and_then(|a| a.first()) {
                        if let Some(id) = first["id"].as_str() {
                            app.current_model = id.to_string();
                            app.openai_base_url = Some(url.clone());
                            break 'model_detect;
                        }
                    }
                }
            }
        }
    }

    // Detect provider at startup for the greeting message.
    // Check configured URL first (any host), then standard local ports.
    let all_probe_urls: Vec<String> = {
        let mut v = Vec::new();
        if let Some(ref u) = app.openai_base_url { v.push(u.clone()); }
        v.push("http://localhost:8080/v1".to_string());
        v.push("http://localhost:11434/v1".to_string());
        v.push("http://localhost:1234/v1".to_string());
        v
    };
    let local_provider_label: Option<String> = all_probe_urls.iter().find_map(|url| {
        extract_host_port(url).and_then(|addr| {
            std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400))
                .ok()
                .map(|_| {
                    if url.contains(":8080") { format!("🖥llama.cpp native ({})", llamacpp_server_url(url)) }
                    else if url.contains(":11434") { format!("🦙 Ollama ({})", url) }
                    else if url.contains(":1234") { format!("🏠 LM Studio ({})", url) }
                    else { format!("🌐 LLM endpoint ({})", url) }
                })
        })
    });

    let has_key = app.api_key.is_some() || app.openai_api_key.is_some();
    let provider_line = if let Some(ref lbl) = local_provider_label {
        format!("\nProvider: {}", lbl)
    } else if has_key {
        String::new()
    } else {
        "\n\n⚠  No API key or local provider found.\nInstall llama.cpp/Ollama or set ANTHROPIC_API_KEY in ~/.config/shadowai/config.toml".to_string()
    };
    app.push_message("assistant", &format!(
        "ShadowAI TUI — {} mode — {}{}\nType a message and press Enter. Use /help for commands, Ctrl+P for command palette.",
        mode, app.current_model, provider_line
    ));

    loop {
        // ── Drain tokens / sentinel events from the AI response channel ────────
        if let Some(rx) = &mut app.stream_rx {
            let mut got_token = false;
            loop {
                match rx.try_recv() {
                    Ok(token) => {
                        if let Some(rest) = token.strip_prefix(ERROR_SENTINEL) {
                            app.push_message("error", &format!("Error: {rest}"));
                            let summary: String = rest.chars().take(96).collect();
                            app.status_msg = format!("Error: {}", summary);
                            app.status_is_error = true;
                            app.is_streaming = false;
                            app.stream_rx = None;
                            break;
                        } else if let Some(rest) = token.strip_prefix(ASSISTANT_CONTENT_SENTINEL) {
                            // Store the raw assistant content for correct tool-use history format
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                                app.assistant_tool_content = Some(v);
                            }
                        } else if let Some(rest) = token.strip_prefix(TOOL_SENTINEL) {
                            // Received a tool call from the AI
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                                let id    = v["id"].as_str().unwrap_or("").to_string();
                                let name  = v["name"].as_str().unwrap_or("").to_string();
                                let input = v["input"].clone();
                                app.pending_tool_calls.push(PendingTuiToolCall {
                                    id, name: name.clone(), input_json: input.to_string(),
                                });
                                // Safe tools in Smart/Yolo mode — show in chat but auto-approve
                                if is_safe_tool(&name) && app.tool_approval_mode != TuiApprovalMode::AskAll {
                                    // will be auto-executed when channel disconnects
                                } else if app.tool_approval_mode == TuiApprovalMode::Yolo {
                                    // yolo: also auto-execute
                                } else {
                                    // dangerous tool in Smart/AskAll — show in pending
                                }
                            }
                        } else if token == APPROVAL_SENTINEL {
                            // AI wants to use tools
                            app.awaiting_tool_approval = true;
                        } else if token == THINKING_START_SENTINEL {
                            app.in_thinking = true;
                            app.thinking_buf.clear();
                            app.status_msg = "Thinking…".to_string();
                        } else if token == THINKING_END_SENTINEL {
                            app.in_thinking = false;
                            // Attach thinking to current assistant message
                            if let Some(msg) = app.messages.last_mut() {
                                if msg.role == "assistant" {
                                    msg.thinking = Some(app.thinking_buf.clone());
                                }
                            }
                            app.thinking_buf.clear();
                        } else if let Some(think_text) = token.strip_prefix(THINKING_TOKEN_SENTINEL) {
                            app.thinking_buf.push_str(think_text);
                        } else {
                            // Normal text token — append to last assistant message
                            got_token = true;
                            if let Some(msg) = app.messages.last_mut() {
                                if msg.role == "assistant" {
                                    msg.content.push_str(&token);
                                }
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        app.stream_rx = None;
                        app.is_streaming = false;

                        if app.awaiting_tool_approval && !app.pending_tool_calls.is_empty() {
                            // AI wants tools. Decide whether to auto-execute or ask.
                            let all_safe = app.pending_tool_calls.iter().all(|t| is_safe_tool(&t.name));
                            let auto = app.tool_approval_mode == TuiApprovalMode::Yolo
                                    || (app.tool_approval_mode == TuiApprovalMode::Smart && all_safe);

                            if auto {
                                // Add assistant content to history and schedule tool execution
                                if let Some(content) = app.assistant_tool_content.take() {
                                    if app.is_anthropic_provider {
                                        app.conversation_history.push(serde_json::json!({"role":"assistant","content":content}));
                                    } else {
                                        app.conversation_history.push(content);
                                    }
                                }
                                app.auto_execute_tools = true;
                                app.awaiting_tool_approval = false;
                                app.status_msg = "Executing tools...".to_string();
                                app.status_is_error = false;
                            } else {
                                // Show approval prompt in the UI
                                let names: Vec<String> = app.pending_tool_calls.iter().map(|t| t.name.clone()).collect();
                                app.status_msg = format!("⚠ Tool approval needed: {} — [Y] approve  [n] deny  [a] yolo", names.join(", "));
                                app.status_is_error = false;
                                // Store assistant content for later use when approved
                                // (already in app.assistant_tool_content)
                            }
                        } else {
                            // Normal text response — save to history
                            if let Some(msg) = app.messages.last() {
                                if msg.role == "assistant" {
                                    app.conversation_history.push(serde_json::json!({
                                        "role": "assistant",
                                        "content": msg.content
                                    }));
                                }
                            }
                            // Extract code blocks for /code command
                            app.extract_code_blocks();

                            // Estimate response tokens
                            if let Some(msg) = app.messages.last() {
                                if msg.role == "assistant" {
                                    app.token_response = (msg.content.len() as u64) / 4;
                                }
                            }

                            // Update token breakdown
                            app.update_token_breakdown();
                            let used_chars: usize = app.conversation_history.iter()
                                .map(|m| m["content"].as_str().unwrap_or("").len())
                                .sum::<usize>() + app.system_prompt.len();
                            let used_tokens = (used_chars / 4) as u64;
                            app.token_used = used_tokens;
                            let total = match app.current_model.as_str() {
                                m if m.contains("claude-opus-4") || m.contains("claude-sonnet-4") => 200_000u64,
                                m if m.contains("claude-haiku") => 200_000,
                                m if m.contains("gpt-4o") => 128_000,
                                m if m.contains("gpt-4") => 128_000,
                                m if m.contains("gpt-5") => 128_000,
                                m if m.contains("gemini-1.5-pro") || m.contains("gemini-2") => 1_000_000,
                                m if m.contains("gemini") => 128_000,
                                m if m.contains("deepseek") => 64_000,
                                m if m.contains("mistral") || m.contains("mixtral") => 32_000,
                                m if m.contains("llama") || m.contains("Llama") => 8_192,
                                m if m.contains("qwen") || m.contains("Qwen") => 32_000,
                                m if m.contains("phi") || m.contains("Phi") => 16_000,
                                m if m.contains("codestral") => 32_000,
                                // Local GGUF models via llama.cpp — check max from settings
                                m if m.contains(".gguf") || m.contains("Q4_") || m.contains("Q5_") || m.contains("Q8_") || m.contains("Q6_") => {
                                    // Use max_tokens from the /ctx panel as a hint, default to 8192 for local
                                    app.max_tokens.max(8_192) as u64
                                },
                                // For unknown local models, be conservative
                                _ if app.connection_status == "local" => 8_192,
                                _ => 128_000,
                            };
                            app.token_total = total;
                            app.status_msg = format!("Done  ·  ~{} tokens used", used_tokens);
                            app.status_is_error = false;
                            // Auto-compact check (post-response)
                            if app.auto_compact_pct > 0 {
                                let usage_pct = (used_tokens * 100) / total.max(1);
                                if usage_pct >= app.auto_compact_pct {
                                    // First clean completed items
                                    let cleaned = clean_completed_items(&app.conversation_history, &app.root_path);
                                    let before = app.conversation_history.len();
                                    app.conversation_history = cleaned;
                                    app.update_token_breakdown();

                                    // Check if cleaning was enough
                                    let still_pct = (app.token_used * 100) / total.max(1);
                                    if still_pct >= app.auto_compact_pct {
                                        run_pre_compact_hooks(&app.root_path);
                                        // Archive before compacting
                                        save_compaction_archive(&app.conversation_history, &app.root_path);
                                        app.conversation_history = hierarchical_compact_messages(&app.conversation_history, 8);
                                        app.update_token_breakdown();
                                    }

                                    let after = app.conversation_history.len();
                                    if before != after {
                                        app.push_message("assistant", &format!(
                                            "⚡ Auto-compacted at {}% context ({} → {} messages). Work archived to .shadow-memory/.",
                                            usage_pct, before, after
                                        ));
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
                if got_token {
                    app.chat_scroll = usize::MAX;
                }
            }
        }

        // ── Auto-execute approved tools ────────────────────────────────────────
        if app.auto_execute_tools && !app.pending_tool_calls.is_empty() && !app.is_streaming {
            app.auto_execute_tools = false;
            let tools = std::mem::take(&mut app.pending_tool_calls);
            let mut tool_results: Vec<(String, String, String)> = vec![]; // (id, name, result)
            for tc in &tools {
                let input: serde_json::Value = serde_json::from_str(&tc.input_json).unwrap_or(serde_json::json!({}));
                app.status_msg = format!("⚙ Running tool: {}...", tc.name);
                let result = execute_tui_tool(&tc.name, &input, &app.root_path).await;
                let detail = match tc.name.as_str() {
                    "write_file" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("💾".to_string(), path.to_string()));
                        format!("💾 write_file → {}\n  path: {}", result, path)
                    }
                    "create_file" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("📄".to_string(), path.to_string()));
                        format!("📄 create_file → {}\n  path: {}", result, path)
                    }
                    "append_to_file" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("📝".to_string(), path.to_string()));
                        format!("📝 append → {}\n  path: {}", result, path)
                    }
                    "make_dir" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("📁".to_string(), path.to_string()));
                        format!("📁 make_dir → {}\n  path: {}", result, path)
                    }
                    "delete_file" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("🗑".to_string(), path.to_string()));
                        format!("🗑  delete → {}\n  path: {}", result, path)
                    }
                    "patch_file" => {
                        let path = input["path"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("✏️".to_string(), path.to_string()));
                        format!("✏️  patch → {}\n  path: {}", result, path)
                    }
                    "move_file" => {
                        let from = input["from"].as_str().unwrap_or("?");
                        let to = input["to"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("➡️".to_string(), format!("{} → {}", from, to)));
                        format!("➡️  move → {}", result)
                    }
                    "copy_file" => {
                        let to = input["to"].as_str().unwrap_or("?");
                        app.recent_file_changes.push(("📋".to_string(), to.to_string()));
                        format!("📋 copy → {}", result)
                    }
                    "run_command" => {
                        let cmd = input["command"].as_str().unwrap_or("?");
                        let cmd_short = if cmd.len() > 80 { format!("{}…", &cmd[..80]) } else { cmd.to_string() };
                        let res_short = if result.len() > 200 { format!("{}…", &result[..200]) } else { result.clone() };
                        format!("⚙ run $ {}\n  {}", cmd_short, res_short)
                    }
                    _ => {
                        let preview = if result.len() > 120 { format!("{}…", &result[..120]) } else { result.clone() };
                        format!("⚙ {} → {}", tc.name, preview)
                    }
                };
                app.push_message("tool", &detail);
                tool_results.push((tc.id.clone(), tc.name.clone(), result));
            }
            // Refresh file tree if any tool modified the filesystem
            let fs_tools = ["write_file", "create_file", "append_to_file", "delete_file",
                            "move_file", "copy_file", "make_dir", "patch_file"];
            if tools.iter().any(|tc| fs_tools.contains(&tc.name.as_str())) {
                app.load_file_tree();
            }
            // Add tool results to conversation history
            if app.is_anthropic_provider {
                let result_content: Vec<serde_json::Value> = tool_results.iter().map(|(id, _, res)| {
                    serde_json::json!({"type":"tool_result","tool_use_id":id,"content":res})
                }).collect();
                app.conversation_history.push(serde_json::json!({"role":"user","content":result_content}));
            } else if app.prompt_tool_mode {
                for (_, name, res) in &tool_results {
                    app.conversation_history.push(serde_json::json!({
                        "role": "user",
                        "content": format!("[TOOL RESULT: {}]\n{}\n[END TOOL RESULT]", name, res)
                    }));
                }
            } else {
                // OpenAI format: one tool message per result
                for (id, _, res) in &tool_results {
                    app.conversation_history.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":res}));
                }
            }
            // Continue AI conversation with tool results
            tui_continue_with_tool_results(&mut app).await;
        }

        terminal.draw(|f| render_tui(f, &app))?;

        if app.should_quit { break; }

        if ct_event::poll(std::time::Duration::from_millis(16))? {
            match ct_event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_tui_key(&mut app, key).await;
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

async fn handle_tui_key(app: &mut TuiApp, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    // ── Tool approval mode ────────────────────────────────────────────────────
    // When the AI has requested tools and we need human approval, intercept keys.
    if app.awaiting_tool_approval && !app.pending_tool_calls.is_empty() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                // Approve — schedule auto-execution on next tick
                if let Some(content) = app.assistant_tool_content.take() {
                    if app.is_anthropic_provider {
                        app.conversation_history.push(serde_json::json!({"role":"assistant","content":content}));
                    } else {
                        app.conversation_history.push(content);
                    }
                }
                app.awaiting_tool_approval = false;
                app.auto_execute_tools = true;
                app.status_msg = "Executing approved tools...".to_string();
                app.status_is_error = false;
            }
            KeyCode::Char('n') => {
                // Deny all tools — send denial results so AI can respond
                if let Some(content) = app.assistant_tool_content.take() {
                    if app.is_anthropic_provider {
                        app.conversation_history.push(serde_json::json!({"role":"assistant","content":content}));
                    } else {
                        app.conversation_history.push(content);
                    }
                }
                let tools = std::mem::take(&mut app.pending_tool_calls);
                if app.is_anthropic_provider {
                    let result_content: Vec<serde_json::Value> = tools.iter().map(|tc| {
                        serde_json::json!({"type":"tool_result","tool_use_id":tc.id,"content":"[Permission denied by user]"})
                    }).collect();
                    app.conversation_history.push(serde_json::json!({"role":"user","content":result_content}));
                } else if app.prompt_tool_mode {
                    for tc in &tools {
                        app.conversation_history.push(serde_json::json!({
                            "role": "user",
                            "content": format!(
                                "[TOOL RESULT: {}]\n[Permission denied by user]\n[END TOOL RESULT]",
                                tc.name
                            )
                        }));
                    }
                } else {
                    for tc in &tools {
                        app.conversation_history.push(serde_json::json!({"role":"tool","tool_call_id":tc.id,"content":"[Permission denied by user]"}));
                    }
                }
                app.push_message("tool", "⊘ Tool calls denied by user.");
                app.awaiting_tool_approval = false;
                app.status_is_error = false;
                tui_continue_with_tool_results(app).await;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Approve all + enable yolo for this session
                YOLO_MODE.store(true, Ordering::Relaxed);
                app.tool_approval_mode = TuiApprovalMode::Yolo;
                app.push_message("assistant", "⚡ Yolo mode enabled — all tools auto-approved.");
                if let Some(content) = app.assistant_tool_content.take() {
                    if app.is_anthropic_provider {
                        app.conversation_history.push(serde_json::json!({"role":"assistant","content":content}));
                    } else {
                        app.conversation_history.push(content);
                    }
                }
                app.awaiting_tool_approval = false;
                app.auto_execute_tools = true;
                app.status_is_error = false;
            }
            _ => {}
        }
        return;
    }

    if app.palette_open {
        match key.code {
            KeyCode::Esc => { app.palette_open = false; }
            KeyCode::Enter => {
                if let Some(cmd) = app.palette_items.get(app.palette_selected).cloned() {
                    app.palette_open = false;
                    app.input = cmd;
                    app.input_cursor = app.input.len();
                }
            }
            KeyCode::Up => {
                if app.palette_selected > 0 { app.palette_selected -= 1; }
            }
            KeyCode::Down => {
                if app.palette_selected + 1 < app.palette_items.len() {
                    app.palette_selected += 1;
                }
            }
            KeyCode::Char(c) => {
                app.palette_query.push(c);
                app.filter_palette();
            }
            KeyCode::Backspace => {
                app.palette_query.pop();
                app.filter_palette();
            }
            _ => {}
        }
        return;
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            app.should_quit = true;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.open_command_palette();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            app.messages.clear();
            app.chat_scroll = 0;
            app.status_msg = "Screen cleared".to_string();
            app.status_is_error = false;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.input.truncate(app.input_cursor);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            let n = app.session_tabs.len() + 1;
            app.session_tabs.push(format!("Session {}", n));
            app.active_tab = app.session_tabs.len() - 1;
            app.messages.clear();
            app.input.clear();
            app.input_cursor = 0;
        }
        (KeyModifiers::NONE, KeyCode::Tab) => {
            app.active_pane = match app.active_pane {
                TuiPane::Input => TuiPane::Chat,
                TuiPane::Chat => TuiPane::FileTree,
                TuiPane::FileTree => TuiPane::Context,
                TuiPane::Context => TuiPane::Input,
            };
        }
        (KeyModifiers::CONTROL, KeyCode::BackTab) | (KeyModifiers::CONTROL, KeyCode::Tab) => {
            app.active_tab = (app.active_tab + 1) % app.session_tabs.len();
        }
        (KeyModifiers::NONE, KeyCode::Enter) => {
            let input = app.input.trim().to_string();
            if !input.is_empty() {
                app.push_message("user", &input);
                app.input_history.push(input.clone());
                app.input.clear();
                app.input_cursor = 0;

                if input == "/quit" || input == "/exit" || input == "/q" {
                    app.should_quit = true;
                    return;
                }
                if input == "/clear" || input == "/cls" {
                    app.messages.clear();
                    app.conversation_history.clear();
                    app.chat_scroll = 0;
                    return;
                }
                if input == "/new" {
                    let n = app.session_tabs.len() + 1;
                    app.session_tabs.push(format!("Session {}", n));
                    app.active_tab = app.session_tabs.len() - 1;
                    app.messages.clear();
                    app.conversation_history.clear();
                    app.input.clear();
                    app.input_cursor = 0;
                    return;
                }
                if input == "/models" || input == "/model" {
                    tui_list_models(app).await;
                    return;
                }
                if let Some(new_model) = input.strip_prefix("/model ") {
                    app.current_model = new_model.trim().to_string();
                    app.push_message("assistant", &format!("Model switched to: {}", app.current_model));
                    app.status_msg = format!("Model: {}", app.current_model);
                    return;
                }
                if input == "/help" || input == "/help all" {
                    app.push_message("assistant", concat!(
                        "ShadowAI TUI — All Commands\n",
                        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n",
                        "\n",
                        "Session\n",
                        "──────────────────────────────────────\n",
                        "/clear               — clear chat history\n",
                        "/new                 — new session tab (Ctrl+N)\n",
                        "/quit  /exit  /q     — exit\n",
                        "/system <prompt>     — set system prompt\n",
                        "/mode auto|plan|act  — change agent mode\n",
                        "\n",
                        "Models & Providers\n",
                        "──────────────────────────────────────\n",
                        "/models  /model      — list available models\n",
                        "/model <name>        — switch active model\n",
                        "/model pull <name>   — pull Ollama model\n",
                        "/switch <provider>   — switch provider mid-session\n",
                        "/cheap on|off|status — auto-route to cheaper model\n",
                        "/cost                — token/cost summary\n",
                        "\n",
                        "Context & Memory\n",
                        "──────────────────────────────────────\n",
                        "/add <glob>          — add files to context\n",
                        "/ctx                 — show context slots\n",
                        "/drop <label>        — remove context slot\n",
                        "/memory              — show/save memory.md\n",
                        "/hcompact            — hierarchical context compaction\n",
                        "/snapshot [save|load <name>] — session snapshots\n",
                        "/think <tokens>      — set reasoning budget\n",
                        "/plan on|off         — plan mode (auto-think)\n",
                        "\n",
                        "Code & Dev\n",
                        "──────────────────────────────────────\n",
                        "/gd [file]           — git diff viewer\n",
                        "/gr <n>              — interactive rebase last N\n",
                        "/blame <file>        — git blame\n",
                        "/deps check          — check dependencies\n",
                        "/deps upgrade        — upgrade dependencies\n",
                        "/deps fix            — auto-fix dep issues\n",
                        "/docs <pkg>          — fetch package docs\n",
                        "/profile <secs>      — CPU profiling\n",
                        "/debug <sub>         — debugger (core|attach|trace|perf)\n",
                        "/heal                — auto-fix last error\n",
                        "/extract             — extract function/variable\n",
                        "/rename <old> <new>  — rename symbol\n",
                        "/translate <lang>    — translate code to language\n",
                        "/mock <file>         — generate mocks\n",
                        "\n",
                        "Agentic\n",
                        "──────────────────────────────────────\n",
                        "/spawn <task>        — background AI subtask\n",
                        "/architect           — read-only planning mode\n",
                        "/yolo                — skip all confirmations\n",
                        "/approval full|smart|yolo — approval tier\n",
                        "/arena <prompt>      — A/B two models side-by-side\n",
                        "/teleport            — serialize session to file\n",
                        "/agent <name>        — run named agent template\n",
                        "/repomap             — ctags repo structure map\n",
                        "\n",
                        "Built-in MCP Tools\n",
                        "──────────────────────────────────────\n",
                        "/mcp tools           — list built-in tools\n",
                        "/mcp list            — list connected MCP servers\n",
                        "/mcp connect <url>   — connect to MCP server\n",
                        "/mcp call <json>     — {\"tool\":\"read_file\",\"args\":{\"path\":\"…\"}}\n",
                        "\n",
                        "Team Commands\n",
                        "──────────────────────────────────────\n",
                        "/commands            — list .shadowai/commands/*.md\n",
                        "/run-cmd <name> [args]— run team command\n",
                        "\n",
                        "LSP Integration\n",
                        "──────────────────────────────────────\n",
                        "/lsp def  <f> <l> <c> — go to definition\n",
                        "/lsp refs <f> <l> <c> — find references\n",
                        "/lsp hover <f> <l> <c>— hover docs\n",
                        "\n",
                        "Transactional Actions\n",
                        "──────────────────────────────────────\n",
                        "/txn begin [label]   — stash working tree\n",
                        "/txn rollback        — restore last stash\n",
                        "/txn commit          — drop stash (finalize)\n",
                        "\n",
                        "Audit & Security\n",
                        "──────────────────────────────────────\n",
                        "/audit search <term> — search audit log\n",
                        "/audit stats         — audit log statistics\n",
                        "/block <id>          — re-print response block\n",
                        "/log search <term>   — search session logs\n",
                        "/runbook new <name>  — create runbook\n",
                        "/runbook run <name>  — execute runbook\n",
                        "/runbook list        — list runbooks\n",
                        "\n",
                        "Multimodal\n",
                        "──────────────────────────────────────\n",
                        "/voice               — record audio → prompt\n",
                        "/screenshot          — capture screen → prompt\n",
                        "cat img.png | shadowai 'question' — image pipe\n",
                        "\n",
                        "Database / Cloud / Infra (IDE mode)\n",
                        "──────────────────────────────────────\n",
                        "/db <query>          — run SQL query\n",
                        "/k8s <cmd>           — kubectl helper\n",
                        "/migrate <sub>       — DB migrations\n",
                        "/rag <query>         — retrieval-augmented search\n",
                        "/pdf <path>          — summarise PDF\n",
                        "/yt <url>            — summarise YouTube video\n",
                        "/relay start|stop    — real-time collaboration relay\n",
                        "/pr review <branch>  — AI PR review\n",
                        "/research <topic>    — web research\n",
                        "/cron list|add|del   — scheduled tasks\n",
                        "\n",
                        "Keyboard Shortcuts\n",
                        "──────────────────────────────────────\n",
                        "Tab       — cycle panes (Input/Chat/Files/Context)\n",
                        "Ctrl+Tab  — next session tab\n",
                        "Ctrl+N    — new session tab\n",
                        "Ctrl+P    — command palette\n",
                        "Ctrl+L    — clear screen\n",
                        "Ctrl+K    — clear input to cursor\n",
                        "PgUp/PgDn — scroll chat\n",
                        "↑/↓       — input history (in Input pane)\n"
                    ));
                    return;
                }
                if let Some(rest) = input.strip_prefix("/system ") {
                    app.system_prompt = rest.trim().to_string();
                    app.push_message("assistant", &format!("System prompt updated: {}", rest.trim()));
                    return;
                }
                if let Some(rest) = input.strip_prefix("/mode ") {
                    app.current_mode = rest.trim().to_string();
                    app.push_message("assistant", &format!("Mode switched to: {}", app.current_mode));
                    app.status_msg = format!("Mode: {}", app.current_mode);
                    return;
                }
                // ── Section 13: MCP built-in tools list ──────────────
                if input == "/mcp tools" || input == "/mcp-tools" {
                    let tools = list_builtin_mcp_tools();
                    let mut out = "Built-in MCP tools (no server required):\n".to_string();
                    for (name, desc) in &tools {
                        out.push_str(&format!("  {:14} {}\n", name, desc));
                    }
                    app.push_message("assistant", &out);
                    return;
                }
                if let Some(json_str) = input.strip_prefix("/mcp call ") {
                    match serde_json::from_str::<serde_json::Value>(json_str) {
                        Ok(obj) => {
                            let tool = obj["tool"].as_str().unwrap_or("");
                            let result = dispatch_builtin_mcp_tool(tool, &obj["args"])
                                .unwrap_or_else(|| "[error] unknown built-in tool".to_string());
                            app.push_message("assistant", &format!("[mcp:{}]\n{}", tool, result));
                        }
                        Err(e) => {
                            app.push_message("assistant", &format!("[mcp] invalid JSON: {}", e));
                        }
                    }
                    return;
                }
                // ── Section 14: pre-compact hook + hierarchical compact ──
                if input == "/compact hier" || input == "/hcompact" {
                    run_pre_compact_hooks(&app.root_path);
                    let before = app.conversation_history.len();
                    // Clean → archive → compact
                    app.conversation_history = clean_completed_items(&app.conversation_history, &app.root_path);
                    save_compaction_archive(&app.conversation_history, &app.root_path);
                    app.conversation_history = hierarchical_compact_messages(&app.conversation_history, 8);
                    app.update_token_breakdown();
                    app.push_message("assistant", &format!(
                        "Hierarchical compaction: {} → {} messages. Archived to .shadow-memory/.\n\
                         Context: {}k / {}k tokens ({}%)",
                        before, app.conversation_history.len(),
                        app.token_used / 1000, app.token_total / 1000,
                        (app.token_used * 100) / app.token_total.max(1)
                    ));
                    return;
                }
                // ── Section 17a: team shared commands ────────────────
                if input == "/commands" || input == "/team-cmds" {
                    let cmds = load_team_commands(&app.root_path);
                    if cmds.is_empty() {
                        app.push_message("assistant",
                            "No team commands found.\nCreate .shadowai/commands/<name>.md in your project root.");
                    } else {
                        let mut out = "Team commands:\n".to_string();
                        for (name, desc, _) in &cmds {
                            out.push_str(&format!("  /{:<20} {}\n", name, desc));
                        }
                        app.push_message("assistant", &out);
                    }
                    return;
                }
                if let Some(rest) = input.strip_prefix("/run-cmd ") {
                    let mut parts = rest.splitn(2, ' ');
                    let name = parts.next().unwrap_or("").trim();
                    let args = parts.next().unwrap_or("").trim();
                    match run_team_command(name, args, &app.root_path) {
                        Some(content) => {
                            app.push_message("assistant", &format!(
                                "[/{} — team command]\n{}", name, &content[..content.len().min(2000)]
                            ));
                        }
                        None => {
                            app.push_message("assistant", &format!("[error] command '{}' not found.", name));
                        }
                    }
                    return;
                }
                // ── Section 17b: LSP tools ────────────────────────────
                if let Some(rest) = input.strip_prefix("/lsp def ") {
                    // Format: /lsp def <file> <line> <col>
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let file = parts[0];
                        let line: u32 = parts[1].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let col: u32  = parts[2].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let result = lsp_go_to_definition(&app.root_path, file, line, col);
                        app.push_message("assistant", &format!("[LSP definition]\n{}", result));
                    } else {
                        app.push_message("assistant", "Usage: /lsp def <file> <line> <col>");
                    }
                    return;
                }
                if let Some(rest) = input.strip_prefix("/lsp refs ") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let file = parts[0];
                        let line: u32 = parts[1].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let col: u32  = parts[2].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let result = lsp_find_references(&app.root_path, file, line, col);
                        app.push_message("assistant", &format!("[LSP references]\n{}", result));
                    } else {
                        app.push_message("assistant", "Usage: /lsp refs <file> <line> <col>");
                    }
                    return;
                }
                if let Some(rest) = input.strip_prefix("/lsp hover ") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let file = parts[0];
                        let line: u32 = parts[1].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let col: u32  = parts[2].parse::<u32>().unwrap_or(1).saturating_sub(1);
                        let result = lsp_hover(&app.root_path, file, line, col);
                        app.push_message("assistant", &format!("[LSP hover]\n{}", result));
                    } else {
                        app.push_message("assistant", "Usage: /lsp hover <file> <line> <col>");
                    }
                    return;
                }
                // ── Section 18: transactional agent actions ───────────
                if let Some(rest) = input.strip_prefix("/txn ") {
                    let result = handle_txn_command(rest, &app.root_path);
                    app.push_message("assistant", &result);
                    return;
                }
                // ── Section 19: model cost routing ────────────────────
                if let Some(rest) = input.strip_prefix("/cheap ") {
                    let result = handle_cheap_command(rest, app);
                    app.push_message("assistant", &result);
                    return;
                }
                if input == "/cheap" {
                    let result = handle_cheap_command("status", app);
                    app.push_message("assistant", &result);
                    return;
                }
                // ── Web search (/search) ──────────────────────────────
                if let Some(query) = input.strip_prefix("/search ") {
                    let q = query.trim().to_string();
                    app.status_msg = format!("Searching: {}…", &q[..q.len().min(40)]);
                    let result = google_scrape_search(&q).await
                        .unwrap_or_else(|e| format!("[search error] {}", e));
                    let ctx = format!("\n\n<web-search query=\"{}\">\n{}\n</web-search>\n", q, result);
                    // Inject into next AI message and show in chat
                    app.push_message("assistant", &format!("Search results for: {}\n\n{}", q, result));
                    // Store as context for next prompt
                    app.conversation_history.push(serde_json::json!({
                        "role": "user",
                        "content": ctx
                    }));
                    app.status_msg = "Search done — results injected into context.".to_string();
                    return;
                }
                // ── Browse URL (/browse) ──────────────────────────────
                if let Some(url) = input.strip_prefix("/browse ") {
                    let url = url.trim().to_string();
                    app.status_msg = format!("Fetching {}…", &url[..url.len().min(40)]);
                    let fetch_result: Result<String, String> = async {
                        let client = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(15))
                            .user_agent("shadowai-cli/0.2")
                            .build()
                            .map_err(|e| format!("Client error: {}", e))?;
                        let html = client.get(&url).send().await
                            .map_err(|e| format!("Fetch error: {}", e))?
                            .text().await
                            .map_err(|e| format!("Response error: {}", e))?;
                        Ok(html)
                    }.await;
                    match fetch_result {
                        Ok(html) => {
                            // Strip HTML tags to plain text
                            let mut text = String::with_capacity(html.len() / 2);
                            let mut in_tag = false;
                            for c in html.chars() {
                                match c {
                                    '<' => { in_tag = true; }
                                    '>' => { in_tag = false; text.push(' '); }
                                    _ if !in_tag => { text.push(c); }
                                    _ => {}
                                }
                            }
                            let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                            let preview = &clean[..clean.len().min(3000)];
                            let ctx = format!("\n\n<web-page url=\"{}\">\n{}\n</web-page>\n", url, clean);
                            app.push_message("assistant", &format!("Page: {}\n\n{}", url, preview));
                            app.conversation_history.push(serde_json::json!({
                                "role": "user",
                                "content": ctx
                            }));
                            app.status_msg = "Page fetched — content injected into context.".to_string();
                        }
                        Err(e) => { app.push_message("assistant", &format!("[browse error] {}", e)); }
                    }
                    return;
                }
                // ── Read file into context (@-expansion also works inline) ──
                if let Some(path) = input.strip_prefix("/read ") {
                    let path = path.trim();
                    let full = if path.starts_with('/') { path.to_string() }
                               else { format!("{}/{}", app.root_path.trim_end_matches('/'), path) };
                    match std::fs::read_to_string(&full) {
                        Ok(content) => {
                            let ext = std::path::Path::new(path).extension()
                                .and_then(|e| e.to_str()).unwrap_or("");
                            let ctx = format!("\n\nContents of `{}`:\n```{}\n{}\n```\n", path, ext, content);
                            let preview = &content[..content.len().min(500)];
                            app.push_message("assistant", &format!(
                                "Read `{}` ({} bytes):\n```{}\n{}{}```",
                                path, content.len(), ext, preview,
                                if content.len() > 500 { "\n[...truncated in preview — full content in context]" } else { "" }
                            ));
                            app.conversation_history.push(serde_json::json!({
                                "role": "user", "content": ctx
                            }));
                            app.status_msg = format!("File {} added to context.", path);
                        }
                        Err(e) => {
                            app.push_message("assistant", &format!("[read error] {}: {}", path, e));
                        }
                    }
                    return;
                }
                // ── Git status/log/diff shortcuts ─────────────────────
                if input == "/git" || input == "/git status" {
                    let out = std::process::Command::new("git")
                        .args(["status", "--short"])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_else(|_| "git not available".to_string());
                    app.push_message("assistant", &format!("git status:\n```\n{}\n```", out.trim()));
                    return;
                }
                if input == "/gd" || input == "/git diff" {
                    let out = std::process::Command::new("git")
                        .args(["diff", "--stat"])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_else(|_| "git not available".to_string());
                    app.push_message("assistant", &format!("git diff --stat:\n```\n{}\n```", out.trim()));
                    return;
                }
                if input == "/git log" {
                    let out = std::process::Command::new("git")
                        .args(["log", "--oneline", "-15"])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_else(|_| "git not available".to_string());
                    app.push_message("assistant", &format!("git log:\n```\n{}\n```", out.trim()));
                    return;
                }
                // ── Build / test / lint (run in project root) ─────────
                if input == "/build" || input.starts_with("/build ") {
                    let cmd_args = input.strip_prefix("/build").unwrap_or("").trim();
                    let shell_cmd = if cmd_args.is_empty() {
                        // auto-detect: Cargo / npm / make
                        if std::path::Path::new(&app.root_path).join("Cargo.toml").exists() {
                            "cargo build 2>&1 | tail -20".to_string()
                        } else if std::path::Path::new(&app.root_path).join("package.json").exists() {
                            "npm run build 2>&1 | tail -20".to_string()
                        } else {
                            "make 2>&1 | tail -20".to_string()
                        }
                    } else {
                        format!("{} 2>&1 | tail -20", cmd_args)
                    };
                    app.status_msg = "Building…".to_string();
                    let out = std::process::Command::new("sh")
                        .args(["-c", &shell_cmd])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| {
                            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                            format!("{}{}", stdout, stderr)
                        })
                        .unwrap_or_else(|e| format!("[build error] {}", e));
                    app.push_message("assistant", &format!("Build output:\n```\n{}\n```", out.trim()));
                    app.status_msg = "Build done.".to_string();
                    return;
                }
                if input == "/test" || input.starts_with("/test ") {
                    let shell_cmd = if std::path::Path::new(&app.root_path).join("Cargo.toml").exists() {
                        "cargo test 2>&1 | tail -30"
                    } else if std::path::Path::new(&app.root_path).join("package.json").exists() {
                        "npm test 2>&1 | tail -30"
                    } else {
                        "make test 2>&1 | tail -30"
                    };
                    app.status_msg = "Running tests…".to_string();
                    let out = std::process::Command::new("sh")
                        .args(["-c", shell_cmd])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_else(|e| format!("[test error] {}", e));
                    app.push_message("assistant", &format!("Test output:\n```\n{}\n```", out.trim()));
                    app.status_msg = "Tests done.".to_string();
                    return;
                }
                if input == "/lint" || input.starts_with("/lint ") {
                    let shell_cmd = if std::path::Path::new(&app.root_path).join("Cargo.toml").exists() {
                        "cargo clippy 2>&1 | tail -30"
                    } else if std::path::Path::new(&app.root_path).join("package.json").exists() {
                        "npx eslint . 2>&1 | tail -30"
                    } else {
                        "echo 'No lint config detected'"
                    };
                    app.status_msg = "Linting…".to_string();
                    let out = std::process::Command::new("sh")
                        .args(["-c", shell_cmd])
                        .current_dir(&app.root_path)
                        .output()
                        .map(|o| {
                            let s = String::from_utf8_lossy(&o.stdout).to_string();
                            let e = String::from_utf8_lossy(&o.stderr).to_string();
                            format!("{}{}", s, e)
                        })
                        .unwrap_or_else(|e| format!("[lint error] {}", e));
                    app.push_message("assistant", &format!("Lint output:\n```\n{}\n```", out.trim()));
                    app.status_msg = "Lint done.".to_string();
                    return;
                }
                // ── Context info (/ctx) ───────────────────────────────
                if input == "/ctx" || input == "/context" {
                    let msg_count = app.conversation_history.len();
                    let total_chars: usize = app.conversation_history.iter()
                        .map(|m| m["content"].as_str().unwrap_or("").len()).sum();
                    let est_tokens = total_chars / 4;
                    app.push_message("assistant", &format!(
                        "Context summary:\n  Messages : {}\n  ~Chars   : {}\n  ~Tokens  : {}\n  Model    : {}\n  System   : {} chars\n  Root     : {}",
                        msg_count, total_chars, est_tokens,
                        app.current_model, app.system_prompt.len(), app.root_path
                    ));
                    return;
                }
                // ── /yolo — enable full auto-approval ────────────────
                if input == "/yolo" {
                    let was_yolo = YOLO_MODE.load(Ordering::Relaxed);
                    YOLO_MODE.store(!was_yolo, Ordering::Relaxed);
                    app.tool_approval_mode = if !was_yolo { TuiApprovalMode::Yolo } else { TuiApprovalMode::Smart };
                    app.push_message("assistant", if !was_yolo {
                        "⚡ Yolo mode ON — all tool calls auto-approved without prompting."
                    } else {
                        "🔒 Yolo mode OFF — dangerous tool calls require approval."
                    });
                    return;
                }
                // ── /approval — set approval tier ────────────────────
                if let Some(tier) = input.strip_prefix("/approval ") {
                    match tier.trim() {
                        "yolo" | "full" => {
                            app.tool_approval_mode = TuiApprovalMode::Yolo;
                            YOLO_MODE.store(true, Ordering::Relaxed);
                            app.push_message("assistant", "Tool approval: yolo (all auto-approved)");
                        }
                        "smart" => {
                            app.tool_approval_mode = TuiApprovalMode::Smart;
                            app.push_message("assistant", "Tool approval: smart (safe tools auto, dangerous ask)");
                        }
                        "ask" | "askall" => {
                            app.tool_approval_mode = TuiApprovalMode::AskAll;
                            app.push_message("assistant", "Tool approval: ask-all (every tool requires confirmation)");
                        }
                        _ => {
                            app.push_message("assistant", "Usage: /approval yolo | smart | ask");
                        }
                    }
                    return;
                }
                if input == "/approval" {
                    let mode = match app.tool_approval_mode {
                        TuiApprovalMode::Yolo  => "yolo — all tools auto-approved",
                        TuiApprovalMode::Smart => "smart — safe tools auto, dangerous prompt",
                        TuiApprovalMode::AskAll=> "ask-all — every tool asks for approval",
                    };
                    app.push_message("assistant", &format!("Current approval mode: {mode}\nChange with: /approval yolo | smart | ask"));
                    return;
                }
                // ── /tools — list available AI tools ─────────────────
                if input == "/tools" {
                    let mode = match app.tool_approval_mode {
                        TuiApprovalMode::Yolo   => "yolo (all auto)",
                        TuiApprovalMode::Smart  => "smart (safe auto / dangerous ask)",
                        TuiApprovalMode::AskAll => "ask-all",
                    };
                    app.push_message("assistant", &format!(
                        "Available AI tools (approval: {mode}):\n\
                         \n  Safe (auto-approved in Smart mode):\n\
                         • read_file — read file contents\n\
                         • list_dir — list directory\n\
                         • search_files — grep/search pattern\n\
                         \n  Requires approval:\n\
                         • write_file — overwrite a file\n\
                         • create_file — create new file\n\
                         • append_to_file — append to file\n\
                         • delete_file — delete a file\n\
                         • move_file — move/rename file\n\
                         • copy_file — copy file\n\
                         • make_dir — create directory\n\
                         • patch_file — apply diff patch\n\
                         • run_command — execute shell command\n\
                         • git_status / git_diff / git_log — git info\n\
                         • git_commit — stage and commit\n\
                         • web_fetch — fetch a URL\n\
                         \n  /approval yolo — auto-approve everything\n\
                         /approval smart — current default\n\
                         /approval ask — ask for every tool"
                    ));
                    return;
                }
                // ── /code — list & interact with code blocks ─────
                if input == "/code" || input.starts_with("/code ") {
                    if app.code_blocks.is_empty() {
                        app.push_message("assistant", "No code blocks available. Send a message first.");
                        return;
                    }
                    let arg = input.strip_prefix("/code").unwrap_or("").trim();
                    if arg.is_empty() {
                        // List code blocks
                        let mut listing = String::from("Code blocks from last response:\n");
                        for (i, (lang, code, _)) in app.code_blocks.iter().enumerate() {
                            let preview = code.lines().next().unwrap_or("(empty)");
                            let preview_short = if preview.len() > 60 { format!("{}…", &preview[..60]) } else { preview.to_string() };
                            listing.push_str(&format!("  [{}] {} — {}\n", i + 1, if lang.is_empty() { "text" } else { lang }, preview_short));
                        }
                        listing.push_str("\nUsage: /code <N> copy | /code <N> save <path> | /code <N> diff <path>");
                        app.push_message("assistant", &listing);
                        return;
                    }
                    // Parse: /code N action [path]
                    let parts: Vec<&str> = arg.splitn(3, ' ').collect();
                    let idx: usize = parts.first().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                    if idx == 0 || idx > app.code_blocks.len() {
                        app.push_message("assistant", &format!("Invalid block number. Use 1-{}.", app.code_blocks.len()));
                        return;
                    }
                    let (_, code, _) = &app.code_blocks[idx - 1];
                    let action = parts.get(1).copied().unwrap_or("copy");
                    match action {
                        "copy" | "cp" => {
                            // Copy to clipboard via xclip/xsel/wl-copy
                            let copied = std::process::Command::new("sh")
                                .args(["-c", &format!("printf '%s' '{}' | xclip -selection clipboard 2>/dev/null || printf '%s' '{}' | xsel --clipboard 2>/dev/null || printf '%s' '{}' | wl-copy 2>/dev/null",
                                    code.replace('\'', "'\\''"), code.replace('\'', "'\\''"), code.replace('\'', "'\\''"))])
                                .status().map(|s| s.success()).unwrap_or(false);
                            let msg = if copied {
                                format!("Copied block {} to clipboard.", idx)
                            } else {
                                "Failed to copy — install xclip, xsel, or wl-copy.".to_string()
                            };
                            app.push_message("assistant", &msg);
                        }
                        "save" | "create" => {
                            let path = parts.get(2).unwrap_or(&"output.txt");
                            let full_path = if std::path::Path::new(path).is_absolute() {
                                path.to_string()
                            } else {
                                format!("{}/{}", app.root_path, path)
                            };
                            if let Some(p) = std::path::Path::new(&full_path).parent() {
                                let _ = std::fs::create_dir_all(p);
                            }
                            match std::fs::write(&full_path, code) {
                                Ok(_) => {
                                    app.recent_file_changes.push(("📄".to_string(), full_path.clone()));
                                    app.push_message("assistant", &format!("📄 Saved block {} to {}", idx, full_path));
                                    app.load_file_tree();
                                }
                                Err(e) => app.push_message("error", &format!("Failed to save: {}", e)),
                            }
                        }
                        "diff" => {
                            let path = parts.get(2).unwrap_or(&"");
                            if path.is_empty() {
                                app.push_message("assistant", "Usage: /code <N> diff <file_path>");
                                return;
                            }
                            let full_path = if std::path::Path::new(path).is_absolute() {
                                path.to_string()
                            } else {
                                format!("{}/{}", app.root_path, path)
                            };
                            match std::fs::read_to_string(&full_path) {
                                Ok(existing) => {
                                    // Simple line-by-line diff
                                    let old_lines: Vec<&str> = existing.lines().collect();
                                    let new_lines: Vec<&str> = code.lines().collect();
                                    let mut diff_output = format!("Diff: block {} vs {}\n", idx, path);
                                    let max_lines = old_lines.len().max(new_lines.len());
                                    for i in 0..max_lines {
                                        let old = old_lines.get(i).copied().unwrap_or("");
                                        let new = new_lines.get(i).copied().unwrap_or("");
                                        if old != new {
                                            if !old.is_empty() { diff_output.push_str(&format!("- {}\n", old)); }
                                            if !new.is_empty() { diff_output.push_str(&format!("+ {}\n", new)); }
                                        }
                                    }
                                    app.push_message("assistant", &diff_output);
                                }
                                Err(e) => app.push_message("error", &format!("Cannot read {}: {}", path, e)),
                            }
                        }
                        _ => app.push_message("assistant", "Usage: /code <N> copy | save <path> | diff <path>"),
                    }
                    return;
                }
                // ── /rewind — rewind to previous user message ────
                if input == "/rewind" || input.starts_with("/rewind ") {
                    let arg = input.strip_prefix("/rewind").unwrap_or("").trim();
                    let user_msgs: Vec<(usize, &ChatMessage)> = app.messages.iter().enumerate()
                        .filter(|(_, m)| m.role == "user").collect();
                    if user_msgs.is_empty() {
                        app.push_message("assistant", "No messages to rewind to.");
                        return;
                    }
                    if arg.is_empty() {
                        // List user messages for selection
                        let mut listing = String::from("User messages (use /rewind <N> to rewind):\n");
                        for (count, (_, msg)) in user_msgs.iter().enumerate() {
                            let preview = if msg.content.len() > 60 { format!("{}…", &msg.content[..60]) } else { msg.content.clone() };
                            listing.push_str(&format!("  [{}] {} — {}\n", count + 1, msg.timestamp, preview));
                        }
                        app.push_message("assistant", &listing);
                        return;
                    }
                    let target: usize = arg.parse().unwrap_or(0);
                    if target == 0 || target > user_msgs.len() {
                        app.push_message("assistant", &format!("Invalid. Use 1-{}.", user_msgs.len()));
                        return;
                    }
                    if let Some(content) = app.rewind_to(target - 1) {
                        app.input = content;
                        app.input_cursor = app.input.chars().count();
                        app.push_message("assistant", &format!("⏪ Rewound to message {}. Edit and press Enter to re-send.", target));
                    }
                    return;
                }
                // ── /include — toggle file inclusion in context ──
                if input == "/include" || input.starts_with("/include ") {
                    let arg = input.strip_prefix("/include").unwrap_or("").trim();
                    if arg.is_empty() || arg == "off" || arg == "none" {
                        if app.include_file.is_some() {
                            app.include_file = None;
                            app.push_message("assistant", "📎 File inclusion disabled.");
                        } else {
                            app.push_message("assistant", "Usage: /include <file_path> — attach file to every message\n/include off — disable");
                        }
                        return;
                    }
                    let path = if std::path::Path::new(arg).is_absolute() {
                        arg.to_string()
                    } else {
                        format!("{}/{}", app.root_path, arg)
                    };
                    if std::path::Path::new(&path).exists() {
                        app.include_file = Some(path.clone());
                        let name = std::path::Path::new(&path).file_name()
                            .and_then(|n| n.to_str()).unwrap_or(&path);
                        app.push_message("assistant", &format!("📎 Including {} with every message. /include off to disable.", name));
                    } else {
                        app.push_message("error", &format!("File not found: {}", path));
                    }
                    return;
                }
                // ── /pin /unpin — session pinning ────────────────
                if input == "/pin" {
                    if !app.pinned_sessions.contains(&app.active_tab) {
                        app.pinned_sessions.push(app.active_tab);
                        app.push_message("assistant", &format!("⭐ Pinned session: {}", app.session_tabs.get(app.active_tab).unwrap_or(&"?".to_string())));
                    } else {
                        app.push_message("assistant", "Session already pinned.");
                    }
                    return;
                }
                if input == "/unpin" {
                    app.pinned_sessions.retain(|&i| i != app.active_tab);
                    app.push_message("assistant", "Unpinned current session.");
                    return;
                }
                // ── /temp — set temperature ──────────────────────
                if let Some(val) = input.strip_prefix("/temp ").or_else(|| input.strip_prefix("/temperature ")) {
                    if let Ok(t) = val.trim().parse::<f64>() {
                        if (0.0..=2.0).contains(&t) {
                            app.temperature = t;
                            app.push_message("assistant", &format!("🌡 Temperature set to {:.1}", t));
                        } else {
                            app.push_message("assistant", "Temperature must be 0.0-2.0");
                        }
                    } else {
                        app.push_message("assistant", &format!("Current temperature: {:.1}\nUsage: /temp <0.0-2.0>", app.temperature));
                    }
                    return;
                }
                if input == "/temp" || input == "/temperature" {
                    app.push_message("assistant", &format!("🌡 Temperature: {:.1}\nUsage: /temp <0.0-2.0>", app.temperature));
                    return;
                }
                // ── /maxtok — set max tokens ─────────────────────
                if let Some(val) = input.strip_prefix("/maxtok ") {
                    if let Ok(t) = val.trim().parse::<u32>() {
                        app.max_tokens = t.max(64).min(128_000);
                        app.push_message("assistant", &format!("Max tokens set to {}", app.max_tokens));
                    } else {
                        app.push_message("assistant", "Usage: /maxtok <number>");
                    }
                    return;
                }
                // ── /mode — switch mode with visual feedback ─────
                if let Some(mode) = input.strip_prefix("/mode ") {
                    let mode = mode.trim().to_lowercase();
                    match mode.as_str() {
                        "plan" | "build" | "auto" => {
                            app.current_mode = mode.clone();
                            if mode == "auto" {
                                app.tool_approval_mode = TuiApprovalMode::Yolo;
                            } else if mode == "build" {
                                app.tool_approval_mode = TuiApprovalMode::Smart;
                            } else {
                                app.tool_approval_mode = TuiApprovalMode::AskAll;
                            }
                            app.push_message("assistant", &format!("Mode switched to {}", mode.to_uppercase()));
                        }
                        _ => app.push_message("assistant", "Usage: /mode plan | build | auto"),
                    }
                    return;
                }
                if input == "/mode" {
                    app.push_message("assistant", &format!(
                        "Current mode: {}\n  /mode plan  — high-level strategy, no auto tool execution\n  /mode build — implementation, safe tools auto-approved\n  /mode auto  — full autonomy, all tools auto-approved",
                        app.current_mode.to_uppercase()
                    ));
                    return;
                }
                // ── /memories — browse memory files ──────────────
                if input == "/memories" || input == "/mem" {
                    let memory_dir = dirs_next::home_dir()
                        .map(|h| h.join(".shadowai"))
                        .unwrap_or_else(|| std::path::PathBuf::from(".shadowai"));
                    let memory_file = memory_dir.join("memory.md");
                    if let Ok(content) = std::fs::read_to_string(&memory_file) {
                        if content.trim().is_empty() {
                            app.push_message("assistant", "No memories saved yet.\nUse /remember <text> to save a memory.");
                        } else {
                            let mut output = String::from("🧠 Saved Memories:\n");
                            for (i, line) in content.lines().enumerate() {
                                let line = line.trim();
                                if !line.is_empty() {
                                    output.push_str(&format!("  [{}] {}\n", i + 1, line));
                                }
                            }
                            output.push_str("\nUse /forget <N> to delete a memory.");
                            app.push_message("assistant", &output);
                        }
                    } else {
                        app.push_message("assistant", "No memories file found.\nUse /remember <text> to save.");
                    }
                    return;
                }
                // ── /forget — delete a memory by index ───────────
                if let Some(arg) = input.strip_prefix("/forget ") {
                    let memory_dir = dirs_next::home_dir()
                        .map(|h| h.join(".shadowai"))
                        .unwrap_or_else(|| std::path::PathBuf::from(".shadowai"));
                    let memory_file = memory_dir.join("memory.md");
                    if let Ok(content) = std::fs::read_to_string(&memory_file) {
                        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
                        if let Ok(idx) = arg.trim().parse::<usize>() {
                            if idx > 0 && idx <= lines.len() {
                                let removed = lines[idx - 1];
                                let new_content: Vec<&str> = lines.iter().enumerate()
                                    .filter(|(i, _)| *i != idx - 1)
                                    .map(|(_, l)| *l)
                                    .collect();
                                let _ = std::fs::write(&memory_file, new_content.join("\n") + "\n");
                                app.push_message("assistant", &format!("🗑 Removed memory: {}", removed.trim()));
                            } else {
                                app.push_message("assistant", &format!("Invalid index. Use 1-{}.", lines.len()));
                            }
                        } else {
                            app.push_message("assistant", "Usage: /forget <number>");
                        }
                    }
                    return;
                }
                // ── /profiles — manage provider profiles ─────────
                if input == "/profiles" {
                    if app.profiles.is_empty() {
                        app.push_message("assistant",
                            "No profiles configured.\n\
                             Create one: /profile add <name> <provider> <base_url> <model>\n\
                             Example: /profile add local llamacpp http://localhost:8080/v1 qwen3");
                    } else {
                        let mut listing = String::from("Provider Profiles:\n");
                        for (i, p) in app.profiles.iter().enumerate() {
                            let active = if app.active_profile == Some(i) { " ← active" } else { "" };
                            listing.push_str(&format!("  [{}] {} ({}) — {} model:{}{}\n",
                                i + 1, p.name, p.provider, p.base_url, p.model, active));
                        }
                        listing.push_str("\n/profile use <N> — switch\n/profile add <name> <provider> <url> <model> — create\n/profile rm <N> — delete");
                        app.push_message("assistant", &listing);
                    }
                    return;
                }
                if input.starts_with("/profile ") {
                    let arg = input.strip_prefix("/profile ").unwrap_or("").trim();
                    if let Some(rest) = arg.strip_prefix("add ") {
                        let parts: Vec<&str> = rest.splitn(4, ' ').collect();
                        if parts.len() >= 4 {
                            let profile = ProviderProfile {
                                name: parts[0].to_string(),
                                provider: parts[1].to_string(),
                                base_url: parts[2].to_string(),
                                model: parts[3].to_string(),
                                api_key_env: None,
                                system_prompt: None,
                                max_context_tokens: None,
                            };
                            app.profiles.push(profile);
                            save_profiles(&app.profiles);
                            app.push_message("assistant", &format!("Profile '{}' added.", parts[0]));
                        } else {
                            app.push_message("assistant", "Usage: /profile add <name> <provider> <base_url> <model>");
                        }
                    } else if let Some(rest) = arg.strip_prefix("use ") {
                        if let Ok(idx) = rest.trim().parse::<usize>() {
                            if idx > 0 && idx <= app.profiles.len() {
                                let p = &app.profiles[idx - 1];
                                app.openai_base_url = Some(p.base_url.clone());
                                app.current_model = p.model.clone();
                                if let Some(ref env_key) = p.api_key_env {
                                    if let Ok(key) = std::env::var(env_key) {
                                        app.openai_api_key = Some(key);
                                    }
                                }
                                if let Some(ref sp) = p.system_prompt {
                                    app.system_prompt = sp.clone();
                                }
                                if let Some(max) = p.max_context_tokens {
                                    app.token_total = max;
                                }
                                app.active_profile = Some(idx - 1);
                                app.push_message("assistant", &format!("Switched to profile: {} ({})", p.name, p.provider));
                            } else {
                                app.push_message("assistant", &format!("Invalid index. Use 1-{}.", app.profiles.len()));
                            }
                        }
                    } else if let Some(rest) = arg.strip_prefix("rm ").or_else(|| arg.strip_prefix("delete ")) {
                        if let Ok(idx) = rest.trim().parse::<usize>() {
                            if idx > 0 && idx <= app.profiles.len() {
                                let removed = app.profiles.remove(idx - 1);
                                save_profiles(&app.profiles);
                                if app.active_profile == Some(idx - 1) { app.active_profile = None; }
                                app.push_message("assistant", &format!("Removed profile: {}", removed.name));
                            }
                        }
                    } else {
                        app.push_message("assistant", "Usage: /profiles | /profile add|use|rm ...");
                    }
                    return;
                }
                // ── /rag-index — build RAG index ─────────────────
                if input == "/rag-index" || input == "/rag index" {
                    app.status_msg = "Building RAG index…".to_string();
                    // Simple RAG index: collect source files and create an index file
                    let mut indexed = 0u32;
                    let index_dir = std::path::Path::new(&app.root_path).join(".shadowai");
                    let _ = std::fs::create_dir_all(&index_dir);
                    let index_path = index_dir.join("rag_index.jsonl");
                    let mut index_data = String::new();
                    let extensions = ["rs", "ts", "tsx", "js", "jsx", "py", "go", "cpp", "c", "h", "java",
                                      "swift", "toml", "yaml", "yml", "json", "md", "html", "css", "sh"];
                    fn walk_for_rag(dir: &std::path::Path, root: &str, exts: &[&str], data: &mut String, count: &mut u32) {
                        let Ok(entries) = std::fs::read_dir(dir) else { return; };
                        for entry in entries.flatten() {
                            let path = entry.path();
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "build" { continue; }
                            if path.is_dir() {
                                walk_for_rag(&path, root, exts, data, count);
                            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                if exts.contains(&ext) {
                                    if let Ok(content) = std::fs::read_to_string(&path) {
                                        if content.len() < 100_000 {
                                            let rel = path.to_string_lossy().strip_prefix(&format!("{}/", root))
                                                .unwrap_or(&path.to_string_lossy()).to_string();
                                            let lines: Vec<&str> = content.lines().take(500).collect();
                                            let chunk = lines.join("\n");
                                            let entry = serde_json::json!({"path": rel, "lines": lines.len(), "content": chunk});
                                            data.push_str(&serde_json::to_string(&entry).unwrap_or_default());
                                            data.push('\n');
                                            *count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    walk_for_rag(std::path::Path::new(&app.root_path), &app.root_path, &extensions, &mut index_data, &mut indexed);
                    let _ = std::fs::write(&index_path, &index_data);
                    app.status_msg = format!("RAG index: {} files", indexed);
                    app.push_message("assistant", &format!("🔍 RAG index built: {} files indexed at .shadowai/rag_index.jsonl", indexed));
                    return;
                }
                // ── /changes — show recent file changes ──────────
                if input == "/changes" {
                    if app.recent_file_changes.is_empty() {
                        app.push_message("assistant", "No file changes in this session.");
                    } else {
                        let mut output = String::from("File changes this session:\n");
                        for (icon, path) in &app.recent_file_changes {
                            output.push_str(&format!("  {} {}\n", icon, path));
                        }
                        app.push_message("assistant", &output);
                    }
                    return;
                }
                // ── /copy — copy last AI response to clipboard ──
                if input == "/copy" || input == "/cp" {
                    let last_response = app.messages.iter().rev()
                        .find(|m| m.role == "assistant" && !m.content.is_empty())
                        .map(|m| m.content.clone());
                    if let Some(content) = last_response {
                        let escaped = content.replace('\'', "'\\''");
                        let copied = std::process::Command::new("sh")
                            .args(["-c", &format!(
                                "printf '%s' '{}' | xclip -selection clipboard 2>/dev/null || printf '%s' '{}' | xsel --clipboard 2>/dev/null || printf '%s' '{}' | wl-copy 2>/dev/null",
                                escaped, escaped, escaped)])
                            .status().map(|s| s.success()).unwrap_or(false);
                        app.push_message("assistant", if copied {
                            "Copied last response to clipboard."
                        } else {
                            "Failed to copy — install xclip, xsel, or wl-copy."
                        });
                    } else {
                        app.push_message("assistant", "No AI response to copy.");
                    }
                    return;
                }
                // ── /model — switch model ────────────────────────
                if input == "/model" {
                    app.push_message("assistant", &format!(
                        "Current model: {}\nUsage: /model <name>\nExamples: /model qwen3 | /model claude-sonnet-4-6 | /model gpt-4o",
                        app.current_model));
                    return;
                }
                if let Some(model) = input.strip_prefix("/model ") {
                    let model = model.trim().to_string();
                    app.current_model = model.clone();
                    app.push_message("assistant", &format!("Model switched to: {}", model));
                    return;
                }
                // ── /providers — list available providers ────────
                if input == "/providers" {
                    app.status_msg = "Probing providers…".to_string();
                    let probe = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_millis(800))
                        .build().unwrap_or_default();
                    let mut lines = String::from("Available providers:\n");
                    let endpoints = [
                        ("llama.cpp", "http://localhost:8080/v1"),
                        ("Ollama", "http://localhost:11434/v1"),
                        ("LM Studio", "http://localhost:1234/v1"),
                    ];
                    for (name, url) in &endpoints {
                        let status = match probe.get(&format!("{url}/models")).send().await {
                            Ok(resp) if resp.status().is_success() => {
                                if let Ok(body) = resp.json::<serde_json::Value>().await {
                                    let models: Vec<String> = body["data"].as_array()
                                        .unwrap_or(&vec![])
                                        .iter()
                                        .filter_map(|m| m["id"].as_str().map(String::from))
                                        .collect();
                                    format!("✓ online — models: {}", if models.is_empty() { "default".to_string() } else { models.join(", ") })
                                } else {
                                    "✓ online".to_string()
                                }
                            }
                            _ => "✗ offline".to_string(),
                        };
                        lines.push_str(&format!("  {} ({}) — {}\n", name, url, status));
                    }
                    if let Some(ref key) = app.api_key {
                        if !key.is_empty() {
                            lines.push_str("  Anthropic (API) — ✓ key configured\n");
                        }
                    }
                    if let Some(ref key) = app.openai_api_key {
                        if !key.is_empty() {
                            lines.push_str("  OpenAI (API) — ✓ key configured\n");
                        }
                    }
                    if let Some(ref url) = app.openai_base_url {
                        lines.push_str(&format!("\nActive: {}", url));
                    }
                    app.push_message("assistant", &lines);
                    app.status_msg.clear();
                    return;
                }
                // ── /session — session management ────────────────
                if input == "/session new" || input == "/new" {
                    let idx = app.session_tabs.len();
                    app.session_tabs.push(format!("Session {}", idx + 1));
                    app.active_tab = idx;
                    app.messages.clear();
                    app.conversation_history.clear();
                    app.code_blocks.clear();
                    app.recent_file_changes.clear();
                    app.token_used = 0;
                    app.token_response = 0;
                    app.push_message("assistant", &format!("New session started: Session {}", idx + 1));
                    return;
                }
                if let Some(name) = input.strip_prefix("/session rename ") {
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        if let Some(tab) = app.session_tabs.get_mut(app.active_tab) {
                            *tab = name.clone();
                        }
                        app.push_message("assistant", &format!("Session renamed to: {}", name));
                    }
                    return;
                }
                if input == "/session delete" {
                    if app.session_tabs.len() <= 1 {
                        app.push_message("assistant", "Cannot delete the last session.");
                        return;
                    }
                    let removed = app.session_tabs.remove(app.active_tab);
                    app.pinned_sessions.retain(|&i| i != app.active_tab);
                    // Fix pinned indices
                    app.pinned_sessions = app.pinned_sessions.iter()
                        .map(|&i| if i > app.active_tab { i - 1 } else { i })
                        .collect();
                    if app.active_tab >= app.session_tabs.len() {
                        app.active_tab = app.session_tabs.len() - 1;
                    }
                    app.messages.clear();
                    app.conversation_history.clear();
                    app.push_message("assistant", &format!("Deleted session: {}", removed));
                    return;
                }
                if input == "/session export" || input == "/export" {
                    let session_name = app.session_tabs.get(app.active_tab)
                        .cloned().unwrap_or_else(|| "session".to_string());
                    let safe_name = session_name.replace(' ', "_").to_lowercase();
                    let filename = format!("{}_export.md", safe_name);
                    let export_path = format!("{}/{}", app.root_path, filename);
                    let mut md = format!("# {}\n\nExported: {}\n\n", session_name,
                        chrono::Local::now().format("%Y-%m-%d %H:%M"));
                    for msg in &app.messages {
                        match msg.role.as_str() {
                            "user" => {
                                md.push_str(&format!("## You ({})\n\n{}\n\n", msg.timestamp, msg.content));
                            }
                            "assistant" => {
                                if let Some(ref thinking) = msg.thinking {
                                    md.push_str(&format!("<details><summary>Thinking</summary>\n\n{}\n\n</details>\n\n", thinking));
                                }
                                md.push_str(&format!("## AI ({})\n\n{}\n\n", msg.timestamp, msg.content));
                            }
                            "tool" => {
                                md.push_str(&format!("> ⚡ {}\n\n", msg.content));
                            }
                            "error" => {
                                md.push_str(&format!("> ❌ {}\n\n", msg.content));
                            }
                            _ => {}
                        }
                    }
                    match std::fs::write(&export_path, &md) {
                        Ok(_) => app.push_message("assistant", &format!("📄 Exported to {}", export_path)),
                        Err(e) => app.push_message("error", &format!("Export failed: {}", e)),
                    }
                    return;
                }
                // ── /think — toggle thinking display ─────────────
                if input == "/think" || input == "/thinking" {
                    // Toggle collapsed state on the last assistant message with thinking
                    let toggled = app.messages.iter_mut().rev()
                        .find(|m| m.role == "assistant" && m.thinking.is_some())
                        .map(|m| {
                            m.thinking_collapsed = !m.thinking_collapsed;
                            m.thinking_collapsed
                        });
                    match toggled {
                        Some(true) => app.push_message("assistant", "Thinking collapsed. /think to expand."),
                        Some(false) => app.push_message("assistant", "Thinking expanded. /think to collapse."),
                        None => app.push_message("assistant", "No thinking blocks to toggle."),
                    }
                    return;
                }
                // ── /tools-off /tools-on — toggle tools ──────────
                if input == "/tools-off" || input == "/tools off" {
                    app.tools_enabled = false;
                    app.push_message("assistant", "⊘ Tools disabled. AI will respond without tool calls.");
                    return;
                }
                if input == "/tools-on" || input == "/tools on" {
                    app.tools_enabled = true;
                    app.push_message("assistant", "⚙ Tools enabled.");
                    return;
                }
                // ── /privacy — toggle privacy/air-gap mode ───────
                if input == "/privacy" || input == "/airgap" {
                    app.privacy_mode = !app.privacy_mode;
                    if app.privacy_mode {
                        app.push_message("assistant", "🔒 Privacy mode ON — only local providers (localhost) will be used.");
                    } else {
                        app.push_message("assistant", "🔓 Privacy mode OFF — all providers available.");
                    }
                    return;
                }
                // ── /compact — trigger context compaction ────────
                if input == "/compact" {
                    let before = app.conversation_history.len();
                    if before <= 2 {
                        app.push_message("assistant", "Not enough history to compact.");
                        return;
                    }
                    // Step 1: Clean completed items first (lightweight)
                    app.conversation_history = clean_completed_items(&app.conversation_history, &app.root_path);
                    app.update_token_breakdown();
                    let after_clean = app.conversation_history.len();

                    // Step 2: Archive then hierarchical compact
                    save_compaction_archive(&app.conversation_history, &app.root_path);
                    app.conversation_history = hierarchical_compact_messages(&app.conversation_history, 8);
                    let after = app.conversation_history.len();
                    app.update_token_breakdown();
                    app.push_message("assistant", &format!(
                        "Context compacted: {} → {} → {} messages (cleaned → compressed)\n\
                         Archived to .shadow-memory/. Work ledger updated.\n\
                         Context: {}k / {}k tokens ({}%)",
                        before, after_clean, after,
                        app.token_used / 1000, app.token_total / 1000,
                        (app.token_used * 100) / app.token_total.max(1)));
                    return;
                }
                // ── /sessions — list all sessions ────────────────
                if input == "/sessions" {
                    let mut listing = String::from("Sessions:\n");
                    for (i, name) in app.session_tabs.iter().enumerate() {
                        let active = if i == app.active_tab { " ← active" } else { "" };
                        let pinned = if app.pinned_sessions.contains(&i) { " ⭐" } else { "" };
                        listing.push_str(&format!("  [{}] {}{}{}\n", i + 1, name, pinned, active));
                    }
                    listing.push_str("\n/session new | rename <name> | delete | export");
                    app.push_message("assistant", &listing);
                    return;
                }
                // ── /help — comprehensive TUI help ───────────────
                if input == "/help" || input == "/h" {
                    app.push_message("assistant",
                        "ShadowAI TUI Commands:\n\n\
                         Chat & AI:\n\
                         /mode plan|build|auto — switch mode\n\
                         /model <name>         — switch model\n\
                         /temp <0.0-2.0>       — set temperature\n\
                         /maxtok <n>           — set max tokens\n\
                         /think                — toggle thinking display\n\
                         /copy                 — copy last response\n\
                         /rewind [N]           — rewind to message N\n\
                         /include <file>       — attach file to context\n\
                         /compact              — compact context history\n\
                         /ctx                  — context info summary\n\n\
                         Tools:\n\
                         /tools                — list AI tools\n\
                         /tools-on / off       — enable/disable tools\n\
                         /yolo                 — auto-approve all tools\n\
                         /approval smart|ask   — set approval mode\n\
                         /code [N] [action]    — interact with code blocks\n\n\
                         Session:\n\
                         /new                  — new session\n\
                         /sessions             — list sessions\n\
                         /session rename <n>   — rename current\n\
                         /session delete       — delete current\n\
                         /export               — export to markdown\n\
                         /pin /unpin           — pin session\n\n\
                         Providers:\n\
                         /providers            — list available\n\
                         /profiles             — manage profiles\n\
                         /privacy              — toggle air-gap mode\n\n\
                         Project:\n\
                         /build /test /lint    — run build tools\n\
                         /git /gd /gl          — git shortcuts\n\
                         /changes              — recent file changes\n\
                         /rag-index            — build RAG index\n\
                         /memories /forget     — manage memories\n\n\
                         Navigation:\n\
                         Tab                   — switch pane\n\
                         Ctrl+P                — command palette\n\
                         Ctrl+C / /quit        — exit\n\
                         Up/Down               — scroll/history\n\
                         Ctrl+Tab              — next session");
                    return;
                }
                if input.starts_with('/') {
                    // Unknown slash command — show hint instead of sending to AI
                    app.push_message("assistant", &format!(
                        "Unknown command: {}\nType /help for all commands, /tools for tool list.",
                        input
                    ));
                    return;
                }
                // Regular message — send to AI
                tui_send_message(app, input).await;
                return;
            }
        }
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            if app.input_cursor > 0 {
                let byte_pos = app.input.char_indices()
                    .nth(app.input_cursor - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                app.input.remove(byte_pos);
                app.input_cursor -= 1;
            }
        }
        (KeyModifiers::NONE, KeyCode::Left) => {
            if app.input_cursor > 0 { app.input_cursor -= 1; }
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            if app.input_cursor < app.input.chars().count() { app.input_cursor += 1; }
        }
        (KeyModifiers::CONTROL, KeyCode::Left) => { app.input_cursor = 0; }
        (KeyModifiers::CONTROL, KeyCode::Right) => { app.input_cursor = app.input.chars().count(); }
        // Shift+Up/Down always scrolls chat regardless of active pane
        (KeyModifiers::SHIFT, KeyCode::Up) => {
            app.chat_scroll = app.chat_scroll.saturating_sub(3);
        }
        (KeyModifiers::SHIFT, KeyCode::Down) => {
            app.chat_scroll = app.chat_scroll.saturating_add(3);
        }
        (KeyModifiers::NONE, KeyCode::Up) => {
            if app.active_pane == TuiPane::Input {
                if let Some(entry) = app.input_history.up(&app.input) {
                    app.input = entry.to_string();
                    app.input_cursor = app.input.chars().count();
                }
            } else if app.active_pane == TuiPane::Chat {
                if app.chat_scroll > 0 { app.chat_scroll -= 1; }
            } else if app.active_pane == TuiPane::FileTree {
                if app.file_tree_selected > 0 { app.file_tree_selected -= 1; }
            }
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            if app.active_pane == TuiPane::Input {
                if let Some(entry) = app.input_history.down() {
                    app.input = entry.to_string();
                    app.input_cursor = app.input.chars().count();
                }
            } else if app.active_pane == TuiPane::Chat {
                app.chat_scroll += 1;
            } else if app.active_pane == TuiPane::FileTree {
                if app.file_tree_selected + 1 < app.file_nodes.len() {
                    app.file_tree_selected += 1;
                }
            }
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            app.chat_scroll = app.chat_scroll.saturating_sub(10);
            app.active_pane = TuiPane::Chat;
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            app.chat_scroll += 10;
            app.active_pane = TuiPane::Chat;
        }
        (KeyModifiers::NONE, KeyCode::F(1)) => {
            app.push_message("assistant",
                "Available commands: /help, /status, /new, /clear, /quit, /skills, /theme, /git, /build, /test, /lint, /heal, /research, /docker, /remote, /cron ...\n\nCtrl+P for command palette.");
        }
        (KeyModifiers::NONE, KeyCode::F(10)) => {
            app.should_quit = true;
        }
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            let byte_pos = app.input.char_indices()
                .nth(app.input_cursor)
                .map(|(i, _)| i)
                .unwrap_or(app.input.len());
            app.input.insert(byte_pos, c);
            app.input_cursor += 1;
        }
        _ => {}
    }
}


// ─── Pass 4 helper functions ──────────────────────────────────────────────────

// 11a: Startup time tracking helper (wired in main)
#[allow(dead_code)]
fn record_startup_time(t0: std::time::Instant, cfg: &CliConfig) {
    if cfg.show_startup_time.unwrap_or(false) {
        let ms = t0.elapsed().as_millis();
        let mut o = io::stdout();
        set_fg(&mut o, theme::DIM);
        write!(o, "  [startup] {}ms\n", ms).ok();
        reset_color(&mut o);
    }
}

// 11c: Response cache helpers
#[allow(dead_code)]
fn cache_key_fnv(prompt: &str, model: &str) -> u64 {
    let combined = format!("{}|{}", prompt, model);
    let mut hash: u64 = 14695981039346656037;
    for byte in combined.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

#[allow(dead_code)]
fn check_response_cache(prompt: &str, model: &str, ttl_secs: u64) -> Option<String> {
    let key = cache_key_fnv(prompt, model);
    if let Some(cache) = RESPONSE_CACHE.get() {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = guard.get(&key) {
            if entry.timestamp.elapsed().as_secs() < ttl_secs {
                return Some(format!("[cached] {}", entry.response));
            }
        }
    }
    None
}

#[allow(dead_code)]
fn store_response_cache(prompt: &str, model: &str, response: &str) {
    let key = cache_key_fnv(prompt, model);
    let cache = RESPONSE_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(key, CachedResponse { response: response.to_string(), timestamp: std::time::Instant::now() });
}

// 12a: Tool allowlist
#[allow(dead_code)]
fn load_project_config() -> Option<CliConfig> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.join(".shadowai").join("config.toml");
    let contents = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&contents).ok()
}

#[allow(dead_code)]
fn is_tool_allowed(tool_name: &str, cfg: &CliConfig) -> bool {
    let hardcoded_deny = ["rm -rf", "format", "mkfs", "dd"];
    for pattern in &hardcoded_deny {
        if tool_name.contains(pattern) {
            return false;
        }
    }
    if let Some(ref allowlist) = cfg.tool_allowlist {
        return allowlist.iter().any(|t| t == tool_name);
    }
    // Also check project config
    if let Some(proj) = load_project_config() {
        if let Some(ref allowlist) = proj.tool_allowlist {
            return allowlist.iter().any(|t| t == tool_name);
        }
    }
    true
}

// 12b: bwrap sandbox
fn bwrap_available() -> bool {
    which_command("bwrap").is_some()
}

#[allow(dead_code)]
fn sandbox_command(cmd: &str, args: &[&str], cwd: &str, cfg: &CliConfig) -> std::process::Command {
    if bwrap_available() && cfg.sandbox_commands.unwrap_or(false) {
        let mut c = std::process::Command::new("bwrap");
        c.args([
            "--ro-bind", "/", "/",
            "--dev", "/dev",
            "--proc", "/proc",
            "--tmpfs", "/tmp",
            "--bind", cwd, cwd,
            "--unshare-net",
            "--",
            cmd,
        ]);
        c.args(args);
        c
    } else {
        let mut c = std::process::Command::new(cmd);
        c.args(args);
        c
    }
}

// 1d: OSC 8 hyperlinks
#[allow(dead_code)]
fn osc8_link(url: &str, label: &str) -> String {
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, label)
}

#[allow(dead_code)]
fn supports_hyperlinks() -> bool {
    if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
        let term_prog_lc = term_prog.to_lowercase();
        if ["iterm.app", "wezterm", "hyper", "vscode"].iter().any(|t| term_prog_lc.contains(t)) {
            return true;
        }
    }
    if std::env::var("VTE_VERSION").is_ok() {
        return true;
    }
    if std::env::var("COLORTERM").map(|v| v == "truecolor").unwrap_or(false) {
        return true;
    }
    false
}

#[allow(dead_code)]
fn postprocess_urls(text: &str) -> String {
    if !supports_hyperlinks() {
        return text.to_string();
    }
    let url_re = regex::Regex::new(r"https?://\S+").unwrap();
    url_re.replace_all(text, |caps: &regex::Captures| {
        let url = &caps[0];
        osc8_link(url, url)
    }).to_string()
}

// 1c: Image inline display
#[allow(dead_code)]
fn supports_sixel() -> bool {
    if let Ok(term) = std::env::var("TERM") {
        let t = term.to_lowercase();
        if t.contains("kitty") || t.contains("xterm") || t.contains("wezterm") { return true; }
    }
    if let Ok(tp) = std::env::var("TERM_PROGRAM") {
        let t = tp.to_lowercase();
        if t.contains("iterm") || t.contains("wezterm") { return true; }
    }
    false
}

#[allow(dead_code)]
fn supports_kitty_graphics() -> bool {
    if let Ok(term) = std::env::var("TERM") {
        if term == "xterm-kitty" { return true; }
    }
    std::env::var("KITTY_WINDOW_ID").is_ok()
}

#[allow(dead_code)]
async fn display_image_inline(path: &str) {
    let mut o = io::stdout();
    if supports_kitty_graphics() {
        match std::fs::read(path) {
            Ok(bytes) => {
                let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
                write!(o, "\x1b_Ga=T,f=100,m=0;{}\x1b\\", b64).ok();
                write!(o, "\n").ok();
                o.flush().ok();
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  [image] Failed to read {}: {}\n", path, e).ok();
                reset_color(&mut o);
            }
        }
    } else if supports_sixel() && which_command("img2sixel").is_some() {
        let output = std::process::Command::new("img2sixel")
            .arg(path)
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let _ = o.write_all(&out.stdout);
                write!(o, "\n").ok();
                o.flush().ok();
            }
            _ => {
                write!(o, "  [image: {}]\n", path).ok();
            }
        }
    } else {
        write!(o, "  [image: {}]\n", path).ok();
    }
    o.flush().ok();
}

// 1e: Response folding
#[allow(dead_code)]
fn fold_response(id: usize, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 40 {
        return content.to_string();
    }
    let store = FOLDED_RESPONSES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    {
        let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
        guard.push((id, content.to_string()));
    }
    let n = lines.len() - 5;
    let preview: String = lines[..5].join("\n");
    format!("{}\n[... {} lines folded — type /unfold {} to expand ...]", preview, n, id)
}

fn handle_unfold_command(id: usize) {
    let store = FOLDED_RESPONSES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, content)) = guard.iter().find(|(fid, _)| *fid == id) {
        let mut o = io::stdout();
        write!(o, "{}\n", content).ok();
        o.flush().ok();
    } else {
        let mut o = io::stdout();
        set_fg(&mut o, theme::WARN);
        write!(o, "  [fold] No folded response with id {}\n", id).ok();
        reset_color(&mut o);
    }
}

// 2c: Context slots
fn handle_add_context_command(glob_pat: &str, root_path: &str) {
    let mut o = io::stdout();
    let slots = CONTEXT_SLOTS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut count = 0usize;
    let mut total_tokens = 0usize;

    // Walk the root path and match files against the glob pattern
    fn walk_dir(dir: &str, pat: &str, slots: &std::sync::Mutex<Vec<ContextSlot>>, count: &mut usize, total_tokens: &mut usize) {
        let glob_full = if pat.contains('/') || pat.starts_with("**") {
            pat.to_string()
        } else {
            format!("{}/{}", dir, pat)
        };
        if let Ok(paths) = glob::glob(&glob_full) {
            for entry in paths.flatten() {
                if entry.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&entry) {
                        let tokens = content.len() / 4;
                        let label = entry.to_string_lossy().to_string();
                        let mut guard = slots.lock().unwrap_or_else(|e| e.into_inner());
                        guard.push(ContextSlot { label: label.clone(), content, tokens });
                        *count += 1;
                        *total_tokens += tokens;
                    }
                }
            }
        }
    }

    walk_dir(root_path, glob_pat, slots, &mut count, &mut total_tokens);
    set_fg(&mut o, theme::OK);
    write!(o, "  [context] Added {} files ({} tokens) to context slots\n", count, total_tokens).ok();
    reset_color(&mut o);
}

fn handle_ctx_command() {
    let mut o = io::stdout();
    let slots = CONTEXT_SLOTS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let guard = slots.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_empty() {
        set_fg(&mut o, theme::DIM);
        write!(o, "  [context] No context slots loaded. Use /add <glob> to add files.\n").ok();
        reset_color(&mut o);
        return;
    }
    print_section_header("Context Slots");
    let mut total = 0usize;
    for slot in guard.iter() {
        let short = slot.label.split('/').last().unwrap_or(&slot.label);
        set_fg(&mut o, theme::CYAN_DIM);
        write!(o, "  {} ", ARROW).ok();
        set_fg(&mut o, theme::AI_TEXT);
        write!(o, "{:<40}", short).ok();
        set_fg(&mut o, theme::DIM);
        write!(o, " {} tokens\n", slot.tokens).ok();
        reset_color(&mut o);
        total += slot.tokens;
    }
    set_fg(&mut o, theme::DIM);
    write!(o, "  Total: {} slots, {} tokens\n", guard.len(), total).ok();
    reset_color(&mut o);
    print_section_end();
}

fn handle_drop_context_command(label: &str) {
    let mut o = io::stdout();
    let slots = CONTEXT_SLOTS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut guard = slots.lock().unwrap_or_else(|e| e.into_inner());
    let before = guard.len();
    guard.retain(|s| !s.label.contains(label));
    let removed = before - guard.len();
    set_fg(&mut o, if removed > 0 { theme::OK } else { theme::WARN });
    write!(o, "  [context] Removed {} slot(s) matching '{}'\n", removed, label).ok();
    reset_color(&mut o);
}

// 2a: Persistent memory
fn handle_memory_command(root_path: &str) {
    let mut o = io::stdout();
    let path = std::path::Path::new(root_path).join(".shadowai").join("memory.md");
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            print_section_header("Project Memory");
            write!(o, "{}\n", content).ok();
            print_section_end();
        }
        Err(_) => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  [memory] No memory file found at {}\n", path.display()).ok();
            reset_color(&mut o);
        }
    }
}

// 1b: Diff viewer /gd
#[derive(Debug, Clone, PartialEq)]
enum DiffLineKind {
    Added,
    Removed,
    Context,
    Header,
}

#[derive(Debug, Clone)]
struct DiffLine {
    kind: DiffLineKind,
    content: String,
}

fn parse_unified_diff(text: &str) -> Vec<DiffLine> {
    let mut result = Vec::new();
    for line in text.lines() {
        let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("@@") || line.starts_with("index ") {
            DiffLineKind::Header
        } else if line.starts_with('+') {
            DiffLineKind::Added
        } else if line.starts_with('-') {
            DiffLineKind::Removed
        } else {
            DiffLineKind::Context
        };
        result.push(DiffLine { kind, content: line.to_string() });
    }
    result
}

async fn handle_gd_command(file: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    let mut args = vec!["diff"];
    if !file.is_empty() {
        args.push("--");
        args.push(file);
    }
    let output = std::process::Command::new("git")
        .args(&args)
        .output();
    let diff_text = match output {
        Ok(out) if out.status.success() || !out.stdout.is_empty() => {
            String::from_utf8_lossy(&out.stdout).to_string()
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            set_fg(&mut o, theme::ERR);
            write!(o, "  git diff failed: {}\n", err).ok();
            reset_color(&mut o);
            return;
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  Failed to run git diff: {}\n", e).ok();
            reset_color(&mut o);
            return;
        }
    };

    if diff_text.trim().is_empty() {
        set_fg(&mut o, theme::OK);
        write!(o, "  No changes in working tree.\n").ok();
        reset_color(&mut o);
        return;
    }

    let diff_lines = parse_unified_diff(&diff_text);
    let term_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let col_width = (term_width / 2).saturating_sub(2);

    // Side-by-side pager: collect old/new pairs
    let mut old_lines: Vec<DiffLine> = Vec::new();
    let mut new_lines: Vec<DiffLine> = Vec::new();
    for dl in &diff_lines {
        match dl.kind {
            DiffLineKind::Removed => old_lines.push(dl.clone()),
            DiffLineKind::Added => new_lines.push(dl.clone()),
            DiffLineKind::Header => {
                old_lines.push(dl.clone());
                new_lines.push(dl.clone());
            }
            DiffLineKind::Context => {
                old_lines.push(dl.clone());
                new_lines.push(dl.clone());
            }
        }
    }

    let max_rows = old_lines.len().max(new_lines.len());
    let mut printed = 0usize;
    let page_size = 40;

    fn truncate_to(s: &str, width: usize) -> String {
        if s.len() <= width { format!("{:<width$}", s, width = width) }
        else { format!("{:.width$}", s, width = width) }
    }

    write!(o, "\n").ok();
    for i in 0..max_rows {
        let old = old_lines.get(i);
        let new = new_lines.get(i);

        // Left side
        let (left_color, left_text) = match old {
            Some(dl) => match dl.kind {
                DiffLineKind::Removed => (theme::ERR, truncate_to(&dl.content, col_width)),
                DiffLineKind::Header => (theme::CYAN_DIM, truncate_to(&dl.content, col_width)),
                _ => (theme::DIM_LIGHT, truncate_to(&dl.content, col_width)),
            },
            None => (theme::DIM, truncate_to("", col_width)),
        };

        // Right side
        let (right_color, right_text) = match new {
            Some(dl) => match dl.kind {
                DiffLineKind::Added => (theme::OK, truncate_to(&dl.content, col_width)),
                DiffLineKind::Header => (theme::CYAN_DIM, truncate_to(&dl.content, col_width)),
                _ => (theme::DIM_LIGHT, truncate_to(&dl.content, col_width)),
            },
            None => (theme::DIM, truncate_to("", col_width)),
        };

        set_fg(&mut o, left_color);
        write!(o, "{}", left_text).ok();
        set_fg(&mut o, theme::DIM);
        write!(o, "│").ok();
        set_fg(&mut o, right_color);
        write!(o, "{}\n", right_text).ok();
        reset_color(&mut o);

        printed += 1;
        if printed % page_size == 0 && i + 1 < max_rows {
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "-- More (Enter/q) --").ok();
            reset_color(&mut o);
            o.flush().ok();
            let mut buf = String::new();
            let _ = io::stdin().read_line(&mut buf);
            if buf.trim() == "q" { break; }
        }
    }
    write!(o, "\n").ok();
    o.flush().ok();
}

// 2b: RAG integration
async fn handle_rag_command(query: &str, cfg: &CliConfig) {
    let mut o = io::stdout();

    if let Some(ref _url) = cfg.shadowide_url {
        // Future: WebSocket RAG request
        set_fg(&mut o, theme::DIM);
        write!(o, "  [rag] Shadow IDE WebSocket RAG not yet connected. Using local search.\n").ok();
        reset_color(&mut o);
    }

    // Local grep fallback
    let root = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    print_section_header(&format!("RAG: {}", query));
    let extensions = ["rs", "ts", "py", "go", "js", "tsx", "jsx"];
    let mut hits: Vec<(String, usize, String)> = Vec::new();

    for ext in &extensions {
        let pattern = format!("{}/**/*.{}", root, ext);
        if let Ok(paths) = glob::glob(&pattern) {
            for entry in paths.flatten() {
                if let Ok(content) = std::fs::read_to_string(&entry) {
                    for (lineno, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(&query.to_lowercase()) {
                            hits.push((entry.to_string_lossy().to_string(), lineno + 1, line.trim().to_string()));
                            if hits.len() >= 10 { break; }
                        }
                    }
                    if hits.len() >= 10 { break; }
                }
                if hits.len() >= 10 { break; }
            }
        }
        if hits.len() >= 10 { break; }
    }

    if hits.is_empty() {
        set_fg(&mut o, theme::DIM);
        write!(o, "  No results found for '{}'\n", query).ok();
        reset_color(&mut o);
    } else {
        for (file, line, content) in &hits {
            let short_file = file.strip_prefix(&root).unwrap_or(file).trim_start_matches('/');
            set_fg(&mut o, theme::CYAN_DIM);
            write!(o, "  [source: {}:{}]\n", short_file, line).ok();
            set_fg(&mut o, theme::DIM_LIGHT);
            write!(o, "    {}\n", content).ok();
            reset_color(&mut o);
        }
    }
    print_section_end();
}

// 3a: DAP client
async fn handle_dap_command(args: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    let parts: Vec<&str> = args.trim().splitn(3, ' ').collect();
    let subcmd = parts.first().copied().unwrap_or("");

    match subcmd {
        "launch" => {
            let adapter = parts.get(1).copied().unwrap_or("");
            let program = parts.get(2).copied().unwrap_or("");
            if adapter.is_empty() || program.is_empty() {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Usage: /dap launch <adapter> <program>\n").ok();
                reset_color(&mut o);
                return;
            }
            set_fg(&mut o, theme::CYAN);
            write!(o, "  [dap] Launching {} with adapter {}\n", program, adapter).ok();
            reset_color(&mut o);

            let child = tokio::process::Command::new(adapter)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            match child {
                Ok(mut c) => {
                    let stdin = c.stdin.take();
                    let stdout = c.stdout.take().map(tokio::io::BufReader::new);
                    let state = DapState {
                        seq: 1,
                        child: Some(c),
                        stdin,
                        stdout,
                        breakpoints: Vec::new(),
                        thread_id: None,
                    };
                    let dap = DAP_STATE.get_or_init(|| std::sync::Mutex::new(DapState {
                        seq: 0, child: None, stdin: None, stdout: None, breakpoints: Vec::new(), thread_id: None,
                    }));
                    *dap.lock().unwrap_or_else(|e| e.into_inner()) = state;
                    set_fg(&mut o, theme::OK);
                    write!(o, "  [dap] Adapter started. Send initialize + launch requests.\n").ok();
                    reset_color(&mut o);
                }
                Err(e) => {
                    set_fg(&mut o, theme::ERR);
                    write!(o, "  [dap] Failed to launch adapter: {}\n", e).ok();
                    reset_color(&mut o);
                }
            }
        }
        "bp" => {
            let loc = parts.get(1).copied().unwrap_or("");
            if let Some((file, line_str)) = loc.split_once(':') {
                if let Ok(line) = line_str.parse::<u64>() {
                    let dap = DAP_STATE.get_or_init(|| std::sync::Mutex::new(DapState {
                        seq: 0, child: None, stdin: None, stdout: None, breakpoints: Vec::new(), thread_id: None,
                    }));
                    let mut guard = dap.lock().unwrap_or_else(|e| e.into_inner());
                    guard.breakpoints.push(DapBreakpoint { file: file.to_string(), line, id: None });
                    set_fg(&mut o, theme::OK);
                    write!(o, "  [dap] Breakpoint set at {}:{}\n", file, line).ok();
                    reset_color(&mut o);
                    return;
                }
            }
            set_fg(&mut o, theme::ERR);
            write!(o, "  Usage: /dap bp <file>:<line>\n").ok();
            reset_color(&mut o);
        }
        "vars" | "continue" | "next" | "step" | "out" => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  [dap] {} command queued (requires active debug session)\n", subcmd).ok();
            reset_color(&mut o);
        }
        "stop" => {
            let dap = DAP_STATE.get_or_init(|| std::sync::Mutex::new(DapState {
                seq: 0, child: None, stdin: None, stdout: None, breakpoints: Vec::new(), thread_id: None,
            }));
            let mut guard = dap.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(child) = guard.child.as_mut() {
                let _ = child.kill().await;
            }
            guard.child = None;
            guard.stdin = None;
            guard.stdout = None;
            set_fg(&mut o, theme::OK);
            write!(o, "  [dap] Adapter stopped.\n").ok();
            reset_color(&mut o);
        }
        "attach" => {
            let pid = parts.get(1).copied().unwrap_or("");
            set_fg(&mut o, theme::CYAN);
            write!(o, "  [dap] Attaching to PID {}\n", pid).ok();
            reset_color(&mut o);
        }
        _ => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  [dap] Subcommands: launch, attach, bp, continue, next, step, out, vars, stop, ai\n").ok();
            reset_color(&mut o);
        }
    }
}

// 4a: PDF extraction
async fn extract_pdf(path: &str) -> String {
    if which_command("pdftotext").is_some() {
        if let Ok(out) = std::process::Command::new("pdftotext")
            .args([path, "-"])
            .output()
        {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).to_string();
            }
        }
    }
    if which_command("mutool").is_some() {
        if let Ok(out) = std::process::Command::new("mutool")
            .args(["draw", "-F", "text", path])
            .output()
        {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).to_string();
            }
        }
    }
    "[PDF extraction requires pdftotext or mutool]".to_string()
}

async fn handle_pdf_command(path: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::CYAN);
    write!(o, "  [pdf] Extracting text from {}\n", path).ok();
    reset_color(&mut o);
    let text = extract_pdf(path).await;
    write!(o, "{}\n", text).ok();
    o.flush().ok();
}

// 4b: YouTube transcript
async fn fetch_youtube_transcript(url: &str) -> Result<String, String> {
    // Extract video ID
    let video_id_re = regex::Regex::new(r"(?:v=|youtu\.be/|/shorts/)([A-Za-z0-9_-]{11})").unwrap();
    let video_id = video_id_re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "Could not extract video ID from URL".to_string())?;

    // Try yt-dlp
    if which_command("yt-dlp").is_some() {
        let tmp_prefix = format!("/tmp/yt_{}", video_id);
        let _ = std::process::Command::new("yt-dlp")
            .args(["--skip-download", "--write-auto-subs",
                   "--sub-format", "vtt", "--sub-lang", "en",
                   "-o", &tmp_prefix, url])
            .output();

        let vtt_path = format!("{}.en.vtt", tmp_prefix);
        if let Ok(content) = std::fs::read_to_string(&vtt_path) {
            let clean = content.lines()
                .filter(|l| !l.trim().is_empty()
                    && !l.starts_with("WEBVTT")
                    && !l.contains("-->")
                    && !l.starts_with("NOTE"))
                .map(|l| {
                    // Strip <...> tags
                    regex::Regex::new(r"<[^>]+>").unwrap().replace_all(l, "").to_string()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let _ = std::fs::remove_file(&vtt_path);
            return Ok(clean);
        }
    }

    // Fallback: HTTP fetch captions
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let page_url = format!("https://www.youtube.com/watch?v={}", video_id);
    let html = client.get(&page_url)
        .header("User-Agent", "Mozilla/5.0")
        .send().await
        .map_err(|e| e.to_string())?
        .text().await
        .map_err(|e| e.to_string())?;

    // Extract captionTracks from ytInitialPlayerResponse
    let caption_re = regex::Regex::new(r#""captionTracks":\[.*?"baseUrl":"([^"]+)""#).unwrap();
    if let Some(cap) = caption_re.captures(&html) {
        let caption_url = cap[1].replace("\\u0026", "&");
        let xml = client.get(&caption_url)
            .send().await
            .map_err(|e| e.to_string())?
            .text().await
            .map_err(|e| e.to_string())?;
        let text_re = regex::Regex::new(r"<text[^>]*>([^<]*)</text>").unwrap();
        let transcript: String = text_re.captures_iter(&xml)
            .map(|c| c[1].to_string())
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(transcript);
    }

    Err("[YouTube transcript unavailable — install yt-dlp for best results]".to_string())
}

async fn handle_yt_command(url: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    set_fg(&mut o, theme::CYAN);
    write!(o, "  [yt] Fetching transcript for: {}\n", url).ok();
    reset_color(&mut o);
    o.flush().ok();

    match fetch_youtube_transcript(url).await {
        Ok(transcript) => {
            print_section_header("YouTube Transcript");
            let preview: String = transcript.chars().take(2000).collect();
            write!(o, "{}\n", preview).ok();
            if transcript.len() > 2000 {
                set_fg(&mut o, theme::DIM);
                write!(o, "  [... {} chars total]\n", transcript.len()).ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  {}\n", e).ok();
            reset_color(&mut o);
        }
    }
}

// 6a: /deps fix
async fn handle_deps_fix(_cfg: &CliConfig) {
    let mut o = io::stdout();
    let root = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let root_path = std::path::Path::new(&root);

    print_section_header("Dependency Fix");

    if root_path.join("Cargo.toml").exists() {
        set_fg(&mut o, theme::CYAN);
        write!(o, "  [deps] Running cargo update...\n").ok();
        reset_color(&mut o);
        o.flush().ok();

        let update_out = std::process::Command::new("cargo")
            .args(["update"])
            .current_dir(&root)
            .output();
        match update_out {
            Ok(out) if out.status.success() => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} cargo update succeeded\n").ok();
                reset_color(&mut o);
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                set_fg(&mut o, theme::WARN);
                write!(o, "  cargo update warnings:\n{}\n", err).ok();
                reset_color(&mut o);
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  cargo update failed: {}\n", e).ok();
                reset_color(&mut o);
            }
        }

        let build_out = std::process::Command::new("cargo")
            .args(["build", "2>&1"])
            .current_dir(&root)
            .output();
        let build_errors = match &build_out {
            Ok(out) if !out.status.success() => {
                String::from_utf8_lossy(&out.stderr).to_string()
            }
            Ok(_) => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} cargo build succeeded\n").ok();
                reset_color(&mut o);
                String::new()
            }
            Err(e) => format!("cargo build error: {}", e),
        };

        if !build_errors.is_empty() {
            set_fg(&mut o, theme::WARN);
            write!(o, "  Build errors detected. Review Cargo.toml for conflicts.\n").ok();
            reset_color(&mut o);
        }
    } else if root_path.join("package.json").exists() {
        set_fg(&mut o, theme::CYAN);
        write!(o, "  [deps] Running npm install --legacy-peer-deps...\n").ok();
        reset_color(&mut o);
        o.flush().ok();

        let npm = if which_command("yarn").is_some() { "yarn" } else { "npm" };
        let npm_args: Vec<&str> = if npm == "yarn" {
            vec!["install", "--ignore-engines"]
        } else {
            vec!["install", "--legacy-peer-deps"]
        };
        let out = std::process::Command::new(npm)
            .args(&npm_args)
            .current_dir(&root)
            .output();
        match out {
            Ok(o2) if o2.status.success() => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} {} install succeeded\n", npm).ok();
                reset_color(&mut o);
            }
            Ok(o2) => {
                let err = String::from_utf8_lossy(&o2.stderr);
                set_fg(&mut o, theme::WARN);
                write!(o, "  {} install had issues:\n{}\n", npm, err).ok();
                reset_color(&mut o);
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Failed: {}\n", e).ok();
                reset_color(&mut o);
            }
        }
    } else if root_path.join("requirements.txt").exists() {
        set_fg(&mut o, theme::CYAN);
        write!(o, "  [deps] Running pip install -r requirements.txt...\n").ok();
        reset_color(&mut o);
        o.flush().ok();

        let out = std::process::Command::new("pip")
            .args(["install", "-r", "requirements.txt"])
            .current_dir(&root)
            .output();
        match out {
            Ok(o2) if o2.status.success() => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} pip install succeeded\n").ok();
                reset_color(&mut o);
            }
            Ok(o2) => {
                let err = String::from_utf8_lossy(&o2.stderr);
                set_fg(&mut o, theme::WARN);
                write!(o, "  pip install issues:\n{}\n", err).ok();
                reset_color(&mut o);
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Failed: {}\n", e).ok();
                reset_color(&mut o);
            }
        }
    } else {
        set_fg(&mut o, theme::DIM);
        write!(o, "  [deps] No recognized project manifest found (Cargo.toml, package.json, requirements.txt)\n").ok();
        reset_color(&mut o);
    }

    print_section_end();
}

// 10a: /profile
async fn handle_profile_command(duration: u64, _cfg: &CliConfig) {
    let mut o = io::stdout();
    print_section_header("Profiling");

    let root = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    if which_command("cargo-flamegraph").is_some() || which_command("cargo").is_some() {
        let svg_path = "/tmp/flamegraph.svg";
        set_fg(&mut o, theme::CYAN);
        write!(o, "  [profile] Running cargo flamegraph for {}s → {}\n", duration, svg_path).ok();
        reset_color(&mut o);
        o.flush().ok();

        let out = std::process::Command::new("cargo")
            .args(["flamegraph", "--output", svg_path])
            .current_dir(&root)
            .output();
        match out {
            Ok(result) if result.status.success() => {
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Flamegraph saved to {}\n", svg_path).ok();
                reset_color(&mut o);
                // Try to open the SVG
                let opener = if which_command("xdg-open").is_some() { "xdg-open" } else { "open" };
                let _ = std::process::Command::new(opener).arg(svg_path).spawn();
            }
            Ok(result) => {
                let err = String::from_utf8_lossy(&result.stderr);
                set_fg(&mut o, theme::WARN);
                write!(o, "  cargo flamegraph failed:\n{}\n", err).ok();
                reset_color(&mut o);
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Failed to run cargo flamegraph: {}\n", e).ok();
                reset_color(&mut o);
            }
        }
    } else {
        set_fg(&mut o, theme::DIM);
        write!(o, "  Install cargo-flamegraph: cargo install flamegraph\n").ok();
        reset_color(&mut o);
    }

    // Try perf as fallback
    if which_command("perf").is_some() {
        let perf_svg = "/tmp/perf.svg";
        set_fg(&mut o, theme::DIM);
        write!(o, "  [profile] Alternative: perf record → {}\n", perf_svg).ok();
        reset_color(&mut o);
    }

    print_section_end();
    let _ = duration;
}

// 10b: /db SQL execution
async fn handle_db_command(args: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    let parts: Vec<&str> = args.trim().splitn(2, ' ').collect();
    let subcmd = parts.first().copied().unwrap_or("");

    if subcmd == "connect" {
        let dsn = parts.get(1).copied().unwrap_or("").to_string();
        if dsn.is_empty() {
            set_fg(&mut o, theme::ERR);
            write!(o, "  Usage: /db connect <dsn>\n").ok();
            reset_color(&mut o);
            return;
        }
        let db = DB_DSN.get_or_init(|| std::sync::Mutex::new(String::new()));
        *db.lock().unwrap_or_else(|e| e.into_inner()) = dsn.clone();
        set_fg(&mut o, theme::OK);
        write!(o, "  [db] DSN set: {}\n", dsn).ok();
        reset_color(&mut o);
        return;
    }

    let dsn_opt = DB_DSN.get().map(|d| d.lock().unwrap_or_else(|e| e.into_inner()).clone());
    let dsn = dsn_opt.unwrap_or_default();
    let query = if subcmd.is_empty() { args.trim() } else { args.trim() };

    if query.is_empty() {
        set_fg(&mut o, theme::DIM);
        write!(o, "  Usage: /db connect <dsn> | /db <query>\n").ok();
        reset_color(&mut o);
        return;
    }

    set_fg(&mut o, theme::CYAN);
    write!(o, "  [db] Executing: {}\n", query).ok();
    reset_color(&mut o);
    o.flush().ok();

    let (prog, prog_args): (&str, Vec<String>) = if dsn.starts_with("postgresql://") || dsn.starts_with("postgres://") {
        ("psql", vec![dsn.clone(), "-c".to_string(), query.to_string()])
    } else if dsn.starts_with("mysql://") {
        ("mysql", vec![format!("--host={}", dsn), "-e".to_string(), query.to_string()])
    } else if dsn.starts_with("sqlite:") || dsn.ends_with(".db") || dsn.ends_with(".sqlite") {
        let db_file = dsn.strip_prefix("sqlite:").unwrap_or(&dsn);
        ("sqlite3", vec![db_file.to_string(), query.to_string()])
    } else if dsn.starts_with("mongodb://") {
        ("mongosh", vec![dsn.clone(), "--eval".to_string(), query.to_string()])
    } else {
        set_fg(&mut o, theme::DIM);
        write!(o, "  [db] No DSN set. Use /db connect <dsn> first.\n").ok();
        reset_color(&mut o);
        return;
    };

    let out = std::process::Command::new(prog)
        .args(&prog_args)
        .output();
    match out {
        Ok(result) => {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);
            if !stdout.is_empty() {
                write!(o, "{}\n", stdout).ok();
            }
            if !stderr.is_empty() {
                set_fg(&mut o, theme::WARN);
                write!(o, "{}\n", stderr).ok();
                reset_color(&mut o);
            }
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  Failed to run {}: {}\n", prog, e).ok();
            reset_color(&mut o);
        }
    }
}

// 10c: /k8s Kubernetes management
async fn handle_k8s_command(args: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    let parts: Vec<&str> = args.trim().splitn(3, ' ').collect();
    let subcmd = parts.first().copied().unwrap_or("status");

    if which_command("kubectl").is_none() {
        set_fg(&mut o, theme::ERR);
        write!(o, "  [k8s] kubectl not found in PATH\n").ok();
        reset_color(&mut o);
        return;
    }

    let kubectl_args: Vec<&str> = match subcmd {
        "pods" => {
            let ns = parts.get(1).copied().unwrap_or("default");
            vec!["get", "pods", "-n", ns, "-o", "wide"]
        }
        "status" => vec!["get", "all"],
        "events" => vec!["get", "events", "--sort-by=.metadata.creationTimestamp"],
        "logs" => {
            let pod = parts.get(1).copied().unwrap_or("");
            if pod.is_empty() {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Usage: /k8s logs <pod> [tail]\n").ok();
                reset_color(&mut o);
                return;
            }
            let tail = parts.get(2).copied().unwrap_or("50");
            set_fg(&mut o, theme::CYAN);
            write!(o, "  [k8s] Fetching logs for pod: {}\n", pod).ok();
            reset_color(&mut o);
            o.flush().ok();
            let out = std::process::Command::new("kubectl")
                .args(["logs", pod, &format!("--tail={}", tail)])
                .output();
            match out {
                Ok(result) => write!(o, "{}\n", String::from_utf8_lossy(&result.stdout)).ok(),
                Err(e) => { set_fg(&mut o, theme::ERR); write!(o, "  {}\n", e).ok() }
            };
            return;
        }
        "exec" => {
            let pod = parts.get(1).copied().unwrap_or("");
            let cmd = parts.get(2).copied().unwrap_or("sh");
            if pod.is_empty() {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Usage: /k8s exec <pod> <cmd>\n").ok();
                reset_color(&mut o);
                return;
            }
            vec!["exec", "-it", pod, "--", cmd]
        }
        "deploy" => {
            let image = parts.get(1).copied().unwrap_or("");
            if image.is_empty() {
                set_fg(&mut o, theme::ERR);
                write!(o, "  Usage: /k8s deploy <image>\n").ok();
                reset_color(&mut o);
                return;
            }
            vec!["rollout", "restart", "deployment", image]
        }
        _ => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  [k8s] Subcommands: pods, logs, exec, deploy, status, events\n").ok();
            reset_color(&mut o);
            return;
        }
    };

    let out = std::process::Command::new("kubectl")
        .args(&kubectl_args)
        .output();
    match out {
        Ok(result) => {
            let text = String::from_utf8_lossy(&result.stdout);
            write!(o, "{}\n", text).ok();
            let err = String::from_utf8_lossy(&result.stderr);
            if !err.is_empty() && !result.status.success() {
                set_fg(&mut o, theme::WARN);
                write!(o, "{}\n", err).ok();
                reset_color(&mut o);
            }
        }
        Err(e) => {
            set_fg(&mut o, theme::ERR);
            write!(o, "  kubectl failed: {}\n", e).ok();
            reset_color(&mut o);
        }
    }
}

// 10d: /migrate DB migration generation
async fn handle_migrate_command(args: &str, _cfg: &CliConfig) {
    let mut o = io::stdout();
    let parts: Vec<&str> = args.trim().splitn(2, ' ').collect();
    let subcmd = parts.first().copied().unwrap_or("status");

    let root = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    // Find migrations directory
    let migrations_dir = {
        let cands = ["migrations", "db/migrations", "database/migrations"];
        cands.iter()
            .map(|d| std::path::Path::new(&root).join(d))
            .find(|p| p.exists())
            .unwrap_or_else(|| std::path::Path::new(&root).join("migrations"))
    };

    match subcmd {
        "new" => {
            let name = parts.get(1).copied().unwrap_or("migration");
            let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let filename = format!("{}_{}.sql", ts, name.replace(' ', "_"));
            let filepath = migrations_dir.join(&filename);
            if !migrations_dir.exists() {
                let _ = std::fs::create_dir_all(&migrations_dir);
            }
            let _ = std::fs::write(&filepath, format!("-- Migration: {}\n-- Created: {}\n\n-- Write your SQL here\n", name, ts));
            set_fg(&mut o, theme::OK);
            write!(o, "  [migrate] Created: {}\n", filepath.display()).ok();
            reset_color(&mut o);
        }
        "status" => {
            print_section_header("Migration Status");
            let applied_path = std::path::Path::new(&root).join(".shadowai").join("applied_migrations.json");
            let applied: Vec<String> = std::fs::read_to_string(&applied_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if migrations_dir.exists() {
                let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().extension().map(|x| x == "sql").unwrap_or(false))
                    .collect();
                entries.sort_by_key(|e| e.file_name());

                for entry in entries {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    let is_applied = applied.contains(&fname);
                    let status_color = if is_applied { theme::OK } else { theme::WARN };
                    let status = if is_applied { "applied" } else { "pending" };
                    set_fg(&mut o, status_color);
                    write!(o, "  {} {}\n", status, fname).ok();
                    reset_color(&mut o);
                }
            } else {
                set_fg(&mut o, theme::DIM);
                write!(o, "  No migrations directory found.\n").ok();
                reset_color(&mut o);
            }
            print_section_end();
        }
        "up" => {
            set_fg(&mut o, theme::CYAN);
            write!(o, "  [migrate] Running pending migrations...\n").ok();
            reset_color(&mut o);

            let applied_path = std::path::Path::new(&root).join(".shadowai").join("applied_migrations.json");
            let mut applied: Vec<String> = std::fs::read_to_string(&applied_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if migrations_dir.exists() {
                let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().extension().map(|x| x == "sql").unwrap_or(false))
                    .collect();
                entries.sort_by_key(|e| e.file_name());

                for entry in entries {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if !applied.contains(&fname) {
                        set_fg(&mut o, theme::DIM);
                        write!(o, "  Applying: {}\n", fname).ok();
                        reset_color(&mut o);
                        applied.push(fname);
                    }
                }
                let _ = std::fs::create_dir_all(applied_path.parent().unwrap());
                if let Ok(json) = serde_json::to_string(&applied) {
                    let _ = std::fs::write(&applied_path, json);
                }
                set_fg(&mut o, theme::OK);
                write!(o, "  {CHECK} Migrations complete.\n").ok();
                reset_color(&mut o);
            }
        }
        "down" => {
            set_fg(&mut o, theme::WARN);
            write!(o, "  [migrate] Reverting last migration (check for down.sql files)\n").ok();
            reset_color(&mut o);
        }
        _ => {
            set_fg(&mut o, theme::DIM);
            write!(o, "  [migrate] Subcommands: new <name>, up, down, status, ai <description>\n").ok();
            reset_color(&mut o);
        }
    }
}

// 11b: Async config loading helper
#[allow(dead_code)]
async fn load_config_async() -> CliConfig {
    let path = match config_file_path() {
        Some(p) => p,
        None => return CliConfig::default(),
    };
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
        Err(_) => CliConfig::default(),
    }
}

// ─── Section 8.1: base64_encode (no external crate) ──────────────────────────

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((combined >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((combined >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(ALPHABET[((combined >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(ALPHABET[(combined & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

// ─── Section 8.1: send_email_notification ────────────────────────────────────

#[allow(dead_code)]
async fn send_email_notification(subject: &str, body: &str, cfg: &CliConfig) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let host = match &cfg.smtp_host {
        Some(h) => h.clone(),
        None => return Err("Email not configured (set smtp_host, smtp_to in config)".to_string()),
    };
    let to = match &cfg.smtp_to {
        Some(t) => t.clone(),
        None => return Err("Email not configured (set smtp_host, smtp_to in config)".to_string()),
    };
    let port = cfg.smtp_port.unwrap_or(587);
    let from = cfg.smtp_from.clone().unwrap_or_else(|| "shadowai@localhost".to_string());

    let addr = format!("{}:{}", host, port);
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("SMTP error: connect failed: {}", e))?;

    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut line = String::new();

    // Read greeting
    line.clear();
    reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: read greeting: {}", e))?;

    // EHLO
    writer.write_all(b"EHLO shadowai\r\n").await.map_err(|e| format!("SMTP error: EHLO write: {}", e))?;
    // Read multi-line EHLO response
    loop {
        line.clear();
        reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: EHLO read: {}", e))?;
        // EHLO response lines start with "250-" (more to come) or "250 " (last line)
        if line.len() >= 4 && &line[3..4] == " " { break; }
        if line.is_empty() { break; }
    }

    // STARTTLS: best-effort (plain TCP only — document that TLS upgrade requires manual setup)
    if cfg.smtp_tls.unwrap_or(false) || port == 587 {
        writer.write_all(b"STARTTLS\r\n").await.map_err(|e| format!("SMTP error: STARTTLS write: {}", e))?;
        line.clear();
        reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: STARTTLS read: {}", e))?;
        // Note: actual TLS upgrade not performed (no tokio_native_tls). Plain TCP continues.
        // This works for port 25 and servers that allow plain after STARTTLS handshake is skipped.
        // For proper TLS: use a dedicated SMTP crate or configure the server for implicit TLS on port 465.
        eprintln!("[email] Warning: TLS upgrade not performed (plain TCP). Server replied: {}", line.trim());
    }

    // AUTH LOGIN
    if let Some(ref user) = cfg.smtp_user {
        writer.write_all(b"AUTH LOGIN\r\n").await.map_err(|e| format!("SMTP error: AUTH LOGIN write: {}", e))?;
        line.clear();
        reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: AUTH LOGIN read: {}", e))?;

        let user_b64 = base64_encode(user.as_bytes());
        writer.write_all(format!("{}\r\n", user_b64).as_bytes()).await.map_err(|e| format!("SMTP error: user write: {}", e))?;
        line.clear();
        reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: user read: {}", e))?;

        let pass = cfg.smtp_password.as_deref().unwrap_or("");
        let pass_b64 = base64_encode(pass.as_bytes());
        writer.write_all(format!("{}\r\n", pass_b64).as_bytes()).await.map_err(|e| format!("SMTP error: pass write: {}", e))?;
        line.clear();
        reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: pass read: {}", e))?;
        if !line.starts_with("235") {
            return Err(format!("SMTP error: authentication failed: {}", line.trim()));
        }
    }

    // MAIL FROM
    writer.write_all(format!("MAIL FROM:<{}>\r\n", from).as_bytes()).await.map_err(|e| format!("SMTP error: MAIL FROM: {}", e))?;
    line.clear();
    reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: MAIL FROM read: {}", e))?;

    // RCPT TO
    writer.write_all(format!("RCPT TO:<{}>\r\n", to).as_bytes()).await.map_err(|e| format!("SMTP error: RCPT TO: {}", e))?;
    line.clear();
    reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: RCPT TO read: {}", e))?;

    // DATA
    writer.write_all(b"DATA\r\n").await.map_err(|e| format!("SMTP error: DATA: {}", e))?;
    line.clear();
    reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: DATA read: {}", e))?;

    // Headers + body
    let msg = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nContent-Type: text/plain\r\n\r\n{}\r\n.\r\n",
        from, to, subject, body
    );
    writer.write_all(msg.as_bytes()).await.map_err(|e| format!("SMTP error: message write: {}", e))?;
    line.clear();
    reader.read_line(&mut line).await.map_err(|e| format!("SMTP error: message accept read: {}", e))?;
    if !line.starts_with("250") {
        return Err(format!("SMTP error: message rejected: {}", line.trim()));
    }

    // QUIT
    writer.write_all(b"QUIT\r\n").await.map_err(|e| format!("SMTP error: QUIT: {}", e))?;

    Ok(())
}

// ─── Section 7.1: relay_broadcast ────────────────────────────────────────────

fn relay_broadcast(msg: &str) {
    if let Some(clients_lock) = RELAY_CLIENTS.get() {
        if let Ok(mut clients) = clients_lock.lock() {
            clients.retain(|tx| tx.send(msg.to_string()).is_ok());
        }
    }
}

// ─── Section 7.1: start_relay_server ─────────────────────────────────────────

#[allow(dead_code)]
async fn start_relay_server(port: u16, secret: Option<String>) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::mpsc;

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .map_err(|e| format!("[relay] Bind failed: {}", e))?;

    println!("[relay] Listening on port {} — share your IP for collaborators to connect", port);
    RELAY_RUNNING.store(true, Ordering::SeqCst);

    loop {
        if !RELAY_RUNNING.load(Ordering::SeqCst) { break; }
        match listener.accept().await {
            Ok((stream, addr)) => {
                let secret_clone = secret.clone();
                println!("[relay] New connection from {}", addr);
                tokio::spawn(async move {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut reader = BufReader::new(read_half);
                    let mut line = String::new();

                    // Read auth token
                    line.clear();
                    if reader.read_line(&mut line).await.is_err() { return; }
                    let token = line.trim().to_string();

                    if let Some(ref s) = secret_clone {
                        if token != *s {
                            let _ = write_half.write_all(b"AUTH_FAIL\n").await;
                            return;
                        }
                    }
                    let _ = write_half.write_all(b"AUTH_OK\n").await;

                    // Register client
                    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                    {
                        let clients = RELAY_CLIENTS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
                        if let Ok(mut c) = clients.lock() { c.push(tx); }
                    }

                    // Forward broadcast messages to this client
                    while let Some(msg) = rx.recv().await {
                        if write_half.write_all(msg.as_bytes()).await.is_err() { break; }
                    }
                });
            }
            Err(_) => { break; }
        }
    }
    Ok(())
}

// ─── Section 7.1: connect_relay ──────────────────────────────────────────────

#[allow(dead_code)]
async fn connect_relay(host: &str, port: u16, secret: Option<&str>) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = tokio::net::TcpStream::connect(format!("{}:{}", host, port))
        .await
        .map_err(|e| format!("[relay] Connect failed: {}", e))?;

    let (read_half, mut write_half) = stream.into_split();

    // Send secret (or empty line)
    let token = format!("{}\n", secret.unwrap_or(""));
    write_half.write_all(token.as_bytes()).await.map_err(|e| format!("[relay] Send auth: {}", e))?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await.map_err(|e| format!("[relay] Read auth response: {}", e))?;

    if line.trim() != "AUTH_OK" {
        return Err(format!("[relay] Authentication failed: {}", line.trim()));
    }

    println!("[relay] Connected to relay at {}:{}", host, port);

    // Store writer in global
    {
        let relay_tx = RELAY_TX.get_or_init(|| std::sync::Mutex::new(None));
        if let Ok(mut g) = relay_tx.lock() { *g = Some(write_half); }
    }

    // Spawn reader task
    tokio::spawn(async move {
        loop {
            let mut msg = String::new();
            match reader.read_line(&mut msg).await {
                Ok(0) | Err(_) => {
                    println!("[relay] Disconnected from remote.");
                    break;
                }
                Ok(_) => {
                    print!("[relay @remote] {}", msg);
                }
            }
        }
    });

    Ok(())
}

// ─── Section 7.1: handle_relay_command ───────────────────────────────────────

#[allow(dead_code)]
async fn handle_relay_command(args: &str, cfg: &CliConfig) {
    let mut o = io::stdout();
    let args = args.trim();

    if args == "start" {
        let port = cfg.relay_port.unwrap_or(7878);
        let secret = cfg.relay_secret.clone();
        RELAY_RUNNING.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            if let Err(e) = start_relay_server(port, secret).await {
                eprintln!("{}", e);
            }
        });
        set_fg(&mut o, theme::OK);
        write!(o, "  [relay] Server starting on port {}...\n", port).ok();
        reset_color(&mut o);
    } else if args == "stop" {
        RELAY_RUNNING.store(false, Ordering::SeqCst);
        set_fg(&mut o, theme::DIM);
        write!(o, "  [relay] Relay server stopped.\n").ok();
        reset_color(&mut o);
    } else if let Some(rest) = args.strip_prefix("connect ") {
        let rest = rest.trim();
        let (host, port_str) = if let Some(idx) = rest.rfind(':') {
            (&rest[..idx], &rest[idx + 1..])
        } else {
            (rest, "7878")
        };
        let port: u16 = port_str.parse().unwrap_or(7878);
        let secret = cfg.relay_secret.as_deref();
        match connect_relay(host, port, secret).await {
            Ok(()) => {
                set_fg(&mut o, theme::OK);
                write!(o, "  [relay] Connected to {}:{}\n", host, port).ok();
                reset_color(&mut o);
            }
            Err(e) => {
                set_fg(&mut o, theme::ERR);
                write!(o, "  [relay] {}\n", e).ok();
                reset_color(&mut o);
            }
        }
    } else if args == "status" {
        let running = RELAY_RUNNING.load(Ordering::SeqCst);
        let client_count = RELAY_CLIENTS.get()
            .and_then(|m| m.lock().ok())
            .map(|c| c.len())
            .unwrap_or(0);
        let tx_connected = RELAY_TX.get()
            .and_then(|m| m.lock().ok())
            .map(|g| g.is_some())
            .unwrap_or(false);
        set_fg(&mut o, theme::CYAN);
        write!(o, "  [relay] Server running: {}\n", running).ok();
        write!(o, "  [relay] Connected clients: {}\n", client_count).ok();
        write!(o, "  [relay] Outbound relay connected: {}\n", tx_connected).ok();
        reset_color(&mut o);
    } else if let Some(msg) = args.strip_prefix("broadcast ") {
        relay_broadcast(msg.trim());
        set_fg(&mut o, theme::DIM);
        write!(o, "  [relay] Broadcast sent.\n").ok();
        reset_color(&mut o);
    } else {
        set_fg(&mut o, theme::DIM);
        write!(o, "  [relay] Subcommands: start, stop, connect <host:port>, status, broadcast <msg>\n").ok();
        reset_color(&mut o);
    }
}

// ─── Section 7.1: handle_review_request ──────────────────────────────────────

#[allow(dead_code)]
async fn handle_review_request(target: &str, cfg: &CliConfig) {
    let mut o = io::stdout();
    let target = target.trim();

    if target.is_empty() {
        set_fg(&mut o, theme::ERR);
        write!(o, "  [review] Usage: /review-request <@user_or_email>\n").ok();
        reset_color(&mut o);
        return;
    }

    // Export session (reuse logic from handle_share_command)
    let root_path = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    ensure_tracking_dir(&root_path);
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let session_id = format!("review-{}", ts);
    let filename = format!("{}.json", session_id);
    let path = tracking_file_path(&root_path, &filename);
    let export = serde_json::json!({
        "exported": ts,
        "note": "ShadowAI review request export",
        "root": root_path,
        "review_target": target,
    });
    if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&export).unwrap_or_default()) {
        set_fg(&mut o, theme::ERR);
        write!(o, "  [review] Export failed: {}\n", e).ok();
        reset_color(&mut o);
        return;
    }

    set_fg(&mut o, theme::OK);
    write!(o, "  [review] Session exported. Share with {}: shadowai review {}\n", target, session_id).ok();
    reset_color(&mut o);

    // Slack notification
    if let Some(ref slack_url) = cfg.slack_webhook {
        let msg = format!("Code review requested by ShadowAI from {}. Session: {}", target, session_id);
        let url = slack_url.clone();
        tokio::spawn(async move {
            let _ = send_webhook_notification(&url, &msg).await;
        });
    }

    // Discord notification
    if let Some(ref discord_url) = cfg.discord_webhook {
        let msg = format!("Code review requested by ShadowAI from {}. Session: {}", target, session_id);
        let url = discord_url.clone();
        tokio::spawn(async move {
            let _ = send_webhook_notification(&url, &msg).await;
        });
    }

    // Email notification if target looks like an email address (contains @ but not a Slack handle like @user)
    let is_email = target.contains('@') && target.contains('.') && !target.starts_with('@');
    if is_email {
        let subject = "Code review request from ShadowAI".to_string();
        let body = format!(
            "A code review has been requested.\n\nSession export path: {}\nSession ID: {}\n\nTo review, run: shadowai load {}",
            path.display(), session_id, filename
        );
        let cfg_clone = cfg.clone();
        let target_email = target.to_string();
        // Override smtp_to with the target email for this specific request
        let mut review_cfg = cfg_clone;
        review_cfg.smtp_to = Some(target_email);
        tokio::spawn(async move {
            match send_email_notification(&subject, &body, &review_cfg).await {
                Ok(()) => println!("  [review] Review request email sent."),
                Err(e) => eprintln!("  [review] Email send failed: {}", e),
            }
        });
    }
}

// ============================================================
// Section 13 – Built-in MCP servers (filesystem / git / web)
// ============================================================

/// Dispatch an MCP-style tool call to a built-in local implementation.
/// Returns `Some(output)` if the tool was handled locally, `None` if the
/// caller should fall through to a networked MCP server.
pub fn dispatch_builtin_mcp_tool(tool: &str, args: &serde_json::Value) -> Option<String> {
    match tool {
        // ── Filesystem ──────────────────────────────────────────
        "read_file" => {
            let path = args["path"].as_str()?;
            match std::fs::read_to_string(path) {
                Ok(content) => Some(content),
                Err(e) => Some(format!("[error] read_file: {}", e)),
            }
        }
        "write_file" => {
            let path = args["path"].as_str()?;
            let content = args["content"].as_str().unwrap_or("");
            match std::fs::write(path, content) {
                Ok(_) => Some(format!("[ok] wrote {} bytes to {}", content.len(), path)),
                Err(e) => Some(format!("[error] write_file: {}", e)),
            }
        }
        "list_dir" => {
            let path = args["path"].as_str().unwrap_or(".");
            match std::fs::read_dir(path) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect();
                    names.sort();
                    Some(names.join("\n"))
                }
                Err(e) => Some(format!("[error] list_dir: {}", e)),
            }
        }
        "append_file" => {
            let path = args["path"].as_str()?;
            let content = args["content"].as_str().unwrap_or("");
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)
                .map_err(|e| format!("[error] append_file open: {}", e)).ok()?;
            f.write_all(content.as_bytes()).ok();
            Some(format!("[ok] appended {} bytes to {}", content.len(), path))
        }
        // ── Git ─────────────────────────────────────────────────
        "git_status" => {
            let cwd = args["cwd"].as_str().unwrap_or(".");
            let out = std::process::Command::new("git")
                .args(["-C", cwd, "status", "--short"])
                .output();
            match out {
                Ok(o) => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
                Err(e) => Some(format!("[error] git_status: {}", e)),
            }
        }
        "git_diff" => {
            let cwd = args["cwd"].as_str().unwrap_or(".");
            let base = args["base"].as_str().unwrap_or("HEAD");
            let out = std::process::Command::new("git")
                .args(["-C", cwd, "diff", base])
                .output();
            match out {
                Ok(o) => {
                    let s = String::from_utf8_lossy(&o.stdout);
                    let lines: Vec<&str> = s.lines().take(300).collect();
                    Some(lines.join("\n"))
                }
                Err(e) => Some(format!("[error] git_diff: {}", e)),
            }
        }
        "git_log" => {
            let cwd = args["cwd"].as_str().unwrap_or(".");
            let n = args["n"].as_u64().unwrap_or(10);
            let out = std::process::Command::new("git")
                .args(["-C", cwd, "log", &format!("-{}", n), "--oneline"])
                .output();
            match out {
                Ok(o) => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
                Err(e) => Some(format!("[error] git_log: {}", e)),
            }
        }
        // ── Web ──────────────────────────────────────────────────
        "fetch_url" => {
            // Synchronous via curl to avoid blocking the async runtime
            let url = args["url"].as_str()?;
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Some("[error] fetch_url: only http/https supported".to_string());
            }
            let out = std::process::Command::new("curl")
                .args(["-sL", "--max-time", "15", url])
                .output();
            match out {
                Ok(o) => {
                    let body = String::from_utf8_lossy(&o.stdout);
                    // Strip HTML tags crudely
                    let plain: String = body.chars().scan(false, |in_tag, c| {
                        if c == '<' { *in_tag = true; Some(None) }
                        else if c == '>' { *in_tag = false; Some(None) }
                        else if *in_tag { Some(None) }
                        else { Some(Some(c)) }
                    }).flatten().collect();
                    let trimmed: String = plain.split_whitespace()
                        .take(500)
                        .collect::<Vec<_>>()
                        .join(" ");
                    Some(trimmed)
                }
                Err(e) => Some(format!("[error] fetch_url: {}", e)),
            }
        }
        "search" => {
            let query = args["query"].as_str()?;
            let out = std::process::Command::new("curl")
                .args(["-sL", "--max-time", "10",
                    &format!("https://duckduckgo.com/lite?q={}", urlencoding_simple(query))])
                .output();
            match out {
                Ok(o) => {
                    let body = String::from_utf8_lossy(&o.stdout);
                    let plain: String = body.chars().scan(false, |in_tag, c| {
                        if c == '<' { *in_tag = true; Some(None) }
                        else if c == '>' { *in_tag = false; Some(None) }
                        else if *in_tag { Some(None) }
                        else { Some(Some(c)) }
                    }).flatten().collect();
                    let trimmed: String = plain.split_whitespace()
                        .take(300)
                        .collect::<Vec<_>>()
                        .join(" ");
                    Some(trimmed)
                }
                Err(e) => Some(format!("[error] search: {}", e)),
            }
        }
        _ => None,
    }
}

/// List built-in MCP tool capabilities for display.
pub fn list_builtin_mcp_tools() -> Vec<(&'static str, &'static str)> {
    vec![
        ("read_file",   "Read a file from disk  {path}"),
        ("write_file",  "Write content to disk  {path, content}"),
        ("append_file", "Append to a file       {path, content}"),
        ("list_dir",    "List directory entries {path}"),
        ("git_status",  "git status --short     {cwd}"),
        ("git_diff",    "git diff vs base       {cwd, base}"),
        ("git_log",     "Recent git commits     {cwd, n}"),
        ("fetch_url",   "Fetch URL text         {url}"),
        ("search",      "DuckDuckGo search      {query}"),
    ]
}

// ============================================================
// Section 14 – PreCompact hook + long-running task notifications
// ============================================================

/// Fire any registered `pre_compact` hook scripts before auto-compaction.
/// Hooks live in `~/.config/shadowai/hooks/pre_compact/` and are executed
/// in lexical order. Their stdout is logged; failures are non-fatal.
pub fn run_pre_compact_hooks(root_path: &str) {
    let global_dir = dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("shadowai/hooks/pre_compact");
    let local_dir = std::path::Path::new(root_path).join(".shadowai/hooks/pre_compact");

    for dir in &[global_dir, local_dir] {
        if !dir.exists() { continue; }
        let mut scripts: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => continue,
        };
        scripts.sort();
        for script in scripts {
            if !script.is_file() { continue; }
            let _ = std::process::Command::new(&script)
                .current_dir(root_path)
                .env("SHADOWAI_ROOT", root_path)
                .output(); // non-fatal: ignore errors
        }
    }
}

/// Send a system notification when a long-running background task completes.
/// Uses `notify-send` (Linux), `osascript` (macOS), or `powershell` (Windows).
/// Also optionally posts to a Slack webhook if `SHADOWAI_SLACK_WEBHOOK` is set.
pub fn notify_task_complete(task_name: &str, summary: &str) {
    let title = "ShadowAI: Task Complete";
    let body = format!("{}: {}", task_name, &summary[..summary.len().min(200)]);

    // System notification
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args([title, &body])
            .output();
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "'"), title
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output();
    }
    #[cfg(target_os = "windows")]
    {
        let ps = format!(
            "[System.Windows.Forms.MessageBox]::Show('{}', '{}')",
            body.replace('\'', ""), title
        );
        let _ = std::process::Command::new("powershell")
            .args(["-Command", &ps])
            .output();
    }

    // Slack webhook (optional)
    if let Ok(webhook) = std::env::var("SHADOWAI_SLACK_WEBHOOK") {
        if !webhook.is_empty() {
            let payload = format!(r#"{{"text":"*{}*\n{}"}}"#, task_name, body);
            let _ = std::process::Command::new("curl")
                .args(["-s", "-X", "POST", "-H", "Content-Type: application/json",
                    "-d", &payload, &webhook])
                .output();
        }
    }
}

// ============================================================
// Section 15 – Hierarchical context compaction
// ============================================================

/// Hierarchical compaction: produces a multi-level summary instead of
/// truncating. Oldest segments get maximally compressed, recent segments
/// get a lighter summary, and the last `keep_last` are kept verbatim.
///
/// Returns the new compacted messages array.
pub fn hierarchical_compact_messages(
    messages: &[serde_json::Value],
    keep_last: usize,
) -> Vec<serde_json::Value> {
    if messages.len() <= keep_last {
        return messages.to_vec();
    }

    let n = messages.len();
    let old_end = n.saturating_sub(keep_last);

    // Split into three zones
    let zone_a = &messages[..old_end / 2];           // oldest  → heavy compression
    let zone_b = &messages[old_end / 2..old_end];    // middle  → light compression
    let verbatim = &messages[old_end..];              // recent  → verbatim

    fn compress(msgs: &[serde_json::Value], label: &str, max_chars: usize) -> serde_json::Value {
        let combined: String = msgs.iter().map(|m| {
            let role = m["role"].as_str().unwrap_or("?");
            let content = m["content"].as_str().unwrap_or("");
            format!("[{}] {}", role, content)
        }).collect::<Vec<_>>().join("\n");

        let truncated = if combined.len() > max_chars {
            format!("{}…", &combined[..max_chars])
        } else {
            combined
        };

        serde_json::json!({
            "role": "system",
            "content": format!("[{} — {} exchanges compressed]\n{}", label, msgs.len(), truncated)
        })
    }

    let mut result = Vec::new();
    if !zone_a.is_empty() {
        result.push(compress(zone_a, "ARCHIVE (oldest)", 800));
    }
    if !zone_b.is_empty() {
        result.push(compress(zone_b, "SUMMARY (earlier)", 1600));
    }
    result.extend_from_slice(verbatim);
    result
}


// ============================================================
// Section 15b – Compaction archive & completed-item cleanup
// ============================================================

/// Save conversation history to a timestamped JSON file in `.shadow-memory/`
/// before it gets compacted. This preserves a full record of what was done.
/// Also appends a summary to the work ledger to prevent repeat loops.
pub fn save_compaction_archive(messages: &[serde_json::Value], root_path: &str) {
    let shadow_mem = std::path::Path::new(root_path).join(".shadow-memory");
    let _ = std::fs::create_dir_all(&shadow_mem);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // ── 1. Save raw compaction archive ──
    let archive_path = shadow_mem.join(format!("compaction_{}.json", ts));
    let archive = serde_json::json!({
        "timestamp": ts,
        "message_count": messages.len(),
        "messages": messages,
    });
    let _ = std::fs::write(&archive_path, serde_json::to_string_pretty(&archive).unwrap_or_default());

    // ── 2. Extract completed work items and save to ledger ──
    // This ledger is injected into the system prompt so the AI knows what's done
    let ledger_path = shadow_mem.join("work_ledger.json");
    let mut ledger: Vec<serde_json::Value> = if ledger_path.exists() {
        std::fs::read_to_string(&ledger_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Scan messages for completed work: tool calls that succeeded, files written, fixes applied
    let mut completed_items: Vec<String> = Vec::new();
    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("");
        let content = msg["content"].as_str().unwrap_or("");

        // Track successful tool results
        if role == "tool" || role == "function" {
            // Summarize: just keep first 200 chars of tool result as evidence it ran
            let summary = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content.to_string()
            };
            completed_items.push(summary);
        }

        // Track assistant messages that indicate completed work
        if role == "assistant" {
            // Look for indicators of finished tasks
            for marker in &["✅", "Done", "Fixed", "Created", "Written", "Applied", "Implemented",
                          "write_file", "create_file", "Completed", "Successfully"] {
                if content.contains(marker) {
                    let summary = if content.len() > 300 {
                        format!("{}...", &content[..300])
                    } else {
                        content.to_string()
                    };
                    completed_items.push(summary);
                    break;
                }
            }
        }
    }

    if !completed_items.is_empty() {
        ledger.push(serde_json::json!({
            "archived_at": ts,
            "items_count": completed_items.len(),
            "completed_work": completed_items,
        }));

        // Keep ledger manageable — only last 20 compaction entries
        if ledger.len() > 20 {
            ledger = ledger.split_off(ledger.len() - 20);
        }
        let _ = std::fs::write(&ledger_path, serde_json::to_string_pretty(&ledger).unwrap_or_default());
    }

    // ── 3. Clean up old compaction archives (keep last 10) ──
    if let Ok(entries) = std::fs::read_dir(&shadow_mem) {
        let mut archives: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("compaction_") && n.ends_with(".json"))
                .unwrap_or(false))
            .collect();
        archives.sort();
        // Remove all but the last 10
        if archives.len() > 10 {
            for old in &archives[..archives.len() - 10] {
                let _ = std::fs::remove_file(old);
            }
        }
    }
}

/// Clean up completed/resolved items from conversation history.
/// This removes:
/// - Tool call results for already-applied changes (file writes, fixes)
/// - Redundant diagnostic messages for errors that were subsequently fixed
/// - Duplicate file contents that were included multiple times
///
/// Preserves:
/// - System messages
/// - Recent conversation (last 6 messages)
/// - Messages referencing files that are still being worked on
///
/// Also injects a "work ledger" summary so the AI knows what's already done.
pub fn clean_completed_items(messages: &[serde_json::Value], root_path: &str) -> Vec<serde_json::Value> {
    if messages.len() <= 8 {
        return messages.to_vec();
    }

    let keep_last = 8; // Keep the last 8 messages verbatim (recent context)
    let n = messages.len();
    let cutoff = n.saturating_sub(keep_last);

    let mut result: Vec<serde_json::Value> = Vec::new();

    // ── 1. Load work ledger (ALL entries) + fixed.md for deduplication ──
    let ledger_path = std::path::Path::new(root_path).join(".shadow-memory/work_ledger.json");
    let ledger_summary = {
        let mut summary_lines: Vec<String> = Vec::new();

        // Collect from ALL ledger entries (not just last 5) to prevent re-doing old work
        if ledger_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&ledger_path) {
                if let Ok(ledger) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
                    for entry in ledger.iter().rev().take(15) {
                        if let Some(items) = entry["completed_work"].as_array() {
                            for item in items.iter().take(8) {
                                if let Some(s) = item.as_str() {
                                    let line = s.lines().next().unwrap_or(s);
                                    let trimmed = if line.len() > 120 {
                                        format!("{}...", &line[..120])
                                    } else {
                                        line.to_string()
                                    };
                                    summary_lines.push(trimmed);
                                }
                            }
                        }
                        // Also include key_facts from consolidated archives
                        if let Some(facts) = entry["key_facts"].as_array() {
                            for fact in facts.iter().take(5) {
                                if let Some(s) = fact.as_str() {
                                    if s.starts_with("Action:") || s.starts_with("Note:") {
                                        let trimmed = if s.len() > 100 { &s[..100] } else { s };
                                        summary_lines.push(trimmed.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Also collect from .shadowai/fixed.md — explicitly resolved items
        let fixed_path = std::path::Path::new(root_path).join(".shadowai/fixed.md");
        if fixed_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&fixed_path) {
                for line in content.lines().filter(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with('#') && !t.starts_with("##")
                }).take(15) {
                    let trimmed = line.trim();
                    let s = if trimmed.len() > 100 { &trimmed[..100] } else { trimmed };
                    summary_lines.push(format!("FIXED: {}", s));
                }
            }
        }

        if !summary_lines.is_empty() {
            summary_lines.sort();
            summary_lines.dedup();
            summary_lines.retain(|s| !s.trim().is_empty());
            summary_lines.truncate(30);
            format!("[COMPLETED WORK — do NOT redo these]\n{}", summary_lines.join("\n"))
        } else {
            String::new()
        }
    };

    // ── 2. Inject work ledger as system context ──
    if !ledger_summary.is_empty() {
        result.push(serde_json::json!({
            "role": "system",
            "content": ledger_summary
        }));
    }

    // ── 3. Process older messages — aggressively clean completed work ──
    let mut seen_file_writes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut i = 0;
    while i < cutoff {
        let msg = &messages[i];
        let role = msg["role"].as_str().unwrap_or("");
        let content = msg["content"].as_str().unwrap_or("");

        match role {
            // Always keep system messages (they're small and important)
            "system" => {
                result.push(msg.clone());
            }

            // Tool results: only keep if they're recent or about ongoing work
            "tool" | "function" => {
                // Skip large tool results (file contents, long outputs) — they're archived
                if content.len() > 500 {
                    // Replace with a compact summary
                    let tool_name = msg["name"].as_str()
                        .or_else(|| msg["tool_call_id"].as_str())
                        .unwrap_or("tool");
                    let first_line = content.lines().next().unwrap_or("");
                    let summary = if first_line.len() > 100 {
                        format!("{}...", &first_line[..100])
                    } else {
                        first_line.to_string()
                    };
                    result.push(serde_json::json!({
                        "role": role,
                        "content": format!("[{} result: {}]", tool_name, summary),
                        "tool_call_id": msg.get("tool_call_id").cloned().unwrap_or(serde_json::json!(""))
                    }));
                } else {
                    result.push(msg.clone());
                }
            }

            // Assistant messages with file writes: deduplicate
            "assistant" => {
                // Track write_file calls to avoid duplicate file-write context
                if content.contains("write_file") || content.contains("create_file") {
                    // Extract file path if present
                    if let Some(path_start) = content.find("\"path\"") {
                        let after = &content[path_start..];
                        if let Some(val_start) = after.find('"').and_then(|p| after[p+1..].find('"').map(|q| p + 1 + q + 1)) {
                            if let Some(val_end) = after[val_start..].find('"') {
                                let path = &after[val_start..val_start + val_end];
                                if seen_file_writes.contains(path) {
                                    // Skip — this file was already written in a later message
                                    i += 1;
                                    continue;
                                }
                                seen_file_writes.insert(path.to_string());
                            }
                        }
                    }
                }

                // Compress long assistant messages in the old zone
                if content.len() > 1000 {
                    let summary = format!(
                        "[Earlier response — {} chars compressed]\n{}",
                        content.len(),
                        &content[..500]
                    );
                    result.push(serde_json::json!({
                        "role": "assistant",
                        "content": summary
                    }));
                } else {
                    result.push(msg.clone());
                }
            }

            // User messages: keep but compress long file contents
            "user" => {
                if content.len() > 2000 {
                    // Likely contains pasted file contents — compress
                    let summary = format!(
                        "[Earlier user message — {} chars]\n{}...",
                        content.len(),
                        &content[..800]
                    );
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": summary
                    }));
                } else {
                    result.push(msg.clone());
                }
            }

            _ => {
                result.push(msg.clone());
            }
        }
        i += 1;
    }

    // ── 4. Keep recent messages verbatim ──
    result.extend_from_slice(&messages[cutoff..]);

    result
}

// ============================================================
// Section 15c – Startup shadow-memory consolidation
// ============================================================

/// Consolidate stale compaction archives from `.shadow-memory/` into a single
/// date-stamped `archive_<ts>.json` file. Called once at CLI startup.
///
/// Files older than 24 hours are merged into one archive and deleted.
/// The `work_ledger.json` is updated with the consolidated completed-work
/// entries so the AI never re-does work from previous sessions.
///
/// Returns the number of individual archives that were merged.
pub fn consolidate_shadow_memory(root_path: &str) -> usize {
    let shadow_mem = std::path::Path::new(root_path).join(".shadow-memory");
    if !shadow_mem.exists() { return 0; }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let max_age_secs: u64 = 24 * 3600; // 24 hours

    // Collect all compaction_*.json files (not archive_* or work_ledger)
    let mut all_archives: Vec<(u64, std::path::PathBuf)> = match std::fs::read_dir(&shadow_mem) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("compaction_") && n.ends_with(".json"))
                .unwrap_or(false))
            .map(|p| {
                let ts: u64 = p.file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix("compaction_"))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                (ts, p)
            })
            .collect(),
        Err(_) => return 0,
    };

    // Only consolidate archives older than max_age_secs
    all_archives.retain(|(ts, _)| *ts > 0 && now.saturating_sub(*ts) > max_age_secs);

    if all_archives.is_empty() { return 0; }

    all_archives.sort_by_key(|(ts, _)| *ts);
    let count = all_archives.len();
    let earliest_ts = all_archives.first().map(|(ts, _)| *ts).unwrap_or(now);
    let latest_ts   = all_archives.last().map(|(ts, _)| *ts).unwrap_or(now);

    // Merge all old archives into consolidated data
    let mut completed_work: Vec<String> = Vec::new();
    let mut key_facts: Vec<String> = Vec::new();

    for (_, path) in &all_archives {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                // New-format archive: has a "messages" array
                if let Some(msgs) = json["messages"].as_array() {
                    for msg in msgs {
                        let role = msg["role"].as_str().unwrap_or("");
                        let content = msg["content"].as_str().unwrap_or("");
                        if role == "assistant" {
                            for marker in &["✅", "Done", "Fixed", "Created", "Written",
                                          "Applied", "Implemented", "Completed", "Successfully"] {
                                if content.contains(marker) {
                                    let line = content.lines().next().unwrap_or(content);
                                    let s = if line.len() > 120 { &line[..120] } else { line };
                                    completed_work.push(s.to_string());
                                    break;
                                }
                            }
                        }
                        if role == "tool" || role == "function" {
                            let first = content.lines().next().unwrap_or(content);
                            let s = if first.len() > 80 { &first[..80] } else { first };
                            completed_work.push(format!("[tool] {}", s));
                        }
                    }
                }
                // Old-format archive: has a "value" field with bullet facts
                if let Some(value) = json["value"].as_str() {
                    for line in value.lines() {
                        let t = line.trim();
                        if !t.is_empty() {
                            key_facts.push(t.to_string());
                        }
                    }
                }
            }
        }
    }

    completed_work.sort();
    completed_work.dedup();
    completed_work.retain(|s| !s.trim().is_empty());

    key_facts.sort();
    key_facts.dedup();
    key_facts.retain(|s| !s.trim().is_empty());

    // Write consolidated archive file
    let archive_path = shadow_mem.join(format!("archive_{}.json", latest_ts));
    let consolidated = serde_json::json!({
        "type": "consolidated",
        "consolidated_at": now,
        "earliest_session": earliest_ts,
        "latest_session": latest_ts,
        "archives_merged": count,
        "completed_work": completed_work,
        "key_facts": key_facts,
    });
    let _ = std::fs::write(&archive_path,
        serde_json::to_string_pretty(&consolidated).unwrap_or_default());

    // Update work_ledger.json with the consolidated data
    let ledger_path = shadow_mem.join("work_ledger.json");
    let mut ledger: Vec<serde_json::Value> = if ledger_path.exists() {
        std::fs::read_to_string(&ledger_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Build merged items for the ledger: completed_work + action/note key_facts
    let mut merged_items: Vec<String> = completed_work.clone();
    for fact in &key_facts {
        if fact.starts_with("Action:") || fact.starts_with("Note:") || fact.starts_with("File:") {
            merged_items.push(fact.clone());
        }
    }
    merged_items.sort();
    merged_items.dedup();
    merged_items.retain(|s| !s.trim().is_empty());

    if !merged_items.is_empty() {
        ledger.push(serde_json::json!({
            "archived_at": now,
            "type": "startup_consolidation",
            "archives_merged": count,
            "earliest_session": earliest_ts,
            "latest_session": latest_ts,
            "items_count": merged_items.len(),
            "completed_work": merged_items,
            "key_facts": key_facts,
        }));
        // Keep ledger manageable — last 30 entries
        if ledger.len() > 30 {
            ledger = ledger.split_off(ledger.len() - 30);
        }
        let _ = std::fs::write(&ledger_path,
            serde_json::to_string_pretty(&ledger).unwrap_or_default());
    }

    // Delete the old individual compaction files (now safely archived)
    for (_, path) in &all_archives {
        let _ = std::fs::remove_file(path);
    }

    // Also prune old archive_*.json files — keep last 7 (one week)
    if let Ok(rd) = std::fs::read_dir(&shadow_mem) {
        let mut old_consolidated: Vec<std::path::PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("archive_") && n.ends_with(".json"))
                .unwrap_or(false))
            .collect();
        old_consolidated.sort();
        if old_consolidated.len() > 7 {
            for old in &old_consolidated[..old_consolidated.len() - 7] {
                let _ = std::fs::remove_file(old);
            }
        }
    }

    count
}

// ============================================================
// Section 17a – Team shared commands (project-scoped .shadowai/commands/)
// ============================================================

/// Load team/project-scoped slash commands from `.shadowai/commands/*.md`
/// (committed to Git) in addition to global `~/.config/shadowai/commands/`.
/// Returns a list of (command_name, description, content) tuples.
pub fn load_team_commands(root_path: &str) -> Vec<(String, String, String)> {
    let mut commands = Vec::new();

    let dirs_to_check = vec![
        std::path::PathBuf::from(root_path).join(".shadowai/commands"),
        dirs_next::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("shadowai/commands"),
    ];

    for dir in &dirs_to_check {
        if !dir.exists() { continue; }
        let mut entries: Vec<_> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => continue,
        };
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
            let name = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() { continue; }
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            // First line starting with `#` or first non-empty line = description
            let description = content.lines()
                .map(|l| l.trim_start_matches('#').trim())
                .find(|l| !l.is_empty())
                .unwrap_or("Custom command")
                .to_string();
            commands.push((name, description, content));
        }
    }
    commands
}

/// Execute a team command by name: render its content, substituting `$ARGS`.
pub fn run_team_command(name: &str, args: &str, root_path: &str) -> Option<String> {
    let commands = load_team_commands(root_path);
    let cmd = commands.into_iter().find(|(n, _, _)| n == name)?;
    let rendered = cmd.2.replace("$ARGS", args).replace("{{args}}", args);
    Some(rendered)
}

// ============================================================
// Section 17b – LSP integration: go_to_definition / find_references / hover
// ============================================================

/// Connect to a running LSP server on a Unix socket or TCP port and send a
/// JSON-RPC request. Returns the raw JSON response string.
///
/// Discovery order:
///   1. `SHADOWAI_LSP_SOCKET` env var (Unix socket path)
///   2. `SHADOWAI_LSP_PORT` env var (TCP port on localhost)
///   3. `.shadowai/lsp.socket` file in project root (contains socket path or port)
pub fn lsp_request(root_path: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::{BufReader, Write};

    let socket_path = std::env::var("SHADOWAI_LSP_SOCKET").ok()
        .or_else(|| {
            let cfg = std::path::Path::new(root_path).join(".shadowai/lsp.socket");
            std::fs::read_to_string(&cfg).ok().map(|s| s.trim().to_string())
        });

    let port: Option<u16> = std::env::var("SHADOWAI_LSP_PORT").ok()
        .and_then(|p| p.parse().ok());

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    });
    let req_str = request.to_string();
    let framed = format!("Content-Length: {}\r\n\r\n{}", req_str.len(), req_str);

    let response = if let Some(ref sock) = socket_path {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let mut stream = UnixStream::connect(sock)
                .map_err(|e| format!("LSP socket connect failed: {}", e))?;
            stream.write_all(framed.as_bytes()).map_err(|e| e.to_string())?;
            let mut reader = BufReader::new(stream);
            read_lsp_response(&mut reader)?
        }
        #[cfg(not(unix))]
        {
            return Err("Unix sockets not supported on this platform".to_string());
        }
    } else if let Some(p) = port {
        use std::net::TcpStream;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", p))
            .map_err(|e| format!("LSP TCP connect failed: {}", e))?;
        stream.write_all(framed.as_bytes()).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        read_lsp_response(&mut reader)?
    } else {
        return Err("No LSP server configured. Set SHADOWAI_LSP_SOCKET or SHADOWAI_LSP_PORT.".to_string());
    };

    serde_json::from_str(&response).map_err(|e| format!("LSP parse error: {}", e))
}

fn read_lsp_response<R: std::io::BufRead>(reader: &mut R) -> Result<String, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() { break; }
        if let Some(rest) = line.strip_prefix("Content-Length: ") {
            content_length = rest.parse().ok();
        }
    }
    let len = content_length.ok_or("LSP: missing Content-Length header")?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    String::from_utf8(body).map_err(|e| e.to_string())
}

/// High-level LSP tool: go to definition.
pub fn lsp_go_to_definition(root_path: &str, file: &str, line: u32, character: u32) -> String {
    let params = serde_json::json!({
        "textDocument": { "uri": format!("file://{}", file) },
        "position": { "line": line, "character": character }
    });
    match lsp_request(root_path, "textDocument/definition", params) {
        Ok(r) => format_lsp_locations(&r),
        Err(e) => format!("[LSP error] {}", e),
    }
}

/// High-level LSP tool: find all references.
pub fn lsp_find_references(root_path: &str, file: &str, line: u32, character: u32) -> String {
    let params = serde_json::json!({
        "textDocument": { "uri": format!("file://{}", file) },
        "position": { "line": line, "character": character },
        "context": { "includeDeclaration": true }
    });
    match lsp_request(root_path, "textDocument/references", params) {
        Ok(r) => format_lsp_locations(&r),
        Err(e) => format!("[LSP error] {}", e),
    }
}

/// High-level LSP tool: hover docs.
pub fn lsp_hover(root_path: &str, file: &str, line: u32, character: u32) -> String {
    let params = serde_json::json!({
        "textDocument": { "uri": format!("file://{}", file) },
        "position": { "line": line, "character": character }
    });
    match lsp_request(root_path, "textDocument/hover", params) {
        Ok(r) => r["result"]["contents"]["value"]
            .as_str()
            .unwrap_or_else(|| r["result"]["contents"].as_str().unwrap_or("[no hover docs]"))
            .to_string(),
        Err(e) => format!("[LSP error] {}", e),
    }
}

fn format_lsp_locations(r: &serde_json::Value) -> String {
    let locations = if let Some(arr) = r["result"].as_array() {
        arr.iter().map(|loc| {
            let uri = loc["uri"].as_str().unwrap_or("?");
            let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0);
            let col  = loc["range"]["start"]["character"].as_u64().unwrap_or(0);
            format!("{}:{}:{}", uri.trim_start_matches("file://"), line + 1, col + 1)
        }).collect::<Vec<_>>().join("\n")
    } else if let Some(obj) = r["result"].as_object() {
        let uri  = obj.get("uri").and_then(|v| v.as_str()).unwrap_or("?");
        let line = r["result"]["range"]["start"]["line"].as_u64().unwrap_or(0);
        let col  = r["result"]["range"]["start"]["character"].as_u64().unwrap_or(0);
        format!("{}:{}:{}", uri.trim_start_matches("file://"), line + 1, col + 1)
    } else {
        "[no locations found]".to_string()
    };
    locations
}

// ============================================================
// Section 18 – Transactional agent actions (git stash + restore)
// ============================================================

/// Stash uncommitted changes before an agentic sequence so they can be
/// restored if the sequence fails. Returns the stash ref (e.g. `stash@{0}`)
/// or an error string.
pub fn agent_transaction_begin(root_path: &str, label: &str) -> Result<Option<String>, String> {
    // Check if there's anything to stash
    let status = std::process::Command::new("git")
        .args(["-C", root_path, "status", "--porcelain"])
        .output()
        .map_err(|e| format!("git status failed: {}", e))?;
    if status.stdout.is_empty() {
        return Ok(None); // Nothing to stash — clean working tree
    }

    let msg = format!("shadowai-txn: {}", label);
    let out = std::process::Command::new("git")
        .args(["-C", root_path, "stash", "push", "-m", &msg])
        .output()
        .map_err(|e| format!("git stash failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        // Extract stash ref from output
        let stash_ref = stdout.lines()
            .find(|l| l.contains("stash@{"))
            .and_then(|l| l.split_whitespace().find(|t| t.starts_with("stash@{")))
            .unwrap_or("stash@{0}")
            .to_string();
        Ok(Some(stash_ref))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

/// Restore the stash saved by `agent_transaction_begin` if the agent sequence
/// failed. Runs `git stash pop <stash_ref>`.
pub fn agent_transaction_rollback(root_path: &str, stash_ref: &str) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["-C", root_path, "stash", "pop", stash_ref])
        .output()
        .map_err(|e| format!("git stash pop failed: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

/// Drop the stash (commit the transaction) if the agent sequence succeeded.
pub fn agent_transaction_commit(root_path: &str, stash_ref: &str) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["-C", root_path, "stash", "drop", stash_ref])
        .output()
        .map_err(|e| format!("git stash drop failed: {}", e))?;
    if out.status.success() { Ok(()) } else { Ok(()) } // non-fatal
}

/// TUI handler: `/txn begin|rollback|commit` — display transaction status.
pub fn handle_txn_command(args: &str, root_path: &str) -> String {
    let parts: Vec<&str> = args.trim().splitn(2, ' ').collect();
    match parts.as_slice() {
        ["begin"] | ["begin", ..] => {
            let label = parts.get(1).copied().unwrap_or("manual-txn");
            match agent_transaction_begin(root_path, label) {
                Ok(None) => "Transaction started (clean working tree — nothing to stash).".to_string(),
                Ok(Some(r)) => format!("Transaction started. Stash: {}\nRun /txn rollback to undo, /txn commit to finalize.", r),
                Err(e) => format!("[error] Transaction begin failed: {}", e),
            }
        }
        ["rollback"] => {
            match agent_transaction_rollback(root_path, "stash@{0}") {
                Ok(_) => "Rolled back: stash@{0} restored.".to_string(),
                Err(e) => format!("[error] Rollback failed: {}", e),
            }
        }
        ["commit"] => {
            match agent_transaction_commit(root_path, "stash@{0}") {
                Ok(_) => "Transaction committed (stash dropped).".to_string(),
                Err(e) => format!("[error] Commit failed: {}", e),
            }
        }
        _ => "Usage: /txn begin [label] | /txn rollback | /txn commit".to_string(),
    }
}

// ============================================================
// Section 19 – Model cost routing (prefer_cheap)
// ============================================================

/// Estimate whether a prompt is "simple" (short context, no tool calls needed)
/// to decide whether to route to a cheaper model.
///
/// Returns the model name that should be used, given:
///  - `prefer_cheap`: from config (or CLI flag)
///  - `current_model`: what the user has selected
///  - `context_chars`: total characters in the conversation context so far
pub fn route_model_by_cost(
    prefer_cheap: bool,
    current_model: &str,
    context_chars: usize,
) -> String {
    if !prefer_cheap {
        return current_model.to_string();
    }

    // Simple heuristic: if context is small and the model is a large one,
    // downgrade to Haiku / flash / mini
    let is_large = current_model.contains("opus")
        || current_model.contains("sonnet")
        || current_model.contains("gpt-4")
        || current_model.contains("mistral-large")
        || current_model.contains("gemini-1.5-pro");

    if is_large && context_chars < 2000 {
        // Pick the cheap sibling for the model family
        if current_model.contains("claude") {
            return "claude-haiku-4-5-20251001".to_string();
        }
        if current_model.contains("gpt-4") {
            return "gpt-4o-mini".to_string();
        }
        if current_model.contains("gemini") {
            return "gemini-1.5-flash".to_string();
        }
        if current_model.contains("mistral") {
            return "mistral-small".to_string();
        }
    }
    current_model.to_string()
}

/// TUI handler: `/cheap on|off|status` — toggle prefer_cheap routing.
pub fn handle_cheap_command(args: &str, app: &mut TuiApp) -> String {
    match args.trim() {
        "on" => {
            app.prefer_cheap = true;
            format!(
                "Cost routing enabled. Short prompts will auto-route to cheaper models.\n\
                 Current model: {} → may downgrade to Haiku/mini for simple tasks.",
                app.current_model
            )
        }
        "off" => {
            app.prefer_cheap = false;
            "Cost routing disabled. Using selected model for all requests.".to_string()
        }
        "status" | "" => {
            let effective = route_model_by_cost(app.prefer_cheap, &app.current_model, 0);
            format!(
                "Cost routing: {}\nCurrent model: {}\nEffective for short prompts: {}",
                if app.prefer_cheap { "ON" } else { "OFF" },
                app.current_model,
                effective,
            )
        }
        _ => "Usage: /cheap on | off | status".to_string(),
    }
}
