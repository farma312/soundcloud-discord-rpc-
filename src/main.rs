#![windows_subsystem = "windows"]

use ab_glyph::{FontRef, PxScale};
use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use eframe::egui;
use image::{ImageBuffer, Rgba};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
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
    EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_STYLE, HWND_TOP,
    SWP_NOMOVE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_RESTORE, SW_SHOW, WS_MAXIMIZEBOX,
    WS_THICKFRAME,
};

const CLIENT_ID: &str = "1249851004971651174";
const FALLBACK_LARGE_IMAGE: &str = "browser_icon";
const PLAY_IMAGE_KEY: &str = "play";
const CONFIG_FILE: &str = "config.json";

const SC_IMAGE_BYTES: &[u8] = include_bytes!("sc.png");
const BETA_HTML_CONTENT: &str = include_str!("beta_v2.html");

static SAVED_HWND: AtomicUsize = AtomicUsize::new(0);

fn default_true() -> bool {
    true
}

fn default_volume() -> f32 {
    1.0
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum UiLayoutMode {
    Spotify,
    Classic,
}

impl UiLayoutMode {
    fn label(&self, _lang: AppLanguage) -> &'static str {
        match self {
            UiLayoutMode::Spotify => "🟢 Spotify Modern",
            UiLayoutMode::Classic => "🎛 Classic Widget",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct UserPlaylist {
    #[serde(default)]
    id: u64,
    title: String,
    #[serde(default)]
    track_count: u32,
    permalink_url: String,
    #[serde(default)]
    artwork_url: String,
    #[serde(default)]
    tracks: Vec<SearchTrackItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct LrcLine {
    sec: f32,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct IncomingTrackPayload {
    track: String,
    artist: String,
    passed: u64,
    duration: u64,
    cover: String,
    is_playing: bool,
    #[serde(default)]
    repeat: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    user_name: String,
    #[serde(default)]
    user_url: String,
    #[serde(default)]
    user_avatar: String,
    #[serde(default = "default_volume")]
    volume: f32,
    #[serde(default)]
    playlists: Vec<UserPlaylist>,
    #[serde(default)]
    spectrum: Vec<u8>,
    #[serde(default)]
    liked: bool,
    #[serde(default)]
    followed: bool,
    #[serde(default)]
    lyrics_plain: String,
    #[serde(default)]
    lyrics_synced: Vec<LrcLine>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct SearchTrackItem {
    title: String,
    artist: String,
    duration_sec: u64,
    permalink_url: String,
    #[serde(default)]
    artwork_url: String,
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

fn launch_beta_hud() {
    let url = "http://127.0.0.1:23456/beta";

    let native_candidates = [
        PathBuf::from("cpp_hud\\build\\Release\\zen_beta_hud.exe"),
        PathBuf::from("cpp_hud\\build\\zen_beta_hud.exe"),
    ];
    for native_path in native_candidates {
        if native_path.exists() && std::process::Command::new(&native_path).spawn().is_ok() {
            return;
        }
    }

    let candidate_paths = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
    ];

    for path in candidate_paths {
        if std::path::Path::new(path).exists() {
            if let Ok(child) = std::process::Command::new(path)
                .args([format!("--app={}", url), "--window-size=1100,720".to_string()])
                .spawn()
            {
                let pid = child.id();
                thread::spawn(move || {
                    for _ in 0..40 {
                        let mut found = None;
                        struct WindowContext { pid: u32, found: Option<HWND> }
                        unsafe extern "system" fn find_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
                            let ctx = &mut *(lparam.0 as *mut WindowContext);
                            let mut window_pid = 0;
                            GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
                            if window_pid == ctx.pid && GetWindowTextLengthW(hwnd) > 0 {
                                ctx.found = Some(hwnd);
                                return BOOL(0);
                            }
                            BOOL(1)
                        }
                        let mut context = WindowContext { pid, found: None };
                        unsafe { let _ = EnumWindows(Some(find_window), LPARAM(&mut context as *mut _ as isize)); }
                        found = context.found;
                        if let Some(hwnd) = found {
                            unsafe {
                                let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                                let fixed_style = style & !(WS_THICKFRAME.0 as isize) & !(WS_MAXIMIZEBOX.0 as isize);
                                let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, fixed_style);
                                let _ = SetWindowPos(hwnd, HWND_TOP, 0, 0, 1100, 720, SWP_NOMOVE | SWP_NOZORDER | SWP_SHOWWINDOW);
                            }
                            break;
                        }
                        thread::sleep(Duration::from_millis(150));
                    }
                });
                return;
            }
        }
    }

    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
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
    fn tab_search(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🔍 Поиск",
            AppLanguage::En => "🔍 Search",
            AppLanguage::Ua => "🔍 Пошук",
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
    fn btn_prev(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Предыдущий трек",
            AppLanguage::En => "Previous track",
            AppLanguage::Ua => "Попередній трек",
        }
    }
    fn btn_next(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Следующий трек",
            AppLanguage::En => "Next track",
            AppLanguage::Ua => "Наступний трек",
        }
    }
    fn btn_play_pause(lang: AppLanguage, is_playing: bool) -> &'static str {
        match (lang, is_playing) {
            (AppLanguage::Ru, true) => "Пауза",
            (AppLanguage::Ru, false) => "Воспроизведение",
            (AppLanguage::En, true) => "Pause",
            (AppLanguage::En, false) => "Play",
            (AppLanguage::Ua, true) => "Пауза",
            (AppLanguage::Ua, false) => "Відтворення",
        }
    }
    fn btn_repeat(lang: AppLanguage, mode: &str) -> String {
        match lang {
            AppLanguage::Ru => match mode {
                "one" => "Повтор: этот трек (🔂)".to_string(),
                "all" => "Повтор: плейлист (🔁)".to_string(),
                _ => "Повтор выключен".to_string(),
            },
            AppLanguage::En => match mode {
                "one" => "Repeat: single track (🔂)".to_string(),
                "all" => "Repeat: playlist (🔁)".to_string(),
                _ => "Repeat: off".to_string(),
            },
            AppLanguage::Ua => match mode {
                "one" => "Повтор: цей трек (🔂)".to_string(),
                "all" => "Повтор: плейліст (🔁)".to_string(),
                _ => "Повтор вимкнено".to_string(),
            },
        }
    }
    fn search_title(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Поиск треков SoundCloud",
            AppLanguage::En => "SoundCloud Track Search",
            AppLanguage::Ua => "Пошук треків SoundCloud",
        }
    }
    fn search_hint(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Название трека или исполнитель...",
            AppLanguage::En => "Track title or artist name...",
            AppLanguage::Ua => "Назва треку або виконавець...",
        }
    }
    fn search_btn(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Найти",
            AppLanguage::En => "Search",
            AppLanguage::Ua => "Знайти",
        }
    }
    fn search_loading(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Ищем треки в SoundCloud...",
            AppLanguage::En => "Searching tracks on SoundCloud...",
            AppLanguage::Ua => "Шукаємо треки в SoundCloud...",
        }
    }
    fn search_empty(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Ничего не найдено по этому запросу",
            AppLanguage::En => "No tracks found for this query",
            AppLanguage::Ua => "Нічого не знайдено за цим запитом",
        }
    }
    fn search_start_prompt(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Введите название трека и нажмите Enter для поиска",
            AppLanguage::En => "Type a track name and press Enter to search",
            AppLanguage::Ua => "Введіть назву треку та натисніть Enter для пошуку",
        }
    }
    fn btn_play_track(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "▶ Включить",
            AppLanguage::En => "▶ Play",
            AppLanguage::Ua => "▶ Увімкнути",
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
    fn settings_title(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Настройки приложения",
            AppLanguage::En => "Application Settings",
            AppLanguage::Ua => "Налаштування застосунку",
        }
    }
    fn settings_layout(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Стиль макета:",
            AppLanguage::En => "Layout Mode:",
            AppLanguage::Ua => "Стиль макета:",
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
    fn settings_use_track_cover(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🖼 Обложка трека SoundCloud (по умолчанию):",
            AppLanguage::En => "🖼 SoundCloud track artwork (default):",
            AppLanguage::Ua => "🖼 Обкладинка треку SoundCloud (за замовчуванням):",
        }
    }
    fn hud_elements_label(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Элементы худа:",
            AppLanguage::En => "HUD Elements:",
            AppLanguage::Ua => "Елементи худа:",
        }
    }
    fn hud_open_btn(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Настроить ▾",
            AppLanguage::En => "Configure ▾",
            AppLanguage::Ua => "Налаштувати ▾",
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
            AppLanguage::Ru => "Направление частиц:",
            AppLanguage::En => "Particles direction:",
            AppLanguage::Ua => "Напрямок частинок:",
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
    fn toggle_eq(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "⚡ Неоновый эквалайзер:",
            AppLanguage::En => "⚡ Neon equalizer:",
            AppLanguage::Ua => "⚡ Неоновий еквалайзер:",
        }
    }
    fn profile_header(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Моя медиатека SoundCloud",
            AppLanguage::En => "My SoundCloud Library",
            AppLanguage::Ua => "Моя медіатека SoundCloud",
        }
    }
    fn profile_user_active(lang: AppLanguage, name: &str) -> String {
        match lang {
            AppLanguage::Ru => format!("Вы вошли как: {}", name),
            AppLanguage::En => format!("Logged in as: {}", name),
            AppLanguage::Ua => format!("Ви увійшли як: {}", name),
        }
    }
    fn profile_guest_hint(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Аккаунт не обнаружен. Убедитесь, что вы авторизованы в браузере.",
            AppLanguage::En => "No active account found. Make sure you are signed in on SoundCloud.",
            AppLanguage::Ua => "Акаунт не знайдено. Переконайтеся, що ви авторизовані в браузері.",
        }
    }
    fn btn_open_stream(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "⚡ Моя лента (Stream)",
            AppLanguage::En => "⚡ My Stream Feed",
            AppLanguage::Ua => "⚡ Моя стрічка (Stream)",
        }
    }
    fn btn_open_user_page(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "🌐 Моя страница профиля",
            AppLanguage::En => "🌐 My Profile Page",
            AppLanguage::Ua => "🌐 Моя сторінка профілю",
        }
    }
    fn playlists_header(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Мои плейлисты",
            AppLanguage::En => "My Playlists",
            AppLanguage::Ua => "Мої плейлісти",
        }
    }
    fn playlists_empty(lang: AppLanguage) -> &'static str {
        match lang {
            AppLanguage::Ru => "Плейлисты не обнаружены. Убедитесь, что SoundCloud открыт в браузере.",
            AppLanguage::En => "No playlists detected. Make sure SoundCloud is open in your browser.",
            AppLanguage::Ua => "Плейлісти не виявлено. Переконайтеся, що SoundCloud відкритий у браузері.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppTheme {
    Spotify,
    Kawaii,
    Neverlose,
    ShadowFiend,
    SoundCloud,
    Custom,
}

