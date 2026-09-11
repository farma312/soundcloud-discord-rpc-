#![windows_subsystem = "windows"]

use ab_glyph::{FontRef, PxScale};
use chrono::Local;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use eframe::egui;
use image::{ImageBuffer, Rgba};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::System;
use urlencoding::encode;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsHungAppWindow,
    IsWindowVisible,
};

const CLIENT_ID: &str = "1249851004971651174";
const FALLBACK_LARGE_IMAGE: &str = "browser_icon";
const PLAY_IMAGE_KEY: &str = "play";
const UPDATE_INTERVAL_MS: u64 = 800;
const CONFIG_FILE: &str = "config.json";

const SC_IMAGE_BYTES: &[u8] = include_bytes!("sc.png");

fn load_app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(SC_IMAGE_BYTES).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppTheme {
    Kawaii,
    Neverlose,
    ShadowFiend,
    SoundCloud,
}

impl AppTheme {
    fn label(&self) -> &'static str {
        match self {
            AppTheme::Kawaii => "🌸 Кавайная (Розовая)",
            AppTheme::Neverlose => "❄️ Neverlose (Cyan)",
            AppTheme::ShadowFiend => "🔥 Shadow Fiend (Ruby)",
            AppTheme::SoundCloud => "⚡ SoundCloud (Orange)",
        }
    }

    fn bg_color(&self) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(22, 18, 26),
            AppTheme::Neverlose => egui::Color32::from_rgb(11, 14, 20),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(20, 16, 19),
            AppTheme::SoundCloud => egui::Color32::from_rgb(22, 22, 24),
        }
    }

    fn widget_bg(&self) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(32, 26, 38),
            AppTheme::Neverlose => egui::Color32::from_rgb(18, 23, 33),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(32, 24, 30),
            AppTheme::SoundCloud => egui::Color32::from_rgb(34, 34, 36),
        }
    }

    fn primary_accent(&self) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(230, 95, 140),
            AppTheme::Neverlose => egui::Color32::from_rgb(0, 225, 255),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(230, 36, 68),
            AppTheme::SoundCloud => egui::Color32::from_rgb(255, 85, 0),
        }
    }

    fn secondary_accent(&self) -> egui::Color32 {
        match self {
            AppTheme::Kawaii => egui::Color32::from_rgb(245, 185, 205),
            AppTheme::Neverlose => egui::Color32::from_rgb(130, 235, 255),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(255, 120, 145),
            AppTheme::SoundCloud => egui::Color32::from_rgb(255, 150, 70),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum BrowserTarget {
    Auto,
    Zen,
    Chrome,
    Opera,
    Yandex,
    Edge,
    Firefox,
}

impl BrowserTarget {
    fn label(&self) -> &'static str {
        match self {
            BrowserTarget::Auto => "Авто (Все браузеры)",
            BrowserTarget::Zen => "Zen Browser",
            BrowserTarget::Chrome => "Google Chrome",
            BrowserTarget::Opera => "Opera / Opera GX",
            BrowserTarget::Yandex => "Яндекс Браузер",
            BrowserTarget::Edge => "Microsoft Edge",
            BrowserTarget::Firefox => "Mozilla Firefox",
        }
    }

    fn matches(&self, proc_name: &str) -> bool {
        let name = proc_name.to_lowercase();
        match self {
            BrowserTarget::Auto => {
                name.contains("zen")
                    || name.contains("chrome")
                    || name.contains("opera")
                    || name.contains("browser")
                    || name.contains("msedge")
                    || name.contains("firefox")
            }
            BrowserTarget::Zen => name.contains("zen"),
            BrowserTarget::Chrome => name.contains("chrome"),
            BrowserTarget::Opera => name.contains("opera"),
            BrowserTarget::Yandex => name.contains("browser"),
            BrowserTarget::Edge => name.contains("msedge"),
            BrowserTarget::Firefox => name.contains("firefox"),
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
    selected_browser: BrowserTarget,
    theme: AppTheme,
    total_tracks_played: u32,
    custom_image_path: Option<String>,
    history: Vec<HistoryEntry>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            selected_browser: BrowserTarget::Auto,
            theme: AppTheme::Kawaii,
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

struct WindowSearchContext {
    pids: HashSet<u32>,
    windows: Vec<(usize, String)>,
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut WindowSearchContext);

    if IsWindowVisible(hwnd).as_bool() && !IsHungAppWindow(hwnd).as_bool() {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        if ctx.pids.contains(&pid) {
            let length = GetWindowTextLengthW(hwnd);
            if length > 0 {
                let mut buffer: Vec<u16> = vec![0; (length + 1) as usize];
                let copied = GetWindowTextW(hwnd, &mut buffer);
                if copied > 0 {
                    buffer.truncate(copied as usize);
                    if let Ok(title) = OsString::from_wide(&buffer).into_string() {
                        ctx.windows.push((hwnd.0 as usize, title));
                    }
                }
            }
        }
    }
    BOOL(1)
}

fn get_browser_pids(sys: &mut System, target: BrowserTarget) -> HashSet<u32> {
    sys.refresh_processes();
    sys.processes()
        .iter()
        .filter_map(|(pid, proc_info)| {
            if target.matches(proc_info.name()) {
                Some(pid.as_u32())
            } else {
                None
            }
        })
        .collect()
}

fn get_browser_windows(pids: &HashSet<u32>) -> Vec<(usize, String)> {
    if pids.is_empty() {
        return Vec::new();
    }

    let mut ctx = WindowSearchContext {
        pids: pids.clone(),
        windows: Vec::new(),
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }

    ctx.windows
}

fn read_hwnd_title(hwnd_raw: usize) -> Option<String> {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsHungAppWindow(hwnd).as_bool() {
            return None;
        }
        let length = GetWindowTextLengthW(hwnd);
        if length > 0 {
            let mut buffer: Vec<u16> = vec![0; (length + 1) as usize];
            let copied = GetWindowTextW(hwnd, &mut buffer);
            if copied > 0 {
                buffer.truncate(copied as usize);
                return OsString::from_wide(&buffer).into_string().ok();
            }
        }
    }
    None
}

fn parse_time_to_seconds(time_str: &str) -> Option<u64> {
    let parts: Vec<&str> = time_str.trim().split(':').collect();
    match parts.len() {
        2 => {
            let mins: u64 = parts[0].parse().ok()?;
            let secs: u64 = parts[1].parse().ok()?;
            Some(mins * 60 + secs)
        }
        3 => {
            let hours: u64 = parts[0].parse().ok()?;
            let mins: u64 = parts[1].parse().ok()?;
            let secs: u64 = parts[2].parse().ok()?;
            Some(hours * 3600 + mins * 60 + secs)
        }
        _ => None,
    }
}

fn parse_soundcloud_title(title: &str) -> Option<(String, String, Option<u64>, Option<u64>, Option<String>)> {
    let mut clean = title.trim().to_string();

    let mut passed_sec = None;
    let mut total_sec = None;
    let mut cover_url = None;
    let mut explicit_artist = None;
    let mut explicit_track = None;

    if let Some(open_b) = clean.find('[') {
        if let Some(close_b) = clean[open_b..].find(']') {
            let close_idx = open_b + close_b;
            let timing_part = &clean[open_b + 1..close_idx];
            if let Some((p, d)) = timing_part.split_once('/') {
                let p_parsed = parse_time_to_seconds(p);
                let d_parsed = parse_time_to_seconds(d);
                if p_parsed.is_some() && d_parsed.is_some() {
                    passed_sec = p_parsed;
                    total_sec = d_parsed;
                    clean = format!("{}{}", &clean[..open_b], &clean[close_idx + 1..]);
                }
            }
        }
    }

    if let Some(start_idx) = clean.find("<<<") {
        if let Some(end_idx) = clean.find(">>>") {
            if end_idx > start_idx + 3 {
                let url = clean[start_idx + 3..end_idx].trim().to_string();
                if url.starts_with("http") {
                    cover_url = Some(url);
                }
                clean = format!("{}{}", &clean[..start_idx], &clean[end_idx + 3..]);
            }
        }
    }

    if let Some(start_idx) = clean.find("|IMG:") {
        let rest = &clean[start_idx + 5..];
        if let Some(end_rel) = rest.find('|') {
            let url = rest[..end_rel].trim().to_string();
            if url.starts_with("http") {
                cover_url = Some(url);
            }
            clean = format!("{}{}", &clean[..start_idx], &rest[end_rel + 1..]);
        }
    }

    if let Some(start_idx) = clean.find("[[[") {
        if let Some(end_idx) = clean.find("]]]") {
            if end_idx > start_idx + 3 {
                let meta = &clean[start_idx + 3..end_idx];
                if let Some((a, t)) = meta.split_once(":::") {
                    let a_str = a.trim();
                    let t_str = t.trim();
                    if !a_str.is_empty() && !t_str.is_empty() {
                        explicit_artist = Some(a_str.to_string());
                        explicit_track = Some(t_str.to_string());
                    }
                }
                clean = format!("{}{}", &clean[..start_idx], &clean[end_idx + 3..]);
            }
        }
    }

    if let (Some(a), Some(t)) = (explicit_artist, explicit_track) {
        return Some((t, a, passed_sec, total_sec, cover_url));
    }

    let junks = [
        "— Zen Browser", "- Zen Browser", "— Zen", "- Zen",
        "— Google Chrome", "- Google Chrome",
        "— Opera", "- Opera", "— Opera GX", "- Opera GX",
        "— Microsoft Edge", "- Microsoft Edge",
        "— Brave", "- Brave",
        "— Mozilla Firefox", "- Mozilla Firefox",
        "| Stream free on SoundCloud", "| Listen free on SoundCloud",
        "| SoundCloud", "- SoundCloud",
    ];

    for junk in &junks {
        clean = clean.replace(junk, "");
    }

    clean = clean.trim().to_string();

    if let Some((track, artist)) = clean.rsplit_once(" by ") {
        let t = track.trim();
        let a = artist.trim();
        if !t.is_empty() && !a.is_empty() {
            return Some((t.to_string(), a.to_string(), passed_sec, total_sec, cover_url));
        }
    }

    if let Some((track, playlist)) = clean.rsplit_once(" in ") {
        let t = track.trim();
        let p = playlist.trim();
        if !t.is_empty() && !p.is_empty() {
            return Some((t.to_string(), p.to_string(), passed_sec, total_sec, cover_url));
        }
    }

    None
}

fn format_str(text: &str) -> String {
    let mut trimmed = text.trim().trim_matches(&['«', '»', '"', '\''][..]).trim();
    if let Some(stripped) = trimmed.strip_prefix("Current track:") {
        trimmed = stripped.trim();
    }
    if trimmed.is_empty() {
        "Неизвестно".to_string()
    } else if trimmed.chars().count() < 2 {
        format!("{} ", trimmed)
    } else {
        trimmed.to_string()
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
) -> Option<PathBuf> {
    let width = 640u32;
    let height = 420u32;
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([22, 18, 26, 255]));

    let (bg_r, bg_g, bg_b) = match theme {
        AppTheme::Kawaii => (24, 18, 28),
        AppTheme::Neverlose => (11, 16, 26),
        AppTheme::ShadowFiend => (22, 14, 18),
        AppTheme::SoundCloud => (24, 20, 18),
    };

    let (ac_r, ac_g, ac_b) = match theme {
        AppTheme::Kawaii => (230, 95, 140),
        AppTheme::Neverlose => (0, 225, 255),
        AppTheme::ShadowFiend => (230, 36, 68),
        AppTheme::SoundCloud => (255, 85, 0),
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

            let stat_line = format!("Total Tracks: {}   |   Time in Music: {:.1} h", total_played, total_hours);
            draw_text_mut(&mut img, white, 45, 82, PxScale::from(17.0), &font, &stat_line);

            draw_text_mut(&mut img, gray, 45, 120, PxScale::from(14.0), &font, "TOP ARTISTS:");

            let max_val = top_artists.first().map(|x| x.1).unwrap_or(1) as f32;

            for (i, (artist, count)) in top_artists.iter().enumerate().take(5) {
                let y_base = 150 + (i as i32 * 46);
                let label = format!("#{} {} ({} tracks)", i + 1, artist, count);
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
        .set("User-Agent", "zen_rpc/1.0")
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
    locked_hwnd: Arc<Mutex<Option<usize>>>,
    status_text: Arc<Mutex<String>>,
    current_track: Arc<Mutex<String>>,
    progress_ratio: Arc<Mutex<f32>>,
    progress_text: Arc<Mutex<String>>,
    current_sec: Arc<Mutex<f32>>,
    track_count: Arc<AtomicU32>,
    selected_browser: Arc<Mutex<BrowserTarget>>,
    theme: Arc<Mutex<AppTheme>>,
    custom_image_bytes: Arc<Mutex<Option<Vec<u8>>>>,
    custom_image_path: Arc<Mutex<Option<String>>>,
    image_version: Arc<AtomicU32>,
    history: Arc<Mutex<Vec<HistoryEntry>>>,
    active_tab: Arc<Mutex<ActiveTab>>,
    export_notify: Arc<Mutex<Option<String>>>,
    lyrics_data: Arc<Mutex<LyricsData>>,
    lyrics_artist: Arc<Mutex<String>>,
    lyrics_track: Arc<Mutex<String>>,
    is_loading_lyrics: Arc<AtomicBool>,
}

impl AppState {
    fn save_current_config(&self) {
        let cfg = AppConfig {
            selected_browser: *self.selected_browser.lock().unwrap(),
            theme: *self.theme.lock().unwrap(),
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

        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::from_rgb(240, 235, 240));
        visuals.panel_fill = current_theme.bg_color();
        visuals.window_fill = current_theme.bg_color();
        visuals.widgets.noninteractive.bg_fill = current_theme.widget_bg();
        visuals.selection.bg_fill = current_theme.primary_accent();
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);

                let mut current_tab = *self.active_tab.lock().unwrap();

                ui.horizontal(|ui| {
                    if ui.selectable_label(current_tab == ActiveTab::Player, "🎵 Плеер").clicked() {
                        current_tab = ActiveTab::Player;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Lyrics, "🎤 Караоке").clicked() {
                        current_tab = ActiveTab::Lyrics;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Stats, "📊 Топ").clicked() {
                        current_tab = ActiveTab::Stats;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::History, "📜 История").clicked() {
                        current_tab = ActiveTab::History;
                        *self.active_tab.lock().unwrap() = current_tab;
                    }
                    if ui.selectable_label(current_tab == ActiveTab::Settings, "⚙").on_hover_text("Настройки").clicked() {
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
                                        .color(current_theme.secondary_accent()),
                                );
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("🔄").on_hover_text("Обновить текст песни").clicked() {
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
                            ui.label(egui::RichText::new("Поиск синхронизированного текста...").color(egui::Color32::GRAY));
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
                                                            .color(current_theme.primary_accent())
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
                                                egui::RichText::new("ℹ️ Обычный текст (без таймкодов караоке):")
                                                    .size(11.0)
                                                    .color(egui::Color32::GRAY),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(egui::RichText::new(text).size(13.0).color(egui::Color32::from_rgb(235, 230, 240)));
                                        });
                                }
                                LyricsData::NotFound => {
                                    ui.add_space(30.0);
                                    ui.label(egui::RichText::new("Текст не найден. Возможно, это инструментал или эксклюзив SoundCloud.").color(egui::Color32::GRAY));
                                    ui.add_space(10.0);
                                    if ui.button("🔍 Искать в Genius / Google").clicked() {
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
                            egui::RichText::new("Настройки приложения")
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent()),
                        );
                        ui.add_space(10.0);

                        ui.group(|ui| {
                            ui.set_width(330.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Тема интерфейса:").strong());
                                let mut theme_val = *self.theme.lock().unwrap();
                                let prev_theme = theme_val;

                                egui::ComboBox::from_id_source("settings_theme_select")
                                    .selected_text(theme_val.label())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut theme_val, AppTheme::Kawaii, AppTheme::Kawaii.label());
                                        ui.selectable_value(&mut theme_val, AppTheme::Neverlose, AppTheme::Neverlose.label());
                                        ui.selectable_value(&mut theme_val, AppTheme::ShadowFiend, AppTheme::ShadowFiend.label());
                                        ui.selectable_value(&mut theme_val, AppTheme::SoundCloud, AppTheme::SoundCloud.label());
                                    });

                                if theme_val != prev_theme {
                                    *self.theme.lock().unwrap() = theme_val;
                                    self.save_current_config();
                                }
                            });

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Браузер:").strong());
                                let mut current_target = *self.selected_browser.lock().unwrap();
                                let prev_target = current_target;

                                egui::ComboBox::from_id_source("settings_browser_select")
                                    .selected_text(current_target.label())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut current_target, BrowserTarget::Auto, BrowserTarget::Auto.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Zen, BrowserTarget::Zen.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Chrome, BrowserTarget::Chrome.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Opera, BrowserTarget::Opera.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Yandex, BrowserTarget::Yandex.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Edge, BrowserTarget::Edge.label());
                                        ui.selectable_value(&mut current_target, BrowserTarget::Firefox, BrowserTarget::Firefox.label());
                                    });

                                if current_target != prev_target {
                                    *self.selected_browser.lock().unwrap() = current_target;
                                    *self.locked_hwnd.lock().unwrap() = None;
                                    self.save_current_config();
                                }
                            });

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.label("Очистить историю треков:");
                                if ui.small_button("🗑 Сбросить").on_hover_text("Очистить историю и счётчики").clicked() {
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
                        sorted_artists.sort_by(|a, b| b.1.cmp(&a.1));

                        let top_artist = sorted_artists.first().cloned();
                        let total_hours = total_seconds as f32 / 3600.0;

                        ui.label(
                            egui::RichText::new("Музыкальная статистика")
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent()),
                        );
                        ui.add_space(4.0);

                        ui.group(|ui| {
                            ui.set_width(330.0);
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("Всего треков: {}", total_played))
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.label(
                                        egui::RichText::new(format!("Время: {:.1} ч", total_hours))
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(180, 175, 195)),
                                    );
                                });

                                ui.separator();

                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("🔥 Топ артист:")
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
                                            .color(current_theme.primary_accent()),
                                    );
                                });
                            });
                        });

                        ui.add_space(6.0);

                        if ui
                            .add_sized(
                                [260.0, 26.0],
                                egui::Button::new(
                                    egui::RichText::new("📸 Поделиться топом (Создать карточку)")
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(current_theme.primary_accent())
                                .rounding(6.0),
                            )
                            .clicked()
                        {
                            if let Some(path) = generate_stats_card(total_played, total_hours, &sorted_artists, current_theme) {
                                *self.export_notify.lock().unwrap() = Some("Карточка сохранена: soundcloud_stats.png".to_string());
                                let _ = std::process::Command::new("cmd")
                                    .args(["/C", "start", "", &path.to_string_lossy()])
                                    .spawn();
                            }
                        }

                        if let Some(ref msg) = *self.export_notify.lock().unwrap() {
                            ui.label(egui::RichText::new(msg).size(11.0).color(current_theme.primary_accent()));
                        }

                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Топ исполнителей")
                                .strong()
                                .size(13.0)
                                .color(egui::Color32::from_rgb(220, 215, 230)),
                        );

                        egui::ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                if sorted_artists.is_empty() {
                                    ui.label(egui::RichText::new("Пока мало данных для топа. Врубай треки!").color(egui::Color32::GRAY));
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
                                                        .color(current_theme.secondary_accent()),
                                                );
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(
                                                        egui::RichText::new(format!("{} треков", count))
                                                            .size(11.5)
                                                            .color(egui::Color32::from_rgb(170, 160, 185)),
                                                    );
                                                });
                                            });

                                            let ratio = (*count as f32 / max_val).clamp(0.05, 1.0);
                                            ui.add(
                                                egui::ProgressBar::new(ratio)
                                                    .desired_height(4.0)
                                                    .fill(current_theme.primary_accent())
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
                            egui::RichText::new("История прослушиваний")
                                .strong()
                                .size(15.0)
                                .color(current_theme.secondary_accent()),
                        );
                        ui.add_space(6.0);

                        egui::ScrollArea::vertical()
                            .max_height(350.0)
                            .show(ui, |ui| {
                                let hist = self.history.lock().unwrap().clone();
                                if hist.is_empty() {
                                    ui.label(egui::RichText::new("История пока пуста...").color(egui::Color32::GRAY));
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
                                                            .color(current_theme.secondary_accent()),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!("{} • {}", item.artist, item.played_at))
                                                            .size(11.0)
                                                            .color(egui::Color32::from_rgb(160, 150, 175)),
                                                    );
                                                });

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.small_button("🔍").on_hover_text("Искать в SoundCloud").clicked() {
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
                        let img_version = self.image_version.load(Ordering::SeqCst);
                        let uri = format!("bytes://avatar_{}.png", img_version);

                        let img_source = {
                            let guard = self.custom_image_bytes.lock().unwrap();
                            match &*guard {
                                Some(custom) => egui::Image::from_bytes(uri, custom.clone()),
                                None => egui::Image::from_bytes("bytes://sc.png", SC_IMAGE_BYTES),
                            }
                        };

                        let img_size = egui::vec2(130.0, 130.0);
                        let (rect, _resp) = ui.allocate_exact_size(img_size, egui::Sense::hover());

                        img_source
                            .fit_to_exact_size(img_size)
                            .rounding(14.0)
                            .paint_at(ui, rect);

                        let badge_size = 28.0;
                        let badge_pos = rect.right_top() - egui::vec2(badge_size - 4.0, -4.0);
                        let badge_rect = egui::Rect::from_min_size(badge_pos, egui::vec2(badge_size, badge_size));
                        let badge_resp = ui.interact(badge_rect, ui.id().with("badge_change_pic"), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Сменить аватарку");

                        let badge_color = if badge_resp.hovered() {
                            current_theme.primary_accent()
                        } else {
                            current_theme.widget_bg()
                        };

                        ui.painter().circle_filled(badge_rect.center(), 14.0, badge_color);
                        ui.painter().circle_stroke(badge_rect.center(), 14.0, egui::Stroke::new(1.5, current_theme.primary_accent()));

                        ui.painter().text(
                            badge_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "🖼",
                            egui::FontId::proportional(13.0),
                            egui::Color32::WHITE,
                        );

                        if badge_resp.clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Изображения", &["png", "jpg", "jpeg", "webp"])
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

                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("SoundCloud RPC")
                                .strong()
                                .size(16.0)
                                .color(current_theme.secondary_accent()),
                        );

                        ui.add_space(8.0);

                        let is_on = self.is_enabled.load(Ordering::SeqCst);
                        let (btn_text, btn_color) = if is_on {
                            ("Статус: Активен", current_theme.primary_accent())
                        } else {
                            ("Статус: Отключен", current_theme.widget_bg())
                        };

                        if ui
                            .add_sized(
                                [220.0, 26.0],
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

                        ui.add_space(6.0);

                        let locked = self.locked_hwnd.lock().unwrap().is_some();
                        let lock_text = if locked {
                            "Окно привязано (Сбросить?)"
                        } else {
                            "Привязать окно браузера"
                        };

                        let lock_btn = egui::Button::new(
                            egui::RichText::new(lock_text)
                                .size(12.0)
                                .color(if locked {
                                    current_theme.primary_accent()
                                } else {
                                    egui::Color32::from_rgb(175, 160, 185)
                                }),
                        )
                        .fill(current_theme.widget_bg())
                        .rounding(8.0);

                        if ui.add(lock_btn).clicked() {
                            let mut lock_guard = self.locked_hwnd.lock().unwrap();
                            if lock_guard.is_some() {
                                *lock_guard = None;
                            } else {
                                let target = *self.selected_browser.lock().unwrap();
                                let mut sys = System::new_all();
                                let pids = get_browser_pids(&mut sys, target);
                                let all_windows = get_browser_windows(&pids);
                                for (hwnd, title) in all_windows {
                                    if parse_soundcloud_title(&title).is_some() || title.to_lowercase().contains("soundcloud") {
                                        *lock_guard = Some(hwnd);
                                        break;
                                    }
                                }
                            }
                        }

                        ui.add_space(6.0);

                        if ui.add_sized(
                            [220.0, 24.0],
                            egui::Button::new(
                                egui::RichText::new("— Свернуть в фон")
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(195, 185, 205)),
                            )
                            .fill(current_theme.widget_bg())
                            .rounding(6.0),
                        ).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);

                        let count = self.track_count.load(Ordering::SeqCst);
                        ui.label(
                            egui::RichText::new(format!("Всего дослушано треков: {}", count))
                                .size(12.0)
                                .color(current_theme.secondary_accent())
                                .strong(),
                        );

                        ui.add_space(2.0);
                        let track = self.current_track.lock().unwrap().clone();
                        ui.label(
                            egui::RichText::new(track)
                                .strong()
                                .size(13.5)
                                .color(egui::Color32::WHITE),
                        );

                        ui.add_space(6.0);

                        let ratio = *self.progress_ratio.lock().unwrap();
                        let p_text = self.progress_text.lock().unwrap().clone();

                        let bar = egui::ProgressBar::new(ratio)
                            .text(egui::RichText::new(p_text).size(11.0).color(egui::Color32::WHITE))
                            .fill(current_theme.primary_accent())
                            .rounding(8.0);

                        ui.add(bar);
                    }
                }
            });
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn main() -> Result<(), eframe::Error> {
    let cfg = AppConfig::load();

    let mut custom_bytes = None;
    if let Some(ref path_str) = cfg.custom_image_path {
        if let Ok(bytes) = fs::read(PathBuf::from(path_str)) {
            custom_bytes = Some(bytes);
        }
    }

    let state = AppState {
        is_enabled: Arc::new(AtomicBool::new(true)),
        locked_hwnd: Arc::new(Mutex::new(None)),
        status_text: Arc::new(Mutex::new("Запуск...".to_string())),
        current_track: Arc::new(Mutex::new("Ожидание трека...".to_string())),
        progress_ratio: Arc::new(Mutex::new(0.0)),
        progress_text: Arc::new(Mutex::new("00:00 / 00:00".to_string())),
        current_sec: Arc::new(Mutex::new(0.0)),
        track_count: Arc::new(AtomicU32::new(cfg.total_tracks_played)),
        selected_browser: Arc::new(Mutex::new(cfg.selected_browser)),
        theme: Arc::new(Mutex::new(cfg.theme)),
        custom_image_bytes: Arc::new(Mutex::new(custom_bytes)),
        custom_image_path: Arc::new(Mutex::new(cfg.custom_image_path)),
        image_version: Arc::new(AtomicU32::new(0)),
        history: Arc::new(Mutex::new(cfg.history)),
        active_tab: Arc::new(Mutex::new(ActiveTab::Player)),
        export_notify: Arc::new(Mutex::new(None)),
        lyrics_data: Arc::new(Mutex::new(LyricsData::NotFound)),
        lyrics_artist: Arc::new(Mutex::new("".to_string())),
        lyrics_track: Arc::new(Mutex::new("".to_string())),
        is_loading_lyrics: Arc::new(AtomicBool::new(false)),
    };

    let bg_state = state.clone();
    thread::spawn(move || {
        let mut sys = System::new_all();
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
                        thread::sleep(Duration::from_millis(UPDATE_INTERVAL_MS));
                        continue;
                    }
                }
            }

            let current_target = *bg_state.selected_browser.lock().unwrap();
            let browser_pids = get_browser_pids(&mut sys, current_target);

            if browser_pids.is_empty() {
                if active_track.is_some() {
                    active_track = None;
                    active_artist = None;
                    last_passed_sec = None;
                    track_counted = false;
                    if let Some(ref mut ipc) = client {
                        let _ = ipc.clear_activity();
                    }
                }
                *bg_state.locked_hwnd.lock().unwrap() = None;
                *bg_state.status_text.lock().unwrap() = "Браузер не запущен".to_string();
                *bg_state.current_track.lock().unwrap() = "Браузер закрыт".to_string();
                *bg_state.progress_ratio.lock().unwrap() = 0.0;
                *bg_state.progress_text.lock().unwrap() = "00:00 / 00:00".to_string();
                *bg_state.current_sec.lock().unwrap() = 0.0;
                thread::sleep(Duration::from_millis(UPDATE_INTERVAL_MS));
                continue;
            }

            let current_locked = *bg_state.locked_hwnd.lock().unwrap();

            let mut target_title: Option<String> = current_locked.and_then(read_hwnd_title);

            if target_title.as_deref().and_then(parse_soundcloud_title).is_none() {
                let all_windows = get_browser_windows(&browser_pids);
                for (hwnd, title) in all_windows {
                    if parse_soundcloud_title(&title).is_some() {
                        *bg_state.locked_hwnd.lock().unwrap() = Some(hwnd);
                        target_title = Some(title);
                        break;
                    }
                }
            }

            let parsed = target_title.as_deref().and_then(parse_soundcloud_title);

            match parsed {
                Some((raw_t, raw_a, passed_opt, total_opt, cover_opt)) => {
                    let track_str = format_str(&raw_t);
                    let artist_str = format_str(&raw_a);

                    *bg_state.current_track.lock().unwrap() = format!("{} — {}", track_str, artist_str);
                    *bg_state.status_text.lock().unwrap() = "Воспроизведение".to_string();

                    if let (Some(p), Some(d)) = (passed_opt, total_opt) {
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
                    }

                    let track_changed = active_track.as_deref() != Some(&track_str) || active_artist.as_deref() != Some(&artist_str);

                    let seeked = match (passed_opt, last_passed_sec) {
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
                        last_passed_sec = passed_opt;

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

                        let small_txt = format!("Дослушано треков: {}", count_val);

                        let mut timestamps = activity::Timestamps::new();
                        if let (Some(p), Some(d)) = (passed_opt, total_opt) {
                            let start_time = now - p as i64;
                            let end_time = start_time + d as i64;
                            timestamps = timestamps.start(start_time).end(end_time);
                        } else {
                            timestamps = timestamps.start(now);
                        }

                        let large_img = match cover_opt.as_deref() {
                            Some(url) if url.starts_with("http") && url.len() <= 256 => url,
                            _ => FALLBACK_LARGE_IMAGE,
                        };

                        let payload = activity::Activity::new()
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
                            if ipc.set_activity(payload).is_err() {
                                is_connected = false;
                                client = DiscordIpcClient::new(CLIENT_ID).ok();
                            }
                        }
                    } else {
                        last_passed_sec = passed_opt;
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
                    *bg_state.progress_ratio.lock().unwrap() = 0.0;
                    *bg_state.progress_text.lock().unwrap() = "00:00 / 00:00".to_string();
                    *bg_state.current_sec.lock().unwrap() = 0.0;
                }
            }

            thread::sleep(Duration::from_millis(UPDATE_INTERVAL_MS));
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