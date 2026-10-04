$.Msg('[SPLIT DEBUG 1] bootstrap loaded');

(function () {
    'use strict';

    // TEMPORARY BOOTSTRAP DIAGNOSTICS: remove after the initialization failure is identified.
    function panelValid(panel) {
        try { return !!(panel && (!panel.IsValid || panel.IsValid())); }
        catch (error) { return false; }
    }

    function contextPanel() {
        try { return $.GetContextPanel(); }
        catch (error) { return null; }
    }

    var debugParent = contextPanel();
    var debugStack = null;
    var debugLine = 0;
    var debugLabels = {};

    function ensureDebugStack(preferredParent) {
        var parent = panelValid(preferredParent) ? preferredParent : debugParent;
        if (!panelValid(parent)) parent = contextPanel();
        if (!panelValid(parent)) return null;
        debugParent = parent;

        if (!panelValid(debugStack)) {
            debugStack = $.CreatePanel('Panel', parent, 'SplitBootstrapDebugStack', {
                hittest: 'false', hittestchildren: 'false'
            });
            debugStack.style.width = '420px';
            debugStack.style.height = 'fit-children';
            debugStack.style.horizontalAlign = 'right';
            debugStack.style.verticalAlign = 'top';
            debugStack.style.marginRight = '24px';
            debugStack.style.marginTop = '24px';
            debugStack.style.padding = '8px';
            debugStack.style.flowChildren = 'down';
            debugStack.style.backgroundColor = '#101010e6';
            debugStack.style.zIndex = '9999';
            debugStack.hittest = false;
            debugStack.hittestchildren = false;
        } else if (debugStack.SetParent) {
            try {
                var currentParent = debugStack.GetParent ? debugStack.GetParent() : null;
                if (currentParent !== parent) debugStack.SetParent(parent);
            } catch (error) {}
        }
        return debugStack;
    }

    function appendDebug(text, preferredParent, key) {
        try {
            var stack = ensureDebugStack(preferredParent);
            if (!panelValid(stack)) return;
            if (key && panelValid(debugLabels[key])) {
                debugLabels[key].text = text;
                return;
            }
            debugLine += 1;
            var label = $.CreatePanel('Label', stack, 'SplitBootstrapDebugLine' + debugLine);
            label.text = text;
            label.style.width = '100%';
            label.style.height = '28px';
            label.style.fontSize = '18px';
            label.style.color = 'white';
            label.style.verticalAlign = 'center';
            label.hittest = false;
            if (key) debugLabels[key] = label;
        } catch (error) {
            $.Warning('[SPLIT DEBUG] visible marker failed: ' + error);
        }
    }

    function reportError(error) {
        var message = String(error || 'unknown error');
        $.Warning('[SPLIT ERROR] ' + message);
        appendDebug('SPLIT ERROR - ' + message.substring(0, 120));
    }

    function missingModuleNames(modules) {
        if (!modules) return ['namespace.modules'];
        var missing = [];
        if (!modules.utils) missing.push('utils');
        if (typeof modules.createState !== 'function') missing.push('createState');
        if (typeof modules.createBridge !== 'function') missing.push('createBridge');
        if (typeof modules.createQuickAccess !== 'function') missing.push('createQuickAccess');
        return missing;
    }

    appendDebug('SPLIT DEBUG 1 - BOOTSTRAP');

    try {
        var shared = $.SplitPanorama;
        var modules = shared && shared.modules;
        if (shared) {
            shared.diagnostics = {
                append: function (text) { appendDebug(text); },
                set: function (key, text) { appendDebug(text, null, key); }
            };
        }
        var missingModules = missingModuleNames(modules);
        var modulesValid = missingModules.length === 0;
        $.Msg('[SPLIT DEBUG 2] modules=' + modulesValid +
            (modulesValid ? '' : ' missing=' + missingModules.join(',')));
        appendDebug(modulesValid ? 'SPLIT DEBUG 2 - MODULES OK' :
            'SPLIT DEBUG 2 - MODULES MISSING: ' + missingModules.join(', '));
        if (!modulesValid) return;

        var utils = modules.utils;
        var previous = shared.runtime;
        if (previous && previous.retire) {
            try { previous.retire(); }
            catch (error) { $.Warning('[SPLIT] previous runtime retire failed: ' + error); }
        }

        var generation = ((previous && previous.generation) || 0) + 1;
        var retired = false;
        var runtime = {
            generation: generation,
            active: function () {
                return !retired && shared.runtime === runtime;
            }
        };
        shared.runtime = runtime;

        var root = utils.root();
        var rootValid = utils.valid(root);
        $.Msg('[SPLIT DEBUG 3] root valid=' + rootValid);
        appendDebug(rootValid ? 'SPLIT DEBUG 3 - ROOT OK' : 'SPLIT DEBUG 3 - ROOT INVALID', root);

        var hud = utils.find(root, 'gameplay_hud');
        var hudValid = utils.valid(hud);
        $.Msg('[SPLIT DEBUG 4] hud valid=' + hudValid);
        appendDebug(hudValid ? 'SPLIT DEBUG 4 - HUD OK' : 'SPLIT DEBUG 4 - HUD MISSING', hud);
        if (!hudValid) return;

        var oldHost = utils.find(hud, 'SplitPanoramaHost');
        if (utils.valid(oldHost)) oldHost.DeleteAsync(0);
        var host = $.CreatePanel('Panel', hud, 'SplitPanoramaHost', {
            hittest: 'false', hittestchildren: 'false'
        });
        host.style.width = '100%';
        host.style.height = '100%';
        host.hittest = false;
        host.hittestchildren = false;

        var hostValid = utils.valid(host);
        $.Msg('[SPLIT DEBUG 5] host valid=' + hostValid);
        appendDebug(hostValid ? 'SPLIT DEBUG 5 - HOST OK' : 'SPLIT DEBUG 5 - HOST INVALID', host);
        if (!hostValid) return;

        var state = modules.createState();
        $.Msg('[SPLIT DEBUG 6] state created');
        appendDebug('SPLIT DEBUG 6 - STATE OK', host);

        var bridge = modules.createBridge(runtime, host, state);
        $.Msg('[SPLIT DEBUG 7] bridge created');
        appendDebug('SPLIT DEBUG 7 - BRIDGE OK', host);

        var quickAccess = modules.createQuickAccess(runtime, host, state, bridge.send);
        $.Msg('[SPLIT DEBUG 8] quick access created');
        appendDebug('SPLIT DEBUG 8 - QUICK ACCESS OK', host);

        runtime.retire = function () {
            if (retired) return;
            retired = true;
            bridge.retire();
            quickAccess.retire();
            host.hittest = false;
            host.hittestchildren = false;
            if (utils.valid(host)) host.DeleteAsync(0);
        };

        $.Msg('[SPLIT DEBUG READY] runtime ready');
        appendDebug('SPLIT DEBUG READY', host);
    } catch (error) {
        reportError(error);
    }
})();
