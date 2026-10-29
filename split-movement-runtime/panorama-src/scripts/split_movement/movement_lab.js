(function () {
    "use strict";

    var state = {
        collapsed: false,
        stamina: false,
        cooldowns: false,
        ammo: false
    };

    function dispatch(command) {
        try {
            $.DispatchEvent("CitadelConCommand", command);
            return true;
        } catch (error) {
            $.Msg("[SPLIT][Panorama] command failed: " + command + " (" + error + ")");
            return false;
        }
    }

    function panel(id) {
        return $.GetContextPanel().FindChildTraverse(id);
    }

    function setToggle(id, enabled) {
        var value = panel(id);
        if (!value) return;
        value.SetSelected(enabled);
        value.style.backgroundColor = enabled ? "#526875" : "#202a31";
    }

    function toggle(kind) {
        if (!(kind in state)) return;
        state[kind] = !state[kind];

        if (kind === "stamina") {
            dispatch("split_practice_stamina " + (state[kind] ? "1" : "0"));
            setToggle("SplitStamina", state[kind]);
        } else if (kind === "cooldowns") {
            dispatch("split_practice_cooldowns " + (state[kind] ? "1" : "0"));
            setToggle("SplitCooldowns", state[kind]);
        } else if (kind === "ammo") {
            dispatch("split_practice_ammo " + (state[kind] ? "1" : "0"));
            setToggle("SplitAmmo", state[kind]);
        }
    }

    function toggleCollapsed() {
        state.collapsed = !state.collapsed;
        var body = panel("SplitMovementBody");
        var root = panel("SplitMovementRoot");
        var glyph = panel("SplitCollapseGlyph");
        if (body) body.visible = !state.collapsed;
        if (root) root.style.height = state.collapsed ? "90px" : "560px";
        if (glyph) glyph.text = state.collapsed ? "+" : "—";
    }

    globalThis.SplitMovement = {
        command: dispatch,
        toggle: toggle,
        toggleCollapsed: toggleCollapsed
    };

    $.Msg("[SPLIT][Panorama] Movement Lab loaded");
})();
