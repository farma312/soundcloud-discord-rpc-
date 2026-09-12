#![windows_subsystem = "windows"]

use ab_glyph::{FontRef, PxScale};
use chrono::Local;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use eframe::egui;
use image::{ImageBuffer, Rgba};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Response, Server};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, TrayIconBuilder, TrayIconEvent,
};
use urlencoding::encode;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
};

const CLIENT_ID: &str = "1249851004971651174";
const FALLBACK_LARGE_IMAGE: &str = "browser_icon";
const PLAY_IMAGE_KEY: &str = "play";
const CONFIG_FILE: &str = "config.json";

const SC_IMAGE_BYTES: &[u8] = include_bytes!("sc.png");

static SAVED_HWND: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct IncomingTrackPayload {
    track: String,
    artist: String,
    passed: u64,
    duration: u64,
    cover: String,
    is_playing: bool,
}

fn load_app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(SC_IMAGE_BYTES).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn load_tray_icon() -> Option<tray_icon::Icon> {
    let img = image::load_from_memory(SC_IMAGE_BYTES).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), width, height).ok()
}

fn get_own_window() -> Option<HWND> {
    let saved = SAVED_HWND.load(Ordering::SeqCst);
    if saved != 0 {
        return Some(HWND(saved as *mut c_void));
    }

    let current_pid = std::process::id();
    struct SearchContext {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut SearchContext);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == ctx.pid {
            let length = GetWindowTextLengthW(hwnd);
            if length > 0 {
                let mut buffer: Vec<u16> = vec![0; (length + 1) as usize];
                let copied = GetWindowTextW(hwnd, &mut buffer);
                if copied > 0 {
                    buffer.truncate(copied as usize);
                    if let Ok(title) = OsString::from_wide(&buffer).into_string() {
                        if title.contains("SoundCloud RPC") {
                            ctx.found = Some(hwnd);
                            return BOOL(0);
                        }
                    }
                }
            }
        }
        BOOL(1)
    }
    let mut ctx = SearchContext {
        pid: current_pid,
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    if let Some(hwnd) = ctx.found {
        SAVED_HWND.store(hwnd.0 as usize, Ordering::SeqCst);
    }
    ctx.found
}

fn restore_app_window() {
    if let Some(hwnd) = get_own_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

fn hide_app_window() {
    if let Some(hwnd) = get_own_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppLanguage {
    Ru,
    En,
    Ua,
}

impl AppLanguage {
    fn label(&self) -> &'static str {
        match self {
            AppLanguage::Ru => "🇷🇺 Русский",
            AppLanguage::En => "🇬🇧 English",
            AppLanguage::Ua => "🇺🇦 Українська",
        }
    }
}

struct I18n;

impl I18n {
    fn tab_player(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🎵 Плеер",
            AppLanguage::En => "🎵 Player",
            AppLanguage::Ua => "🎵 Плеєр",
        }
    }
    fn tab_karaoke(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🎤 Караоке",
            AppLanguage::En => "🎤 Karaoke",
            AppLanguage::Ua => "🎤 Караоке",
        }
    }
    fn tab_top(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "📊 Топ",
            AppLanguage::En => "📊 Top",
            AppLanguage::Ua => "📊 Топ",
        }
    }
    fn tab_history(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "📜 История",
            AppLanguage::En => "📜 History",
            AppLanguage::Ua => "📜 Історія",
        }
    }
    fn status_active(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Статус: Активен",
            AppLanguage::En => "Status: Active",
            AppLanguage::Ua => "Статус: Активний",
        }
    }
    fn status_disabled(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Статус: Отключен",
            AppLanguage::En => "Status: Disabled",
            AppLanguage::Ua => "Статус: Вимкнено",
        }
    }
    fn btn_minimize(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "— Свернуть в фон",
            AppLanguage::En => "— Minimize to background",
            AppLanguage::Ua => "— Згорнути у фон",
        }
    }
    fn total_played(lang: AppLanguage, count: u32) -> String {
        match lang {
            AppLanguage::Ru => format!("Всего дослушано треков: {}", count),
            AppLanguage::En => format!("Total tracks finished: {}", count),
            AppLanguage::Ua => format!("Всього дослухано треків: {}", count),
        }
    }
    fn stats_header(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Музыкальная статистика",
            AppLanguage::En => "Music Statistics",
            AppLanguage::Ua => "Музична статистика",
        }
    }
    fn stats_total(lang: AppLanguage, count: u32) -> String {
        match lang {
            AppLanguage::Ru => format!("Всего треков: {}", count),
            AppLanguage::En => format!("Total tracks: {}", count),
            AppLanguage::Ua => format!("Всього треків: {}", count),
        }
    }
    fn stats_time(lang: AppLanguage, hours: f32) -> String {
        match lang {
            AppLanguage::Ru => format!("Время: {:.1} ч", hours),
            AppLanguage::En => format!("Time: {:.1} h", hours),
            AppLanguage::Ua => format!("Час: {:.1} год", hours),
        }
    }
    fn stats_fav_artist(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🔥 Топ артист:",
            AppLanguage::En => "🔥 Top Artist:",
            AppLanguage::Ua => "🔥 Топ виконавець:",
        }
    }
    fn btn_share_top(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "📸 Поделиться топом (Создать карточку)",
            AppLanguage::En => "📸 Share Top (Create card)",
            AppLanguage::Ua => "📸 Поділитися топом (Створити картку)",
        }
    }
    fn top_artists_header(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Топ исполнителей",
            AppLanguage::En => "Top Artists",
            AppLanguage::Ua => "Топ виконавців",
        }
    }
    fn tracks_word(lang: AppLanguage, cnt: usize) -> String {
        match lang {
            AppLanguage::Ru => format!("{} треков", cnt),
            AppLanguage::En => format!("{} tracks", cnt),
            AppLanguage::Ua => format!("{} треків", cnt),
        }
    }
    fn history_empty(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "История пока пуста...",
            AppLanguage::En => "History is currently empty...",
            AppLanguage::Ua => "Історія поки порожня...",
        }
    }
    fn lyrics_not_found(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Текст не найден. Возможно, это бит или эксклюзив SoundCloud.",
            AppLanguage::En => "Lyrics not found. It might be a beat or SoundCloud exclusive.",
            AppLanguage::Ua => "Текст не знайдено. Можливо, це біт або ексклюзив SoundCloud.",
        }
    }
    fn lyrics_plain_hint(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "ℹ️ Обычный текст (без таймкодов караоке):",
            AppLanguage::En => "ℹ️ Plain lyrics (no karaoke sync):",
            AppLanguage::Ua => "ℹ️ Звичайний текст (без таймкодів караоке):",
        }
    }
    fn settings_title(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Настройки приложения",
            AppLanguage::En => "Application Settings",
            AppLanguage::Ua => "Налаштування застосунку",
        }
    }
    fn settings_lang(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Язык интерфейса:",
            AppLanguage::En => "Interface Language:",
            AppLanguage::Ua => "Мова інтерфейсу:",
        }
    }
    fn settings_theme(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Тема интерфейса:",
            AppLanguage::En => "Theme style:",
            AppLanguage::Ua => "Тема інтерфейсу:",
        }
    }
    fn settings_clear_hist(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Очистить историю треков:",
            AppLanguage::En => "Clear track history:",
            AppLanguage::Ua => "Очистити історію треків:",
        }
    }
    fn btn_reset(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🗑 Сбросить",
            AppLanguage::En => "🗑 Reset",
            AppLanguage::Ua => "🗑 Скинути",
        }
    }
    fn custom_accent_label(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Цвет акцента:",
            AppLanguage::En => "Accent color:",
            AppLanguage::Ua => "Колір акценту:",
        }
    }
    fn custom_bg_label(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Цвет фона:",
            AppLanguage::En => "Background color:",
            AppLanguage::Ua => "Колір фону:",
        }
    }
    fn copy_success(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "✔ Скопировано в буфер!",
            AppLanguage::En => "✔ Copied to clipboard!",
            AppLanguage::Ua => "✔ Скопійовано в буфер!",
        }
    }
    fn toggle_particles(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🌌 Фоновые частицы (Sparks):",
            AppLanguage::En => "🌌 Background particles (Sparks):",
            AppLanguage::Ua => "🌌 Фонові частинки (Sparks):",
        }
    }
    fn particles_dir_label(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Направление полёта:",
            AppLanguage::En => "Flight direction:",
            AppLanguage::Ua => "Напрямок польоту:",
        }
    }
    fn dir_up(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "⬆ Вверх",
            AppLanguage::En => "⬆ Upward",
            AppLanguage::Ua => "⬆ Вгору",
        }
    }
    fn dir_down(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "⬇ Вниз",
            AppLanguage::En => "⬇ Downward",
            AppLanguage::Ua => "⬇ Вниз",
        }
    }
    fn toggle_vinyl(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "💿 Вращающийся винил:",
            AppLanguage::En => "💿 Rotating vinyl disc:",
            AppLanguage::Ua => "💿 Вінілова платівка, що обертається:",
        }
    }
    fn toggle_eq(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "⚡ Неоновый эквалайзер:",
            AppLanguage::En => "⚡ Neon equalizer:",
            AppLanguage::Ua => "⚡ Неоновий еквалайзер:",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppTheme {
    Kawaii,
    Neverlose,
    ShadowFiend,
    SoundCloud,
    Custom,
}

impl AppTheme {
    fn label(&self, lang: AppLanguage) -> &'static str {
        match self {
            AppTheme::Kawaii => match lang {
                AppLanguage::Ru => "🌸 Кавайная (Розовая)",
                AppLanguage::En => "🌸 Kawaii (Pink)",
                AppLanguage::Ua => "🌸 Кавайна (Рожева)",
            },
            AppTheme::Neverlose => "❄️ Neverlose (Cyan)",
            AppTheme::ShadowFiend => "🔥 Shadow Fiend (Ruby)",
            AppTheme::SoundCloud => "⚡ SoundCloud (Orange)",
            AppTheme::Custom => match lang {
                AppLanguage::Ru => "🎨 Своя тема",
                AppLanguage::En => "🎨 Custom Theme",
                AppLanguage::Ua => "🎨 Власна тема",
            },
        }
    }

    fn bg_color(&self, custom_bg: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(22, 18, 26),
            AppTheme::Neverlose => egui::Color32::from_rgb(11, 14, 20),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(20, 16, 19),
            AppTheme::SoundCloud => egui::Color32::from_rgb(22, 22, 24),
            AppTheme::Custom => egui::Color32::from_rgb(custom_bg[0], custom_bg[1], custom_bg[2]),
        }
    }

    fn widget_bg(&self, custom_bg: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(32, 26, 38),
            AppTheme::Neverlose => egui::Color32::from_rgb(18, 23, 33),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(32, 24, 30),
            AppTheme::SoundCloud => egui::Color32::from_rgb(34, 34, 36),
            AppTheme::Custom => {
                let r = custom_bg[0].saturating_add(14);
                let g = custom_bg[1].saturating_add(14);
                let b = custom_bg[2].saturating_add(16);
                egui::Color32::from_rgb(r, g, b)
            }
        }
    }

    fn primary_accent(&self, custom_accent: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(230, 95, 140),
            AppTheme::Neverlose => egui::Color32::from_rgb(0, 225, 255),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(230, 36, 68),
            AppTheme::SoundCloud => egui::Color32::from_rgb(255, 85, 0),
            AppTheme::Custom => egui::Color32::from_rgb(custom_accent[0], custom_accent[1], custom_accent[2]),
        }
    }

    fn secondary_accent(&self, custom_accent: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(245, 185, 205),
            AppTheme::Neverlose => egui::Color32::from_rgb(130, 235, 255),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(255, 120, 145),
            AppTheme::SoundCloud => egui::Color32::from_rgb(255, 150, 70),
            AppTheme::Custom => {
                let r = custom_accent[0].saturating_add(40);
                let g = custom_accent[1].saturating_add(40);
                let b = custom_accent[2].saturating_add(40);
                egui::Color32::from_rgb(r, g, b)
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct HistoryEntry {
    track: String,
    artist: String,
    played_at: String,
    duration_sec: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct AppConfig {
    theme: AppTheme,
    language: AppLanguage,
    custom_accent: [u8; 3],
    custom_bg: [u8; 3],
    enable_particles: bool,
    particles_direction_up: bool,
    enable_vinyl: bool,
    enable_equalizer: bool,
    total_tracks_played: u32,
    custom_image_path: Option<String>,
    history: Vec<HistoryEntry>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: AppTheme::Kawaii,
            language: AppLanguage::Ru,
            custom_accent: [230, 95, 140],
            custom_bg: [22, 18, 26],
            enable_particles: false,
            particles_direction_up: true,
            enable_vinyl: false,
            enable_equalizer: false,
            total_tracks_played: 0,
            custom_image_path: None,
            history: Vec::new(),
        }
    }
}

impl AppConfig {
    fn load() -> Self {
        if let Ok(data) = fs::read_to_string(CONFIG_FILE) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
        Self::default()
    }

    fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(CONFIG_FILE, data);
        }
    }
}

