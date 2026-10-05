(function () {
    'use strict';

    var shared = $.SplitPanorama = $.SplitPanorama || {};
    var modules = shared.modules = shared.modules || {};
    var utils = modules.utils;
    var STATE_BIT_URL = 'http://127.0.0.1:32146/ipc/state-bit';
    var ACTION_URL = 'http://127.0.0.1:32146/action';
    var FRAME_BYTES = 16;
    var FRAME_BITS = 128;
    var FRAME_PAYLOAD_BYTES = 9;
    var FRAME_MAGIC = 0xA0;
    var FRAME_VERSION = 1;
    var FRAME_END_FLAG = 0x08;
    var NORMAL_CLOSE_SECONDS = 0.60;
    var STORM_CLOSE_SECONDS = 2.5;
    var STORM_TRIGGER_FAILURES = 2;
    var STORM_ATTEMPTS = 4;
    var ROUND_RETRY_DELAY = 0.10;
    var STORM_RETRY_DELAY = 1.0;
    var MAX_ROUND_RETRIES = 3;
    var STATE_PANEL_PREFIX = 'SplitStateBit';

    function visual(key, text) {
        if (shared.diagnostics && shared.diagnostics.set) shared.diagnostics.set(key, text);
    }

    function crc16(bytes, length) {
        var crc = 0xFFFF;
        for (var index = 0; index < length; index += 1) {
            crc ^= bytes[index] << 8;
            for (var bit = 0; bit < 8; bit += 1) {
                crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xFFFF : (crc << 1) & 0xFFFF;
            }
        }
        return crc;
    }

    function utf8(bytes) {
        var result = '';
        for (var index = 0; index < bytes.length;) {
            var first = bytes[index++];
            var code;
            if (first < 0x80) {
                code = first;
            } else if ((first & 0xE0) === 0xC0 && index < bytes.length) {
                code = ((first & 0x1F) << 6) | (bytes[index++] & 0x3F);
            } else if ((first & 0xF0) === 0xE0 && index + 1 < bytes.length) {
                code = ((first & 0x0F) << 12) | ((bytes[index++] & 0x3F) << 6) |
                    (bytes[index++] & 0x3F);
            } else if ((first & 0xF8) === 0xF0 && index + 2 < bytes.length) {
                code = ((first & 0x07) << 18) | ((bytes[index++] & 0x3F) << 12) |
                    ((bytes[index++] & 0x3F) << 6) | (bytes[index++] & 0x3F);
                code -= 0x10000;
                result += String.fromCharCode(0xD800 + (code >> 10), 0xDC00 + (code & 0x3FF));
                continue;
            } else {
                throw new Error('invalid UTF-8 payload');
            }
            result += String.fromCharCode(code);
        }
        return result;
    }

    function unwrapSequence(raw, previous) {
        if (previous < 0) return raw;
        var previousRaw = previous & 0xFFFF;
        var delta = (raw - previousRaw + 0x10000) & 0xFFFF;
        if (delta === 0) return previous;
        if (delta > 0x8000) return previous - (0x10000 - delta);
        return previous + delta;
    }

    function decodeFastState(bytes, previousSequence) {
        if (!bytes || bytes.length !== 9 || bytes[0] !== 1 || bytes[8] !== 0) {
            throw new Error('invalid fast state payload');
        }
        var flags = bytes[3];
        var modes = ['hidden', 'passive', 'interactive'];
        var mode = modes[flags & 0x03];
        if (!mode || (flags & 0xF0) !== 0) throw new Error('invalid fast state flags');
        var rawSequence = bytes[1] | (bytes[2] << 8);
        return {
            protocolVersion: bytes[0],
            sequence: unwrapSequence(rawSequence, previousSequence),
            visibility: mode,
            activePreset: bytes[4],
            populatedMask: bytes[5],
            canUndo: (flags & 0x04) !== 0,
            canRedo: (flags & 0x08) !== 0,
            metadataRevision: bytes[6] | (bytes[7] << 8)
        };
    }

    function decodeMetadataItem(bytes) {
        if (!bytes || bytes.length < 7 || bytes[0] !== 1) {
            throw new Error('invalid metadata item payload');
        }
        var item = bytes[4];
        var length = bytes[5] | (bytes[6] << 8);
        if (item > 8 || bytes.length !== 7 + length) {
            throw new Error('invalid metadata item fields');
        }
        return {
            protocolVersion: bytes[0],
            revision: bytes[1] | (bytes[2] << 8),
            activePreset: bytes[3],
            item: item,
            value: utf8(bytes.slice(7))
        };
    }

    function metadataMatches(cache, fast) {
        return !!cache && cache.revision === fast.metadataRevision &&
            cache.activePreset === fast.activePreset;
    }

    function createMetadataCache(fast) {
        return {
            revision: fast.metadataRevision,
            activePreset: fast.activePreset,
            items: [null, null, null, null, null, null, null, null, null],
            received: [false, false, false, false, false, false, false, false, false]
        };
    }

    function nextMissingMetadataItem(cache) {
        for (var index = 0; index < 9; index += 1) {
            if (!cache.received[index]) return index;
        }
        return -1;
    }

    function storeMetadataItem(cache, metadata) {
        if (!cache || metadata.revision !== cache.revision ||
            metadata.activePreset !== cache.activePreset ||
            metadata.item < 0 || metadata.item > 8) return false;
        cache.items[metadata.item] = metadata.value;
        cache.received[metadata.item] = true;
        return true;
    }

    function slotsForFast(fast, prior, cached) {
        var slots = [];
        for (var index = 0; index < 8; index += 1) {
            var name = cached && cached.received[index + 1] ? cached.items[index + 1] : null;
            if (typeof name !== 'string' && prior.activePreset === fast.activePreset &&
                prior.slots[index] && typeof prior.slots[index].name === 'string') {
                name = prior.slots[index].name;
            }
            if (typeof name !== 'string') name = 'Slot ' + (index + 1);
            slots.push({
                index: index + 1,
                name: name,
                populated: (fast.populatedMask & (1 << index)) !== 0
            });
        }
        return slots;
    }

    function composeFastState(fast, prior, cached) {
        return {
            protocolVersion: fast.protocolVersion,
            sequence: fast.sequence,
            visibility: fast.visibility,
            activePreset: fast.activePreset,
            activePresetName: cached && cached.received[0] ? cached.items[0] :
                (prior.activePreset === fast.activePreset ? prior.activePresetName : ''),
            metadataRevision: fast.metadataRevision,
            slots: slotsForFast(fast, prior, cached),
            canUndo: fast.canUndo,
            canRedo: fast.canRedo
        };
    }

    if (shared.testHooks) {
        shared.testHooks.bridgeProtocol = {
            decodeFastState: decodeFastState,
            decodeMetadataItem: decodeMetadataItem,
            metadataMatches: metadataMatches,
            createMetadataCache: createMetadataCache,
            nextMissingMetadataItem: nextMissingMetadataItem,
            storeMetadataItem: storeMetadataItem,
            composeFastState: composeFastState
        };
    }

    modules.createBridge = function (runtime, host, state) {
        var images = [];
        var actionImage = null;
        var activeAttempt = null;
        var attemptSerial = 0;
        var imageLoadedEvents = 0;
        var messageId = 0;
        var transferKind = 'fast';
        var requestedMetadataRevision = 0;
        var requestedMetadataPreset = 0;
        var requestedMetadataItem = -1;
        var metadataCache = null;
        var round = 0;
        var retry = 0;
        var crcFailStreak = 0;
        var stormAttemptsRemaining = 0;
        var nonce = 0;
        var payload = [];
        var closeHandle = null;
        var nextHandle = null;
        var handlerToken = null;

        function active() { return runtime.active() && utils.valid(host); }

        function scheduleFast(delay) {
            utils.cancel(nextHandle);
            nextHandle = $.Schedule(delay, startFastMessage);
        }

        function failTransfer(reason) {
            $.Warning('[SPLIT IPC] ' + reason);
            visual('ipc_crc', (transferKind === 'fast' ? 'FAST ' : 'METADATA ') + 'CRC FAIL');
            if (transferKind === 'metadata') {
                visual('ipc_item_complete', 'ITEM FAILED ' +
                    (requestedMetadataItem + 1) + '/9');
                visual('ipc_metadata_retry', 'RETRY LATER');
                scheduleFast(1.0);
            } else {
                state.disconnect();
                scheduleFast(1.0);
            }
        }

        function parseFrame(bytes) {
            if ((bytes[0] & 0xF0) !== FRAME_MAGIC || (bytes[0] & 0x07) !== FRAME_VERSION) {
                throw new Error('header mismatch');
            }
            var frameMessage = bytes[1] | (bytes[2] << 8);
            if (frameMessage !== messageId || bytes[3] !== round) throw new Error('frame identity mismatch');
            var length = bytes[4];
            if (length > FRAME_PAYLOAD_BYTES) throw new Error('payload length mismatch');
            var expected = bytes[14] | (bytes[15] << 8);
            if (crc16(bytes, 14) !== expected) throw new Error('CRC mismatch');
            return { end: (bytes[0] & FRAME_END_FLAG) !== 0, length: length };
        }

        function pollDelay(nextState) {
            return nextState.visibility === 'interactive' ? 0.25 :
                (nextState.visibility === 'passive' ? 0.5 : 1.0);
        }

        function completeFastMessage() {
            var prior = state.get();
            var fast = decodeFastState(payload, prior.sequence);
            if (!metadataMatches(metadataCache, fast)) {
                metadataCache = createMetadataCache(fast);
            }
            var nextState = composeFastState(fast, prior, metadataCache);

            visual('ipc_complete', 'FAST STATE');
            visual('ipc_sequence', 'SEQ ' + nextState.sequence);
            visual('ipc_mode', 'MODE ' + nextState.visibility);
            var maskHex = fast.populatedMask.toString(16).toUpperCase();
            if (maskHex.length < 2) maskHex = '0' + maskHex;
            visual('ipc_slots_mask', 'SLOTS MASK 0x' + maskHex);
            visual('ipc_meta_revision', 'META REV ' + fast.metadataRevision);
            visual('ipc_fast_frames', 'FAST FRAMES ' + (round + 1));
            $.Msg('[SPLIT IPC] FAST STATE message=' + messageId + ' sequence=' +
                nextState.sequence + ' mode=' + nextState.visibility + ' slotsMask=0x' +
                maskHex + ' metadataRevision=' + fast.metadataRevision +
                ' frames=' + (round + 1));

            // Apply the latency-sensitive state before any metadata transfer begins.
            state.apply(nextState);
            var missingItem = nextMissingMetadataItem(metadataCache);
            if (missingItem >= 0) {
                startMetadataMessage(fast.metadataRevision, fast.activePreset, missingItem);
            } else {
                scheduleFast(pollDelay(nextState));
            }
        }

        function completeMetadataMessage() {
            var metadata = decodeMetadataItem(payload);
            if (metadata.revision !== requestedMetadataRevision ||
                metadata.activePreset !== requestedMetadataPreset ||
                metadata.item !== requestedMetadataItem ||
                !storeMetadataItem(metadataCache, metadata)) {
                throw new Error('metadata item identity mismatch');
            }
            state.mergeMetadataItem(metadata);
            visual('ipc_item_complete', 'ITEM COMPLETE ' + (metadata.item + 1) + '/9');
            visual('ipc_meta_revision', 'META REV ' + metadata.revision);
            visual('ipc_meta_frames', 'META FRAMES ' + (round + 1));
            $.Msg('[SPLIT IPC] ITEM COMPLETE ' + (metadata.item + 1) +
                '/9 revision=' + metadata.revision + ' frames=' + (round + 1));
            var missingItem = nextMissingMetadataItem(metadataCache);
            if (missingItem < 0) {
                visual('ipc_metadata', 'METADATA COMPLETE 9/9');
                $.Msg('[SPLIT IPC] METADATA COMPLETE 9/9 revision=' + metadata.revision);
                scheduleFast(pollDelay(state.get()));
            } else {
                startMetadataMessage(metadata.revision, metadata.activePreset, missingItem);
            }
        }

        function completeMessage() {
            try {
                if (transferKind === 'fast') completeFastMessage();
                else completeMetadataMessage();
            } catch (error) {
                failTransfer(transferKind + ' message parse failed: ' + error);
            }
        }

        function closeRound(attempt) {
            closeHandle = null;
            if (!active() || activeAttempt !== attempt || !attempt.open) return;
            attempt.open = false;
            activeAttempt = null;
            if (attempt.storm && stormAttemptsRemaining > 0) stormAttemptsRemaining -= 1;
            var bytes = [];
            var loadedCount = 0;
            for (var byteIndex = 0; byteIndex < FRAME_BYTES; byteIndex += 1) {
                var value = 0;
                for (var bitIndex = 0; bitIndex < 8; bitIndex += 1) {
                    if (attempt.loadedBits[byteIndex * 8 + bitIndex]) {
                        value |= 1 << bitIndex;
                        loadedCount += 1;
                    }
                }
                bytes.push(value);
            }

            visual('ipc_loaded', 'LOADED BITS ' + loadedCount);
            $.Msg('[SPLIT IPC] round=' + round + ' retry=' + retry +
                ' loadedBits=' + loadedCount);

            var hex = '';
            for (var hexIndex = 0; hexIndex < bytes.length; hexIndex += 1) {
                var part = bytes[hexIndex].toString(16).toUpperCase();
                if (part.length < 2) part = '0' + part;
                hex += part + (hexIndex + 1 < bytes.length ? ' ' : '');
            }

            visual('ipc_bytes', 'BYTES ' + hex);
            $.Msg('[SPLIT IPC] bytes=' + hex);

            var receivedCrc = bytes[14] | (bytes[15] << 8);
            var calculatedCrc = crc16(bytes, 14);

            visual(
                'ipc_crc_detail',
                'CRC RX ' + receivedCrc.toString(16).toUpperCase() +
                ' / CALC ' + calculatedCrc.toString(16).toUpperCase()
            );

            var frame;
            try {
                frame = parseFrame(bytes);
            } catch (error) {
                crcFailStreak += 1;
                visual('ipc_crc', (transferKind === 'fast' ? 'FAST ' : 'METADATA ') + 'CRC FAIL');
                visual('ipc_crc_streak', 'CRC FAIL STREAK ' + crcFailStreak);
                if (crcFailStreak >= STORM_TRIGGER_FAILURES) {
                    var enteringStorm = stormAttemptsRemaining === 0;
                    stormAttemptsRemaining = STORM_ATTEMPTS;
                    visual('ipc_storm', 'STORM MODE');
                    if (enteringStorm) {
                        $.Warning('[SPLIT IPC] entering storm mode after ' +
                            crcFailStreak + ' consecutive CRC failures');
                    }
                }
                if (retry < MAX_ROUND_RETRIES) {
                    retry += 1;

                    $.Warning('[SPLIT IPC] round=' + round + ' retry=' + retry + ': ' + error);

                    var retryDelay = stormAttemptsRemaining > 0
                        ? STORM_RETRY_DELAY
                        : ROUND_RETRY_DELAY;

                    nextHandle = $.Schedule(retryDelay, requestRound);
                } else {
                    failTransfer(transferKind + ' round=' + round +
                        ' failed after retries: ' + error);
                }
                return;
            }

            crcFailStreak = 0;
            visual('ipc_crc', (transferKind === 'fast' ? 'FAST ' : 'METADATA ') + 'CRC OK');
            visual('ipc_crc_streak', 'CRC FAIL STREAK 0');
            if (attempt.storm && stormAttemptsRemaining === 0) {
                visual('ipc_storm', 'STORM MODE ENDED');
            }
            for (var index = 0; index < frame.length; index += 1) payload.push(bytes[5 + index]);
            retry = 0;
            if (frame.end) {
                completeMessage();
            } else {
                round += 1;
                nextHandle = $.Schedule(ROUND_RETRY_DELAY, requestRound);
            }
        }

        function requestRound() {
            nextHandle = null;
            if (!active()) return;
            var storm = stormAttemptsRemaining > 0;
            var closeSeconds = storm ? STORM_CLOSE_SECONDS : NORMAL_CLOSE_SECONDS;
            var attempt = {
                serial: ++attemptSerial,
                round: round,
                retry: retry,
                storm: storm,
                open: true,
                loadedBits: []
            };
            activeAttempt = attempt;
            visual('ipc_round', transferKind === 'fast' ? 'FAST ROUND ' + round :
                'ROUND ' + (round + 1));
            visual('ipc_retry', 'RETRY ' + retry);
            visual('ipc_close', storm ? 'CLOSE 2500ms STORM' : 'CLOSE 600ms');
            var kindQuery = '&kind=' + transferKind;
            if (transferKind === 'metadata') {
                kindQuery += '&revision=' + requestedMetadataRevision +
                    '&preset=' + requestedMetadataPreset + '&item=' + requestedMetadataItem;
            }
            for (var index = 0; index < FRAME_BITS; index += 1) {
                attempt.loadedBits[index] = false;
                images[index].SetImage(STATE_BIT_URL + '?message=' + messageId + '&round=' + round +
                    '&retry=' + retry + '&attempt=' + attempt.serial + '&bit=' + index +
                    kindQuery + '&nonce=' + (++nonce));
            }
            utils.cancel(closeHandle);
            closeHandle = $.Schedule(closeSeconds, function () { closeRound(attempt); });
        }

        function startMessage(kind, metadataRevision, metadataPreset, metadataItem) {
            nextHandle = null;
            if (!active()) return;
            transferKind = kind;
            requestedMetadataRevision = metadataRevision || 0;
            requestedMetadataPreset = metadataPreset || 0;
            requestedMetadataItem = metadataItem === undefined ? -1 : metadataItem;
            messageId = messageId >= 65535 ? 1 : messageId + 1;
            round = 0;
            retry = 0;
            crcFailStreak = 0;
            stormAttemptsRemaining = 0;
            payload = [];
            visual('ipc_status', 'SPLIT IPC');
            visual('ipc_request', kind === 'fast' ? 'FAST STATE' : 'METADATA REQUEST');
            if (kind === 'metadata') {
                visual('ipc_metadata', 'METADATA');
                visual('ipc_metadata_item', 'ITEM ' + (requestedMetadataItem + 1) + '/9');
                visual('ipc_metadata_name', requestedMetadataItem === 0 ? 'PRESET' :
                    'SLOT ' + requestedMetadataItem);
                visual('ipc_metadata_retry', '');
                visual('ipc_meta_revision', 'META REV ' + requestedMetadataRevision);
                $.Msg('[SPLIT IPC] METADATA REQUEST item=' +
                    (requestedMetadataItem + 1) + '/9 revision=' +
                    requestedMetadataRevision + ' preset=' + requestedMetadataPreset);
            }
            visual('ipc_crc_streak', 'CRC FAIL STREAK 0');
            requestRound();
        }

        function startFastMessage() {
            startMessage('fast', 0, 0, -1);
        }

        function startMetadataMessage(revision, preset, item) {
            startMessage('metadata', revision, preset, item);
        }

        function onImageLoaded(panel) {
            if (!active() || !panel) return;

            var id = '';
            try {
                id = String(panel.id || '');
            } catch (error) {
                return;
            }

            if (id.indexOf(STATE_PANEL_PREFIX) !== 0) return;

            var suffix = id.substring(STATE_PANEL_PREFIX.length);
            if (!/^\d+$/.test(suffix)) return;
            var index = Number(suffix);

            if (!isFinite(index) ||
                Math.floor(index) !== index ||
                index < 0 ||
                index >= FRAME_BITS) {
                return;
            }

            imageLoadedEvents += 1;
            visual('ipc_image_events', 'IMAGELOADED EVENTS ' + imageLoadedEvents);
            visual('ipc_last_image', 'LAST IMAGE BIT ' + index);

            var attempt = activeAttempt;
            if (!attempt || !attempt.open || attempt.round !== round || attempt.retry !== retry) return;
            attempt.loadedBits[index] = true;
        }

        function createImage(id) {
            var image = $.CreatePanel('Image', host, id, {
                hittest: 'false', hittestchildren: 'false'
            });
            image.visible = false;
            image.style.width = '2px';
            image.style.height = '2px';
            image.hittest = false;
            image.hittestchildren = false;
            return image;
        }

        if (!shared.imageLoadedHandlerInstalled) {
            try {
                $.RegisterForUnhandledEvent('ImageLoaded', function (panel) {
                    var target = shared.currentImageLoadedHandler;
                    if (typeof target === 'function') target(panel);
                });
                shared.imageLoadedHandlerInstalled = true;
            } catch (error) {
                $.Warning('[SPLIT IPC] global ImageLoaded registration failed: ' + error);
                visual('ipc_handler', 'IMAGELOADED HANDLER FAILED');
            }
        }

        handlerToken = function (panel) {
            var currentRuntime = shared.runtime;
            if (!handlerToken.active || !currentRuntime ||
                currentRuntime.generation !== handlerToken.generation ||
                !currentRuntime.active()) return;
            onImageLoaded(panel);
        };
        handlerToken.generation = runtime.generation;
        handlerToken.active = true;
        shared.currentImageLoadedHandler = handlerToken;

        for (var index = 0; index < FRAME_BITS; index += 1) {
            var image = createImage('SplitStateBit' + index);
            images.push(image);
        }
        actionImage = createImage('SplitActionRequest');

        function send(action, slot) {
            if (!active() || state.get().visibility !== 'interactive') return;
            visual('ipc_action', 'ACTION REQUEST ' + action);
            $.Msg('[SPLIT IPC] action request ' + action);
            var query = '?action=' + action + '&nonce=' + (++nonce);
            if (slot !== undefined) query += '&slot=' + slot;
            actionImage.SetImage(ACTION_URL + query);
        }

        visual('ipc_status', 'SPLIT IPC');
        startFastMessage();

        return {
            send: send,
            retire: function () {
                if (activeAttempt) activeAttempt.open = false;
                activeAttempt = null;
                utils.cancel(closeHandle);
                utils.cancel(nextHandle);
                handlerToken.active = false;
                if (shared.currentImageLoadedHandler === handlerToken) {
                    shared.currentImageLoadedHandler = null;
                }
                state.disconnect();
                for (var index = 0; index < images.length; index += 1) {
                    if (utils.valid(images[index])) images[index].DeleteAsync(0);
                }
                if (utils.valid(actionImage)) actionImage.DeleteAsync(0);
                images = [];
                actionImage = null;
            }
        };
    };
})();
