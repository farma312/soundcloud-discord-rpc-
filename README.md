# soundcloud-discord-rpc-
Custom Discord Rich Presence for SoundCloud with live timeline, synced karaoke lyrics, themes, and stats.
# 🎵 SoundCloud Discord RPC

Кастомный Discord Rich Presence для **SoundCloud** с живым таймлайном, караоке-текстом, топом исполнителей и кастомными темами.

---

## ✨ Возможности

* 🎧 **Статус «Слушает soundcloud»:** полноценная бегущая полоска прогресса в профиле Discord.
* 🖼 **Обложка трека:** автоматически подтягивает оригинальную обложку песни в высоком качестве.
* 🎤 **Режим караоке (LRC Sync):** автоматический поиск текста трека с синхронизированной подсветкой активной строки.
* 📊 **Статистика и Топ:** персональный чарт любимых исполнителей за всё время с возможностью сохранить инфографику в PNG (`📸 Поделиться топом`).
* 🎨 **Темы оформления:** Кавайная розовая, Neverlose (Cyan), Shadow Fiend (Ruby) и SoundCloud Classic.
* 🌐 **Поддержка браузеров:** Zen Browser, Chrome, Opera / Opera GX, Яндекс Браузер, Edge, Firefox.

---

## ⚠️ Важное примечание по работе браузеров

> **Важно:** Браузеры на базе Chromium (Chrome, Opera GX, Edge, Яндекс) передают в заголовок окна только название **активной** вкладки. 
> 
> Чтобы трек считывался всегда (даже когда вы серфите по другим сайтам или играете), **вынесите вкладку с SoundCloud в отдельное окно браузера** (просто перетащите вкладку мышкой наружу) и сверните/оставьте на фоне. В самом приложении можно нажать кнопку **«Привязать окно браузера»**.

---

## 🚀 Установка и запуск (за 1 минуту)

### Шаг 1. Скачай программу
1. Перейди в раздел [**Releases**](../../releases) справа.
2. Скачай файл `zen_rpc.exe` из последнего релиза.

### Шаг 2. Установи браузерный скрипт
1. Установи расширение [Tampermonkey](https://www.tampermonkey.net/) для своего браузера.
2. Открой Tampermonkey -> **Создать новый скрипт**.
3. Вставь следующий код и нажми `Ctrl + S`:

```javascript
// ==UserScript==
// @name         SoundCloud Time & Cover in Title
// @namespace    [https://tampermonkey.net/](https://tampermonkey.net/)
// @version      4.0
// @description  Adds playback times, cover image and true artist to window title
// @match        [https://soundcloud.com/](https://soundcloud.com/)*
// @grant        none
// @run-at       document-idle
// ==/UserScript==

(function() {
    'use strict';

    setInterval(() => {
        const passedEl = document.querySelector('.playbackTimeline__timePassed span[aria-hidden="true"]');
        const durationEl = document.querySelector('.playbackTimeline__duration span[aria-hidden="true"]');

        if (!passedEl || !durationEl) return;

        const passed = passedEl.textContent.trim();
        const duration = durationEl.textContent.trim();

        let imgUrl = '';
        const coverSpan = document.querySelector('.playControls__elements .image span');
        if (coverSpan) {
            const bg = coverSpan.style.backgroundImage || '';
            const match = bg.match(/url\(["']?(.*?)["']?\)/);
            if (match && match[1]) {
                let raw = match[1];
                raw = raw.replace('t50x50', 't500x500');
                raw = raw.replace(/["']/g, '');
                imgUrl = raw;
            }
        }

        let artist = '';
        const artistEl = document.querySelector('.playbackSoundBadge__lightLink');
        if (artistEl) {
            artist = artistEl.textContent.trim();
        }

        let track = '';
        const titleSpan = document.querySelector('.playbackSoundBadge__titleLink span[aria-hidden="true"]');
        if (titleSpan) {
            track = titleSpan.textContent.trim();
        } else {
            const titleLink = document.querySelector('.playbackSoundBadge__titleLink');
            if (titleLink) {
                track = (titleLink.getAttribute('title') || titleLink.textContent || '').replace(/^Current track:\s*/i, '').trim();
            }
        }

        const imgTag = imgUrl ? `<<<${imgUrl}>>> ` : '';
        const metaTag = (artist && track) ? `[[[${artist}:::${track}]]] ` : '';
        const prefix = `[${passed}/${duration}] ${imgTag}${metaTag}`;

        let cleanTitle = document.title
            .replace(/^\[\d+:\d+(?::\d+)?\/\d+:\d+(?::\d+)?\]\s*/, '')
            .replace(/<<<.*?>>>\s*/g, '')
            .replace(/\[\[\[.*?\]\]\]\s*/g, '')
            .replace(/\|IMG:.*?\|\s*/g, '');

        if (passed && duration && passed !== duration) {
            document.title = prefix + cleanTitle;
        }
    }, 500);
})();