fn format_duration(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    format!("{:02}:{:02}", mins, secs)
}

fn load_system_font() -> Option<Vec<u8>> {
    let font_paths = [
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
    ];
    for p in font_paths {
        if let Ok(bytes) = fs::read(p) {
            return Some(bytes);
        }
    }
    None
}

fn generate_stats_card(
    total_played: u32,
    total_hours: f32,
    top_artists: &[(String, usize)],
    theme: AppTheme,
    custom_accent: [u8; 3],
    custom_bg: [u8; 3],
    lang: AppLanguage,
) -> Option<PathBuf> {
    let width = 640u32;
    let height = 420u32;
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([22, 18, 26, 255]));

    let (bg_r, bg_g, bg_b) = match theme {
        AppTheme::Kawaii => (24, 18, 28),
        AppTheme::Neverlose => (11, 16, 26),
        AppTheme::ShadowFiend => (22, 14, 18),
        AppTheme::SoundCloud => (24, 20, 18),
        AppTheme::Custom => (custom_bg[0], custom_bg[1], custom_bg[2]),
    };

    let (ac_r, ac_g, ac_b) = match theme {
        AppTheme::Kawaii => (230, 95, 140),
        AppTheme::Neverlose => (0, 225, 255),
        AppTheme::ShadowFiend => (230, 36, 68),
        AppTheme::SoundCloud => (255, 85, 0),
        AppTheme::Custom => (custom_accent[0], custom_accent[1], custom_accent[2]),
    };

    for y in 0..height {
        let factor = (y as f32 / height as f32) * 0.45;
        let r = ((bg_r as f32 * (1.0 - factor)) as u8).max(8);
        let g = ((bg_g as f32 * (1.0 - factor)) as u8).max(6);
        let b = ((bg_b as f32 * (1.0 - factor)) as u8).max(10);
        for x in 0..width {
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(width, 6), Rgba([ac_r, ac_g, ac_b, 255]));
    draw_hollow_rect_mut(&mut img, Rect::at(25, 25).of_size(width - 50, height - 50), Rgba([ac_r, ac_g, ac_b, 100]));

    if let Some(font_bytes) = load_system_font() {
        if let Ok(font) = FontRef::try_from_slice(&font_bytes) {
            let white = Rgba([245, 245, 250, 255]);
            let accent = Rgba([ac_r, ac_g, ac_b, 255]);
            let gray = Rgba([175, 165, 185, 255]);

            draw_text_mut(&mut img, accent, 45, 45, PxScale::from(24.0), &font, "SOUNDCLOUD SUMMARY");

            let date_str = Local::now().format("%d.%m.%Y").to_string();
            draw_text_mut(&mut img, gray, (width - 160) as i32, 48, PxScale::from(16.0), &font, &date_str);

            let stat_line = match lang {
                AppLanguage::Ru => format!("Всего треков: {}   |   Время: {:.1} ч", total_played, total_hours),
                AppLanguage::En => format!("Total Tracks: {}   |   Time: {:.1} h", total_played, total_hours),
                AppLanguage::Ua => format!("Всього треків: {}   |   Час: {:.1} год", total_played, total_hours),
            };
            draw_text_mut(&mut img, white, 45, 82, PxScale::from(17.0), &font, &stat_line);

            let top_label = match lang {
                AppLanguage::Ru => "ТОП ИСПОЛНИТЕЛЕЙ:",
                AppLanguage::En => "TOP ARTISTS:",
                AppLanguage::Ua => "ТОП ВИКОНАВЦІВ:",
            };
            draw_text_mut(&mut img, gray, 45, 120, PxScale::from(14.0), &font, top_label);

            let max_val = top_artists.first().map(|x| x.1).unwrap_or(1) as f32;

            for (i, (artist, count)) in top_artists.iter().enumerate().take(5) {
                let y_base = 150 + (i as i32 * 46);
                let label = format!("#{} {} ({} tr.)", i + 1, artist, count);
                draw_text_mut(&mut img, white, 45, y_base, PxScale::from(16.0), &font, &label);

                let bar_width = (((width as f32 - 120.0) * (*count as f32 / max_val)) as u32).max(18);
                draw_filled_rect_mut(&mut img, Rect::at(45, y_base + 22).of_size(bar_width, 6), accent);
            }
        }
    }

    let save_path = PathBuf::from("soundcloud_stats.png");
    if img.save(&save_path).is_ok() {
        Some(save_path)
    } else {
        None
    }
}

