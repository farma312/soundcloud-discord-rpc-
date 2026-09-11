// ==UserScript==
// @name         SoundCloud Rich Presence Sync
// @namespace    http://tampermonkey.net/
// @version      1.2
// @description  Accurate metadata, artwork and progress extraction for SoundCloud RPC
// @author       farma312
// @match        https://soundcloud.com/*
// @grant        none
// @run-at       document-idle
// ==/UserScript==

(function () {
    'use strict';

    function extractTime(element) {
        if (!element) return null;
        const text = element.innerText || element.textContent || '';
        const match = text.match(/\d+:\d+(:\d+)?/);
        return match ? match[0] : null;
    }

    function getCoverUrl() {
        // 1. Из нижнего бара воспроизведения
        const bottomArtwork = document.querySelector('.playbackSoundBadge__avatar span.sc-artwork');
        if (bottomArtwork && bottomArtwork.style.backgroundImage) {
            const m = bottomArtwork.style.backgroundImage.match(/url\(["']?(.*?)["']?\)/);
            if (m && m[1] && !m[1].includes('default_avatar')) return m[1];
        }

        // 2. Из большой обложки на странице трека
        const pageArtwork = document.querySelector('.listenArtworkWrapper span.sc-artwork, .listenArtworkWrapper img');
        if (pageArtwork) {
            if (pageArtwork.tagName === 'IMG' && pageArtwork.src) return pageArtwork.src;
            if (pageArtwork.style.backgroundImage) {
                const m = pageArtwork.style.backgroundImage.match(/url\(["']?(.*?)["']?\)/);
                if (m && m[1]) return m[1];
            }
        }

        // 3. Из мета-тегов страницы
        const metaImg = document.querySelector('meta[property="og:image"]');
        if (metaImg && metaImg.content) return metaImg.content;

        return '';
    }

    setInterval(() => {
        // Извлечение названия и автора
        let title = '';
        let artist = '';

        const titleLink = document.querySelector('.playbackSoundBadge__titleLink');
        const artistLink = document.querySelector('.playbackSoundBadge__lightLink');

        if (titleLink && artistLink) {
            title = titleLink.getAttribute('title') || titleLink.innerText || '';
            artist = artistLink.getAttribute('title') || artistLink.innerText || '';
        } else {
            // Если нижний бар ещё не заполнился, берем из шапки страницы
            const heroTitle = document.querySelector('.soundTitle__title');
            const heroArtist = document.querySelector('.soundTitle__username');
            if (heroTitle && heroArtist) {
                title = heroTitle.innerText || '';
                artist = heroArtist.innerText || '';
            }
        }

        if (!title || !artist) return;

        // Извлечение времени
        let passed = extractTime(document.querySelector('.playbackTimeline__timePassed')) 
                  || extractTime(document.querySelector('.playbackTimeline__timePassed > span:last-child'));
        
        let duration = extractTime(document.querySelector('.playbackTimeline__duration')) 
                    || extractTime(document.querySelector('.playbackTimeline__duration > span:last-child'));

        if (!passed) passed = "00:00";
        if (!duration) duration = "00:00";

        // Получение лучшего качества обложки (t500x500 вместо крошечной t50x50)
        let cover = getCoverUrl();
        if (cover) {
            cover = cover.replace('-t50x50.', '-t500x500.').replace('-t120x120.', '-t500x500.');
        }

        // Формирование точного заголовка для Rust приложения
        document.title = `[${passed}/${duration}] [[[${artist.trim()}:::${title.trim()}]]] <<<${cover.trim()}>>> | SoundCloud`;
    }, 600);
})();
