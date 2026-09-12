// ==UserScript==
// @name         SoundCloud RPC Direct Connector
// @namespace    http://tampermonkey.net/
// @version      2.0
// @description  Отправляет данные воспроизведения в локальное приложение zen_rpc
// @author       farma312
// @match        https://soundcloud.com/*
// @grant        GM_xmlhttpRequest
// @connect      127.0.0.1
// @run-at       document-idle
// ==/UserScript==

(function () {
    'use strict';

    function parseTimeToSeconds(tStr) {
        if (!tStr) return 0;
        const parts = tStr.trim().split(':').map(Number);
        if (parts.length === 2) return parts[0] * 60 + parts[1];
        if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
        return 0;
    }

    function sendPlaybackState() {
        const titleLink = document.querySelector('.playbackSoundBadge__titleLink');
        const artistLink = document.querySelector('.playbackSoundBadge__lightLink');
        const playBtn = document.querySelector('.playControl');

        if (!titleLink || !artistLink) return;

        const isPlaying = playBtn ? playBtn.classList.contains('playing') : false;
        const trackTitle = (titleLink.getAttribute('title') || titleLink.textContent || '').trim();
        const artistName = (artistLink.getAttribute('title') || artistLink.textContent || '').trim();

        const passedEl = document.querySelector('.playbackTimeline__timePassed > span:last-child');
        const totalEl = document.querySelector('.playbackTimeline__duration > span:last-child');

        const passedSec = passedEl ? parseTimeToSeconds(passedEl.textContent) : 0;
        const totalSec = totalEl ? parseTimeToSeconds(totalEl.textContent) : 0;

        let coverUrl = '';
        const badgeImg = document.querySelector('.playbackSoundBadge__avatar .image__lightOutline span');
        if (badgeImg) {
            const bg = window.getComputedStyle(badgeImg).backgroundImage;
            const match = bg.match(/url\(["']?(.*?)["']?\)/);
            if (match && match[1] && match[1].startsWith('http')) {
                coverUrl = match[1].replace('-t50x50.', '-t500x500.');
            }
        }

        const payload = {
            track: trackTitle,
            artist: artistName,
            passed: passedSec,
            duration: totalSec,
            cover: coverUrl,
            is_playing: isPlaying
        };

        GM_xmlhttpRequest({
            method: 'POST',
            url: 'http://127.0.0.1:23456/update',
            headers: { 'Content-Type': 'application/json' },
            data: JSON.stringify(payload),
            timeout: 1000
        });
    }

    setInterval(sendPlaybackState, 800);
})();