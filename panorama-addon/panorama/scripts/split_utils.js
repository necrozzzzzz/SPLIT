(function () {
    'use strict';

    var shared = $.SplitPanorama = $.SplitPanorama || {};
    var modules = shared.modules = shared.modules || {};

    modules.utils = {
        valid: function (panel) {
            return !!(panel && (!panel.IsValid || panel.IsValid()));
        },
        root: function () {
            var panel = $.GetContextPanel();
            var guard = 0;
            while (panel && panel.GetParent && panel.GetParent() && guard < 50) {
                panel = panel.GetParent();
                guard += 1;
            }
            return panel || $.GetContextPanel();
        },
        find: function (root, id) {
            if (!this.valid(root) || !root.FindChildTraverse) return null;
            try { return root.FindChildTraverse(id); } catch (error) { return null; }
        },
        cancel: function (handle) {
            if (handle === null || typeof $.CancelScheduled !== 'function') return;
            try { $.CancelScheduled(handle); } catch (error) {}
        },
        command: function (value) {
            try { $.DispatchEvent('CitadelConCommand', value); }
            catch (error) { $.Warning('[SPLIT] command failed: ' + error); }
        }
    };
})();
