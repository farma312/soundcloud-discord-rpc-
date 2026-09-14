// ==UserScript==
// @name         SoundCloud RPC Direct Connector
// @namespace    http://tampermonkey.net/
// @version      5.6
// @author       farma312
// @match        https://soundcloud.com/*
// @noframes
// @grant        GM_xmlhttpRequest
// @connect      127.0.0.1
// @connect      api-v2.soundcloud.com
// @connect      lrclib.net
// @connect      genius.com
// @run-at       document-start
// ==/UserScript==

(function () {
    'use strict';

    if (window.top !== window.self) return;

    const SERVER_URL = 'http://127.0.0.1:23456/update';
    const SEARCH_RESULT_URL = 'http://127.0.0.1:23456/search_results';
    const activeMediaElements = new Set();
    let capturedClientId = '2t9loNfh900mioJ2DU11YtSXQuVUbmyt';
    let capturedAuth = '';
    let cachedPlaylists = [];
    let lastPlaylistFetch = 0;

    let lastTrackKey = '';
    let lyricsPlain = '';
    let lyricsSynced = [];

    const origPlay = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
        activeMediaElements.add(this);
        return origPlay.apply(this, arguments);
    };

    function checkUrlForCredentials(url) {
        if (typeof url !== 'string') return;
        if (url.includes('client_id=')) {
            const m = url.match(/client_id=([a-zA-Z0-9_-]+)/);
            if (m && m[1]) capturedClientId = m[1];
        }
    }

    const origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (method, url) {
        checkUrlForCredentials(url);
        return origOpen.apply(this, arguments);
    };

    const origSetHeader = XMLHttpRequest.prototype.setRequestHeader;
    XMLHttpRequest.prototype.setRequestHeader = function (header, val) {
        if (typeof header === 'string' && header.toLowerCase() === 'authorization') {
            capturedAuth = val;
        }
        return origSetHeader.apply(this, arguments);
    };

    const origFetch = window.fetch;
    window.fetch = function (...args) {
        const url = typeof args[0] === 'string' ? args[0] : (args[0] && args[0].url);
        checkUrlForCredentials(url);
        const headers = args[1] && args[1].headers;
        if (headers) {
            if (typeof headers.get === 'function') {
                const a = headers.get('Authorization') || headers.get('authorization');
                if (a) capturedAuth = a;
            } else if (typeof headers === 'object') {
                const a = headers['Authorization'] || headers['authorization'];
                if (a) capturedAuth = a;
            }
        }
        return origFetch.apply(this, args);
    };

    function cleanTitle(rawArtist, rawTitle) {
        let artist = rawArtist || '';
        let track = rawTitle || '';
        if (track.includes(' - ')) {
            const parts = track.split(' - ');
            artist = parts[0];
            track = parts.slice(1).join(' - ');
        } else if (track.includes(' — ')) {
            const parts = track.split(' — ');
            artist = parts[0];
            track = parts.slice(1).join(' — ');
        }
        const filterJunk = (s) => s.replace(/\([^)]*(prod|feat|ft)[^)]*\)/gi, '')
                                   .replace(/\[[^\]]*(prod|feat|ft)[^\]]*\]/gi, '')
                                   .replace(/\((official|video|audio|lyrics?)\)/gi, '')
                                   .replace(/\[(official|video|audio|lyrics?)\]/gi, '')
                                   .trim();
        return { artist: filterJunk(artist), track: filterJunk(track) };
    }

    function parseLrc(lrcStr) {
        const lines = [];
        for (const line of lrcStr.split('\n')) {
            const trimmed = line.trim();
            if (trimmed.startsWith('[')) {
                const closeIdx = trimmed.indexOf(']');
                if (closeIdx !== -1) {
                    const timePart = trimmed.substring(1, closeIdx);
                    const text = trimmed.substring(closeIdx + 1).trim();
                    const parts = timePart.split(':');
                    if (parts.length === 2) {
                        const mins = parseFloat(parts[0]);
                        const secs = parseFloat(parts[1]);
                        if (!isNaN(mins) && !isNaN(secs)) {
                            lines.push({ sec: mins * 60 + secs, text });
                        }
                    }
                }
            }
        }
        lines.sort((a, b) => a.sec - b.sec);
        return lines;
    }

    function searchGenius(query) {
        GM_xmlhttpRequest({
            method: 'GET',
            url: `https://genius.com/api/search/multi?q=${encodeURIComponent(query)}`,
            headers: { 'User-Agent': navigator.userAgent, 'Accept': 'application/json' },
            onload: function (res) {
                try {
                    const json = JSON.parse(res.responseText);
                    const sections = json.response.sections;
                    let songPath = null;
                    for (const sec of sections) {
                        if (sec.type === 'song' && sec.hits && sec.hits.length > 0) {
                            songPath = sec.hits[0].result.path;
                            break;
                        }
                    }
                    if (songPath) fetchGeniusPage(`https://genius.com${songPath}`);
                } catch (e) {}
            }
        });
    }

    function fetchGeniusPage(pageUrl) {
        GM_xmlhttpRequest({
            method: 'GET',
            url: pageUrl,
            headers: { 'User-Agent': navigator.userAgent },
            onload: function (res) {
                try {
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(res.responseText, 'text/html');
                    const containers = doc.querySelectorAll('[data-lyrics-container="true"]');
                    if (containers && containers.length > 0) {
                        let text = '';
                        containers.forEach(c => {
                            text += c.innerHTML.replace(/<br\s*\/?>/gi, '\n').replace(/<[^>]+>/g, '') + '\n\n';
                        });
                        text = text.replace(/&#x27;/g, "'").replace(/&amp;/g, '&').replace(/&quot;/g, '"').trim();
                        if (text.length > 20) {
                            lyricsPlain = text;
                            lyricsSynced = [];
                        }
                    }
                } catch (e) {}
            }
        });
    }

    function fetchLyrics(artist, track) {
        lyricsPlain = '';
        lyricsSynced = [];
        const clean = cleanTitle(artist, track);
        const query = `${clean.artist} ${clean.track}`.trim();
        GM_xmlhttpRequest({
            method: 'GET',
            url: `https://lrclib.net/api/search?q=${encodeURIComponent(query)}`,
            headers: { 'User-Agent': 'zen_rpc/5.6' },
            onload: function (res) {
                try {
                    const items = JSON.parse(res.responseText);
                    if (Array.isArray(items) && items.length > 0) {
                        for (const item of items) {
                            if (item.syncedLyrics && item.syncedLyrics.trim().length > 0) {
                                lyricsSynced = parseLrc(item.syncedLyrics);
                                return;
                            }
                            if (item.plainLyrics && item.plainLyrics.trim().length > 0 && !lyricsPlain) {
                                lyricsPlain = item.plainLyrics.trim();
                                return;
                            }
                        }
                    }
                } catch (e) {}
                searchGenius(query);
            }
        });
    }

    function parseTimeToSeconds(tStr) {
        if (!tStr) return 0;
        const parts = tStr.trim().split(':').map(Number);
        if (parts.length === 2) return parts[0] * 60 + parts[1];
        if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
        return 0;
    }

    function setVolume(val) {
        const vol = Math.max(0, Math.min(1, parseFloat(val)));
        if (isNaN(vol)) return;
        activeMediaElements.forEach(a => { try { a.volume = vol; } catch (e) {} });
        document.querySelectorAll('audio, video').forEach(a => { try { a.volume = vol; } catch (e) {} });
        const volContainer = document.querySelector('.playControls .volume, .volume');
        if (volContainer) {
            volContainer.classList.add('hover');
            const slider = volContainer.querySelector('.volume__sliderWrapper, .volume__sliderProgress');
            if (slider) {
                const rect = slider.getBoundingClientRect();
                if (rect.height > 0) {
                    const cx = rect.left + rect.width / 2;
                    const cy = rect.bottom - (rect.height * vol);
                    ['mousedown', 'mouseup', 'click'].forEach(t => {
                        slider.dispatchEvent(new MouseEvent(t, { bubbles: true, clientX: cx, clientY: cy, buttons: 1 }));
                    });
                }
            }
        }
    }

    function seekTrackTo(targetSeconds) {
        const sec = parseFloat(targetSeconds);
        if (isNaN(sec) || sec < 0) return;
        activeMediaElements.forEach(a => { try { a.currentTime = sec; } catch (e) {} });
        document.querySelectorAll('audio, video').forEach(a => { try { a.currentTime = sec; } catch (e) {} });
        const totalEl = document.querySelector('.playbackTimeline__duration > span:last-child');
        const totalSec = totalEl ? parseTimeToSeconds(totalEl.textContent) : 0;
        const progressBar = document.querySelector('.playbackTimeline__progressWrapper, .playbackTimeline__progressBar');
        if (progressBar && totalSec > 0) {
            const rect = progressBar.getBoundingClientRect();
            const ratio = Math.max(0, Math.min(1, sec / totalSec));
            const clientX = rect.left + (rect.width * ratio);
            const clientY = rect.top + (rect.height / 2);
            const evtInit = {
                bubbles: true, cancelable: true, view: window,
                clientX: clientX, clientY: clientY, pageX: clientX + window.scrollX, pageY: clientY + window.scrollY, buttons: 1
            };
            progressBar.dispatchEvent(new MouseEvent('mousedown', evtInit));
            progressBar.dispatchEvent(new MouseEvent('mouseup', evtInit));
            progressBar.dispatchEvent(new MouseEvent('click', evtInit));
        }
    }

    function getArtworkUrl() {
        const els = document.querySelectorAll(
            '.playbackSoundBadge__avatar span.sc-artwork, ' +
            '.playbackSoundBadge__avatar .image__lightOutline span, ' +
            '.playbackSoundBadge__avatar .image span, ' +
            '.playbackSoundBadge__avatar span[style*="background-image"], ' +
            'a.playbackSoundBadge__avatar span'
        );
        for (const el of els) {
            const bg = el.style.backgroundImage || window.getComputedStyle(el).backgroundImage;
            if (bg) {
                const match = bg.match(/url\(["']?(.*?)["']?\)/);
                if (match && match[1] && match[1].startsWith('http')) {
                    return match[1].replace(/-t\d+x\d+\./, '-t500x500.').replace(/-large\./, '-t500x500.');
                }
            }
        }
        const img = document.querySelector('.playbackSoundBadge__avatar img, a.playbackSoundBadge__avatar img');
        if (img && img.src && img.src.startsWith('http')) {
            return img.src.replace(/-t\d+x\d+\./, '-t500x500.').replace(/-large\./, '-t500x500.');
        }
        return '';
    }

    function getOAuthToken() {
        try {
            const raw = window.localStorage.getItem('oauth_token');
            if (raw) return raw.startsWith('"') ? JSON.parse(raw) : raw;
        } catch (e) {}
        const m = document.cookie.match(/(?:^|;\s*)oauth_token=([^;]+)/);
        return m ? decodeURIComponent(m[1]) : '';
    }

    function isTrackLiked() {
        const btn = document.querySelector('.playbackSoundBadge__like, .playControls .sc-button-like, [data-testid*="like" i]');
        if (!btn) return false;
        const state = `${btn.getAttribute('aria-label') || ''} ${btn.getAttribute('title') || ''} ${btn.textContent || ''}`.toLowerCase();
        return btn.classList.contains('sc-button-selected') || /unlike|liked|понравил|убрать лайк/.test(state);
    }

    function toggleTrackLike() {
        const btn = document.querySelector('.playbackSoundBadge__like, .playControls .sc-button-like');
        if (btn) btn.click();
    }

    function isArtistFollowed() {
        const btn = document.querySelector('.listenContext .sc-button-follow, .sidebar .sc-button-follow, .userHeader__followButton .sc-button-follow, [data-testid*="follow" i], button[aria-label*="follow" i]');
        if (!btn) return false;
        const state = `${btn.getAttribute('aria-label') || ''} ${btn.getAttribute('title') || ''} ${btn.textContent || ''}`.toLowerCase();
        return btn.classList.contains('sc-button-selected') || /following|unfollow|подписан|отписат/.test(state);
    }

    function toggleArtistFollow() {
        const btn = document.querySelector('.listenContext .sc-button-follow, .sidebar .sc-button-follow, .userHeader__followButton .sc-button-follow, .sc-button-follow');
        if (btn) {
            btn.click();
            return;
        }
        const artistLink = document.querySelector('.playbackSoundBadge__lightLink');
        if (artistLink && artistLink.href) {
            window.location.href = artistLink.href;
        }
    }

    function doSearch(query) {
        if (!query) return;
        const token = getOAuthToken();
        const auth = capturedAuth || (token ? 'OAuth ' + token : '');
        const headers = { 'Accept': 'application/json' };
        if (auth) headers['Authorization'] = auth;

        GM_xmlhttpRequest({
            method: 'GET',
            url: `https://api-v2.soundcloud.com/search/tracks?q=${encodeURIComponent(query)}&client_id=${capturedClientId}&limit=50`,
            headers: headers,
            onload: function (res) {
                try {
                    const data = JSON.parse(res.responseText);
                    const results = [];
                    if (data && Array.isArray(data.collection)) {
                        for (const item of data.collection) {
                            let art = item.artwork_url || (item.user ? item.user.avatar_url : '');
                            if (art) art = art.replace(/-large\./, '-t200x200.');
                            results.push({
                                id: item.id || 0,
                                title: item.title || '',
                                artist: item.user ? item.user.username : 'SoundCloud',
                                duration_sec: Math.floor((item.duration || 0) / 1000),
                                permalink_url: item.permalink_url || '',
                                artwork_url: art
                            });
                        }
                    }
                    GM_xmlhttpRequest({
                        method: 'POST',
                        url: SEARCH_RESULT_URL,
                        headers: { 'Content-Type': 'application/json' },
                        data: JSON.stringify(results)
                    });
                } catch (e) {}
            }
        });
    }

    function fetchBatchTrackDetails(ids, auth, onComplete) {
        if (!ids.length) {
            onComplete([]);
            return;
        }
        const chunks = [];
        // SoundCloud accepts at most 50 ids in one /tracks request.
        for (let i = 0; i < ids.length; i += 50) chunks.push(ids.slice(i, i + 50));
        const loaded = [];
        const loadChunk = (index) => {
            if (index >= chunks.length) {
                onComplete(loaded);
                return;
            }
            const chunk = chunks[index].join('%2C');
            GM_xmlhttpRequest({
                method: 'GET',
                url: `https://api-v2.soundcloud.com/tracks?ids=${chunk}&client_id=${capturedClientId}`,
                headers: auth ? { 'Authorization': auth } : {},
                onload: function (res) {
                    try {
                        const list = JSON.parse(res.responseText);
                        if (Array.isArray(list)) loaded.push(...list);
                    } catch (e) {}
                    loadChunk(index + 1);
                },
                onerror: function () {
                    loadChunk(index + 1);
                }
            });
        };
        loadChunk(0);
    }

    function fetchFullPlaylistDetails(playlistId, auth) {
        if (!playlistId) return;
        const target = cachedPlaylists.find(p => p.id === playlistId);
        if (!target) return;

        const initialTracks = Array.isArray(target.tracks) ? target.tracks.slice() : [];
        const populated = [];
        const missingIds = [];
        const seenIds = new Set();

        const finish = () => {
            const applyTracks = (tracks) => {
                tracks.forEach(t => {
                    if (!t || !t.id || seenIds.has(`loaded:${t.id}`)) return;
                    seenIds.add(`loaded:${t.id}`);
                    let tArt = t.artwork_url || (t.user ? t.user.avatar_url : '');
                    if (tArt) tArt = tArt.replace(/-large\./, '-t200x200.').replace(/-badge\./, '-t200x200.');
                    populated.push({
                        id: t.id,
                        title: t.title || '',
                        artist: t.user ? t.user.username : 'SoundCloud',
                        duration_sec: Math.floor((t.duration || 0) / 1000),
                        permalink_url: t.permalink_url || '',
                        artwork_url: tArt
                    });
                });
            };

            if (missingIds.length) {
                fetchBatchTrackDetails(missingIds, auth, function (loaded) {
                    applyTracks(loaded);
                    if (populated.length) target.tracks = populated;
                    target.track_count = Math.max(Number(target.track_count) || 0, target.tracks.length);
                });
            } else {
                if (populated.length) target.tracks = populated;
                target.track_count = Math.max(Number(target.track_count) || 0, target.tracks.length);
            }
        };

        if (initialTracks.length) {
            initialTracks.forEach(t => {
                if (t && t.id) seenIds.add(`initial:${t.id}`);
            });
        }

        const loadPage = (url, visited) => {
            if (visited.size > 100) {
                finish();
                return;
            }
            visited.add(url);
            GM_xmlhttpRequest({
                method: 'GET',
                url: url,
                headers: auth ? { 'Authorization': auth } : {},
                onload: function (res) {
                    try {
                        const page = JSON.parse(res.responseText);
                        if (!page) {
                            finish();
                            return;
                        }
                        const pageTracks = Array.isArray(page.collection)
                            ? page.collection
                            : (Array.isArray(page.tracks)
                                ? page.tracks
                                : (page.tracks && Array.isArray(page.tracks.collection) ? page.tracks.collection : []));
                        if (!pageTracks.length && visited.size === 1 && url.includes(`/playlists/${playlistId}?`)) {
                            loadPage(`https://api-v2.soundcloud.com/playlists/${playlistId}/tracks?client_id=${capturedClientId}&limit=100&linked_partitioning=1`, new Set());
                            return;
                        }
                        pageTracks.forEach(t => {
                            t = t && t.track ? t.track : t;
                            if (!t || !t.id) return;
                            if (t.title && t.permalink_url) {
                                if (seenIds.has(`full:${t.id}`)) return;
                                seenIds.add(`full:${t.id}`);
                                let tArt = t.artwork_url || (t.user ? t.user.avatar_url : '');
                                if (tArt) tArt = tArt.replace(/-large\./, '-t200x200.').replace(/-badge\./, '-t200x200.');
                                populated.push({
                                    id: t.id,
                                    title: t.title,
                                    artist: t.user ? t.user.username : 'SoundCloud',
                                    duration_sec: Math.floor((t.duration || 0) / 1000),
                                    permalink_url: t.permalink_url,
                                    artwork_url: tArt
                                });
                            } else if (!missingIds.includes(t.id)) {
                                missingIds.push(t.id);
                            }
                        });
                        const next = page.next_href || page.collection_next_href ||
                            (page.tracks && page.tracks.next_href);
                        if (next && !visited.has(next)) loadPage(next, visited);
                        else finish();
                    } catch (e) {
                        finish();
                    }
                },
                onerror: finish
            });
        };

        loadPage(`https://api-v2.soundcloud.com/playlists/${playlistId}?client_id=${capturedClientId}&limit=100&linked_partitioning=1`, new Set());
    }

    function fetchPlaylists() {
        const now = Date.now();
        if (now - lastPlaylistFetch < 20000 && cachedPlaylists.length > 0) return;
        const token = getOAuthToken();
        const auth = capturedAuth || (token ? 'OAuth ' + token : '');
        if (!auth || !capturedClientId) return;

        lastPlaylistFetch = now;
        GM_xmlhttpRequest({
            method: 'GET',
            url: `https://api-v2.soundcloud.com/me/library/all?client_id=${capturedClientId}&limit=100`,
            headers: { 'Authorization': auth },
            onload: function (res) {
                try {
                    const data = JSON.parse(res.responseText);
                    if (data && Array.isArray(data.collection)) {
                        cachedPlaylists = data.collection
                            .filter(x => x.type === 'playlist' || x.playlist)
                            .map(x => x.playlist || x)
                            .map(p => {
                                let art = p.artwork_url || '';
                                if (!art && p.user && p.user.avatar_url) art = p.user.avatar_url;
                                if (art) art = art.replace(/-large\./, '-t200x200.').replace(/-badge\./, '-t200x200.');

                                let trackList = [];
                                if (Array.isArray(p.tracks)) {
                                    trackList = p.tracks
                                        .filter(t => t && t.title && t.permalink_url)
                                        .map(t => {
                                            let tArt = t.artwork_url || (t.user ? t.user.avatar_url : '');
                                            if (tArt) tArt = tArt.replace(/-large\./, '-t200x200.').replace(/-badge\./, '-t200x200.');
                                            return {
                                                id: t.id || 0,
                                                title: t.title || '',
                                                artist: t.user ? t.user.username : '',
                                                duration_sec: Math.floor((t.duration || 0) / 1000),
                                                permalink_url: t.permalink_url || '',
                                                artwork_url: tArt
                                            };
                                        });
                                }

                                const plObj = {
                                    id: p.id || 0,
                                    title: p.title || 'Untitled Playlist',
                                     track_count: Math.max(Number(p.track_count) || 0, trackList.length),
                                    permalink_url: p.permalink_url || '',
                                    artwork_url: art,
                                    tracks: trackList
                                };

                                return plObj;
                            })
                            .filter(p => p.permalink_url);
                        cachedPlaylists.forEach(p => {
                            if (p.id) fetchFullPlaylistDetails(p.id, auth);
                        });
                    }
                } catch (e) {}
            }
        });
    }

    function triggerControl(action) {
        if (!action) return;
        if (action === 'toggle_play') {
            const playBtn = document.querySelector('.playControl');
            if (playBtn) playBtn.click();
        } else if (action === 'next') {
            const btn = document.querySelector('.playControls__elements button.skipControl__next, .playControls button.playControls__next');
            if (btn && !btn.disabled) btn.click();
        } else if (action === 'prev') {
            const btn = document.querySelector('.playControls__elements button.skipControl__previous, .playControls button.playControls__prev');
            if (btn && !btn.disabled) btn.click();
        } else if (action === 'repeat') {
            const repeatBtn = document.querySelector('.repeatControl');
            if (repeatBtn) repeatBtn.click();
        } else if (action === 'toggle_like') {
            toggleTrackLike();
        } else if (action === 'toggle_follow') {
            toggleArtistFollow();
        } else if (action.startsWith('play_track:')) {
            const targetUrl = action.slice(11).trim();
            if (targetUrl.startsWith('http')) window.location.href = targetUrl;
        } else if (action.startsWith('set_volume:')) {
            setVolume(action.slice(11));
        } else if (action.startsWith('seek:')) {
            seekTrackTo(action.slice(5));
        } else if (action.startsWith('search_query:')) {
            doSearch(action.slice(13));
        } else if (action.startsWith('fetch_playlist:')) {
            const pid = parseInt(action.slice(15).trim());
            const token = getOAuthToken();
            const auth = capturedAuth || (token ? 'OAuth ' + token : '');
            if (pid) fetchFullPlaylistDetails(pid, auth);
        }
    }

    let isSending = false;

    function sendPlaybackState() {
        if (isSending) return;
        fetchPlaylists();

        const titleLink = document.querySelector('.playbackSoundBadge__titleLink');
        const artistLink = document.querySelector('.playbackSoundBadge__lightLink');
        const playBtn = document.querySelector('.playControl');

        if (!titleLink || !artistLink) {
            setTimeout(sendPlaybackState, 250);
            return;
        }

        const trackTitle = (titleLink.getAttribute('title') || titleLink.textContent || '').trim();
        const artistName = (artistLink.getAttribute('title') || artistLink.textContent || '').trim();
        if (!trackTitle) {
            setTimeout(sendPlaybackState, 250);
            return;
        }

        const trackKey = `${artistName}___${trackTitle}`;
        if (trackKey !== lastTrackKey) {
            lastTrackKey = trackKey;
            fetchLyrics(artistName, trackTitle);
        }

        let userName = '';
        let userUrl = '';
        let userAvatar = '';
        const userBtn = document.querySelector('.header__userNavLink, .userNav__usernameButton, a[href^="/you"]');
        if (userBtn) {
            userName = (userBtn.getAttribute('title') || userBtn.textContent || '').trim();
            userUrl = userBtn.getAttribute('href') || '';
            if (userUrl && !userUrl.startsWith('http')) userUrl = 'https://soundcloud.com' + userUrl;
            const avatar = document.querySelector('.header__userNavLink img, .userNav__usernameButton img, .header__userNavLink .sc-artwork, .userNav__usernameButton .sc-artwork');
            userAvatar = avatar?.src || avatar?.getAttribute('src') || '';
        }

        const isPlaying = playBtn ? playBtn.classList.contains('playing') : false;
        const passedEl = document.querySelector('.playbackTimeline__timePassed > span:last-child');
        const totalEl = document.querySelector('.playbackTimeline__duration > span:last-child');

        const passedSec = passedEl ? parseTimeToSeconds(passedEl.textContent) : 0;
        const totalSec = totalEl ? parseTimeToSeconds(totalEl.textContent) : 0;
        const coverUrl = getArtworkUrl();

        const payload = {
            track: trackTitle,
            artist: artistName,
            passed: passedSec,
            duration: totalSec,
            cover: coverUrl,
            is_playing: isPlaying,
            repeat: document.querySelector('.repeatControl.m-one') ? 'one' : (document.querySelector('.repeatControl.m-all') ? 'all' : 'none'),
            client_id: capturedClientId,
            user_name: userName,
            user_url: userUrl,
            user_avatar: userAvatar,
            playlists: cachedPlaylists,
            spectrum: [isTrackLiked() ? 1 : 0, isArtistFollowed() ? 1 : 0],
            liked: isTrackLiked(),
            followed: isArtistFollowed(),
            lyrics_plain: lyricsPlain,
            lyrics_synced: lyricsSynced
        };

        isSending = true;

        GM_xmlhttpRequest({
            method: 'POST',
            url: SERVER_URL,
            headers: { 'Content-Type': 'application/json' },
            data: JSON.stringify(payload),
            timeout: 600,
            onload: function (res) {
                isSending = false;
                try {
                    const data = JSON.parse(res.responseText);
                    if (data && data.command) triggerControl(data.command);
                } catch (e) {}
                setTimeout(sendPlaybackState, 250);
            },
            onerror: function () {
                isSending = false;
                setTimeout(sendPlaybackState, 500);
            },
            ontimeout: function () {
                isSending = false;
                setTimeout(sendPlaybackState, 500);
            }
        });
    }

    sendPlaybackState();
})();