#[derive(Clone)]
struct LrcLine {
    sec: f32,
    text: String,
}

#[derive(Clone)]
enum LyricsData {
    Synced(Vec<LrcLine>),
    Plain(String),
    NotFound,
}

fn parse_lrc(lrc_str: &str) -> Vec<LrcLine> {
    let mut lines = Vec::new();
    for line in lrc_str.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if let Some(close_b) = line.find(']') {
                let time_part = &line[1..close_b];
                let text = line[close_b + 1..].trim().to_string();

                if let Some((m, s)) = time_part.split_once(':') {
                    if let (Ok(mins), Ok(secs)) = (m.parse::<f32>(), s.parse::<f32>()) {
                        let total = mins * 60.0 + secs;
                        lines.push(LrcLine { sec: total, text });
                    }
                }
            }
        }
    }
    lines.sort_by(|a, b| a.sec.partial_cmp(&b.sec).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

fn fetch_lyrics_from_api(artist: &str, track: &str) -> LyricsData {
    let clean_track = track.split('(').next().unwrap_or(track).split('[').next().unwrap_or(track).trim();
    let query_url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        encode(clean_track),
        encode(artist)
    );

    let resp = match ureq::get(&query_url)
        .set("User-Agent", "zen_rpc/2.0")
        .timeout(Duration::from_secs(4))
        .call()
    {
        Ok(r) => r,
        Err(_) => return LyricsData::NotFound,
    };

    let items: serde_json::Value = match resp.into_json() {
        Ok(v) => v,
        Err(_) => return LyricsData::NotFound,
    };

    if let Some(arr) = items.as_array() {
        for item in arr {
            if let Some(synced) = item.get("syncedLyrics").and_then(|v| v.as_str()) {
                if !synced.trim().is_empty() {
                    let parsed = parse_lrc(synced);
                    if !parsed.is_empty() {
                        return LyricsData::Synced(parsed);
                    }
                }
            }
            if let Some(plain) = item.get("plainLyrics").and_then(|v| v.as_str()) {
                if !plain.trim().is_empty() {
                    return LyricsData::Plain(plain.trim().to_string());
                }
            }
        }
    }
    LyricsData::NotFound
}

fn render_neon_equalizer(ui: &mut egui::Ui, is_playing: bool, accent_color: egui::Color32, anim_time: f64) {
    let desired_width = 280.0;
    let desired_height = 24.0;

    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(desired_width, desired_height), egui::Sense::hover());

    let num_bars = 28;
    let bar_width = 5.5;
    let gap = (desired_width - (num_bars as f32 * bar_width)) / (num_bars as f32 - 1.0);

    for i in 0..num_bars {
        let x = rect.min.x + (i as f32 * (bar_width + gap));
        
        let height_factor = if is_playing {
            let t = anim_time as f32;
            let wave1 = ((t * 4.2 + (i as f32 * 0.35)).sin() * 0.5 + 0.5) * 0.65;
            let wave2 = ((t * 7.5 - (i as f32 * 0.5)).cos().abs()) * 0.35;
            (wave1 + wave2).clamp(0.12, 0.98)
        } else {
            0.08
        };

        let bar_height = rect.height() * height_factor;
        let y = rect.max.y - bar_height;

        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x, y),
            egui::pos2(x + bar_width, rect.max.y),
        );

        let alpha = if is_playing {
            (150 + ((height_factor * 105.0) as u8)).min(255)
        } else {
            60
        };

        let color = egui::Color32::from_rgba_unmultiplied(
            accent_color.r(),
            accent_color.g(),
            accent_color.b(),
            alpha,
        );

        ui.painter().rect_filled(bar_rect, 2.5, color);
    }
}

fn render_background_particles(
    painter: &egui::Painter,
    screen_rect: egui::Rect,
    accent: egui::Color32,
    anim_time: f64,
    mouse_pos: Option<egui::Pos2>,
    direction_up: bool,
) {
    let t = anim_time as f32;
    let count = 36;
    let w = screen_rect.width();
    let h = screen_rect.height();

    for i in 0..count {
        let seed = i as f32 * 19.341;
        let speed = 25.0 + (seed.sin().abs() * 30.0);
        let base_x = (seed.cos() * 0.5 + 0.5) * w + (t * (seed * 0.2).sin() * 12.0);
        let mut x = (base_x % w + w) % w + screen_rect.min.x;

        let mut y = if direction_up {
            screen_rect.max.y - ((t * speed + seed * 85.0) % h)
        } else {
            screen_rect.min.y + ((t * speed + seed * 85.0) % h)
        };

        if let Some(mp) = mouse_pos {
            let dx = x - mp.x;
            let dy = y - mp.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 65.0 && dist > 1.0 {
                let push = (65.0 - dist) * 0.35;
                x += (dx / dist) * push;
                y += (dy / dist) * push;
            }
        }

        let radius = 1.2 + ((seed * 0.7 + t * 2.0).sin().abs() * 1.6);
        let alpha = (35.0 + ((seed * 0.9 + t * 3.0).sin().abs() * 85.0)) as u8;
        let color = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha);

        painter.circle_filled(egui::pos2(x, y), radius, color);
    }
}