impl AppTheme {
    fn label(&self, lang: AppLanguage) -> &'static str {
        match self {
            AppTheme::Spotify => "🟢 Spotify (Green & Dark)",
            AppTheme::Kawaii => match lang {
                AppLanguage::Ru => "🌸 Кавайная (Розовая)",
                AppLanguage::En => "🌸 Kawaii (Pink)",
                AppLanguage::Ua => "🌸 Кавайна (Рожева)",
            },
            AppTheme::Neverlose => "❄️ Neverlose (Cyan)",
            AppTheme::ShadowFiend => "🔥 Shadow Fiend (Ruby)",
            AppTheme::SoundCloud => "⚡ SoundCloud (Orange)",
            AppTheme::Custom => "🎨 Своя тема",
        }
    }

    fn bg_color(&self, custom_bg: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Spotify => egui::Color32::from_rgb(18, 18, 18),
            AppTheme::Kawaii => egui::Color32::from_rgb(22, 18, 26),
            AppTheme::Neverlose => egui::Color32::from_rgb(11, 14, 20),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(20, 16, 19),
            AppTheme::SoundCloud => egui::Color32::from_rgb(22, 22, 24),
            AppTheme::Custom => egui::Color32::from_rgb(custom_bg[0], custom_bg[1], custom_bg[2]),
        }
    }

    fn widget_bg(&self, custom_bg: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Spotify => egui::Color32::from_rgb(32, 32, 32),
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
            AppTheme::Spotify => egui::Color32::from_rgb(30, 215, 96),
            AppTheme::Kawaii => egui::Color32::from_rgb(230, 95, 140),
            AppTheme::Neverlose => egui::Color32::from_rgb(0, 225, 255),
            AppTheme::ShadowFiend => egui::Color32::from_rgb(230, 36, 68),
            AppTheme::SoundCloud => egui::Color32::from_rgb(255, 85, 0),
            AppTheme::Custom => egui::Color32::from_rgb(custom_accent[0], custom_accent[1], custom_accent[2]),
        }
    }

    fn secondary_accent(&self, custom_accent: [u8; 3]) -> egui::Color32 {
        match self {
            AppTheme::Spotify => egui::Color32::from_rgb(160, 245, 185),
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
    #[serde(default)]
    cover: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum UserRole {
    Owner,
    Beta,
    User,
}

impl Default for UserRole {
    fn default() -> Self { UserRole::User }
}

impl UserRole {
    fn label(self) -> &'static str {
        match self {
            UserRole::Owner => "Owner",
            UserRole::Beta => "Beta",
            UserRole::User => "User",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default = "default_layout_spotify")]
    layout_mode: UiLayoutMode,
    theme: AppTheme,
    language: AppLanguage,
    #[serde(default)]
    role: UserRole,
    custom_accent: [u8; 3],
    custom_bg: [u8; 3],
    enable_particles: bool,
    particles_direction_up: bool,
    enable_equalizer: bool,
    #[serde(default = "default_true")]
    show_controls: bool,
    #[serde(default = "default_true")]
    show_search_tab: bool,
    #[serde(default = "default_true")]
    show_top_tab: bool,
    #[serde(default = "default_true")]
    show_history_tab: bool,
    #[serde(default = "default_true")]
    show_profile_tab: bool,
    #[serde(default = "default_true")]
    use_track_cover: bool,
    #[serde(default = "default_volume")]
    volume: f32,
    total_tracks_played: u32,
    custom_image_path: Option<String>,
    history: Vec<HistoryEntry>,
}

fn default_layout_spotify() -> UiLayoutMode {
    UiLayoutMode::Spotify
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            layout_mode: UiLayoutMode::Spotify,
            theme: AppTheme::Spotify,
            language: AppLanguage::Ru,
            role: UserRole::User,
            custom_accent: [30, 215, 96],
            custom_bg: [18, 18, 18],
            enable_particles: false,
            particles_direction_up: true,
            enable_equalizer: true,
            show_controls: true,
            show_search_tab: true,
            show_top_tab: true,
            show_history_tab: true,
            show_profile_tab: true,
            use_track_cover: true,
            volume: 1.0,
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

fn fetch_fresh_soundcloud_client_id() -> Option<String> {
    let html = ureq::get("https://soundcloud.com")
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(5))
        .call()
        .ok()?
        .into_string()
        .ok()?;

    let mut scripts = Vec::new();
    for chunk in html.split("<script") {
        if let Some(pos) = chunk.find("src=\"") {
            let s = &chunk[pos + 5..];
            if let Some(end) = s.find('"') {
                let url = &s[..end];
                if url.contains("sndcdn.com/assets/") && url.ends_with(".js") {
                    scripts.push(url.to_string());
                }
            }
        }
    }

    for url in scripts.into_iter().rev().take(6) {
        if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(5)).call() {
            if let Ok(js) = resp.into_string() {
                if let Some(idx) = js.find("client_id:\"") {
                    let s = &js[idx + 11..];
                    if let Some(end) = s.find('"') {
                        let cid = &s[..end];
                        if cid.len() >= 20 {
                            return Some(cid.to_string());
                        }
                    }
                } else if let Some(idx) = js.find("client_id=\"") {
                    let s = &js[idx + 11..];
                    if let Some(end) = s.find('"') {
                        let cid = &s[..end];
                        if cid.len() >= 20 {
                            return Some(cid.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn search_soundcloud_tracks(query: &str, client_id_arc: Arc<Mutex<String>>) -> Vec<SearchTrackItem> {
    let mut cid = client_id_arc.lock().unwrap().clone();
    if cid.trim().is_empty() {
        cid = "2t9loNfh900mioJ2DU11YtSXQuVUbmyt".to_string();
    }

    let make_request = |id: &str| -> Result<serde_json::Value, ureq::Error> {
        let url = format!(
            "https://api-v2.soundcloud.com/search/tracks?q={}&client_id={}&limit=25",
            encode(query),
            encode(id)
        );
        let resp = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .set("Accept", "application/json, text/javascript, */*; q=0.01")
            .timeout(Duration::from_secs(6))
            .call()?;
        let j = resp.into_json::<serde_json::Value>()?;
        Ok(j)
    };

    let mut json_res = make_request(&cid);
    if json_res.is_err() {
        if let Some(fresh) = fetch_fresh_soundcloud_client_id() {
            *client_id_arc.lock().unwrap() = fresh.clone();
            json_res = make_request(&fresh);
        }
    }

    let json = match json_res {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    if let Some(collection) = json.get("collection").and_then(|c| c.as_array()) {
        for item in collection {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let artist = item.get("user").and_then(|u| u.get("username")).and_then(|un| un.as_str()).unwrap_or("SoundCloud Artist").to_string();
            let duration_ms = item.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
            let permalink_url = item.get("permalink_url").and_then(|p| p.as_str()).unwrap_or("").to_string();
            let raw_art = item.get("artwork_url")
                .and_then(|a| a.as_str())
                .or_else(|| item.get("user").and_then(|u| u.get("avatar_url")).and_then(|a| a.as_str()))
                .unwrap_or("");
            
            let artwork_url = if raw_art.contains("-large.") {
                raw_art.replace("-large.", "-t200x200.")
            } else if raw_art.contains("-badge.") {
                raw_art.replace("-badge.", "-t200x200.")
            } else if raw_art.contains("-t500x500.") {
                raw_art.replace("-t500x500.", "-t200x200.")
            } else {
                raw_art.to_string()
            };

            if !title.is_empty() && !permalink_url.is_empty() {
                results.push(SearchTrackItem {
                    title,
                    artist,
                    duration_sec: duration_ms / 1000,
                    permalink_url,
                    artwork_url,
                });
            }
        }
    }
    results
}

fn download_cover_bytes(raw_url: &str) -> Option<(Vec<u8>, [usize; 2])> {
    let clean = raw_url.trim().trim_matches('"').trim_matches('\'').to_string();
    if !clean.starts_with("http") {
        return None;
    }

    let mut candidate_urls = Vec::new();
    if clean.contains("-t500x500.") {
        candidate_urls.push(clean.clone());
        candidate_urls.push(clean.replace("-t500x500.", "-t300x300."));
        candidate_urls.push(clean.replace("-t500x500.", "-large."));
    } else {
        candidate_urls.push(clean.clone());
    }

    for url in candidate_urls {
        if let Ok(resp) = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(5))
            .call()
        {
            let mut buf = Vec::new();
            if resp.into_reader().read_to_end(&mut buf).is_ok() {
                if let Ok(dyn_img) = image::load_from_memory(&buf) {
                    let rgba = dyn_img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    return Some((rgba.into_raw(), [w as usize, h as usize]));
                }
            }
        }
    }
    None
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
    let width = 720u32;
    let height = 460u32;
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([18, 18, 18, 255]));

    let (bg_r, bg_g, bg_b) = match theme {
        AppTheme::Spotify => (18, 18, 18),
        AppTheme::Kawaii => (24, 18, 28),
        AppTheme::Neverlose => (11, 16, 26),
        AppTheme::ShadowFiend => (22, 14, 18),
        AppTheme::SoundCloud => (24, 20, 18),
        AppTheme::Custom => (custom_bg[0], custom_bg[1], custom_bg[2]),
    };

    let (ac_r, ac_g, ac_b) = match theme {
        AppTheme::Spotify => (30, 215, 96),
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

    // Match the Beta HUD: dark glass-like layers with a bright accent glow.
    for y in 36..height - 28 {
        for x in 28..width - 28 {
            if (x + y) % 7 == 0 {
                let p = img.get_pixel_mut(x, y);
                p.0[3] = 248;
            }
        }
    }
    for radius in (90..220).step_by(8) {
        let r = radius as i32;
        let cx = width as i32 - 70;
        let cy = 70i32;
        for y in (cy - r).max(0)..(cy + r).min(height as i32) {
            for x in (cx - r).max(0)..(cx + r).min(width as i32) {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r * r {
                    let p = img.get_pixel_mut(x as u32, y as u32);
                    p.0[0] = ((p.0[0] as u16 + ac_r as u16) / 2) as u8;
                    p.0[1] = ((p.0[1] as u16 + ac_g as u16) / 2) as u8;
                    p.0[2] = ((p.0[2] as u16 + ac_b as u16) / 2) as u8;
                }
            }
        }
    }

    draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(width, 6), Rgba([ac_r, ac_g, ac_b, 255]));
    draw_hollow_rect_mut(&mut img, Rect::at(25, 25).of_size(width - 50, height - 50), Rgba([ac_r, ac_g, ac_b, 100]));

    if let Some(font_bytes) = load_system_font() {
        if let Ok(font) = FontRef::try_from_slice(&font_bytes) {
            let white = Rgba([245, 245, 250, 255]);
            let accent = Rgba([ac_r, ac_g, ac_b, 255]);
            let gray = Rgba([175, 165, 185, 255]);

            draw_text_mut(&mut img, accent, 45, 45, PxScale::from(24.0), &font, "BETA HUD // LISTENING REPORT");
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
            draw_text_mut(&mut img, gray, 45, 130, PxScale::from(14.0), &font, top_label);

            let max_val = top_artists.first().map(|x| x.1).unwrap_or(1) as f32;
            for (i, (artist, count)) in top_artists.iter().enumerate().take(5) {
                let y_base = 160 + (i as i32 * 52);
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

fn render_neon_equalizer(
    ui: &mut egui::Ui,
    accent_color: egui::Color32,
    spectrum: &[u8],
) {
    let desired_width = 280.0;
    let desired_height = 24.0;
    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(desired_width, desired_height), egui::Sense::hover());

    let num_bars = 28;
    let bar_width = 5.5;
    let gap = (desired_width - (num_bars as f32 * bar_width)) / (num_bars as f32 - 1.0);

    for i in 0..num_bars {
        let x = rect.min.x + (i as f32 * (bar_width + gap));
        
        let spec_idx = (i * spectrum.len()) / num_bars;
        let raw_val = spectrum.get(spec_idx).copied().unwrap_or(0);
        let height_factor = (raw_val as f32 / 255.0).clamp(0.08, 0.98);

        let bar_height = rect.height() * height_factor;
        let y = rect.max.y - bar_height;
        let bar_rect = egui::Rect::from_min_max(egui::pos2(x, y), egui::pos2(x + bar_width, rect.max.y));

        let alpha = (60.0 + (height_factor * 195.0)) as u8;
        ui.painter().rect_filled(bar_rect, 2.5, egui::Color32::from_rgba_unmultiplied(accent_color.r(), accent_color.g(), accent_color.b(), alpha));
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
        painter.circle_filled(egui::pos2(x, y), radius, egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Player,
    Lyrics,
    Search,
    Stats,
    History,
    Profile,
    Settings,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Mutex<AppConfig>>,
    is_enabled: Arc<AtomicBool>,
    status_text: Arc<Mutex<String>>,
    current_track: Arc<Mutex<String>>,
    current_artist: Arc<Mutex<String>>,
    current_title: Arc<Mutex<String>>,
    progress_ratio: Arc<Mutex<f32>>,
    progress_text: Arc<Mutex<String>>,
    current_sec: Arc<Mutex<f32>>,
    track_duration: Arc<Mutex<f32>>,
    track_count: Arc<AtomicU32>,
    show_volume_popup: Arc<AtomicBool>,
    vol_btn_rect: Arc<Mutex<egui::Rect>>,
    custom_image_bytes: Arc<Mutex<Option<Vec<u8>>>>,
    track_cover_rgba: Arc<Mutex<Option<(Vec<u8>, [usize; 2])>>>,
    track_cover_url: Arc<Mutex<String>>,
    track_cover_version: Arc<AtomicU32>,
    image_version: Arc<AtomicU32>,
    active_tab: Arc<Mutex<ActiveTab>>,
    export_notify: Arc<Mutex<Option<String>>>,
    copy_notify: Arc<Mutex<Option<f64>>>,
    shared_track_payload: Arc<Mutex<Option<(IncomingTrackPayload, Instant)>>>,
    pending_command: Arc<Mutex<Option<String>>>,
    is_playing_now: Arc<AtomicBool>,
    repeat_mode: Arc<Mutex<String>>,
    sc_client_id: Arc<Mutex<String>>,
    user_name: Arc<Mutex<String>>,
    user_url: Arc<Mutex<String>>,
    user_playlists: Arc<Mutex<Vec<UserPlaylist>>>,
    search_query: Arc<Mutex<String>>,
    search_results: Arc<Mutex<Vec<SearchTrackItem>>>,
    is_searching: Arc<AtomicBool>,
    has_searched: Arc<AtomicBool>,
    live_spectrum: Arc<Mutex<Vec<u8>>>,
    liked_now: Arc<AtomicBool>,
    followed_now: Arc<AtomicBool>,
}

impl AppState {
    fn save_config(&self) {
        self.config.lock().unwrap().save();
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let btn_box = *self.vol_btn_rect.lock().unwrap();
        let expected_popup_rect = egui::Rect::from_min_size(
            egui::pos2(btn_box.center().x - 18.0, btn_box.max.y + 2.0),
            egui::vec2(36.0, 130.0),
        );

        if self.show_volume_popup.load(Ordering::SeqCst) {
            if ctx.input(|i| i.pointer.any_pressed()) {
                if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    if !btn_box.expand(4.0).contains(pos) && !expected_popup_rect.expand(16.0).contains(pos) {
                        self.show_volume_popup.store(false, Ordering::SeqCst);
                    }
                }
            }
        }

        let cfg = self.config.lock().unwrap().clone();
        let is_on = self.is_enabled.load(Ordering::SeqCst);
        let anim_time = ctx.input(|i| i.time);
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());

        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::from_rgb(240, 235, 240));
        visuals.panel_fill = cfg.theme.bg_color(cfg.custom_bg);
        visuals.window_fill = cfg.theme.bg_color(cfg.custom_bg);
        visuals.widgets.noninteractive.bg_fill = cfg.theme.widget_bg(cfg.custom_bg);
        visuals.selection.bg_fill = cfg.theme.primary_accent(cfg.custom_accent);
        ctx.set_visuals(visuals);

        let spectrum_snapshot = self.live_spectrum.lock().unwrap().clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            if cfg.enable_particles {
                render_background_particles(
                    ui.painter(),
                    ui.max_rect(),
                    cfg.theme.primary_accent(cfg.custom_accent),
                    anim_time,
                    mouse_pos,
                    cfg.particles_direction_up,
                );
            }

            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                let mut current_tab = *self.active_tab.lock().unwrap();

                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(cfg.theme.widget_bg(cfg.custom_bg))
                    .rounding(12.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                    .stroke(egui::Stroke::new(1.0_f32, cfg.theme.primary_accent(cfg.custom_accent).linear_multiply(0.25)))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("ZEN RPC").strong().size(14.0).color(cfg.theme.primary_accent(cfg.custom_accent)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("🚀 Beta HUD").size(11.0).strong().color(egui::Color32::WHITE))
                                    .fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(8.0)).clicked() {
                                    launch_beta_hud();
                                    hide_app_window();
                                }
                            });
                        });
                        ui.add_space(5.0);
                        egui::ScrollArea::horizontal().id_source("main_nav_scroll").show(ui, |ui| {
                        ui.horizontal(|ui| {

                            let mut tab_button = |tab: ActiveTab, label: &'static str| {
                                let selected = current_tab == tab;
                                let button = egui::Button::new(egui::RichText::new(label).size(12.0).color(
                                    if selected { egui::Color32::BLACK } else { egui::Color32::LIGHT_GRAY }
                                ))
                                .fill(if selected { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) })
                                .rounding(8.0);
                                if ui.add(button).clicked() {
                                    current_tab = tab;
                                    *self.active_tab.lock().unwrap() = tab;
                                }
                            };

                            tab_button(ActiveTab::Player, "🎵 Плеер");
                            tab_button(ActiveTab::Lyrics, "📜 Текст");
                            if cfg.show_search_tab { tab_button(ActiveTab::Search, "🔍 Поиск"); }
                            if cfg.show_top_tab { tab_button(ActiveTab::Stats, "📊 Статистика"); }
                            if cfg.show_history_tab { tab_button(ActiveTab::History, "📜 История"); }
                            if cfg.show_profile_tab { tab_button(ActiveTab::Profile, "👤"); }
                            tab_button(ActiveTab::Settings, "⚙");

                        });
                        });
                    });

                ui.add_space(8.0);
                let card_width = (ui.available_width() - 28.0).max(280.0);

                match current_tab {
                    ActiveTab::Player => {
                        let ratio = *self.progress_ratio.lock().unwrap();
                        let playing_now = self.is_playing_now.load(Ordering::SeqCst);

                        let img_source = {
                            let t_guard = self.track_cover_rgba.lock().unwrap();
                            let c_guard = self.custom_image_bytes.lock().unwrap();

                            if cfg.use_track_cover && t_guard.is_some() {
                                let (ref rgba_raw, size) = *t_guard.as_ref().unwrap();
                                let col_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba_raw);
                                let t_ver = self.track_cover_version.load(Ordering::SeqCst);
                                let tex = ctx.load_texture(format!("sc_cover_{}", t_ver), col_img, egui::TextureOptions::LINEAR);
                                egui::Image::from_texture(&tex)
                            } else if let Some(custom) = &*c_guard {
                                let img_version = self.image_version.load(Ordering::SeqCst);
                                egui::Image::from_bytes(format!("bytes://avatar_{}.png", img_version), custom.clone())
                            } else {
                                egui::Image::from_bytes("bytes://sc.png", SC_IMAGE_BYTES)
                            }
                        };

                        let track = self.current_track.lock().unwrap().clone();
                        let artist_raw = self.current_artist.lock().unwrap().clone();
                        let title_raw = self.current_title.lock().unwrap().clone();
                        let count = self.track_count.load(Ordering::SeqCst);
                        let liked_now = self.liked_now.load(Ordering::SeqCst);
                        let followed_now = self.followed_now.load(Ordering::SeqCst);

                        if cfg.layout_mode == UiLayoutMode::Spotify {
                            let cover_size = egui::vec2(160.0, 160.0);
                            let (disk_rect, _resp) = ui.allocate_exact_size(cover_size, egui::Sense::hover());
                            img_source.fit_to_exact_size(cover_size).rounding(12.0).paint_at(ui, disk_rect);

                            let badge_size = 24.0;
                            let badge_pos = disk_rect.right_top() - egui::vec2(badge_size - 4.0, -4.0);
                            let badge_rect = egui::Rect::from_min_size(badge_pos, egui::vec2(badge_size, badge_size));
                            let badge_resp = ui.interact(badge_rect, ui.id().with("badge_change_pic"), egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Change avatar");

                            ui.painter().circle_filled(badge_rect.center(), 12.0, cfg.theme.widget_bg(cfg.custom_bg));
                            ui.painter().circle_stroke(badge_rect.center(), 12.0, egui::Stroke::new(1.5_f32, cfg.theme.primary_accent(cfg.custom_accent)));
                            ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, "🖼", egui::FontId::proportional(11.0), egui::Color32::WHITE);

                            if badge_resp.clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "webp"]).pick_file() {
                                    if let Ok(bytes) = fs::read(&path) {
                                        *self.custom_image_bytes.lock().unwrap() = Some(bytes);
                                        self.config.lock().unwrap().custom_image_path = Some(path.to_string_lossy().to_string());
                                        self.image_version.fetch_add(1, Ordering::SeqCst);
                                        self.save_config();
                                    }
                                }
                            }

                            ui.add_space(8.0);
                            let display_title = if !title_raw.is_empty() { &title_raw } else { &track };
                            let display_artist = if !artist_raw.is_empty() { &artist_raw } else { "SoundCloud" };

                            ui.scope(|ui| {
                                ui.set_max_width(380.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(display_title).strong().size(16.5).color(egui::Color32::WHITE));
                                        ui.label(egui::RichText::new(display_artist).size(13.0).color(egui::Color32::from_rgb(175, 175, 175)));
                                    });

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if !artist_raw.is_empty() && !title_raw.is_empty() {
                                            if ui.small_button("📋").on_hover_text("Копировать ссылку").clicked() {
                                                let q = format!("{} {}", artist_raw, title_raw);
                                                let share_link = format!("🎧 Слушаю: {} — {} | https://soundcloud.com/search/sounds?q={}", artist_raw, title_raw, encode(&q));
                                                ctx.output_mut(|o| o.copied_text = share_link);
                                                *self.copy_notify.lock().unwrap() = Some(ctx.input(|i| i.time));
                                            }
                                        }
                                    });
                                });
                            });

                            let now_t = ctx.input(|i| i.time);
                            if let Some(t_copied) = *self.copy_notify.lock().unwrap() {
                                if now_t - t_copied < 2.0 {
                                    ui.label(egui::RichText::new(I18n::copy_success(cfg.language)).size(11.0).color(cfg.theme.primary_accent(cfg.custom_accent)));
                                }
                            }

                            ui.add_space(6.0);
                            let p_sec = *self.current_sec.lock().unwrap() as u64;
                            let dur_sec = *self.track_duration.lock().unwrap();
                            let raw_p_text = self.progress_text.lock().unwrap().clone();
                            let total_part = raw_p_text.split('/').nth(1).unwrap_or("00:00").trim();

                            ui.horizontal(|ui| {
                                ui.set_max_width(380.0);
                                ui.add_space(14.0);
                                ui.label(egui::RichText::new(format_duration(p_sec)).size(11.0).color(egui::Color32::GRAY));

                                let bar_size = egui::vec2(280.0, 8.0);
                                let (rect, resp) = ui.allocate_exact_size(bar_size, egui::Sense::click_and_drag());
                                ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(35, 38, 44));
                                let fill_w = (rect.width() * ratio.clamp(0.0, 1.0)).max(0.0);
                                let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height()));
                                ui.painter().rect_filled(fill_rect, 4.0, cfg.theme.primary_accent(cfg.custom_accent));

                                if (resp.clicked() || resp.dragged()) && dur_sec > 0.0 {
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        let click_ratio = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
                                        let target_time = click_ratio * dur_sec;
                                        *self.pending_command.lock().unwrap() = Some(format!("seek:{:.0}", target_time));
                                    }
                                }

                                ui.label(egui::RichText::new(total_part).size(11.0).color(egui::Color32::GRAY));
                            });

                            ui.add_space(10.0);

                            if cfg.show_controls {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let total_w = 34.0 + 38.0 + 44.0 + 38.0 + 36.0 + (4.0 * 8.0);
                                    let offset = (ui.available_width() - total_w) * 0.5;
                                    if offset > 0.0 { ui.add_space(offset); }

                                    let rep_mode = self.repeat_mode.lock().unwrap().clone();
                                    let is_rep_active = rep_mode == "one" || rep_mode == "all";
                                    let rep_icon = if rep_mode == "one" { "🔂" } else { "🔁" };
                                    let rep_color = if is_rep_active { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) };

                                    if ui.add_sized(
                                        egui::vec2(34.0, 32.0),
                                        egui::Button::new(egui::RichText::new(rep_icon).size(14.0).color(egui::Color32::WHITE))
                                            .fill(rep_color)
                                            .rounding(16.0),
                                    ).on_hover_text(I18n::btn_repeat(cfg.language, &rep_mode)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("repeat".to_string());
                                    }

                                    if ui.add_sized(
                                        egui::vec2(38.0, 34.0),
                                        egui::Button::new(egui::RichText::new("⏮").size(15.0).color(egui::Color32::WHITE))
                                            .fill(cfg.theme.widget_bg(cfg.custom_bg))
                                            .rounding(17.0),
                                    ).on_hover_text(I18n::btn_prev(cfg.language)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("prev".to_string());
                                    }

                                    let play_icon = if playing_now { "⏸" } else { "▶" };

                                    if ui.add_sized(
                                        egui::vec2(44.0, 44.0),
                                        egui::Button::new(egui::RichText::new(play_icon).size(18.0).color(egui::Color32::BLACK).strong())
                                            .fill(cfg.theme.primary_accent(cfg.custom_accent))
                                            .rounding(22.0),
                                    ).on_hover_text(I18n::btn_play_pause(cfg.language, playing_now)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("toggle_play".to_string());
                                    }

                                    if ui.add_sized(
                                        egui::vec2(38.0, 34.0),
                                        egui::Button::new(egui::RichText::new("⏭").size(15.0).color(egui::Color32::WHITE))
                                            .fill(cfg.theme.widget_bg(cfg.custom_bg))
                                            .rounding(17.0),
                                    ).on_hover_text(I18n::btn_next(cfg.language)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("next".to_string());
                                    }

                                    let cur_vol = cfg.volume;
                                    let vol_icon = if cur_vol <= 0.01 { "🔇" } else if cur_vol < 0.5 { "🔉" } else { "🔊" };
                                    let is_open = self.show_volume_popup.load(Ordering::SeqCst);
                                    let vol_btn_color = if is_open { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) };

                                    let vol_btn = ui.add_sized(
                                        egui::vec2(36.0, 32.0),
                                        egui::Button::new(egui::RichText::new(vol_icon).size(14.0).color(egui::Color32::WHITE))
                                            .fill(vol_btn_color)
                                            .rounding(16.0),
                                    ).on_hover_text("Громкость");

                                    *self.vol_btn_rect.lock().unwrap() = vol_btn.rect;
                                    if vol_btn.clicked() {
                                        self.show_volume_popup.store(!is_open, Ordering::SeqCst);
                                    }
                                });
                            }

                            ui.horizontal(|ui| {
                                let like_icon = if liked_now { "♥" } else { "♡" };
                                let like_color = if liked_now { egui::Color32::from_rgb(245, 60, 88) } else { cfg.theme.widget_bg(cfg.custom_bg) };
                                if ui.add_sized([42.0, 32.0], egui::Button::new(egui::RichText::new(like_icon).size(22.0).color(if liked_now { egui::Color32::WHITE } else { egui::Color32::LIGHT_GRAY })).fill(like_color).rounding(16.0)).on_hover_text("Поставить лайк").clicked() {
                                    *self.pending_command.lock().unwrap() = Some("toggle_like".to_string());
                                }
                                let follow_text = if followed_now { "✔ Вы подписаны" } else { "Подписаться" };
                                let follow_color = if followed_now { cfg.theme.widget_bg(cfg.custom_bg) } else { cfg.theme.primary_accent(cfg.custom_accent) };
                                let follow_text_color = if followed_now { egui::Color32::WHITE } else { egui::Color32::BLACK };
                                if ui.add_sized([150.0, 32.0], egui::Button::new(egui::RichText::new(follow_text).strong().color(follow_text_color)).fill(follow_color).rounding(8.0)).on_hover_text("Подписаться на исполнителя").clicked() {
                                    *self.pending_command.lock().unwrap() = Some("toggle_follow".to_string());
                                }
                            });

                            if cfg.enable_equalizer {
                                ui.add_space(8.0);
                                render_neon_equalizer(ui, cfg.theme.primary_accent(cfg.custom_accent), &spectrum_snapshot);
                            }

                            ui.add_space(10.0);
                            let bottom_total_w = 170.0 + 170.0 + 10.0;
                            let b_offset = (ui.available_width() - bottom_total_w) * 0.5;

                            ui.horizontal(|ui| {
                                if b_offset > 0.0 { ui.add_space(b_offset); }
                                let (btn_text, btn_color) = if is_on {
                                    (I18n::status_active(cfg.language), cfg.theme.primary_accent(cfg.custom_accent))
                                } else {
                                    (I18n::status_disabled(cfg.language), cfg.theme.widget_bg(cfg.custom_bg))
                                };

                                if ui.add_sized(
                                    [170.0, 28.0],
                                    egui::Button::new(egui::RichText::new(btn_text).color(if is_on { egui::Color32::BLACK } else { egui::Color32::WHITE }).strong())
                                        .fill(btn_color)
                                        .rounding(14.0),
                                ).clicked() {
                                    self.is_enabled.store(!is_on, Ordering::SeqCst);
                                }

                                ui.add_space(10.0);
                                if ui.add_sized([170.0, 28.0], egui::Button::new(egui::RichText::new(I18n::btn_minimize(cfg.language)).color(egui::Color32::WHITE)).fill(cfg.theme.widget_bg(cfg.custom_bg)).rounding(14.0)).clicked() {
                                    hide_app_window();
                                }
                            });

                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(I18n::total_played(cfg.language, count)).size(11.0).color(egui::Color32::GRAY));
                        } else {
                            let img_size = egui::vec2(120.0, 120.0);
                            let (disk_rect, _resp) = ui.allocate_exact_size(img_size, egui::Sense::hover());
                            img_source.fit_to_exact_size(img_size).rounding(14.0).paint_at(ui, disk_rect);

                            let badge_size = 26.0;
                            let badge_pos = disk_rect.right_top() - egui::vec2(badge_size - 3.0, -3.0);
                            let badge_rect = egui::Rect::from_min_size(badge_pos, egui::vec2(badge_size, badge_size));
                            let badge_resp = ui.interact(badge_rect, ui.id().with("badge_change_pic_classic"), egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Change avatar");

                            let badge_color = if badge_resp.hovered() { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) };
                            ui.painter().circle_filled(badge_rect.center(), 13.0, badge_color);
                            ui.painter().circle_stroke(badge_rect.center(), 13.0, egui::Stroke::new(1.5_f32, cfg.theme.primary_accent(cfg.custom_accent)));
                            ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, "🖼", egui::FontId::proportional(12.0), egui::Color32::WHITE);

                            if badge_resp.clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "webp"]).pick_file() {
                                    if let Ok(bytes) = fs::read(&path) {
                                        *self.custom_image_bytes.lock().unwrap() = Some(bytes);
                                        self.config.lock().unwrap().custom_image_path = Some(path.to_string_lossy().to_string());
                                        self.image_version.fetch_add(1, Ordering::SeqCst);
                                        self.save_config();
                                    }
                                }
                            }

                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("SoundCloud RPC").strong().size(16.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                            ui.add_space(6.0);

                            let (btn_text, btn_color) = if is_on {
                                (I18n::status_active(cfg.language), cfg.theme.primary_accent(cfg.custom_accent))
                            } else {
                                (I18n::status_disabled(cfg.language), cfg.theme.widget_bg(cfg.custom_bg))
                            };

                            if ui.add_sized([220.0, 25.0], egui::Button::new(egui::RichText::new(btn_text).color(egui::Color32::WHITE).strong()).fill(btn_color).rounding(8.0)).clicked() {
                                self.is_enabled.store(!is_on, Ordering::SeqCst);
                            }

                            ui.add_space(4.0);
                            if ui.add_sized([220.0, 23.0], egui::Button::new(egui::RichText::new(I18n::btn_minimize(cfg.language)).size(12.0).color(egui::Color32::from_rgb(195, 185, 205))).fill(cfg.theme.widget_bg(cfg.custom_bg)).rounding(6.0)).clicked() {
                                hide_app_window();
                            }

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(2.0);

                            ui.label(egui::RichText::new(I18n::total_played(cfg.language, count)).size(11.5).color(cfg.theme.secondary_accent(cfg.custom_accent)).strong());
                            ui.add_space(2.0);

                            ui.scope(|ui| {
                                let row_width = 300.0;
                                ui.set_max_width(row_width);
                                ui.horizontal(|ui| {
                                    let has_meta = !artist_raw.is_empty() && !title_raw.is_empty();
                                    let btn_width = if has_meta { 26.0 } else { 0.0 };
                                    let text_width = (row_width - btn_width - 8.0).max(60.0);

                                    let label = egui::Label::new(egui::RichText::new(&track).strong().size(13.0).color(egui::Color32::WHITE)).truncate(true);
                                    ui.add_sized([text_width, 20.0], label).on_hover_text(&track);

                                    if has_meta {
                                        if ui.small_button("📋").on_hover_text("Копировать ссылку").clicked() {
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
                                    ui.label(egui::RichText::new(I18n::copy_success(cfg.language)).size(11.0).color(cfg.theme.primary_accent(cfg.custom_accent)));
                                }
                            }

                            ui.add_space(4.0);
                            let dur_sec = *self.track_duration.lock().unwrap();

                            let bar_size = egui::vec2(280.0, 14.0);
                            let (rect, resp) = ui.allocate_exact_size(bar_size, egui::Sense::click_and_drag());
                            ui.painter().rect_filled(rect, 6.0, egui::Color32::from_rgb(35, 38, 44));
                            let fill_w = (rect.width() * ratio.clamp(0.0, 1.0)).max(0.0);
                            let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height()));
                            ui.painter().rect_filled(fill_rect, 6.0, cfg.theme.primary_accent(cfg.custom_accent));

                            let p_text = self.progress_text.lock().unwrap().clone();
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, p_text, egui::FontId::proportional(10.5), egui::Color32::WHITE);

                            if (resp.clicked() || resp.dragged()) && dur_sec > 0.0 {
                                if let Some(pos) = resp.interact_pointer_pos() {
                                    let click_ratio = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
                                    let target_time = click_ratio * dur_sec;
                                    *self.pending_command.lock().unwrap() = Some(format!("seek:{:.0}", target_time));
                                }
                            }

                            if cfg.enable_equalizer {
                                ui.add_space(6.0);
                                render_neon_equalizer(ui, cfg.theme.primary_accent(cfg.custom_accent), &spectrum_snapshot);
                            }

                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let like_icon = if liked_now { "♥" } else { "♡" };
                                let like_color = if liked_now { egui::Color32::from_rgb(245, 60, 88) } else { cfg.theme.widget_bg(cfg.custom_bg) };
                                if ui.add_sized([42.0, 30.0], egui::Button::new(egui::RichText::new(like_icon).size(21.0).color(if liked_now { egui::Color32::WHITE } else { egui::Color32::LIGHT_GRAY })).fill(like_color).rounding(15.0)).on_hover_text("Поставить лайк").clicked() {
                                    *self.pending_command.lock().unwrap() = Some("toggle_like".to_string());
                                }
                                let follow_text = if followed_now { "✔ Вы подписаны" } else { "Подписаться" };
                                let follow_color = if followed_now { cfg.theme.widget_bg(cfg.custom_bg) } else { cfg.theme.primary_accent(cfg.custom_accent) };
                                let follow_text_color = if followed_now { egui::Color32::WHITE } else { egui::Color32::BLACK };
                                if ui.add_sized([150.0, 30.0], egui::Button::new(egui::RichText::new(follow_text).strong().color(follow_text_color)).fill(follow_color).rounding(8.0)).on_hover_text("Подписаться на исполнителя").clicked() {
                                    *self.pending_command.lock().unwrap() = Some("toggle_follow".to_string());
                                }
                            });

                            if cfg.show_controls {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let total_w = 38.0 + 44.0 + 50.0 + 44.0 + 36.0 + (4.0 * 6.0);
                                    let offset = (ui.available_width() - total_w) * 0.5;
                                    if offset > 0.0 { ui.add_space(offset); }

                                    let rep_mode = self.repeat_mode.lock().unwrap().clone();
                                    let is_rep_active = rep_mode == "one" || rep_mode == "all";
                                    let rep_icon = if rep_mode == "one" { "🔂" } else { "🔁" };
                                    let rep_btn_color = if is_rep_active { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) };

                                    if ui.add_sized(egui::vec2(38.0, 26.0), egui::Button::new(egui::RichText::new(rep_icon).size(14.0).color(egui::Color32::WHITE)).fill(rep_btn_color).rounding(7.0)).on_hover_text(I18n::btn_repeat(cfg.language, &rep_mode)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("repeat".to_string());
                                    }

                                    if ui.add_sized(egui::vec2(44.0, 26.0), egui::Button::new(egui::RichText::new("⏮").size(14.0).color(egui::Color32::WHITE)).fill(cfg.theme.widget_bg(cfg.custom_bg)).rounding(7.0)).on_hover_text(I18n::btn_prev(cfg.language)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("prev".to_string());
                                    }

                                    let play_icon = if playing_now { "⏸" } else { "▶" };
                                    if ui.add_sized(egui::vec2(50.0, 26.0), egui::Button::new(egui::RichText::new(play_icon).size(15.0).color(egui::Color32::WHITE)).fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(7.0)).on_hover_text(I18n::btn_play_pause(cfg.language, playing_now)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("toggle_play".to_string());
                                    }

                                    if ui.add_sized(egui::vec2(44.0, 28.0), egui::Button::new(egui::RichText::new("⏭").size(14.0).color(egui::Color32::WHITE)).fill(cfg.theme.widget_bg(cfg.custom_bg)).rounding(7.0)).on_hover_text(I18n::btn_next(cfg.language)).clicked() {
                                        *self.pending_command.lock().unwrap() = Some("next".to_string());
                                    }

                                    let cur_vol = cfg.volume;
                                    let vol_icon = if cur_vol <= 0.01 { "🔇" } else if cur_vol < 0.5 { "🔉" } else { "🔊" };
                                    let is_open = self.show_volume_popup.load(Ordering::SeqCst);
                                    let vol_btn_color = if is_open { cfg.theme.primary_accent(cfg.custom_accent) } else { cfg.theme.widget_bg(cfg.custom_bg) };

                                    let vol_btn = ui.add_sized(egui::vec2(36.0, 26.0), egui::Button::new(egui::RichText::new(vol_icon).size(13.0).color(egui::Color32::WHITE)).fill(vol_btn_color).rounding(7.0)).on_hover_text("Громкость");
                                    *self.vol_btn_rect.lock().unwrap() = vol_btn.rect;
                                    if vol_btn.clicked() {
                                        self.show_volume_popup.store(!is_open, Ordering::SeqCst);
                                    }
                                });
                            }
                        }
                    }

                    ActiveTab::Profile => {
                        ui.label(egui::RichText::new(I18n::profile_header(cfg.language)).strong().size(15.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(6.0);

                        let current_user = self.user_name.lock().unwrap().clone();
                        let current_u_url = self.user_url.lock().unwrap().clone();

                        ui.group(|ui| {
                            ui.set_width(card_width);
                            ui.vertical_centered(|ui| {
                                if !current_user.is_empty() {
                                    ui.label(egui::RichText::new(I18n::profile_user_active(cfg.language, &current_user)).strong().size(13.5).color(egui::Color32::WHITE));
                                    if !current_u_url.is_empty() {
                                        ui.add_space(3.0);
                                        if ui.small_button(I18n::btn_open_user_page(cfg.language)).clicked() {
                                            let _ = std::process::Command::new("cmd").args(["/C", "start", "", &current_u_url]).spawn();
                                        }
                                    }
                                } else {
                                    ui.label(egui::RichText::new(I18n::profile_guest_hint(cfg.language)).size(11.5).color(egui::Color32::from_rgb(180, 175, 190)));
                                }
                            });
                        });

                        ui.add_space(4.0);
                        if ui.add_sized([card_width, 26.0], egui::Button::new(egui::RichText::new(I18n::btn_open_stream(cfg.language)).strong().color(egui::Color32::WHITE)).fill(cfg.theme.widget_bg(cfg.custom_bg)).rounding(7.0)).clicked() {
                            *self.pending_command.lock().unwrap() = Some("play_track:https://soundcloud.com/stream".to_string());
                        }

                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(I18n::playlists_header(cfg.language)).strong().size(14.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(4.0);

                        let playlists = self.user_playlists.lock().unwrap().clone();
                        if playlists.is_empty() {
                            ui.add_space(20.0);
                            ui.label(egui::RichText::new(I18n::playlists_empty(cfg.language)).size(11.5).color(egui::Color32::GRAY));
                        } else {
                            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(260.0).show(ui, |ui| {
                                ui.set_width(card_width);
                                for item in playlists {
                                    ui.group(|ui| {
                                        ui.set_width(card_width);
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.set_max_width((card_width - 90.0).max(60.0));
                                                ui.label(egui::RichText::new(&item.title).strong().size(12.5).color(egui::Color32::WHITE));
                                                let count_str = if item.track_count > 0 { format!("{} треков", item.track_count) } else { "Плейлист".to_string() };
                                                ui.label(egui::RichText::new(count_str).size(11.0).color(egui::Color32::from_rgb(160, 160, 160)));
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.add_space(4.0);
                                                let play_btn = egui::Button::new(egui::RichText::new(I18n::btn_play_track(cfg.language)).size(11.0).color(egui::Color32::BLACK).strong()).fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(6.0);
                                                if ui.add(play_btn).clicked() {
                                                    *self.pending_command.lock().unwrap() = Some(format!("play_track:{}", item.permalink_url));
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(2.0);
                                }
                            });
                        }
                    }

                    ActiveTab::Lyrics => {
                        let payload = self.shared_track_payload.lock().unwrap().clone();
                        let (plain, synced) = payload
                            .map(|(data, _)| (data.lyrics_plain, data.lyrics_synced))
                            .unwrap_or_default();
                        let passed = *self.current_sec.lock().unwrap();
                        let playing_now = self.is_playing_now.load(Ordering::SeqCst);
                        let title = self.current_title.lock().unwrap().clone();
                        let artist = self.current_artist.lock().unwrap().clone();

                        ui.label(egui::RichText::new("Текст песни").strong().size(16.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.label(egui::RichText::new(format!("{} — {}", title, artist)).size(12.0).color(egui::Color32::GRAY));
                        ui.add_space(8.0);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(card_width);
                            egui::ScrollArea::vertical().id_source("rust_lyrics_scroll").max_height(330.0).auto_shrink([false, false]).show(ui, |ui| {
                                if !synced.is_empty() {
                                    let mut active_idx = 0;
                                    for (idx, line) in synced.iter().enumerate() {
                                        if passed >= line.sec { active_idx = idx; }
                                    }
                                    for (idx, line) in synced.iter().enumerate() {
                                        let active = idx == active_idx;
                                        let response = ui.add(egui::Label::new(egui::RichText::new(&line.text).size(if active { 16.0 } else { 13.0 }).strong().color(
                                            if active { cfg.theme.primary_accent(cfg.custom_accent) } else { egui::Color32::GRAY }
                                        )).sense(egui::Sense::click()))
                                            .on_hover_cursor(egui::CursorIcon::Default)
                                            .on_hover_text(format!("Перемотать на {}", format_duration(line.sec as u64)));
                                        if response.clicked() {
                                            *self.pending_command.lock().unwrap() = Some(format!("seek:{:.0}", line.sec));
                                        }
                                        if active && playing_now {
                                            ui.scroll_to_rect(response.rect.expand(12.0), Some(egui::Align::Center));
                                        }
                                        ui.add_space(6.0);
                                    }
                                } else if !plain.is_empty() {
                                    ui.label(egui::RichText::new(plain).size(14.0).color(egui::Color32::LIGHT_GRAY));
                                } else {
                                    ui.label(egui::RichText::new("Текст пока не найден").color(egui::Color32::GRAY));
                                }
                            });
                        });
                    }

                    ActiveTab::Search => {
                        ui.label(egui::RichText::new(I18n::search_title(cfg.language)).strong().size(15.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(6.0);

                        let mut query = self.search_query.lock().unwrap().clone();
                        let mut trigger_search = false;

                        ui.horizontal(|ui| {
                            let text_edit = egui::TextEdit::singleline(&mut query).hint_text(I18n::search_hint(cfg.language)).desired_width(280.0);
                            let resp = ui.add(text_edit);
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                trigger_search = true;
                            }
                            if ui.button(I18n::search_btn(cfg.language)).clicked() {
                                trigger_search = true;
                            }
                        });

                        *self.search_query.lock().unwrap() = query.clone();

                        if trigger_search && !query.trim().is_empty() {
                            let q_clone = query.trim().to_string();
                            let cid_arc = self.sc_client_id.clone();
                            let results_arc = self.search_results.clone();
                            let searching_arc = self.is_searching.clone();
                            let searched_arc = self.has_searched.clone();

                            searching_arc.store(true, Ordering::SeqCst);
                            searched_arc.store(true, Ordering::SeqCst);

                            thread::spawn(move || {
                                let items = search_soundcloud_tracks(&q_clone, cid_arc);
                                *results_arc.lock().unwrap() = items;
                                searching_arc.store(false, Ordering::SeqCst);
                            });
                        }

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        if self.is_searching.load(Ordering::SeqCst) {
                            ui.add_space(50.0);
                            ui.spinner();
                            ui.label(egui::RichText::new(I18n::search_loading(cfg.language)).color(egui::Color32::GRAY));
                        } else if !self.has_searched.load(Ordering::SeqCst) {
                            ui.add_space(50.0);
                            ui.label(egui::RichText::new(I18n::search_start_prompt(cfg.language)).color(egui::Color32::GRAY));
                        } else {
                            let results = self.search_results.lock().unwrap().clone();
                            if results.is_empty() {
                                ui.add_space(50.0);
                                ui.label(egui::RichText::new(I18n::search_empty(cfg.language)).color(egui::Color32::GRAY));
                            } else {
                                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(350.0).show(ui, |ui| {
                                    ui.set_width(card_width);
                                    for item in results {
                                        ui.group(|ui| {
                                            ui.set_width(card_width);
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.set_max_width((card_width - 90.0).max(60.0));
                                                    ui.label(egui::RichText::new(&item.title).strong().size(12.5).color(egui::Color32::WHITE));
                                                    let meta = format!("{} • {}", item.artist, format_duration(item.duration_sec));
                                                    ui.label(egui::RichText::new(meta).size(11.0).color(egui::Color32::from_rgb(165, 155, 180)));
                                                });

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.add_space(4.0);
                                                    let play_btn = egui::Button::new(egui::RichText::new(I18n::btn_play_track(cfg.language)).size(11.5).color(egui::Color32::WHITE).strong()).fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(5.0);
                                                    if ui.add(play_btn).clicked() {
                                                        *self.pending_command.lock().unwrap() = Some(format!("play_track:{}", item.permalink_url));
                                                    }
                                                });
                                            });
                                        });
                                        ui.add_space(2.0);
                                    }
                                });
                            }
                        }
                    }

                    ActiveTab::Stats => {
                        let hist = cfg.history.clone();
                        let total_played = self.track_count.load(Ordering::SeqCst);
                        let mut artist_counts: HashMap<String, usize> = HashMap::new();
                        let mut total_seconds: u64 = 0;

                        for entry in &hist {
                            *artist_counts.entry(entry.artist.clone()).or_insert(0) += 1;
                            total_seconds += entry.duration_sec.unwrap_or(150);
                        }

                        let mut sorted_artists: Vec<(String, usize)> = artist_counts.into_iter().collect();
                        sorted_artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                        let top_artist = sorted_artists.first().cloned();
                        let total_hours = total_seconds as f32 / 3600.0;

                        ui.label(egui::RichText::new(I18n::stats_header(cfg.language)).strong().size(15.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(4.0);

                        ui.group(|ui| {
                            ui.set_width(card_width);
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(I18n::stats_total(cfg.language, total_played)).strong().color(egui::Color32::WHITE));
                                    ui.label(egui::RichText::new(I18n::stats_time(cfg.language, total_hours)).size(12.0).color(egui::Color32::from_rgb(180, 175, 195)));
                                });
                                ui.separator();
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(I18n::stats_fav_artist(cfg.language)).size(11.0).color(egui::Color32::from_rgb(180, 175, 195)));
                                    let fav_name = match &top_artist {
                                        Some((name, cnt)) => format!("{} ({}×)", name, cnt),
                                        None => "—".to_string(),
                                    };
                                    ui.label(egui::RichText::new(fav_name).strong().color(cfg.theme.primary_accent(cfg.custom_accent)));
                                });
                            });
                        });

                        ui.add_space(6.0);
                        if ui.add_sized([260.0, 26.0], egui::Button::new(egui::RichText::new(I18n::btn_share_top(cfg.language)).color(egui::Color32::BLACK).strong()).fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(13.0)).clicked() {
                            if let Some(path) = generate_stats_card(total_played, total_hours, &sorted_artists, cfg.theme, cfg.custom_accent, cfg.custom_bg, cfg.language) {
                                *self.export_notify.lock().unwrap() = Some("soundcloud_stats.png".to_string());
                                let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
                            }
                        }

                        if let Some(ref msg) = *self.export_notify.lock().unwrap() {
                            ui.label(egui::RichText::new(format!("Saved: {}", msg)).size(11.0).color(cfg.theme.primary_accent(cfg.custom_accent)));
                        }

                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(I18n::top_artists_header(cfg.language)).strong().size(13.0).color(egui::Color32::from_rgb(220, 215, 230)));

                        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(200.0).show(ui, |ui| {
                            ui.set_width(card_width);
                            if sorted_artists.is_empty() {
                                ui.label(egui::RichText::new(I18n::history_empty(cfg.language)).color(egui::Color32::GRAY));
                            } else {
                                let max_val = sorted_artists.first().map(|x| x.1).unwrap_or(1) as f32;
                                for (i, (artist, count)) in sorted_artists.iter().enumerate().take(15) {
                                    ui.group(|ui| {
                                        ui.set_width(card_width);
                                        ui.horizontal(|ui| {
                                            let rank_badge = match i { 0 => "🥇", 1 => "🥈", 2 => "🥉", _ => "•" };
                                            ui.label(egui::RichText::new(format!("{} #{}:", rank_badge, i + 1)).strong());
                                            ui.label(egui::RichText::new(artist).strong().color(cfg.theme.secondary_accent(cfg.custom_accent)));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.add_space(4.0);
                                                ui.label(egui::RichText::new(I18n::tracks_word(cfg.language, *count)).size(11.5).color(egui::Color32::from_rgb(170, 160, 185)));
                                            });
                                        });

                                        let ratio = (*count as f32 / max_val).clamp(0.05, 1.0);
                                        ui.add(egui::ProgressBar::new(ratio).desired_height(4.0).fill(cfg.theme.primary_accent(cfg.custom_accent)).rounding(4.0));
                                    });
                                    ui.add_space(2.0);
                                }
                            }
                        });
                    }

                    ActiveTab::History => {
                        ui.label(egui::RichText::new(I18n::tab_history(cfg.language)).strong().size(15.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(6.0);

                        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(350.0).show(ui, |ui| {
                            ui.set_width(card_width);
                            let hist = cfg.history.clone();
                            if hist.is_empty() {
                                ui.label(egui::RichText::new(I18n::history_empty(cfg.language)).color(egui::Color32::GRAY));
                            } else {
                                for item in hist.iter().rev() {
                                    ui.group(|ui| {
                                        ui.set_width(card_width);
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.set_max_width((card_width - 80.0).max(60.0));
                                                ui.label(egui::RichText::new(&item.track).strong().size(13.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                                                ui.label(egui::RichText::new(format!("{} • {}", item.artist, item.played_at)).size(11.0).color(egui::Color32::from_rgb(160, 150, 175)));
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.add_space(4.0);
                                                if ui.small_button("🔍").on_hover_text("Search SoundCloud").clicked() {
                                                    let q = format!("{} {}", item.artist, item.track);
                                                    let url = format!("https://soundcloud.com/search/sounds?q={}", encode(&q));
                                                    let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(2.0);
                                }
                            }
                        });
                    }

                    ActiveTab::Settings => {
                        ui.label(egui::RichText::new(I18n::settings_title(cfg.language)).strong().size(15.0).color(cfg.theme.secondary_accent(cfg.custom_accent)));
                        ui.add_space(6.0);

                        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(410.0).show(ui, |ui| {
                            ui.group(|ui| {
                                ui.set_width(card_width);

                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(I18n::settings_use_track_cover(cfg.language)).strong());
                                    let mut utc = cfg.use_track_cover;
                                    if ui.checkbox(&mut utc, "").changed() {
                                        self.config.lock().unwrap().use_track_cover = utc;
                                        self.save_config();
                                    }
                                });

                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(I18n::hud_elements_label(cfg.language)).strong());
                                    ui.menu_button(I18n::hud_open_btn(cfg.language), |ui| {
                                        let mut conf = self.config.lock().unwrap();
                                        let mut changed = false;

                                        let mut toggle = |val: &mut bool, label: &str, ui: &mut egui::Ui| {
                                            let text = if *val {
                                                format!("✔ {}", label)
                                            } else {
                                                format!("   {}", label)
                                            };
                                            if ui.button(text).clicked() {
                                                *val = !*val;
                                                true
                                            } else {
                                                false
                                            }
                                        };

                                        if toggle(&mut conf.show_controls, "Кнопки управления", ui) { changed = true; }
                                        if toggle(&mut conf.show_search_tab, "Вкладка «Поиск»", ui) { changed = true; }
                                        if toggle(&mut conf.show_top_tab, "Вкладка «Топ»", ui) { changed = true; }
                                        if toggle(&mut conf.show_history_tab, "Вкладка «История»", ui) { changed = true; }
                                        if toggle(&mut conf.show_profile_tab, "Вкладка «Профиль»", ui) { changed = true; }

                                        if changed {
                                            conf.save();
                                        }
                                    });
                                });

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(I18n::settings_layout(cfg.language)).strong());
                                    let mut l_mode = cfg.layout_mode;
                                    egui::ComboBox::from_id_source("layout_sel").selected_text(l_mode.label(cfg.language)).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut l_mode, UiLayoutMode::Spotify, UiLayoutMode::Spotify.label(cfg.language));
                                        ui.selectable_value(&mut l_mode, UiLayoutMode::Classic, UiLayoutMode::Classic.label(cfg.language));
                                    });
                                    if l_mode != cfg.layout_mode {
                                        self.config.lock().unwrap().layout_mode = l_mode;
                                        self.save_config();
                                    }
                                });

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(I18n::settings_lang(cfg.language)).strong());
                                    let mut lang_val = cfg.language;
                                    egui::ComboBox::from_id_source("lang_sel").selected_text(lang_val.label()).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut lang_val, AppLanguage::Ru, AppLanguage::Ru.label());
                                        ui.selectable_value(&mut lang_val, AppLanguage::En, AppLanguage::En.label());
                                        ui.selectable_value(&mut lang_val, AppLanguage::Ua, AppLanguage::Ua.label());
                                    });
                                    if lang_val != cfg.language {
                                        self.config.lock().unwrap().language = lang_val;
                                        self.save_config();
                                    }
                                });

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(I18n::settings_theme(cfg.language)).strong());
                                    let mut theme_val = cfg.theme;
                                    egui::ComboBox::from_id_source("theme_sel").selected_text(theme_val.label(cfg.language)).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut theme_val, AppTheme::Spotify, AppTheme::Spotify.label(cfg.language));
                                        ui.selectable_value(&mut theme_val, AppTheme::Kawaii, AppTheme::Kawaii.label(cfg.language));
                                        ui.selectable_value(&mut theme_val, AppTheme::Neverlose, AppTheme::Neverlose.label(cfg.language));
                                        ui.selectable_value(&mut theme_val, AppTheme::ShadowFiend, AppTheme::ShadowFiend.label(cfg.language));
                                        ui.selectable_value(&mut theme_val, AppTheme::SoundCloud, AppTheme::SoundCloud.label(cfg.language));
                                        ui.selectable_value(&mut theme_val, AppTheme::Custom, AppTheme::Custom.label(cfg.language));
                                    });
                                    if theme_val != cfg.theme {
                                        self.config.lock().unwrap().theme = theme_val;
                                        self.save_config();
                                    }
                                });

                                if cfg.theme == AppTheme::Custom {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.label(I18n::custom_accent_label(cfg.language));
                                        let mut color_32 = egui::Color32::from_rgb(cfg.custom_accent[0], cfg.custom_accent[1], cfg.custom_accent[2]);
                                        if ui.color_edit_button_srgba(&mut color_32).changed() {
                                            self.config.lock().unwrap().custom_accent = [color_32.r(), color_32.g(), color_32.b()];
                                            self.save_config();
                                        }

                                        ui.add_space(8.0);
                                        ui.label(I18n::custom_bg_label(cfg.language));
                                        let mut color_bg32 = egui::Color32::from_rgb(cfg.custom_bg[0], cfg.custom_bg[1], cfg.custom_bg[2]);
                                        if ui.color_edit_button_srgba(&mut color_bg32).changed() {
                                            self.config.lock().unwrap().custom_bg = [color_bg32.r(), color_bg32.g(), color_bg32.b()];
                                            self.save_config();
                                        }
                                    });
                                }

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.horizontal(|ui| {
                                    ui.label(I18n::toggle_particles(cfg.language));
                                    let mut p_val = cfg.enable_particles;
                                    if ui.checkbox(&mut p_val, "").changed() {
                                        self.config.lock().unwrap().enable_particles = p_val;
                                        self.save_config();
                                    }
                                });

                                if cfg.enable_particles {
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.label(I18n::particles_dir_label(cfg.language));
                                        let mut dir_val = cfg.particles_direction_up;
                                        egui::ComboBox::from_id_source("particles_dir_sel")
                                            .selected_text(if dir_val { I18n::dir_up(cfg.language) } else { I18n::dir_down(cfg.language) })
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut dir_val, true, I18n::dir_up(cfg.language));
                                                ui.selectable_value(&mut dir_val, false, I18n::dir_down(cfg.language));
                                            });
                                        if dir_val != cfg.particles_direction_up {
                                            self.config.lock().unwrap().particles_direction_up = dir_val;
                                            self.save_config();
                                        }
                                    });
                                }

                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(I18n::toggle_eq(cfg.language));
                                    let mut eq_val = cfg.enable_equalizer;
                                    if ui.checkbox(&mut eq_val, "").changed() {
                                        self.config.lock().unwrap().enable_equalizer = eq_val;
                                        self.save_config();
                                    }
                                });

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.horizontal(|ui| {
                                    ui.label(I18n::settings_clear_hist(cfg.language));
                                    if ui.small_button(I18n::btn_reset(cfg.language)).clicked() {
                                        let mut conf = self.config.lock().unwrap();
                                        conf.history.clear();
                                        conf.total_tracks_played = 0;
                                        conf.save();
                                        self.track_count.store(0, Ordering::SeqCst);
                                    }
                                });

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);

                                ui.label(egui::RichText::new("🧪 Экспериментальные возможности").strong().color(cfg.theme.primary_accent(cfg.custom_accent)));
                                ui.add_space(4.0);

                                if ui.add_sized([card_width, 30.0], egui::Button::new(egui::RichText::new("🚀 Запустить Beta HUD (Web UI)").strong().color(egui::Color32::WHITE)).fill(cfg.theme.widget_bg(cfg.custom_bg)).stroke(egui::Stroke::new(1.0_f32, cfg.theme.primary_accent(cfg.custom_accent))).rounding(6.0)).clicked() {
                                    launch_beta_hud();
                                    hide_app_window();
                                }
                            });
                        });
                    }
                }
            });
        });

        if self.show_volume_popup.load(Ordering::SeqCst) {
            let popup_pos = expected_popup_rect.min;
            egui::Area::new(egui::Id::new("vol_vertical_dropdown"))
                .fixed_pos(popup_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(cfg.theme.widget_bg(cfg.custom_bg))
                        .stroke(egui::Stroke::new(1.0_f32, cfg.theme.primary_accent(cfg.custom_accent)))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(6.0, 8.0))
                        .show(ui, |ui| {
                            ui.set_width(20.0);
                            ui.vertical_centered(|ui| {
                                let mut cur_vol = cfg.volume;
                                ui.spacing_mut().slider_width = 100.0;

                                if ui.add(egui::Slider::new(&mut cur_vol, 0.0..=1.0).vertical().show_value(false).trailing_fill(true)).changed() {
                                    self.config.lock().unwrap().volume = cur_vol;
                                    self.save_config();
                                    *self.pending_command.lock().unwrap() = Some(format!("set_volume:{:.2}", cur_vol));
                                }
                            });
                        });
                });
        }

        let lyrics_active = *self.active_tab.lock().unwrap() == ActiveTab::Lyrics;
        if is_on && (cfg.enable_equalizer || cfg.enable_particles || lyrics_active) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }
}

