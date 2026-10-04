(function () {
    'use strict';

    var shared = $.SplitPanorama = $.SplitPanorama || {};
    var modules = shared.modules = shared.modules || {};
    var utils = modules.utils;

    function visual(key, text) {
        if (shared.diagnostics && shared.diagnostics.set) shared.diagnostics.set(key, text);
    }

    modules.createQuickAccess = function (runtime, parent, state, sendAction) {
        var overlay = $.CreatePanel('Panel', parent, 'SplitQuickAccess');
        var preset = $.CreatePanel('Label', overlay, 'SplitPresetName');
        var slots = $.CreatePanel('Panel', overlay, 'SplitSlots');
        var footer = $.CreatePanel('Panel', overlay, 'SplitFooter');
        var status = $.CreatePanel('Label', overlay, 'SplitStatus');
        var interactive = false;

        overlay.AddClass('SplitQuickAccess');
        slots.AddClass('SplitSlots');
        footer.AddClass('SplitFooter');
        status.AddClass('SplitStatus');

        function button(parentPanel, text, action, slot) {
            var control = $.CreatePanel('Panel', parentPanel, '');
            var label = $.CreatePanel('Label', control, '');
            control.AddClass('SplitButton');
            label.text = text;
            control.SetPanelEvent('onactivate', function () {
                if (interactive) sendAction(action, slot);
            });
            return control;
        }

        var previous = button(footer, '‹ PRESET', 'previous_preset');
        var undo = button(footer, 'UNDO', 'undo');
        var redo = button(footer, 'REDO', 'redo');
        var next = button(footer, 'PRESET ›', 'next_preset');

        function setEnabled(panel, enabled) {
            panel.SetHasClass('SplitDisabled', !enabled);
            panel.hittest = enabled && interactive;
        }

        function render(nextState) {
            if (!runtime.active()) return;
            $.Msg('[SPLIT QUICK ACCESS] update received mode=' + nextState.visibility);
            visual('qa_update', 'QUICK ACCESS UPDATE RECEIVED');
            var wasInteractive = interactive;
            interactive = nextState.visibility === 'interactive';
            overlay.style.visibility = nextState.visibility !== 'hidden' ? 'visible' : 'collapse';
            overlay.hittest = interactive;
            overlay.hittestchildren = interactive;
            parent.hittestchildren = interactive;
            overlay.SetHasClass('SplitInteractive', interactive);
            visual('qa_mode', 'QA MODE: ' + nextState.visibility);
            visual('qa_panel', 'QA PANEL VALID: ' + utils.valid(overlay));
            visual('qa_visible', 'QA VISIBLE: ' + (overlay.style.visibility !== 'collapse'));
            visual('qa_class', 'QA CLASS SplitInteractive: ' + interactive);
            $.Msg('[SPLIT QUICK ACCESS] panel valid=' + utils.valid(overlay) +
                ' visibility=' + overlay.style.visibility + ' interactiveClass=' + interactive);

            if (interactive && !wasInteractive) utils.command('hud_free_cursor 1');
            if (!interactive && wasInteractive) utils.command('hud_free_cursor -1');

            preset.text = nextState.activePresetName || 'SPLIT';
            status.text = nextState.connected ? nextState.visibility.toUpperCase() : 'DISCONNECTED';
            slots.RemoveAndDeleteChildren();
            for (var index = 0; index < nextState.slots.length; index += 1) {
                (function (slot) {
                    var row = $.CreatePanel('Panel', slots, 'SplitSlot' + slot.index);
                    var name = $.CreatePanel('Label', row, '');
                    row.AddClass('SplitSlot');
                    row.SetHasClass('SplitSlotEmpty', !slot.populated);
                    name.AddClass('SplitSlotName');
                    name.text = slot.index + '. ' + slot.name;
                    var load = button(row, 'LOAD', 'load_slot', slot.index);
                    button(row, 'SAVE', 'save_slot', slot.index);
                    load.SetHasClass('SplitDisabled', !slot.populated);
                    load.hittest = interactive && slot.populated;
                })(nextState.slots[index]);
            }
            setEnabled(previous, true);
            setEnabled(next, true);
            setEnabled(undo, nextState.canUndo);
            setEnabled(redo, nextState.canRedo);
        }

        state.subscribe(render);
        return {
            retire: function () {
                if (interactive) utils.command('hud_free_cursor -1');
                overlay.hittest = false;
                overlay.hittestchildren = false;
                parent.hittestchildren = false;
                if (utils.valid(overlay)) overlay.DeleteAsync(0);
            }
        };
    };
})();
