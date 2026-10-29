(function () {
    "use strict";

    var bridgeURL = "https://hyperline.gg/app/relay/bridge.html?port=27169";
    var titlePrefix = "modlock:frame:";
    var context = $.GetContextPanel();
    var statePanel = context.FindChildTraverse("SplitBridgeState");
    var root = null;
    var parts = null;
    var lastTitle = "";
    var acked = false;
    var toggles = {
        stamina: false,
        cooldowns: false,
        ammo: false
    };

    function findGameHud(panel) {
        for (var at = panel; at; at = at.GetParent()) {
            if (at.paneltype === "CitadelHud") return at;
        }
        return null;
    }

    function command(text) {
        try {
            $.DispatchEvent("CitadelConCommand", text);
            return true;
        } catch (error) {
            $.Msg("[SPLIT][Panorama] command failed: " + text + " (" + error + ")");
            return false;
        }
    }

    function panel(id) {
        return root ? root.FindChildTraverse(id) : null;
    }

    function setToggle(id, enabled) {
        var value = panel(id);
        if (!value) return;
        value.SetSelected(enabled);
        value.style.backgroundColor = enabled ? "#526875" : "#202a31";
    }

    function toggle(kind) {
        if (!(kind in toggles)) return;
        toggles[kind] = !toggles[kind];

        if (kind === "stamina") {
            command("split_practice_stamina " + (toggles[kind] ? "1" : "0"));
            setToggle("SplitStamina", toggles[kind]);
        } else if (kind === "cooldowns") {
            command("split_practice_cooldowns " + (toggles[kind] ? "1" : "0"));
            setToggle("SplitCooldowns", toggles[kind]);
        } else if (kind === "ammo") {
            command("split_practice_ammo " + (toggles[kind] ? "1" : "0"));
            setToggle("SplitAmmo", toggles[kind]);
        }
    }

    function toggleCollapsed() {
        if (!root) return;
        var body = panel("SplitMovementBody");
        var frame = panel("SplitMovementRoot");
        var glyph = panel("SplitCollapseGlyph");
        var collapsed = body ? body.visible : true;
        if (body) body.visible = !collapsed;
        if (frame) frame.style.height = collapsed ? "90px" : "560px";
        if (glyph) glyph.text = collapsed ? "+" : "—";
    }

    function show(active) {
        var game = findGameHud(context);
        if (!active || !game) {
            if (root && root.IsValid()) root.DeleteAsync(0);
            root = null;
            return;
        }

        if (root && root.IsValid()) return;

        root = $.CreatePanel("Panel", game, "SplitMovementRootHost");
        root.hittest = false;
        root.hittestchildren = true;
        root.style.width = "100%";
        root.style.height = "100%";
        root.style.horizontalAlign = "left";
        root.style.verticalAlign = "top";
        root.BLoadLayout(
            "file://{resources}/layout/split_movement/movement_lab.xml",
            false,
            false
        );
        $.Msg("[SPLIT][Panorama] Movement Lab visible");
    }

    function unpack(text) {
        var output = "";
        for (var i = 0; i < text.length; i++) {
            var code = text.charCodeAt(i);
            if (code >= 1 && code <= 127) {
                output += String.fromCharCode(code);
            } else if (code >= 0x100 && code <= 0x1ff) {
                output += String.fromCharCode(code - 0x100);
            } else {
                return null;
            }
        }
        return output;
    }

    function applyFrame(text) {
        try {
            var json = unpack(text);
            if (json === null) throw new Error("invalid packed bytes");
            var next = JSON.parse(json);
            show(next.active === true);
        } catch (error) {
            $.Msg("[SPLIT][Panorama] invalid bridge frame: " + error);
            show(false);
        }
    }

    function readTitle(title) {
        if (!title.startsWith(titlePrefix)) {
            parts = null;
            lastTitle = "";
            if (title === "") show(false);
            return false;
        }

        if (title === lastTitle) return false;
        lastTitle = title;

        try {
            var fields = title.slice(titlePrefix.length).split(":", 4);
            if (fields.length !== 4) throw new Error("malformed relay frame");

            var id = Number(fields[0]);
            var offset = Number(fields[1]);
            var total = Number(fields[2]);
            var prefixLength = titlePrefix.length + fields.join(":").length + 1;
            var chunk = title.slice(prefixLength);

            if (!id || !total) throw new Error("invalid relay frame header");
            if (offset === 0) parts = { id: id, total: total, text: "" };
            if (!parts || parts.id !== id || parts.text.length !== offset) {
                throw new Error("missing relay frame part");
            }

            parts.text += chunk;
            acked = !acked;
            statePanel.SetURL(bridgeURL + "#" + (acked ? "1" : "0"));

            if (parts.text.length < parts.total) return false;

            var complete = parts.text;
            parts = null;
            applyFrame(complete);
        } catch (error) {
            $.Msg("[SPLIT][Panorama] bridge read failed: " + error);
            parts = null;
            show(false);
        }
        return false;
    }

    globalThis.SplitMovement = {
        command: command,
        toggle: toggle,
        toggleCollapsed: toggleCollapsed
    };

    if (!statePanel) {
        $.Msg("[SPLIT][Panorama] bridge HTML panel missing");
        return;
    }

    $.RegisterEventHandler("HTMLStartRequest", statePanel, function () {
        return false;
    });
    $.RegisterEventHandler("HTMLTitle", statePanel, function (_, title) {
        return readTitle(title);
    });

    show(false);
    statePanel.SetURL(bridgeURL);
    $.Msg("[SPLIT][Panorama] bridge started");
})();