fn start_wasapi_capture(live_spectrum: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || {
        loop {
            let host = cpal::default_host();
            let device = match host.default_output_device() {
                Some(d) => d,
                None => {
                    thread::sleep(Duration::from_millis(1000));
                    continue;
                }
            };

            let config = match device.default_output_config() {
                Ok(c) => c,
                Err(_) => {
                    thread::sleep(Duration::from_millis(1000));
                    continue;
                }
            };

            let channels = config.channels() as usize;
            let sample_buf = Arc::new(Mutex::new(Vec::<f32>::with_capacity(4096)));
            let sample_buf_clone = sample_buf.clone();

            let err_fn = |_| {};
            let stream_config: cpal::StreamConfig = config.clone().into();

            let stream_res = match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[f32], _| {
                            let mut buf = sample_buf_clone.lock().unwrap();
                            for frame in data.chunks(channels) {
                                let mono = frame.iter().sum::<f32>() / channels as f32;
                                buf.push(mono);
                            }
                            if buf.len() > 8192 {
                                let excess = buf.len() - 4096;
                                buf.drain(0..excess);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::I16 => {
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[i16], _| {
                            let mut buf = sample_buf_clone.lock().unwrap();
                            for frame in data.chunks(channels) {
                                let mono = (frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>()) / channels as f32;
                                buf.push(mono);
                            }
                            if buf.len() > 8192 {
                                let excess = buf.len() - 4096;
                                buf.drain(0..excess);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::U16 => {
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[u16], _| {
                            let mut buf = sample_buf_clone.lock().unwrap();
                            for frame in data.chunks(channels) {
                                let mono = (frame.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum::<f32>()) / channels as f32;
                                buf.push(mono);
                            }
                            if buf.len() > 8192 {
                                let excess = buf.len() - 4096;
                                buf.drain(0..excess);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                _ => Err(cpal::BuildStreamError::DeviceNotAvailable),
            };

            let stream = match stream_res {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(1500));
                    continue;
                }
            };

            if stream.play().is_err() {
                thread::sleep(Duration::from_millis(1500));
                continue;
            }

            let fft_size = 1024;
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(fft_size);
            let mut smoothed = [0.0f32; 32];
            let mut dynamic_peak = 0.05f32;

            while let Ok(mut buf) = sample_buf.lock() {
                if buf.len() >= fft_size {
                    let start = buf.len() - fft_size;
                    let samples = buf[start..].to_vec();
                    buf.drain(0..start);
                    drop(buf);

                    let rms: f32 = (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt();

                    if rms < 0.0008 {
                        for s in smoothed.iter_mut() {
                            *s += (0.0 - *s) * 0.25;
                        }
                        let mut bands = vec![0u8; 32];
                        for i in 0..32 {
                            bands[i] = smoothed[i] as u8;
                        }
                        *live_spectrum.lock().unwrap() = bands;
                        thread::sleep(Duration::from_millis(16));
                        continue;
                    }

                    let mut complex_buf: Vec<Complex<f32>> = samples
                        .iter()
                        .enumerate()
                        .map(|(i, &s)| {
                            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos());
                            Complex::new(s * window, 0.0)
                        })
                        .collect();

                    fft.process(&mut complex_buf);

                    let num_bands = 32;
                    let mut raw_energies = [0.0f32; 32];
                    let mut frame_max = 0.0001f32;

                    for i in 0..num_bands {
                        let p1 = (i as f32 / num_bands as f32).powf(1.7);
                        let p2 = ((i + 1) as f32 / num_bands as f32).powf(1.7);
                        let b_start = ((1.0 + p1 * 360.0) as usize).min(fft_size / 2);
                        let b_end = (((1.0 + p2 * 360.0) as usize) + 1).min(fft_size / 2).max(b_start + 1);

                        let mut sum = 0.0;
                        for b in b_start..b_end {
                            let mag = (complex_buf[b].re.powi(2) + complex_buf[b].im.powi(2)).sqrt();
                            sum += mag;
                        }

                        let avg = sum / (b_end - b_start) as f32;
                        let weight = if i < 8 {
                            2.9 - (i as f32 * 0.14)
                        } else {
                            1.5 + ((i - 8) as f32 / (num_bands - 8) as f32) * 2.2
                        };

                        let energy = avg * weight;
                        raw_energies[i] = energy;
                        if energy > frame_max {
                            frame_max = energy;
                        }
                    }

                    if frame_max > dynamic_peak {
                        dynamic_peak += (frame_max - dynamic_peak) * 0.35;
                    } else {
                        dynamic_peak -= (dynamic_peak - frame_max) * 0.025;
                    }
                    dynamic_peak = dynamic_peak.clamp(0.004, 50.0);

                    let mut bands = vec![0u8; num_bands];
                    for i in 0..num_bands {
                        let norm = (raw_energies[i] / dynamic_peak).clamp(0.0, 1.0);
                        let boosted = norm.powf(0.5);
                        let target = (boosted * 252.0).clamp(0.0, 255.0);

                        if target > smoothed[i] {
                            smoothed[i] += (target - smoothed[i]) * 0.55;
                        } else {
                            smoothed[i] += (target - smoothed[i]) * 0.22;
                        }

                        bands[i] = smoothed[i] as u8;
                    }

                    *live_spectrum.lock().unwrap() = bands;
                } else {
                    drop(buf);
                }

                thread::sleep(Duration::from_millis(16));
            }
        }
    });
}

fn main() -> Result<(), eframe::Error> {
    let cfg = Arc::new(Mutex::new(AppConfig::load()));

    let tray_menu = Menu::new();
    let show_beta = MenuItem::new("Открыть Beta HUD", true, None);
    let show_item = MenuItem::new("Открыть SoundCloud RPC", true, None);
    let quit_item = MenuItem::new("Выход", true, None);
    let _ = tray_menu.append_items(&[&show_beta, &show_item, &quit_item]);

    let _tray_icon = load_tray_icon().and_then(|icon| {
        TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("SoundCloud RPC")
            .with_icon(icon)
            .build()
            .ok()
    });

    let show_beta_id = show_beta.id().clone();
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    thread::spawn(move || {
        loop {
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                    restore_app_window();
                }
            }

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == show_beta_id {
                    launch_beta_hud();
                    hide_app_window();
                } else if event.id == show_id {
                    restore_app_window();
                } else if event.id == quit_id {
                    std::process::exit(0);
                }
            }

            thread::sleep(Duration::from_millis(50));
        }
    });

    let shared_payload = Arc::new(Mutex::new(None));
    let pending_command = Arc::new(Mutex::new(None));
    let is_playing_now = Arc::new(AtomicBool::new(false));
    let repeat_mode = Arc::new(Mutex::new("none".to_string()));
    let sc_client_id = Arc::new(Mutex::new("2t9loNfh900mioJ2DU11YtSXQuVUbmyt".to_string()));
    let user_name = Arc::new(Mutex::new("".to_string()));
    let user_url = Arc::new(Mutex::new("".to_string()));
    let user_playlists = Arc::new(Mutex::new(Vec::new()));
    let live_spectrum = Arc::new(Mutex::new(vec![0u8; 32]));

    start_wasapi_capture(live_spectrum.clone());

    let mut custom_bytes = None;
    if let Some(ref path_str) = cfg.lock().unwrap().custom_image_path {
        if let Ok(bytes) = fs::read(PathBuf::from(path_str)) {
            custom_bytes = Some(bytes);
        }
    }

    let state = AppState {
        config: cfg.clone(),
        is_enabled: Arc::new(AtomicBool::new(true)),
        status_text: Arc::new(Mutex::new("Запуск...".to_string())),
        current_track: Arc::new(Mutex::new("Ожидание трека...".to_string())),
        current_artist: Arc::new(Mutex::new("".to_string())),
        current_title: Arc::new(Mutex::new("".to_string())),
        progress_ratio: Arc::new(Mutex::new(0.0)),
        progress_text: Arc::new(Mutex::new("00:00 / 00:00".to_string())),
        current_sec: Arc::new(Mutex::new(0.0)),
        track_duration: Arc::new(Mutex::new(0.0)),
        track_count: Arc::new(AtomicU32::new(cfg.lock().unwrap().total_tracks_played)),
        show_volume_popup: Arc::new(AtomicBool::new(false)),
        vol_btn_rect: Arc::new(Mutex::new(egui::Rect::ZERO)),
        custom_image_bytes: Arc::new(Mutex::new(custom_bytes)),
        track_cover_rgba: Arc::new(Mutex::new(None)),
        track_cover_url: Arc::new(Mutex::new(String::new())),
        track_cover_version: Arc::new(AtomicU32::new(0)),
        image_version: Arc::new(AtomicU32::new(0)),
        active_tab: Arc::new(Mutex::new(ActiveTab::Player)),
        export_notify: Arc::new(Mutex::new(None)),
        copy_notify: Arc::new(Mutex::new(None)),
        shared_track_payload: shared_payload.clone(),
        pending_command: pending_command.clone(),
        is_playing_now: is_playing_now.clone(),
        repeat_mode: repeat_mode.clone(),
        sc_client_id: sc_client_id.clone(),
        user_name: user_name.clone(),
        user_url: user_url.clone(),
        user_playlists: user_playlists.clone(),
        search_query: Arc::new(Mutex::new("".to_string())),
        search_results: Arc::new(Mutex::new(Vec::new())),
        is_searching: Arc::new(AtomicBool::new(false)),
        has_searched: Arc::new(AtomicBool::new(false)),
        live_spectrum: live_spectrum.clone(),
        liked_now: Arc::new(AtomicBool::new(false)),
        followed_now: Arc::new(AtomicBool::new(false)),
    };

    let http_payload_arc = shared_payload.clone();
    let http_cmd_arc = pending_command.clone();
    let http_playing_arc = is_playing_now.clone();
    let http_repeat_arc = repeat_mode.clone();
    let http_cid_arc = sc_client_id.clone();
    let http_uname_arc = user_name.clone();
    let http_uurl_arc = user_url.clone();
    let search_results_arc: Arc<Mutex<Vec<SearchTrackItem>>> = Arc::new(Mutex::new(Vec::new()));
    let http_search_results_arc = search_results_arc.clone();
    let http_playlists_arc = user_playlists.clone();
    let http_cfg_arc = cfg.clone();
    let http_spectrum_arc = live_spectrum.clone();
    let http_liked_arc = state.liked_now.clone();
    let http_followed_arc = state.followed_now.clone();

    let server_cover_rgba = state.track_cover_rgba.clone();
    let server_cover_url = state.track_cover_url.clone();
    let server_cover_ver = state.track_cover_version.clone();
    let server_track_str = state.current_track.clone();
    let server_artist_str = state.current_artist.clone();
    let server_title_str = state.current_title.clone();
    let server_ratio = state.progress_ratio.clone();
    let server_ptext = state.progress_text.clone();
    let server_psec = state.current_sec.clone();
    let server_dur = state.track_duration.clone();

    thread::spawn(move || {
        if let Ok(server) = Server::http("127.0.0.1:23456") {
            for mut request in server.incoming_requests() {
                let raw_url = request.url().to_string();
                let path = raw_url.split('?').next().unwrap_or("").trim_end_matches('/');

                if request.method().as_str() == "OPTIONS" {
                    let mut resp = Response::from_string("ok");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, OPTIONS"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"*"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/beta" && request.method().as_str() == "GET" {
                    let mut resp = Response::from_string(BETA_HTML_CONTENT);
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store, no-cache, must-revalidate"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if (path == "/state" || path == "/status") && request.method().as_str() == "GET" {
                    let mut state_val = if let Some((ref data, _)) = *http_payload_arc.lock().unwrap() {
                        serde_json::to_value(data).unwrap_or_else(|_| serde_json::json!({}))
                    } else {
                        serde_json::json!({
                            "track": "",
                            "artist": "",
                            "passed": 0,
                            "duration": 0,
                            "cover": "",
                            "is_playing": false,
                        })
                    };

                    let current_cfg = http_cfg_arc.lock().unwrap().clone();
                    let hist = current_cfg.history.clone();
                    let total_played = current_cfg.total_tracks_played;
                    let mut artist_counts: HashMap<String, usize> = HashMap::new();
                    let mut total_seconds: u64 = 0;
                    for entry in &hist {
                        *artist_counts.entry(entry.artist.clone()).or_insert(0) += 1;
                        total_seconds += entry.duration_sec.unwrap_or(150);
                    }
                    let mut sorted_artists: Vec<(String, usize)> = artist_counts.into_iter().collect();
                    sorted_artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    let top_artist = sorted_artists.first().map(|x| x.0.clone()).unwrap_or_else(|| "—".to_string());
                    let total_hours = total_seconds as f32 / 3600.0;

                    let current_spectrum = http_spectrum_arc.lock().unwrap().clone();
                    let liked = state_val.get("liked").and_then(|v| v.as_bool()).unwrap_or(false);
                    let followed = state_val.get("followed").and_then(|v| v.as_bool()).unwrap_or(false);

                    if let Some(obj) = state_val.as_object_mut() {
                        obj.insert("spectrum".to_string(), serde_json::to_value(current_spectrum).unwrap_or_default());
                        obj.insert("liked".to_string(), serde_json::json!(liked));
                        obj.insert("followed".to_string(), serde_json::json!(followed));
                        obj.insert("repeat".to_string(), serde_json::json!(http_repeat_arc.lock().unwrap().clone()));
                        obj.insert("user_name".to_string(), serde_json::json!(http_uname_arc.lock().unwrap().clone()));
                        obj.insert("user_url".to_string(), serde_json::json!(http_uurl_arc.lock().unwrap().clone()));
                        obj.insert("user_avatar".to_string(), serde_json::json!(http_payload_arc.lock().unwrap().as_ref().map(|(p, _)| p.user_avatar.clone()).unwrap_or_default()));
                        obj.insert("search_results".to_string(), serde_json::to_value(http_search_results_arc.lock().unwrap().clone()).unwrap_or_default());
                        obj.insert("volume".to_string(), serde_json::json!(current_cfg.volume));
                        obj.insert("history".to_string(), serde_json::to_value(&current_cfg.history).unwrap_or_default());
                        obj.insert("stats".to_string(), serde_json::json!({
                            "total_played": total_played,
                            "total_hours": total_hours,
                            "top_artist": top_artist,
                            "top_artists": sorted_artists
                        }));
                        obj.insert("config".to_string(), serde_json::json!({
                            "theme": match current_cfg.theme {
                                AppTheme::Kawaii => "Kawaii",
                                AppTheme::Neverlose => "Neverlose",
                                AppTheme::ShadowFiend => "ShadowFiend",
                                AppTheme::SoundCloud => "SoundCloud",
                                _ => "Spotify",
                            },
                            "language": match current_cfg.language {
                                AppLanguage::En => "En",
                                AppLanguage::Ua => "Ua",
                                _ => "Ru",
                            },
                            "enable_particles": current_cfg.enable_particles,
                            "particles_direction_up": current_cfg.particles_direction_up,
                            "use_track_cover": current_cfg.use_track_cover,
                            "enable_equalizer": current_cfg.enable_equalizer,
                            "role": current_cfg.role.label(),
                        }));
                    }

                    let state_json = state_val.to_string();
                    let mut resp = Response::from_string(state_json);
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store, no-cache, must-revalidate"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/spectrum" && request.method().as_str() == "GET" {
                    let spec = http_spectrum_arc.lock().unwrap().clone();
                    let mut resp = Response::from_string(serde_json::to_string(&spec).unwrap_or_else(|_| "[]".to_string()));
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/search" && request.method().as_str() == "GET" {
                    let mut q = String::new();
                    if let Some(query_part) = raw_url.split('?').nth(1) {
                        for pair in query_part.split('&') {
                            if let Some((k, v)) = pair.split_once('=') {
                                if k == "q" {
                                    q = urlencoding::decode(v).unwrap_or_default().to_string();
                                }
                            }
                        }
                    }
                    let results = search_soundcloud_tracks(&q, http_cid_arc.clone());
                    let res_json = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
                    let mut resp = Response::from_string(res_json);
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/search_results" && request.method().as_str() == "POST" {
                    let mut content = String::new();
                    if request.as_reader().read_to_string(&mut content).is_ok() {
                        if let Ok(results) = serde_json::from_str::<Vec<SearchTrackItem>>(&content) {
                            *http_search_results_arc.lock().unwrap() = results;
                        }
                    }
                    let mut resp = Response::from_string("{\"status\":\"ok\"}");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/soundcloud-mark.svg" && request.method().as_str() == "GET" {
                    let mut resp = Response::from_string(include_str!("soundcloud-mark.svg"));
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"image/svg+xml"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/settings" && request.method().as_str() == "POST" {
                    let mut content = String::new();
                    if request.as_reader().read_to_string(&mut content).is_ok() {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            let mut conf = http_cfg_arc.lock().unwrap();
                            if let Some(t) = val.get("theme").and_then(|v| v.as_str()) {
                                conf.theme = match t {
                                    "Kawaii" => AppTheme::Kawaii,
                                    "Neverlose" => AppTheme::Neverlose,
                                    "ShadowFiend" => AppTheme::ShadowFiend,
                                    "SoundCloud" => AppTheme::SoundCloud,
                                    _ => AppTheme::Spotify,
                                };
                            }
                            if let Some(l) = val.get("language").and_then(|v| v.as_str()) {
                                conf.language = match l {
                                    "En" => AppLanguage::En,
                                    "Ua" => AppLanguage::Ua,
                                    _ => AppLanguage::Ru,
                                };
                            }
                            if let Some(p) = val.get("enable_particles").and_then(|v| v.as_bool()) {
                                conf.enable_particles = p;
                            }
                            if let Some(pu) = val.get("particles_direction_up").and_then(|v| v.as_bool()) {
                                conf.particles_direction_up = pu;
                            }
                            if let Some(c) = val.get("use_track_cover").and_then(|v| v.as_bool()) {
                                conf.use_track_cover = c;
                            }
                            if let Some(e) = val.get("enable_equalizer").and_then(|v| v.as_bool()) {
                                conf.enable_equalizer = e;
                            }
                            conf.save();
                        }
                    }

                    let mut resp = Response::from_string("{\"status\":\"ok\"}");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/command" && request.method().as_str() == "POST" {
                    let mut content = String::new();
                    if request.as_reader().read_to_string(&mut content).is_ok() {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(cmd) = val.get("command").and_then(|c| c.as_str()) {
                                if cmd == "open_settings" || cmd == "close_beta" {
                                    restore_app_window();
                                } else if cmd == "clear_history" {
                                    let mut conf = http_cfg_arc.lock().unwrap();
                                    conf.history.clear();
                                    conf.total_tracks_played = 0;
                                    conf.save();
                                } else if cmd == "export_stats_card" {
                                    let current_cfg = http_cfg_arc.lock().unwrap().clone();
                                    let hist = current_cfg.history.clone();
                                    let total_played = current_cfg.total_tracks_played;
                                    let mut artist_counts: HashMap<String, usize> = HashMap::new();
                                    let mut total_seconds: u64 = 0;
                                    for entry in &hist {
                                        *artist_counts.entry(entry.artist.clone()).or_insert(0) += 1;
                                        total_seconds += entry.duration_sec.unwrap_or(150);
                                    }
                                    let mut sorted_artists: Vec<(String, usize)> = artist_counts.into_iter().collect();
                                    sorted_artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                                    let total_hours = total_seconds as f32 / 3600.0;
                                    if let Some(path) = generate_stats_card(total_played, total_hours, &sorted_artists, current_cfg.theme, current_cfg.custom_accent, current_cfg.custom_bg, current_cfg.language) {
                                        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
                                    }
                                } else {
                                    if cmd.starts_with("set_volume:") {
                                        if let Ok(v) = cmd[11..].parse::<f32>() {
                                            let mut conf = http_cfg_arc.lock().unwrap();
                                            conf.volume = v;
                                            conf.save();
                                        }
                                    }
                                    *http_cmd_arc.lock().unwrap() = Some(cmd.to_string());
                                }
                            }
                        }
                    }

                    let mut resp = Response::from_string("{\"status\":\"ok\"}");
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                    continue;
                }

                if path == "/update" && request.method().as_str() == "POST" {
                    let mut content = String::new();
                    if request.as_reader().read_to_string(&mut content).is_ok() {
                        if let Ok(data) = serde_json::from_str::<IncomingTrackPayload>(&content) {
                            if !data.track.trim().is_empty() {
                                http_playing_arc.store(data.is_playing, Ordering::SeqCst);
                                http_liked_arc.store(data.liked || data.spectrum.first() == Some(&1), Ordering::SeqCst);
                                http_followed_arc.store(data.followed || data.spectrum.get(1) == Some(&1), Ordering::SeqCst);
                                *http_repeat_arc.lock().unwrap() = data.repeat.clone();
                                if !data.client_id.trim().is_empty() {
                                    *http_cid_arc.lock().unwrap() = data.client_id.clone();
                                }
                                if !data.user_name.trim().is_empty() {
                                    *http_uname_arc.lock().unwrap() = data.user_name.clone();
                                }
                                if !data.user_url.trim().is_empty() {
                                    *http_uurl_arc.lock().unwrap() = data.user_url.clone();
                                }
                                if !data.playlists.is_empty() {
                                    *http_playlists_arc.lock().unwrap() = data.playlists.clone();
                                }

                                *server_artist_str.lock().unwrap() = data.artist.trim().to_string();
                                *server_title_str.lock().unwrap() = data.track.trim().to_string();
                                *server_track_str.lock().unwrap() = format!("{} — {}", data.track.trim(), data.artist.trim());
                                *server_ratio.lock().unwrap() = if data.duration > 0 { (data.passed as f32 / data.duration as f32).clamp(0.0, 1.0) } else { 0.0 };
                                *server_ptext.lock().unwrap() = format!("{} / {}", format_duration(data.passed), format_duration(data.duration));
                                *server_psec.lock().unwrap() = data.passed as f32;
                                *server_dur.lock().unwrap() = data.duration as f32;

                                if data.cover.starts_with("http") {
                                    let last_cov = server_cover_url.lock().unwrap().clone();
                                    if last_cov != data.cover {
                                        *server_cover_url.lock().unwrap() = data.cover.clone();
                                        let c_url = data.cover.clone();
                                        let c_rgba = server_cover_rgba.clone();
                                        let c_ver = server_cover_ver.clone();

                                        thread::spawn(move || {
                                            if let Some((raw_bytes, dims)) = download_cover_bytes(&c_url) {
                                                *c_rgba.lock().unwrap() = Some((raw_bytes, dims));
                                                c_ver.fetch_add(1, Ordering::SeqCst);
                                            }
                                        });
                                    }
                                }

                                *http_payload_arc.lock().unwrap() = Some((data, Instant::now()));
                            }
                        }
                    }

                    let cmd = {
                        let mut guard = http_cmd_arc.lock().unwrap();
                        guard.take()
                    };

                    let resp_str = match cmd {
                        Some(c) => serde_json::json!({ "status": "ok", "command": c }).to_string(),
                        None => "{\"status\":\"ok\"}".to_string(),
                    };

                    let mut resp = Response::from_string(resp_str);
                    resp.add_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                    resp.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = request.respond(resp);
                } else {
                    let _ = request.respond(Response::from_string("404").with_status_code(404));
                }
            }
        }
    });

    let bg_state = state.clone();
    thread::spawn(move || {
        let mut client = DiscordIpcClient::new(CLIENT_ID).ok();
        let mut is_connected = false;
        let mut active_track: Option<String> = None;
        let mut active_artist: Option<String> = None;
        let mut last_passed_sec: Option<u64> = None;
        let mut last_playing: Option<bool> = None;
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
                    last_playing = None;
                    track_counted = false;
                }
                thread::sleep(Duration::from_millis(400));
                continue;
            }

            if !is_connected {
                if let Some(ref mut ipc) = client {
                    if ipc.connect().is_ok() {
                        is_connected = true;
                    }
                }
            }

            let maybe_data = {
                let guard = bg_state.shared_track_payload.lock().unwrap();
                guard.clone()
            };

            let valid_payload = match maybe_data {
                Some((data, updated_at)) if updated_at.elapsed() < Duration::from_secs(5) => {
                    if !data.track.is_empty() { Some(data) } else { None }
                }
                _ => None,
            };

            match valid_payload {
                Some(payload) => {
                    let track_str = payload.track.trim().to_string();
                    let artist_str = payload.artist.trim().to_string();
                    let p = payload.passed;
                    let d = payload.duration;
                    let is_playing = payload.is_playing;

                    if is_playing && !track_counted && d >= 20 {
                        if (d > p && (d - p) <= 4) || (p as f32 / d as f32 >= 0.85) {
                            track_counted = true;
                            bg_state.track_count.fetch_add(1, Ordering::SeqCst);
                            {
                                let mut conf = bg_state.config.lock().unwrap();
                                conf.total_tracks_played += 1;
                                conf.history.push(HistoryEntry {
                                    track: track_str.clone(),
                                    artist: artist_str.clone(),
                                    played_at: Local::now().format("%H:%M").to_string(),
                                    duration_sec: Some(d),
                                    cover: payload.cover.clone(),
                                });
                                if conf.history.len() > 200 { conf.history.remove(0); }
                                conf.save();
                            }
                        }
                    }

                    let track_changed = active_track.as_deref() != Some(&track_str) || active_artist.as_deref() != Some(&artist_str);
                    let play_state_changed = last_playing != Some(is_playing);
                    let seeked = match (Some(p), last_passed_sec) {
                        (Some(curr), Some(prev)) => (curr as i64 - prev as i64).abs() > 2,
                        _ => false,
                    };

                    if track_changed {
                        track_counted = false;
                    }

                    if is_connected && (track_changed || play_state_changed || seeked) {
                        active_track = Some(track_str.clone());
                        active_artist = Some(artist_str.clone());
                        last_passed_sec = Some(p);
                        last_playing = Some(is_playing);

                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                        let count_val = bg_state.track_count.load(Ordering::SeqCst);
                        let large_img = if payload.cover.starts_with("http") && payload.cover.len() <= 256 {
                            payload.cover.as_str()
                        } else {
                            FALLBACK_LARGE_IMAGE
                        };

                        let state_str = if is_playing {
                            if count_val > 0 { format!("by {} • [#{}]", artist_str, count_val) } else { format!("by {}", artist_str) }
                        } else {
                            format!("⏸ На паузе • {}", artist_str)
                        };

                        let small_txt = format!("Tracks: {}", count_val);
                        let start_time = now - p as i64;
                        let end_time = start_time + d as i64;
                        let timestamps = activity::Timestamps::new().start(start_time).end(end_time);

                        let mut assets = activity::Assets::new().large_image(large_img).large_text(&track_str);
                        if is_playing {
                            assets = assets.small_image(PLAY_IMAGE_KEY).small_text(&small_txt);
                        }

                        let mut discord_payload = activity::Activity::new()
                            .activity_type(activity::ActivityType::Listening)
                            .details(&track_str)
                            .state(&state_str)
                            .assets(assets);

                        if is_playing {
                            discord_payload = discord_payload.timestamps(timestamps);
                        }

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
                        last_playing = None;
                        track_counted = false;
                        if let Some(ref mut ipc) = client {
                            let _ = ipc.clear_activity();
                        }
                    }
                }
            }

            thread::sleep(Duration::from_millis(400));
        }
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([440.0, 540.0])
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
