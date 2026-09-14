# 🎵 SoundCloud Discord RPC (zen_rpc)

Кастомный Discord Rich Presence для SoundCloud с поддержкой работы в любых фоновых вкладках, сворачиванием в трей, таймлайном трека, караоке и статистикой.

## ✨ Возможности
- 🌐 **Работает на любой вкладке:** вкладку с музыкой больше не нужно выносить в отдельное окно — данные передаются напрямую через локальный сервер.
- 🎧 **Синхронизация:** отображение названия трека, исполнителя и оригинальной обложки высокого качества.
- ⏱️ **Таймлайн:** интерактивный прогресс-бар и оставшееся время воспроизведения в Discord.
- 📥 **Системный трей:** сворачивание в фон в один клик без захламления панели задач (быстрое открытие по клику или меню ПКМ).
- 🎤 **Караоке:** синхронизированный текст песен в реальном времени (через LRCLIB).
- 📊 **Музыкальная статистика:** история прослушиваний и генерация красивой карточки с топом артистов.
- 🎨 **Кастомизация:** встроенные темы (Kawaii, Neverlose, Shadow Fiend, SoundCloud), выбор своих цветов и анимированные искры.
- 🖥️ **Beta HUD:** отдельное нативное Windows-окно на C++ и WebView2 с фиксированным размером, собственными кнопками окна и полноэкранным режимом.

---

## 🚀 Установка и настройка

### 1. Браузер (Tampermonkey)
1. Установите расширение **[Tampermonkey](https://www.tampermonkey.net/)** в ваш браузер (Chrome, Zen, Opera, Edge, Firefox и др.).
2. Создайте новый скрипт и вставьте код из файла [`soundcloud.user.js`](./soundcloud.user.js).
3. Сохраните скрипт (`Ctrl + S`).

### 2. Запуск приложения
1. Скачайте последний релиз `zen_rpc.exe` из раздела **Releases** (или соберите самостоятельно: `cargo build --release`).
2. Запустите `zen_rpc.exe`.
3. Откройте любой трек на [SoundCloud](https://soundcloud.com/) — статус подтянется автоматически.

---

## 🛠 Сборка из исходников

Требуется установленный Rust (Cargo):

```bash
git clone [https://github.com/farma312/soundcloud-discord-rpc-.git](https://github.com/farma312/soundcloud-discord-rpc-.git)
cd soundcloud-discord-rpc-
cargo run --release
```

### Нативный Beta HUD

Основной HUD запускается через локальный Rust HTTP-сервер, а нативное окно Beta HUD размещено в [`cpp_hud`](./cpp_hud). Интерфейс загружается через Microsoft WebView2: Rust отвечает за синхронизацию и команды, C++ - за Windows-окно.

Требуется:

- Visual Studio 2022 с workload **Desktop development with C++**;
- CMake 3.21 или новее;
- WebView2 Runtime;
- Windows 10 или новее.

Сборка из Developer PowerShell:

```powershell
cd cpp_hud
cmake -S . -B build -A x64
cmake --build build --config Release
```

После запуска `zen_rpc.exe` готовый host находится здесь:

```text
cpp_hud\build\Release\zen_beta_hud.exe
```

При нажатии **Beta HUD** Rust автоматически запускает этот host. Если executable еще не собран, используется браузерный fallback.

Исходник интерфейса находится в [`src/beta_v2.html`](./src/beta_v2.html), а host WebView2 - в [`cpp_hud/main.cpp`](./cpp_hud/main.cpp).