fn render_vinyl_disk(
    ui: &mut egui::Ui,
    img_source: egui::Image,
    is_playing: bool,
    anim_time: f64,
    accent: egui::Color32,
) -> egui::Rect {
    let size = 135.0;
    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let center = rect.center();
    let radius = size / 2.0;

    let painter = ui.painter();

    painter.circle_stroke(center, radius, egui::Stroke::new(1.8_f32, accent));
    painter.circle_filled(center, radius - 1.0, egui::Color32::from_rgb(14, 14, 16));

    for r in [radius * 0.88, radius * 0.77, radius * 0.66] {
        painter.circle_stroke(center, r, egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18)));
    }

    let angle = if is_playing { (anim_time as f32 * 2.5) % (std::f32::consts::TAU) } else { 0.0 };

    let glare_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 25);
    let glare_pos1 = center + egui::vec2(angle.cos() * (radius * 0.7), angle.sin() * (radius * 0.7));
    let glare_pos2 = center - egui::vec2(angle.cos() * (radius * 0.7), angle.sin() * (radius * 0.7));
    painter.line_segment([center, glare_pos1], egui::Stroke::new(2.5_f32, glare_color));
    painter.line_segment([center, glare_pos2], egui::Stroke::new(2.5_f32, glare_color));

    let inner_size = size * 0.44;
    let inner_rect = egui::Rect::from_center_size(center, egui::vec2(inner_size, inner_size));

    img_source
        .fit_to_exact_size(egui::vec2(inner_size, inner_size))
        .rounding(inner_size / 2.0)
        .paint_at(ui, inner_rect);

    painter.circle_filled(center, 4.0, egui::Color32::from_rgb(220, 220, 230));
    painter.circle_stroke(center, 4.0, egui::Stroke::new(1.0_f32, egui::Color32::BLACK));

    rect
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Player,
    Stats,
    History,
    Lyrics,
    Settings,
}

#[derive(Clone)]
struct AppState {
    is_enabled: Arc<AtomicBool>,
    status_text: Arc<Mutex<String>>,
    current_track: Arc<Mutex<String>>,
    current_artist: Arc<Mutex<String>>,
    current_title: Arc<Mutex<String>>,
    progress_ratio: Arc<Mutex<f32>>,
    progress_text: Arc<Mutex<String>>,
    current_sec: Arc<Mutex<f32>>,
    track_count: Arc<AtomicU32>,
    theme: Arc<Mutex<AppTheme>>,
    language: Arc<Mutex<AppLanguage>>,
    custom_accent: Arc<Mutex<[u8; 3]>>,
    custom_bg: Arc<Mutex<[u8; 3]>>,
    enable_particles: Arc<AtomicBool>,
    particles_direction_up: Arc<AtomicBool>,
    enable_vinyl: Arc<AtomicBool>,
    enable_equalizer: Arc<AtomicBool>,
    custom_image_bytes: Arc<Mutex<Option<Vec<u8>>>>,
    custom_image_path: Arc<Mutex<Option<String>>>,
    image_version: Arc<AtomicU32>,
    history: Arc<Mutex<Vec<HistoryEntry>>>,
    active_tab: Arc<Mutex<ActiveTab>>,
    export_notify: Arc<Mutex<Option<String>>>,
    copy_notify: Arc<Mutex<Option<f64>>>,
    lyrics_data: Arc<Mutex<LyricsData>>,
    lyrics_artist: Arc<Mutex<String>>,
    lyrics_track: Arc<Mutex<String>>,
    is_loading_lyrics: Arc<AtomicBool>,
    shared_track_payload: Arc<Mutex<Option<(IncomingTrackPayload, Instant)>>>,
}

