(function () {
    'use strict';

    var shared = $.SplitPanorama = $.SplitPanorama || {};
    var modules = shared.modules = shared.modules || {};
    var utils = modules.utils;
    var BRIDGE_URL = 'http://127.0.0.1:32146/bridge.html';
    var ACTION_URL = 'http://127.0.0.1:32146/action';
    var PROTOCOL_PREFIX = 'SPLIT_V1:';

    function visual(key, text) {
        if (shared.diagnostics && shared.diagnostics.set) shared.diagnostics.set(key, text);
    }

    function visualAppend(text) {
        if (shared.diagnostics && shared.diagnostics.append) shared.diagnostics.append(text);
    }

    function brief(value, limit) {
        var type = typeof value;
        if (type === 'string') return value.substring(0, limit);
        if (value === null) return '<null>';
        if (type === 'undefined') return '<undefined>';
        if (type === 'object') {
            try {
                if (value.message) return String(value.message).substring(0, limit);
            } catch (error) {}
            var id = '';
            try { id = value.id ? ' id=' + value.id : ''; } catch (error) {}
            return '<object' + id + '>';
        }
        try { return String(value).substring(0, limit); }
        catch (error) { return '<' + type + '>'; }
    }

    modules.createBridge = function (runtime, host, state) {
        var panel = null;
        var reloadHandle = null;
        var watchdogHandle = null;
        var layoutHandle = null;
        var healthHandles = [];
        var actions = [];
        var nonce = 0;
        var lastMessageAt = 0;
        var htmlTitleCount = 0;
        var remoteTitleReceived = false;
        var healthChecksScheduled = false;
        var transportHost = null;

        function active() { return runtime.active() && utils.valid(host); }
        function now() { return new Date().getTime(); }

        function rejectTitle(reason, detail, parseFailed) {
            $.Warning('[SPLIT BRIDGE] ' + reason + (detail ? ': ' + detail : ''));
            visual('bridge_rejection', reason);
            if (parseFailed !== false) visual('bridge_parse', 'STATE PARSE ERROR');
            return null;
        }

        function parseTitle(title) {
            if (typeof title !== 'string') return rejectTitle('TITLE TYPE INVALID', typeof title);
            if (title.indexOf(PROTOCOL_PREFIX) !== 0) {
                return rejectTitle('TITLE PREFIX INVALID', brief(title, 80));
            }
            $.Msg('[SPLIT BRIDGE] protocol detected');
            visual('bridge_protocol', 'TITLE PREFIX OK');

            var separator = title.indexOf(':', PROTOCOL_PREFIX.length);
            if (separator < 0) return rejectTitle('TITLE PAYLOAD MISSING', 'separator missing');

            var sequenceText = title.substring(PROTOCOL_PREFIX.length, separator);
            var sequence = Number(sequenceText);
            if (!sequenceText || !isFinite(sequence) || Math.floor(sequence) !== sequence) {
                return rejectTitle('TITLE SEQUENCE INVALID', brief(sequenceText, 40));
            }
            $.Msg('[SPLIT BRIDGE] sequence=' + sequence);
            visual('bridge_sequence', 'STATE SEQ: ' + sequence);

            var encodedPayload = title.substring(separator + 1);
            if (!encodedPayload) return rejectTitle('TITLE PAYLOAD MISSING');

            var decoderAvailable = typeof decodeURIComponent === 'function';
            $.Msg('[SPLIT BRIDGE] decodeURIComponent available=' + decoderAvailable);
            visual('bridge_decoder', 'DECODE URI AVAILABLE: ' + decoderAvailable);
            if (!decoderAvailable) return rejectTitle('TITLE DECODE UNSUPPORTED');

            var decodedPayload;
            try {
                decodedPayload = decodeURIComponent(encodedPayload);
            } catch (error) {
                return rejectTitle('TITLE DECODE FAILED', brief(error, 80));
            }
            visual('bridge_decode', 'STATE DECODE OK');

            var payload;
            try {
                payload = JSON.parse(decodedPayload);
            } catch (error) {
                return rejectTitle('TITLE JSON FAILED', brief(error, 80));
            }
            if (!payload || payload.sequence !== sequence || payload.protocolVersion !== 1) {
                return rejectTitle('TITLE PAYLOAD INVALID');
            }

            $.Msg('[SPLIT BRIDGE] parse success');
            visual('bridge_rejection', 'TITLE ACCEPTED');
            visual('bridge_parse', 'STATE PARSE OK');
            visual('state_mode', 'STATE MODE: ' + payload.visibility);
            visual('state_preset', 'PRESET: ' + (payload.activePresetName || ''));
            visual('state_slots', 'SLOTS: ' + (payload.slots ? payload.slots.length : 0));
            visual('state_undo', 'UNDO: ' + !!payload.canUndo);
            visual('state_redo', 'REDO: ' + !!payload.canRedo);
            return payload;
        }

        function onTitle() {
            var count = arguments.length;
            htmlTitleCount += 1;
            $.Msg('[SPLIT BRIDGE] HTMLTitle callback args=' + count);
            for (var index = 0; index < count; index += 1) {
                var argumentSummary = brief(arguments[index], 100);
                $.Msg('[SPLIT BRIDGE] arg' + index + ' type=' + typeof arguments[index] +
                    ' value=' + argumentSummary);
                if (index < 3) visual('bridge_arg' + index, 'TITLE ARG' + index + ': ' + argumentSummary);
            }

            if (!active()) return;
            var source = arguments[0];
            if (source !== panel) {
                rejectTitle('TITLE SOURCE INVALID');
                return;
            }

            var title = null;
            for (var titleIndex = 0; titleIndex < count; titleIndex += 1) {
                if (typeof arguments[titleIndex] === 'string' &&
                    arguments[titleIndex].indexOf(PROTOCOL_PREFIX) === 0) {
                    title = arguments[titleIndex];
                    break;
                }
            }
            if (title === null && typeof arguments[1] === 'string') title = arguments[1];
            var titleSummary = brief(title, 120);
            $.Msg('[SPLIT BRIDGE] HTML TITLE #' + htmlTitleCount + ': ' + titleSummary);
            if (htmlTitleCount <= 5) {
                visualAppend('HTML TITLE #' + htmlTitleCount + ': ' + titleSummary);
            } else {
                visual('bridge_title_latest', 'HTML TITLE #' + htmlTitleCount + ': ' + titleSummary);
            }
            visual('bridge_title', 'BRIDGE TITLE RECEIVED');
            visual('bridge_parser_input', 'TITLE PARSER INPUT: ' + brief(title, 150));
            $.Msg('[SPLIT BRIDGE] parser input: ' + brief(title, 150));

            if (title === null || typeof title === 'undefined' || title === '' || title === 'about:blank') {
                var initialTitle = title === '' ? '<empty>' : brief(title, 80);
                $.Msg('[SPLIT BRIDGE] normal initial title: ' + initialTitle);
                visual('bridge_initial_title', 'BRIDGE INITIAL TITLE: ' + initialTitle);
                return;
            }

            remoteTitleReceived = true;
            visual('bridge_remote_title', 'BRIDGE REMOTE TITLE RECEIVED');

            var payload = parseTitle(title);
            if (!payload) return;
            if (payload.sequence <= state.get().sequence) {
                rejectTitle('TITLE STALE SEQUENCE', String(payload.sequence), false);
                return;
            }
            var applied = state.apply(payload);
            $.Msg('[SPLIT BRIDGE] state applied=' + applied);
            visual('state_applied', 'STATE APPLIED: ' + applied);
            if (!applied) {
                rejectTitle('TITLE STATE REJECTED', '', false);
                return;
            }
            lastMessageAt = now();
        }

        function scheduleHealthCheck(delay, label) {
            healthHandles.push($.Schedule(delay, function () {
                if (!active()) return;
                var result = remoteTitleReceived ? 'BRIDGE REMOTE TITLE RECEIVED' :
                    'BRIDGE +' + label + ': NO REMOTE TITLE';
                $.Msg('[SPLIT BRIDGE] health +' + label + ' remoteTitle=' + remoteTitleReceived);
                visual('bridge_health_' + label, result);
            }));
        }

        function scheduleInitialHealthChecks() {
            if (healthChecksScheduled) return;
            healthChecksScheduled = true;
            scheduleHealthCheck(0.5, '500ms');
            scheduleHealthCheck(1.5, '1500ms');
            scheduleHealthCheck(3.0, '3000ms');
        }

        function requestBridgeUrl(reload) {
            if (!active() || !utils.valid(panel)) return;
            var url = reload ? BRIDGE_URL + '?reload=' + (++nonce) : BRIDGE_URL;
            try {
                panel.SetURL(url);
                $.Msg('[SPLIT BRIDGE] URL requested: ' + url);
                visual('bridge_url', 'BRIDGE URL REQUESTED');
                scheduleInitialHealthChecks();
                if (!reload && reloadHandle === null) reloadHandle = $.Schedule(2.0, reloadBridge);
            } catch (error) {
                $.Warning('[SPLIT BRIDGE] SetURL failed: ' + error);
                visual('bridge_url', 'BRIDGE URL REQUEST FAILED');
            }
        }

        function startBridgeAfterHostLayout(attempt) {
            if (!active() || !utils.valid(transportHost) || !utils.valid(panel)) return;

            var width = Math.round(Number(transportHost.actuallayoutwidth) || 0);
            var height = Math.round(Number(transportHost.actuallayoutheight) || 0);

            if ((width <= 0 || height <= 0) && attempt < 40) {
                layoutHandle = $.Schedule(0.05, function () {
                    startBridgeAfterHostLayout(attempt + 1);
                });
                return;
            }

            $.Msg('[SPLIT BRIDGE] transport host layout=' + width + 'x' + height + ' attempt=' + attempt);
            visual(
                'bridge_layout',
                'BRIDGE HOST LAYOUT: ' + width + 'x' + height + ' / TRY ' + attempt
            );

            requestBridgeUrl(false);
        }

        function createPanel() {
    if (!active()) return;

    if (utils.valid(panel)) panel.DeleteAsync(0);
    if (utils.valid(transportHost)) transportHost.DeleteAsync(0);

    var gameplayHud = host.GetParent ? host.GetParent() : null;
    if (!utils.valid(gameplayHud)) {
        $.Warning('[SPLIT BRIDGE] gameplay HUD parent missing');
        visual('bridge_panel', 'BRIDGE GAMEPLAY HUD MISSING');
        return;
    }

    transportHost = $.CreatePanel('Panel', gameplayHud, 'SplitPanoramaTransportHost', {
        hittest: 'false',
        hittestchildren: 'false'
    });

    transportHost.hittest = false;
    transportHost.hittestchildren = false;
    transportHost.style.width = '2px';
    transportHost.style.height = '2px';
    transportHost.style.horizontalAlign = 'left';
    transportHost.style.verticalAlign = 'top';
    transportHost.style.position = '2px 2px 0px';
    transportHost.style.opacity = '0.01';
    transportHost.style.visibility = 'visible';
    transportHost.style.overflow = 'clip';

    panel = $.CreatePanel('CitadelHTMLPanel', transportHost, 'SplitPanoramaBridge', {
                hittest: 'false',
                hittestchildren: 'false',
                acceptsfocus: 'false'
            });

            panel.hittest = false;
            panel.hittestchildren = false;
            panel.acceptsfocus = false;
            panel.style.width = '2px';
            panel.style.height = '2px';
            panel.style.opacity = '0.01';
            panel.style.visibility = 'visible';

            $.Msg('[SPLIT BRIDGE] CitadelHTMLPanel created');
            visual('bridge_panel', 'BRIDGE PANEL CREATED');

            $.RegisterEventHandler('HTMLTitle', panel, onTitle);

            $.Msg('[SPLIT BRIDGE] HTMLTitle handler registered');
            visual('bridge_handler', 'BRIDGE HANDLER REGISTERED');

            startBridgeAfterHostLayout(0);
        }

        function reloadBridge() {
            if (!active()) return;
            reloadHandle = null;
            if (!utils.valid(panel)) createPanel();
            else if (now() - lastMessageAt > 2000) requestBridgeUrl(true);
            reloadHandle = $.Schedule(2.0, reloadBridge);
        }

        function watchdog() {
            if (!active()) return;
            if (state.get().connected && now() - lastMessageAt > 1200) state.disconnect();
            watchdogHandle = $.Schedule(0.25, watchdog);
        }

        function send(action, slot) {
            if (!active() || state.get().visibility !== 'interactive') return;
            var query = '?action=' + action + '&nonce=' + (++nonce);
            if (slot !== undefined) query += '&slot=' + slot;
            var request = $.CreatePanel('CitadelHTMLPanel', transportHost, 'SplitAction' + nonce, {
                hittest: 'false', hittestchildren: 'false', acceptsfocus: 'false'
            });
            request.style.width = '1px';
            request.style.height = '1px';
            request.style.opacity = '0.01';
            request.SetURL(ACTION_URL + query);
            actions.push(request);
            $.Schedule(2.0, function () {
                var index = actions.indexOf(request);
                if (index >= 0) actions.splice(index, 1);
                if (utils.valid(request)) request.DeleteAsync(0);
            });
        }

        createPanel();
        watchdogHandle = $.Schedule(0.25, watchdog);

        return {
            send: send,
            retire: function () {
                utils.cancel(reloadHandle);
                utils.cancel(watchdogHandle);
                utils.cancel(layoutHandle);
                for (var healthIndex = 0; healthIndex < healthHandles.length; healthIndex += 1) {
                    utils.cancel(healthHandles[healthIndex]);
                }
                healthHandles = [];
                for (var index = 0; index < actions.length; index += 1) {
                    if (utils.valid(actions[index])) actions[index].DeleteAsync(0);
                }
                actions = [];
                if (utils.valid(panel)) panel.DeleteAsync(0);
                if (utils.valid(transportHost)) transportHost.DeleteAsync(0);
                state.disconnect();
            }
        };
    };
})();
