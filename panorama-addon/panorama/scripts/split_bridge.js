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
    var ROUND_TIMEOUT = 0.60;
    var ROUND_RETRY_DELAY = 0.06;
    var MAX_ROUND_RETRIES = 2;

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

    function decodeState(bytes) {
        var compact = JSON.parse(utf8(bytes));
        if (!compact || compact.length !== 8 || compact[0] !== 1) {
            throw new Error('invalid state payload');
        }
        var modes = ['hidden', 'passive', 'interactive'];
        var mode = modes[compact[2]];
        if (!mode || !(compact[5] instanceof Array)) throw new Error('invalid state fields');
        var slots = [];
        for (var index = 0; index < compact[5].length; index += 1) {
            var slot = compact[5][index];
            if (!slot || slot.length !== 3) throw new Error('invalid slot');
            slots.push({ index: slot[0], name: slot[1], populated: slot[2] === 1 });
        }
        return {
            protocolVersion: compact[0],
            sequence: compact[1],
            visibility: mode,
            activePreset: compact[3],
            activePresetName: compact[4],
            slots: slots,
            canUndo: compact[6] === 1,
            canRedo: compact[7] === 1
        };
    }

    modules.createBridge = function (runtime, host, state) {
        var transport = $.CreatePanel('Panel', host, 'SplitImageTransport', {
            hittest: 'false', hittestchildren: 'false'
        });
        var images = [];
        var actionImage = null;
        var loadedBits = [];
        var roundOpen = false;
        var loadedCount = 0;
        for (var loadedIndex = 0; loadedIndex < FRAME_BITS; loadedIndex += 1) {
            if (loadedBits[loadedIndex]) loadedCount += 1;
        }

        visual('ipc_loaded', 'LOADED BITS ' + loadedCount);
        $.Msg('[SPLIT IPC] round=' + round + ' loadedBits=' + loadedCount);
        var messageId = 0;
        var round = 0;
        var retry = 0;
        var nonce = 0;
        var payload = [];
        var closeHandle = null;
        var nextHandle = null;

        transport.style.width = '2px';
        transport.style.height = '2px';
        transport.style.horizontalAlign = 'left';
        transport.style.verticalAlign = 'top';
        transport.style.opacity = '0.01';
        transport.style.overflow = 'clip';
        transport.hittest = false;
        transport.hittestchildren = false;

        function active() { return runtime.active() && utils.valid(transport); }

        function scheduleNext(delay) {
            utils.cancel(nextHandle);
            nextHandle = $.Schedule(delay, startMessage);
        }

        function disconnect(reason) {
            $.Warning('[SPLIT IPC] ' + reason);
            visual('ipc_crc', 'CRC FAIL');
            state.disconnect();
            scheduleNext(1.0);
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

        function completeMessage() {
            var nextState;
            try {
                nextState = decodeState(payload);
            } catch (error) {
                disconnect('message parse failed: ' + error);
                return;
            }
            visual('ipc_complete', 'MESSAGE COMPLETE');
            visual('ipc_sequence', 'SEQ ' + nextState.sequence);
            visual('ipc_mode', 'MODE ' + nextState.visibility);
            visual('ipc_preset', 'PRESET ' + (nextState.activePresetName || ''));
            visual('ipc_slots', 'SLOTS ' + nextState.slots.length);
            $.Msg('[SPLIT IPC] message=' + messageId + ' sequence=' + nextState.sequence +
                ' mode=' + nextState.visibility + ' slots=' + nextState.slots.length);
            state.apply(nextState);
            scheduleNext(nextState.visibility === 'interactive' ? 0.25 :
                (nextState.visibility === 'passive' ? 0.5 : 1.0));
        }

        function closeRound() {
            closeHandle = null;
            if (!active() || !roundOpen) return;
            roundOpen = false;
            var bytes = [];
            for (var byteIndex = 0; byteIndex < FRAME_BYTES; byteIndex += 1) {
                var value = 0;
                for (var bitIndex = 0; bitIndex < 8; bitIndex += 1) {
                    if (loadedBits[byteIndex * 8 + bitIndex]) value |= 1 << bitIndex;
                }
                bytes.push(value);
            }

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
                visual('ipc_crc', 'CRC FAIL');
                if (retry < MAX_ROUND_RETRIES) {
                    retry += 1;
                    $.Warning('[SPLIT IPC] round=' + round + ' retry=' + retry + ': ' + error);
                    nextHandle = $.Schedule(ROUND_RETRY_DELAY, requestRound);
                } else {
                    disconnect('round=' + round + ' failed after retries: ' + error);
                }
                return;
            }

            visual('ipc_crc', 'CRC OK');
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
            loadedBits = [];
            roundOpen = true;
            visual('ipc_round', 'ROUND ' + round);
            for (var index = 0; index < FRAME_BITS; index += 1) {
                loadedBits[index] = false;
                images[index].SetImage(STATE_BIT_URL + '?message=' + messageId + '&round=' + round +
                    '&bit=' + index + '&nonce=' + (++nonce));
            }
            utils.cancel(closeHandle);
            closeHandle = $.Schedule(ROUND_TIMEOUT, closeRound);
        }

        function startMessage() {
            nextHandle = null;
            if (!active()) return;
            messageId = messageId >= 65535 ? 1 : messageId + 1;
            round = 0;
            retry = 0;
            payload = [];
            visual('ipc_status', 'SPLIT IPC');
            visual('ipc_request', 'STATE REQUEST');
            requestRound();
        }

        function markLoaded(index) {
            return function () {
                if (active() && roundOpen) loadedBits[index] = true;
            };
        }

        function createImage(id) {
            var image = $.CreatePanel('Image', transport, id, {
                hittest: 'false', hittestchildren: 'false'
            });
            image.style.width = '1px';
            image.style.height = '1px';
            image.style.opacity = '0.01';
            image.hittest = false;
            return image;
        }

        for (var index = 0; index < FRAME_BITS; index += 1) {
            var image = createImage('SplitStateBit' + index);
            $.RegisterEventHandler('ImageLoaded', image, markLoaded(index));
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
        startMessage();

        return {
            send: send,
            retire: function () {
                roundOpen = false;
                utils.cancel(closeHandle);
                utils.cancel(nextHandle);
                state.disconnect();
                if (utils.valid(transport)) transport.DeleteAsync(0);
                images = [];
                actionImage = null;
            }
        };
    };
})();
