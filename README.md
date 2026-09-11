# 🎵 SoundCloud Discord Rich Presence (zen_rpc)

A modern, customizable Discord Rich Presence client for **SoundCloud**, built with **Rust** and the **egui** framework.

---

## English

### ✨ Features
- **Real-Time Discord Presence:** Accurate live progress bar, timestamps, and track album art displayed directly on your Discord profile.
- **Interactive Player UI:**
  - 🎤 Real-time synced karaoke lyrics display (LRC).
  - ⚡ 60 FPS smooth neon audio visualizer (Equalizer).
  - 💿 Animated rotating vinyl record effect.
  - 🌌 Interactive floating background particles (Sparks) with flight direction control (Upward/Downward).
- **Themes & Aesthetics:**
  - Presets: Kawaii (Pink), Neverlose (Cyan), Shadow Fiend (Ruby), SoundCloud (Orange).
  - 🎨 Built-in Custom Theme Editor with live color pickers for accent and background.
  - Custom local avatar image support.
- **Multilingual Support:** English, Russian (Русский), and Ukrainian (Українська).
- **Listening Stats & History:** Local track counter, top artists calculation, and single-click `.png` summary card generation.
- **Smart Clipboard:** Quick `📋` button to copy shareable track links with one click.
- **Multi-Browser Support:** Google Chrome, Opera GX, Zen Browser, Microsoft Edge, Mozilla Firefox, Yandex Browser.

### 🚀 Quick Start
1. Download the latest `zen_rpc.exe` from the [Releases](https://github.com/farma312/soundcloud-discord-rpc-/blob/main/soundcloud.user.js) tab.
2. Install the **Tampermonkey** extension in your browser.
3. **Enable Developer Mode** (Crucial for Chromium browsers):
   - Open your browser's Extensions page (`chrome://extensions` or `opera://extensions`).
   - Turn on the **Developer mode** toggle in the top-right corner (required for Tampermonkey to run userscripts).
4. Import the userscript from [`soundcloud_rpc.user.js`](./soundcloud_rpc.user.js).
5. Open SoundCloud and detach the tab into a **separate browser window** (so other active tabs don't override the window title).
6. Launch `zen_rpc.exe` and enjoy!

---

## Русский

### ✨ Возможности
- **Синхронизация в реальном времени:** точный прогресс-бар, тайминги и оригинальная обложка трека прямо в профиле Discord.
- **Интерактивный плеер:**
  - 🎤 Вкладка караоке с синхронизацией строк в реальном времени (LRC).
  - ⚡ 60 FPS плавный неоновый эквалайзер.
  - 💿 Анимированная вращающаяся виниловая пластинка.
  - 🌌 Интерактивные фоновые искры (Sparks) с выбором направления полета (Вверх/Вниз).
- **Кастомизация UI:**
  - Готовые темы: Kawaii (Pink), Neverlose (Cyan), Shadow Fiend (Ruby), SoundCloud (Orange).
  - 🎨 Встроенный редактор своей темы с живыми пипетками цвета.
  - Поддержка установки своей аватарки.
- **Мультиязычность:** English, Русский, Українська.
- **Статистика и история:** трекинг дослушанных треков, таблица топа артистов и экспорт инфографики в `.png`.
- **Умный буфер:** кнопка `📋` для мгновенного копирования ссылки на трек для друзей.
- **Поддержка браузеров:** Google Chrome, Opera GX, Zen Browser, Microsoft Edge, Mozilla Firefox, Яндекс Браузер.

### 🚀 Установка и запуск
1. Скачай последнюю версию `zen_rpc.exe` из раздела [Releases](https://github.com/farma312/soundcloud-discord-rpc-/blob/main/soundcloud.user.js).
2. Установи расширение **Tampermonkey** в свой браузер.
3. **Включи «Режим разработчика»** (Обязательно для Chrome, Opera GX и Chromium):
   - Перейди на страницу расширений (`chrome://extensions` или `opera://extensions`).
   - В правом верхнем углу включи тумблер **«Режим разработчика»** (иначе браузер блокирует запуск пользовательских скриптов).
4. Добавь скрипт из файла [`soundcloud_rpc.user.js`](./soundcloud_rpc.user.js).
5. Открой SoundCloud и вынеси вкладку в **отдельное окно браузера** (чтобы другие вкладки не перебивали заголовок окна).
6. Запусти `zen_rpc.exe` и слушай музыку!
