// ==UserScript==
// @name         SoundCloud Time & Cover in Title
// @namespace    https://tampermonkey.net/
// @version      4.0
// @description  Adds playback times, cover image and true artist to window title
// @match        https://soundcloud.com/*
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

        // 1. Ссылка на обложку
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

        // 2. Настоящий автор и название
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

        // Счищаем старые служебные теги
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