impl AppState {
    fn save_current_config(&self) {
        let cfg = AppConfig {
            theme: *self.theme.lock().unwrap(),
            language: *self.language.lock().unwrap(),
            custom_accent: *self.custom_accent.lock().unwrap(),
            custom_bg: *self.custom_bg.lock().unwrap(),
            enable_particles: self.enable_particles.load(Ordering::SeqCst),
            particles_direction_up: self.particles_direction_up.load(Ordering::SeqCst),
            enable_vinyl: self.enable_vinyl.load(Ordering::SeqCst),
            enable_equalizer: self.enable_equalizer.load(Ordering::SeqCst),
            total_tracks_played: self.track_count.load(Ordering::SeqCst),
            custom_image_path: self.custom_image_path.lock().unwrap().clone(),
            history: self.history.lock().unwrap().clone(),
        };
        cfg.save();
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let current_theme = *self.theme.lock().unwrap();
        let current_lang = *self.language.lock().unwrap();
        let c_accent = *self.custom_accent.lock().unwrap();
        let c_bg = *self.custom_bg.lock().unwrap();

        let anim_time = ctx.input(|i| i.time);
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());

        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::from_rgb(240, 235, 240));
        visuals.panel_fill = current_theme.bg_color(c_bg);
        visuals.window_fill = current_theme.bg_color(c_bg);
        visuals.widgets.noninteractive.bg_fill = current_theme.widget_bg(c_bg);
        visuals.selection.bg_fill = current_theme.primary_accent(c_accent);
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.enable_particles.load(Ordering::SeqCst) {
                render_background_particles(
                    ui.painter(),
                    ui.max_rect(),
                    current_theme.primary_accent(c_accent),
                    anim_time,
                    mouse_pos,
                    self.particles_direction_up.load(Ordering::SeqCst),
                );
            }

            ui.vertical_centered(|ui| {
                ui.add_space(4.0);

                let mut current_tab = *self.active_tab.lock().unwrap();

                ui.horizontal(|ui| {
                    if ui.selectable_label(current_tab == ActiveTab::Player, I18n::tab_player(current_lang)).clicked() {
                        current_tab = ActiveTab::Player;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Lyrics, I18n::tab_karaoke(current_lang)).clicked() {
                        current_tab = ActiveTab::Lyrics;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Stats, I18n::tab_top(current_lang)).clicked() {
                        current_tab = ActiveTab::Stats;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::History, I18n::tab_history(current_lang)).clicked() {
                        current_tab = ActiveTab::History;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Settings, "⚙").clicked() {
                        current_tab = ActiveTab::Settings;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                });

                ui.add_space(6.0);

                match current_tab {
                    ActiveTab::Lyrics => {
                        let cur_artist = self.lyrics_artist.lock().unwrap().clone();
                        let cur_track = self.lyrics_track.lock().unwrap().clone();
                        let is_loading = self.is_loading_lyrics.load(Ordering::SeqCst);
                        let lyrics = self.lyrics_data.lock().unwrap().clone();
                        let current_s = *self.current_sec.lock().unwrap();

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{} — {}", cur_track, cur_artist))
                                        .strong()
                                        .size(13.5)
                                        .color(current_theme.secondary_accent(c_accent)),
                                );
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("🔄").on_hover_text("Refresh").clicked() {
                                    let a_clone = cur_artist.clone();
                                    let t_clone = cur_track.clone();
                                    let lyr_arc = self.lyrics_data.clone();
                                    let load_arc = self.is_loading_lyrics.clone();
                                    load_arc.store(true, Ordering::SeqCst);
                                    thread::spawn(move || {
                                        let res = fetch_lyrics_from_api(&a_clone, &t_clone);
                                        *lyr_arc.lock().unwrap() = res;
                                        load_arc.store(false, Ordering::SeqCst);
                                    });
                                }
                            });
                        });

                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);

                        if is_loading {
                            ui.add_space(30.0);
                            ui.spinner();
                            ui.label(egui::RichText::new("Loading lyrics...").color(egui::Color32::GRAY));
                        } else {
                            match lyrics {
                                LyricsData::Synced(lines) => {
                                    let mut active_idx = 0;
                                    for (i, line) in lines.iter().enumerate() {
                                        if current_s >= line.sec {
                                            active_idx = i;
                                        } else {
                                            break;
                                        }
                                    }

                                    egui::ScrollArea::vertical()
                                        .max_height(350.0)
                                        .show(ui, |ui| {
                                            ui.set_width(330.0);
                                            ui.vertical_centered(|ui| {
                                                for (i, line) in lines.iter().enumerate() {
                                                    let is_active = i == active_idx;
                                                    let label = if is_active {
                                                        egui::RichText::new(&line.text)
                                                            .strong()
                                                            .size(15.5)
                                                            .color(current_theme.primary_accent(c_accent))
                                                    } else {
                                                        egui::RichText::new(&line.text)
                                                            .size(13.0)
                                                            .color(egui::Color32::from_rgb(135, 130, 145))
                                                    };

                                                    let resp = ui.label(label);
                                                    if is_active {
                                                        resp.scroll_to_me(Some(egui::Align::Center));
                                                    }
                                                    ui.add_space(6.0);
                                                }
                                            });
                                        });
                                }
                                LyricsData::Plain(text) => {
                                    egui::ScrollArea::vertical()
                                        .max_height(350.0)
                                        .show(ui, |ui| {
                                            ui.set_width(330.0);
                                            ui.label(
                                                egui::RichText::new(I18n::lyrics_plain_hint(current_lang))
                                                    .size(11.0)
                                                    .color(egui::Color32::GRAY),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(egui::RichText::new(text).size(13.0).color(egui::Color32::from_rgb(235, 230, 240)));
                                        });
                                }
                                LyricsData::NotFound => {
                                    ui.add_space(30.0);
                                    ui.label(egui::RichText::new(I18n::lyrics_not_found(current_lang)).color(egui::Color32::GRAY));
                                    ui.add_space(10.0);
                                    if ui.button("🔍 Search in Genius / Google").clicked() {
                                        let q = format!("{} {} lyrics", cur_artist, cur_track);
                                        let url = format!("https://www.google.com/search?q={}", encode(&q));
                                        let _ = std::process::Command::new("cmd")
                                            .args(["/C", "start", "", &url])
                                            .spawn();
                                    }
                                }
                            }
                        }
                    }

                    ActiveTab::Settings => {
                        ui.label(
                            egui::RichText::new(I18n::settings_title(current_lang))
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent(c_accent)),
                        );
                        ui.add_space(8.0);

                        ui.group(|ui| {
                            ui.set_width(330.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(I18n::settings_lang(current_lang)).strong());
                                let mut lang_val = *self.language.lock().unwrap();
                                let prev_lang = lang_val;

                                egui::ComboBox::from_id_source("settings_lang_select")
                                    .selected_text(lang_val.label())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut lang_val, AppLanguage::Ru, AppLanguage::Ru.label());
                                        ui.selectable_value(&mut lang_val, AppLanguage::En, AppLanguage::En.label());
                                        ui.selectable_value(&mut lang_val, AppLanguage::Ua, AppLanguage::Ua.label());
                                    });

                                if lang_val != prev_lang {
                                    *self.language.lock().unwrap() = lang_val;
                                    self.save_current_config();
                                }
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(I18n::settings_theme(current_lang)).strong());
                                let mut theme_val = *self.theme.lock().unwrap();
                                let prev_theme = theme_val;

                                egui::ComboBox::from_id_source("settings_theme_select")
                                    .selected_text(theme_val.label(current_lang))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut theme_val, AppTheme::Kawaii, AppTheme::Kawaii.label(current_lang));
                                        ui.selectable_value(&mut theme_val, AppTheme::Neverlose, AppTheme::Neverlose.label(current_lang));
                                        ui.selectable_value(&mut theme_val, AppTheme::ShadowFiend, AppTheme::ShadowFiend.label(current_lang));
                                        ui.selectable_value(&mut theme_val, AppTheme::SoundCloud, AppTheme::SoundCloud.label(current_lang));
                                        ui.selectable_value(&mut theme_val, AppTheme::Custom, AppTheme::Custom.label(current_lang));
                                    });

                                if theme_val != prev_theme {
                                    *self.theme.lock().unwrap() = theme_val;
                                    self.save_current_config();
                                }
                            });

                            if *self.theme.lock().unwrap() == AppTheme::Custom {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(I18n::custom_accent_label(current_lang));
                                    let mut mut_accent = *self.custom_accent.lock().unwrap();
                                    let mut color_32 = egui::Color32::from_rgb(mut_accent[0], mut_accent[1], mut_accent[2]);
                                    if ui.color_edit_button_srgba(&mut color_32).changed() {
                                        mut_accent = [color_32.r(), color_32.g(), color_32.b()];
                                        *self.custom_accent.lock().unwrap() = mut_accent;
                                        self.save_current_config();
                                    }

                                    ui.add_space(8.0);
                                    ui.label(I18n::custom_bg_label(current_lang));
                                    let mut mut_bg = *self.custom_bg.lock().unwrap();
                                    let mut color_bg32 = egui::Color32::from_rgb(mut_bg[0], mut_bg[1], mut_bg[2]);
                                    if ui.color_edit_button_srgba(&mut color_bg32).changed() {
                                        mut_bg = [color_bg32.r(), color_bg32.g(), color_bg32.b()];
                                        *self.custom_bg.lock().unwrap() = mut_bg;
                                        self.save_current_config();
                                    }
                                });
                            }

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                ui.label(I18n::toggle_particles(current_lang));
                                let mut p_val = self.enable_particles.load(Ordering::SeqCst);
                                if ui.checkbox(&mut p_val, "").changed() {
                                    self.enable_particles.store(p_val, Ordering::SeqCst);
                                    self.save_current_config();
                                }
                            });

                            if self.enable_particles.load(Ordering::SeqCst) {
                                ui.add_space(3.0);
                                ui.horizontal(|ui| {
                                    ui.label(I18n::particles_dir_label(current_lang));
                                    let mut is_up = self.particles_direction_up.load(Ordering::SeqCst);
                                    let prev_dir = is_up;

                                    let dir_text = if is_up { I18n::dir_up(current_lang) } else { I18n::dir_down(current_lang) };

                                    egui::ComboBox::from_id_source("particles_direction_select")
                                        .selected_text(dir_text)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut is_up, true, I18n::dir_up(current_lang));
                                            ui.selectable_value(&mut is_up, false, I18n::dir_down(current_lang));
                                        });

                                    if is_up != prev_dir {
                                        self.particles_direction_up.store(is_up, Ordering::SeqCst);
                                        self.save_current_config();
                                    }
                                });
                            }

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(I18n::toggle_vinyl(current_lang));
                                let mut v_val = self.enable_vinyl.load(Ordering::SeqCst);
                                if ui.checkbox(&mut v_val, "").changed() {
                                    self.enable_vinyl.store(v_val, Ordering::SeqCst);
                                    self.save_current_config();
                                }
                            });

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(I18n::toggle_eq(current_lang));
                                let mut eq_val = self.enable_equalizer.load(Ordering::SeqCst);
                                if ui.checkbox(&mut eq_val, "").changed() {
                                    self.enable_equalizer.store(eq_val, Ordering::SeqCst);
                                    self.save_current_config();
                                }
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                ui.label(I18n::settings_clear_hist(current_lang));
                                if ui.small_button(I18n::btn_reset(current_lang)).clicked() {
                                    self.history.lock().unwrap().clear();
                                    self.track_count.store(0, Ordering::SeqCst);
                                    self.save_current_config();
                                }
                            });
                        });
                    }

                    ActiveTab::Stats => {
                        let hist = self.history.lock().unwrap().clone();
                        let total_played = self.track_count.load(Ordering::SeqCst);

                        let mut artist_counts: HashMap<String, usize> = HashMap::new();
                        let mut total_seconds: u64 = 0;

                        for entry in &hist {
                            *artist_counts.entry(entry.artist.clone()).or_insert(0) += 1;
                            total_seconds += entry.duration_sec.unwrap_or(150);
                        }

                        let mut sorted_artists: Vec<(String, usize)> = artist_counts.into_iter().collect();
                        
                        sorted_artists.sort_by(|a, b| {
                            b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))
                        });

                        let top_artist = sorted_artists.first().cloned();
                        let total_hours = total_seconds as f32 / 3600.0;

                        ui.label(
                            egui::RichText::new(I18n::stats_header(current_lang))
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent(c_accent)),
                        );
                        ui.add_space(4.0);

                        ui.group(|ui| {
                            ui.set_width(330.0);
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(I18n::stats_total(current_lang, total_played))
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.label(
                                        egui::RichText::new(I18n::stats_time(current_lang, total_hours))
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(180, 175, 195)),
                                    );
                                });

                                ui.separator();

                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(I18n::stats_fav_artist(current_lang))
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(180, 175, 195)),
                                    );
                                    let fav_name = match &top_artist {
                                        Some((name, cnt)) => format!("{} ({}×)", name, cnt),
                                        None => "—".to_string(),
                                    };
                                    ui.label(
                                        egui::RichText::new(fav_name)
                                            .strong()
                                            .color(current_theme.primary_accent(c_accent)),
                                    );
                                });
                            });
                        });

                        ui.add_space(6.0);

                        if ui
                            .add_sized(
                                [260.0, 26.0],
                                egui::Button::new(
                                    egui::RichText::new(I18n::btn_share_top(current_lang))
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(current_theme.primary_accent(c_accent))
                                .rounding(6.0),
                            )
                            .clicked()
                        {
                            if let Some(path) = generate_stats_card(total_played, total_hours, &sorted_artists, current_theme, c_accent, c_bg, current_lang) {
                                *self.export_notify.lock().unwrap() = Some("soundcloud_stats.png".to_string());
                                let _ = std::process::Command::new("cmd")
                                    .args(["/C", "start", "", &path.to_string_lossy()])
                                    .spawn();
                            }
                        }

                        if let Some(ref msg) = *self.export_notify.lock().unwrap() {
                            ui.label(egui::RichText::new(format!("Saved: {}", msg)).size(11.0).color(current_theme.primary_accent(c_accent)));
                        }

                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(I18n::top_artists_header(current_lang))
                                .strong()
                                .size(13.0)
                                .color(egui::Color32::from_rgb(220, 215, 230)),
                        );

                        egui::ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                if sorted_artists.is_empty() {
                                    ui.label(egui::RichText::new(I18n::history_empty(current_lang)).color(egui::Color32::GRAY));
                                } else {
                                    let max_val = sorted_artists.first().map(|x| x.1).unwrap_or(1) as f32;
                                    for (i, (artist, count)) in sorted_artists.iter().enumerate().take(15) {
                                        ui.group(|ui| {
                                            ui.set_width(330.0);
                                            ui.horizontal(|ui| {
                                                let rank_badge = match i {
                                                    0 => "🥇",
                                                    1 => "🥈",
                                                    2 => "🥉",
                                                    _ => "•",
                                                };
                                                ui.label(egui::RichText::new(format!("{} #{}:", rank_badge, i + 1)).strong());
                                                ui.label(
                                                    egui::RichText::new(artist)
                                                        .strong()
                                                        .color(current_theme.secondary_accent(c_accent)),
                                                );
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(
                                                        egui::RichText::new(I18n::tracks_word(current_lang, *count))
                                                            .size(11.5)
                                                            .color(egui::Color32::from_rgb(170, 160, 185)),
                                                    );
                                                });
                                            });

                                            let ratio = (*count as f32 / max_val).clamp(0.05, 1.0);
                                            ui.add(
                                                egui::ProgressBar::new(ratio)
                                                    .desired_height(4.0)
                                                    .fill(current_theme.primary_accent(c_accent))
                                                    .rounding(4.0),
                                            );
                                        });
                                        ui.add_space(2.0);
                                    }
                                }
                            });
                    }

                    ActiveTab::History => {
                        ui.label(
                            egui::RichText::new(I18n::tab_history(current_lang))
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent(c_accent)),
                        );
                        ui.add_space(6.0);

                        egui::ScrollArea::vertical()
                            .max_height(350.0)
                            .show(ui, |ui| {
                                let hist = self.history.lock().unwrap().clone();
                                if hist.is_empty() {
                                    ui.label(egui::RichText::new(I18n::history_empty(current_lang)).color(egui::Color32::GRAY));
                                } else {
                                    for item in hist.iter().rev() {
                                        ui.group(|ui| {
                                            ui.set_width(330.0);
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(&item.track)
                                                            .strong()
                                                            .size(13.0)
                                                            .color(current_theme.secondary_accent(c_accent)),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!("{} • {}", item.artist, item.played_at))
                                                            .size(11.0)
                                                            .color(egui::Color32::from_rgb(160, 150, 175)),
                                                    );
                                                });

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.small_button("🔍").on_hover_text("Search SoundCloud").clicked() {
                                                        let q = format!("{} {}", item.artist, item.track);
                                                        let url = format!("https://soundcloud.com/search/sounds?q={}", encode(&q));
                                                        let _ = std::process::Command::new("cmd")
                                                            .args(["/C", "start", "", &url])
                                                            .spawn();
                                                    }
                                                });
                                            });
                                        });
                                        ui.add_space(2.0);
                                    }
                                }
                            });
                    }

                    ActiveTab::Player => {
                        let is_on = self.is_enabled.load(Ordering::SeqCst);
                        let ratio = *self.progress_ratio.lock().unwrap();
                        let is_playing = is_on && ratio > 0.0;

                        let img_version = self.image_version.load(Ordering::SeqCst);
                        let uri = format!("bytes://avatar_{}.png", img_version);

                        let img_source = {
                            let guard = self.custom_image_bytes.lock().unwrap();
                            match &*guard {
                                Some(custom) => egui::Image::from_bytes(uri, custom.clone()),
                                None => egui::Image::from_bytes("bytes://sc.png", SC_IMAGE_BYTES),
                            }
                        };

                        let disk_rect = if self.enable_vinyl.load(Ordering::SeqCst) {
                            render_vinyl_disk(ui, img_source, is_playing, anim_time, current_theme.primary_accent(c_accent))
                        } else {
                            let img_size = egui::vec2(125.0, 125.0);
                            let (rect, _resp) = ui.allocate_exact_size(img_size, egui::Sense::hover());
                            img_source
                                .fit_to_exact_size(img_size)
                                .rounding(14.0)
                                .paint_at(ui, rect);
                            rect
                        };

                        let badge_size = 26.0;
                        let badge_pos = disk_rect.right_top() - egui::vec2(badge_size - 3.0, -3.0);
                        let badge_rect = egui::Rect::from_min_size(badge_pos, egui::vec2(badge_size, badge_size));
                        let badge_resp = ui.interact(badge_rect, ui.id().with("badge_change_pic"), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Change avatar");

                        let badge_color = if badge_resp.hovered() {
                            current_theme.primary_accent(c_accent)
                        } else {
                            current_theme.widget_bg(c_bg)
                        };

                        ui.painter().circle_filled(badge_rect.center(), 13.0, badge_color);
                        ui.painter().circle_stroke(badge_rect.center(), 13.0, egui::Stroke::new(1.5_f32, current_theme.primary_accent(c_accent)));

                        ui.painter().text(
                            badge_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "🖼",
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        );

                        if badge_resp.clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                                .pick_file()
                            {
                                if let Ok(bytes) = fs::read(&path) {
                                    *self.custom_image_bytes.lock().unwrap() = Some(bytes);
                                    *self.custom_image_path.lock().unwrap() = Some(path.to_string_lossy().to_string());
                                    self.image_version.fetch_add(1, Ordering::SeqCst);
                                    self.save_current_config();
                                }
                            }
                        }

                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("SoundCloud RPC")
                                .strong()
                                .size(16.0)
                                .color(current_theme.secondary_accent(c_accent)),
                        );

                        ui.add_space(6.0);

                        let (btn_text, btn_color) = if is_on {
                            (I18n::status_active(current_lang), current_theme.primary_accent(c_accent))
                        } else {
                            (I18n::status_disabled(current_lang), current_theme.widget_bg(c_bg))
                        };

                        if ui
                            .add_sized(
                                [220.0, 25.0],
                                egui::Button::new(
                                    egui::RichText::new(btn_text)
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(btn_color)
                                .rounding(8.0),
                            )
                            .clicked()
                        {
                            self.is_enabled.store(!is_on, Ordering::SeqCst);
                        }

                        ui.add_space(4.0);

                        if ui.add_sized(
                            [220.0, 23.0],
                            egui::Button::new(
                                egui::RichText::new(I18n::btn_minimize(current_lang))
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(195, 185, 205)),
                            )
                            .fill(current_theme.widget_bg(c_bg))
                            .rounding(6.0),
                        ).clicked() {
                            hide_app_window();
                        }

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(2.0);

                        let count = self.track_count.load(Ordering::SeqCst);
                        ui.label(
                            egui::RichText::new(I18n::total_played(current_lang, count))
                                .size(11.5)
                                .color(current_theme.secondary_accent(c_accent))
                                .strong(),
                        );

                        ui.add_space(2.0);

                        let track = self.current_track.lock().unwrap().clone();
                        let artist_raw = self.current_artist.lock().unwrap().clone();
                        let title_raw = self.current_title.lock().unwrap().clone();

                        ui.scope(|ui| {
                            let row_width = 300.0;
                            ui.set_max_width(row_width);

                            ui.horizontal(|ui| {
                                let has_meta = !artist_raw.is_empty() && !title_raw.is_empty();
                                let btn_width = if has_meta { 26.0 } else { 0.0 };
                                let text_width = (row_width - btn_width - 8.0).max(60.0);

                                let label = egui::Label::new(
                                    egui::RichText::new(&track)
                                        .strong()
                                        .size(13.0)
                                        .color(egui::Color32::WHITE),
                                )
                                .truncate(true);

                                ui.add_sized([text_width, 20.0], label)
                                    .on_hover_text(&track);

                                if has_meta {
                                    if ui.small_button("📋").on_hover_text("Копировать ссылку для друзей").clicked() {
                                        let q = format!("{} {}", artist_raw, title_raw);
                                        let share_link = format!("🎧 Слушаю: {} — {} | https://soundcloud.com/search/sounds?q={}", artist_raw, title_raw, encode(&q));
                                        ctx.output_mut(|o| o.copied_text = share_link);
                                        *self.copy_notify.lock().unwrap() = Some(ctx.input(|i| i.time));
                                    }
                                }
                            });
                        });

                        let now_t = ctx.input(|i| i.time);
                        if let Some(t_copied) = *self.copy_notify.lock().unwrap() {
                            if now_t - t_copied < 2.0 {
                                ui.label(egui::RichText::new(I18n::copy_success(current_lang)).size(11.0).color(current_theme.primary_accent(c_accent)));
                            }
                        }

                        ui.add_space(4.0);

                        let p_text = self.progress_text.lock().unwrap().clone();

                        let bar = egui::ProgressBar::new(ratio)
                            .text(egui::RichText::new(p_text).size(10.5).color(egui::Color32::WHITE))
                            .fill(current_theme.primary_accent(c_accent))
                            .rounding(8.0);

                        ui.add(bar);

                        if self.enable_equalizer.load(Ordering::SeqCst) {
                            ui.add_space(6.0);
                            render_neon_equalizer(ui, is_playing, current_theme.primary_accent(c_accent), anim_time);
                        }
                    }
                }
            });
        });

        let is_on = self.is_enabled.load(Ordering::SeqCst);
        let ratio = *self.progress_ratio.lock().unwrap();
        let particles_on = self.enable_particles.load(Ordering::SeqCst);

        if (is_on && ratio > 0.0) || particles_on {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let cfg = AppConfig::load();

    let tray_menu = Menu::new();
    let show_item = MenuItem::new("Открыть SoundCloud RPC", true, None);
    let quit_item = MenuItem::new("Выход", true, None);
    let _ = tray_menu.append_items(&[&show_item, &quit_item]);

    let _tray_icon = load_tray_icon().and_then(|icon| {
        TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("SoundCloud RPC")
            .with_icon(icon)
            .build()
            .ok()
    });

    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    thread::spawn(move || {
        loop {
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                } = event
                {
                    restore_app_window();
                }
            }

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == show_id {
                    restore_app_window();
                } else if event.id == quit_id {
                    std::process::exit(0);
                }
            }

            thread::sleep(Duration::from_millis(50));
        }
    });

    let mut custom_bytes = None;
    if let Some(ref path_str) = cfg.custom_image_path {
        if let Ok(bytes) = fs::read(PathBuf::from(path_str)) {
            custom_bytes = Some(bytes);
        }
    }

    let shared_payload = Arc::new(Mutex::new(None));

    let state = AppState {
        is_enabled: Arc::new(AtomicBool::new(true)),
        status_text: Arc::new(Mutex::new("Запуск...".to_string())),
        current_track: Arc::new(Mutex::new("Ожидание трека...".to_string())),
        current_artist: Arc::new(Mutex::new("".to_string())),
        current_title: Arc::new(Mutex::new("".to_string())),
        progress_ratio: Arc::new(Mutex::new(0.0)),
        progress_text: Arc::new(Mutex::new("00:00 / 00:00".to_string())),
        current_sec: Arc::new(Mutex::new(0.0)),
        track_count: Arc::new(AtomicU32::new(cfg.total_tracks_played)),
        theme: Arc::new(Mutex::new(cfg.theme)),
        language: Arc::new(Mutex::new(cfg.language)),
        custom_accent: Arc::new(Mutex::new(cfg.custom_accent)),
        custom_bg: Arc::new(Mutex::new(cfg.custom_bg)),
        enable_particles: Arc::new(AtomicBool::new(cfg.enable_particles)),
        particles_direction_up: Arc::new(AtomicBool::new(cfg.particles_direction_up)),
        enable_vinyl: Arc::new(AtomicBool::new(cfg.enable_vinyl)),
        enable_equalizer: Arc::new(AtomicBool::new(cfg.enable_equalizer)),
        custom_image_bytes: Arc::new(Mutex::new(custom_bytes)),
        custom_image_path: Arc::new(Mutex::new(cfg.custom_image_path)),
        image_version: Arc::new(AtomicU32::new(0)),
        history: Arc::new(Mutex::new(cfg.history)),
        active_tab: Arc::new(Mutex::new(ActiveTab::Player)),
        export_notify: Arc::new(Mutex::new(None)),
        copy_notify: Arc::new(Mutex::new(None)),
        lyrics_data: Arc::new(Mutex::new(LyricsData::NotFound)),
        lyrics_artist: Arc::new(Mutex::new("".to_string())),
        lyrics_track: Arc::new(Mutex::new("".to_string())),
        is_loading_lyrics: Arc::new(AtomicBool::new(false)),
        shared_track_payload: shared_payload.clone(),
    };

    // Фоновый HTTP сервер (принимает JSON от Tampermonkey)
    let http_payload_arc = shared_payload.clone();
    thread::spawn(move || {
        if let Ok(server) = Server::http("127.0.0.1:23456") {
            for mut request in server.incoming_requests() {
                if request.method().as_str() == "OPTIONS" {
                    let mut resp = Response::from_string("ok");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, OPTIONS"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if request.url() == "/update" && request.method().as_str() == "POST" {
                    let mut content = String::new();
                    if request.as_reader().read_to_string(&mut content).is_ok() {
                        if let Ok(data) = serde_json::from_str::<IncomingTrackPayload>(&content) {
                            *http_payload_arc.lock().unwrap() = Some((data, Instant::now()));
                        }
                    }
                    let mut resp = Response::from_string("{\"status\":\"ok\"}");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                } else {
                    let _ = request.respond(Response::from_string("404").with_status_code(404));
                }
            }
        }
    });

    // Фоновый поток обновления Discord Rich Presence
    let bg_state = state.clone();
    thread::spawn(move || {
        let mut client = DiscordIpcClient::new(CLIENT_ID).ok();
        let mut is_connected = false;
        let mut active_track: Option<String> = None;
        let mut active_artist: Option<String> = None;
        let mut last_passed_sec: Option<u64> = None;
        let mut track_counted = false;

        loop {
            let enabled = bg_state.is_enabled.load(Ordering::SeqCst);

            if !enabled {
                if active_track.is_some() {
                    if let Some(ref mut ipc) = client {
                        let _ = ipc.clear_activity();
                    }
                    active_track = None;
                    active_artist = None;
                    last_passed_sec = None;
                    track_counted = false;
                }
                *bg_state.status_text.lock().unwrap() = "На паузе".to_string();
                thread::sleep(Duration::from_millis(400));
                continue;
            }

            if !is_connected {
                if let Some(ref mut ipc) = client {
                    if ipc.connect().is_ok() {
                        is_connected = true;
                        active_track = None;
                        active_artist = None;
                        last_passed_sec = None;
                        track_counted = false;
                    } else {
                        *bg_state.status_text.lock().unwrap() = "Ожидание Discord...".to_string();
                        thread::sleep(Duration::from_millis(600));
                        continue;
                    }
                }
            }

            let maybe_data = {
                let guard = bg_state.shared_track_payload.lock().unwrap();
                guard.clone()
            };

            let valid_payload = match maybe_data {
                Some((data, updated_at)) if updated_at.elapsed() < Duration::from_secs(3) => {
                    if data.is_playing && !data.track.is_empty() {
                        Some(data)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            match valid_payload {
                Some(payload) => {
                    let track_str = payload.track.trim().to_string();
                    let artist_str = payload.artist.trim().to_string();
                    let p = payload.passed;
                    let d = payload.duration;

                    *bg_state.current_artist.lock().unwrap() = artist_str.clone();
                    *bg_state.current_title.lock().unwrap() = track_str.clone();
                    *bg_state.current_track.lock().unwrap() = format!("{} — {}", track_str, artist_str);
                    *bg_state.status_text.lock().unwrap() = "Воспроизведение".to_string();

                    let ratio = if d > 0 { (p as f32 / d as f32).clamp(0.0, 1.0) } else { 0.0 };
                    *bg_state.progress_ratio.lock().unwrap() = ratio;
                    *bg_state.progress_text.lock().unwrap() = format!("{} / {}", format_duration(p), format_duration(d));
                    *bg_state.current_sec.lock().unwrap() = p as f32;

                    if !track_counted && d >= 20 {
                        if (d > p && (d - p) <= 4) || (p as f32 / d as f32 >= 0.85) {
                            track_counted = true;
                            bg_state.track_count.fetch_add(1, Ordering::SeqCst);

                            {
                                let mut hist = bg_state.history.lock().unwrap();
                                let now_str = Local::now().format("%H:%M").to_string();
                                hist.push(HistoryEntry {
                                    track: track_str.clone(),
                                    artist: artist_str.clone(),
                                    played_at: now_str,
                                    duration_sec: Some(d),
                                });
                                if hist.len() > 200 {
                                    hist.remove(0);
                                }
                            }
                            bg_state.save_current_config();
                        }
                    }

                    let track_changed = active_track.as_deref() != Some(&track_str) || active_artist.as_deref() != Some(&artist_str);
                    let seeked = match (Some(p), last_passed_sec) {
                        (Some(curr), Some(prev)) => (curr as i64 - prev as i64).abs() > 2,
                        _ => false,
                    };

                    if track_changed {
                        track_counted = false;
                        *bg_state.lyrics_artist.lock().unwrap() = artist_str.clone();
                        *bg_state.lyrics_track.lock().unwrap() = track_str.clone();
                        let a_clone = artist_str.clone();
                        let t_clone = track_str.clone();
                        let lyr_arc = bg_state.lyrics_data.clone();
                        let load_arc = bg_state.is_loading_lyrics.clone();

                        load_arc.store(true, Ordering::SeqCst);
                        thread::spawn(move || {
                            let res = fetch_lyrics_from_api(&a_clone, &t_clone);
                            *lyr_arc.lock().unwrap() = res;
                            load_arc.store(false, Ordering::SeqCst);
                        });
                    }

                    if track_changed || seeked {
                        active_track = Some(track_str.clone());
                        active_artist = Some(artist_str.clone());
                        last_passed_sec = Some(p);

                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;

                        let count_val = bg_state.track_count.load(Ordering::SeqCst);
                        let state_str = if count_val > 0 {
                            format!("by {} • [#{}]", artist_str, count_val)
                        } else {
                            format!("by {}", artist_str)
                        };

                        let small_txt = format!("Tracks: {}", count_val);

                        let start_time = now - p as i64;
                        let end_time = start_time + d as i64;
                        let timestamps = activity::Timestamps::new().start(start_time).end(end_time);

                        let large_img = if payload.cover.starts_with("http") && payload.cover.len() <= 256 {
                            payload.cover.as_str()
                        } else {
                            FALLBACK_LARGE_IMAGE
                        };

                        let discord_payload = activity::Activity::new()
                            .activity_type(activity::ActivityType::Listening)
                            .details(&track_str)
                            .state(&state_str)
                            .assets(
                                activity::Assets::new()
                                    .large_image(large_img)
                                    .large_text(&track_str)
                                    .small_image(PLAY_IMAGE_KEY)
                                    .small_text(&small_txt),
                            )
                            .timestamps(timestamps);

                        if let Some(ref mut ipc) = client {
                            if ipc.set_activity(discord_payload).is_err() {
                                is_connected = false;
                                client = DiscordIpcClient::new(CLIENT_ID).ok();
                            }
                        }
                    } else {
                        last_passed_sec = Some(p);
                    }
                }
                None => {
                    if active_track.is_some() {
                        active_track = None;
                        active_artist = None;
                        last_passed_sec = None;
                        track_counted = false;
                        if let Some(ref mut ipc) = client {
                            let _ = ipc.clear_activity();
                        }
                    }
                    *bg_state.status_text.lock().unwrap() = "Ожидание трека".to_string();
                    *bg_state.current_track.lock().unwrap() = "Включите музыку в SoundCloud".to_string();
                    *bg_state.current_artist.lock().unwrap() = "".to_string();
                    *bg_state.current_title.lock().unwrap() = "".to_string();
                    *bg_state.progress_ratio.lock().unwrap() = 0.0;
                    *bg_state.progress_text.lock().unwrap() = "00:00 / 00:00".to_string();
                    *bg_state.current_sec.lock().unwrap() = 0.0;
                }
            }

            thread::sleep(Duration::from_millis(500));
        }
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([370.0, 480.0])
        .with_resizable(false)
        .with_maximize_button(false);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let app_ui = state.clone();
    eframe::run_native(
        "SoundCloud RPC",
        options,
        Box::new(move |_cc| {
            egui_extras::install_image_loaders(&_cc.egui_ctx);
            Box::new(app_ui) as Box<dyn eframe::App>
        }),
    )
}