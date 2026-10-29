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

    function findGameHud(panel) {
        for (var at = panel; at; at = at.GetParent()) {
            if (at.paneltype === "CitadelHud") return at;
        }
        return null;
    }

    function ensureHud() {
        var game = findGameHud(context);
        if (!game) return null;
        if (root && root.IsValid()) return root;

        root = $.CreatePanel("Panel", game, "SplitMovementHudHost");
        root.hittest = false;
        root.hittestchildren = false;
        root.style.width = "100%";
        root.style.height = "100%";
        root.BLoadLayout("file://{resources}/layout/split_movement/movement_lab.xml", false, false);
        return root;
    }

    function label(id) {
        var hud = ensureHud();
        return hud ? hud.FindChildTraverse(id) : null;
    }

    function formatTime(seconds) {
        if (typeof seconds !== "number" || !isFinite(seconds)) return "--:--.---";
        var wholeMinutes = Math.floor(seconds / 60);
        var remaining = seconds - wholeMinutes * 60;
        var sec = remaining.toFixed(3);
        if (remaining < 10) sec = "0" + sec;
        return (wholeMinutes < 10 ? "0" : "") + wholeMinutes + ":" + sec;
    }

    function setVisible(panel, visible) {
        if (panel) panel.style.visibility = visible ? "visible" : "collapse";
    }

    function applyState(next) {
        if (next.active !== true) {
            if (root && root.IsValid()) root.DeleteAsync(0);
            root = null;
            return;
        }

        var hud = ensureHud();
        if (!hud) return;

        var status = label("SplitHudStatus");
        var timer = label("SplitHudTimer");
        var pb = label("SplitHudPb");
        var split = label("SplitHudSplit");
        var rec = label("SplitHudRec");
        var assists = label("SplitHudAssists");

        var phase = next.racePhase || "idle";
        var racing = phase === "armed" || phase === "running" || phase === "finished";

        if (status) {
            status.text = phase === "armed" ? "ROUTE ARMED" :
                          phase === "running" ? "ROUTE" :
                          phase === "finished" ? "FINISHED" :
                          next.recording ? "RECORDING ROUTE" :
                          "MOVEMENT LAB";
        }

        if (timer) timer.text = formatTime(next.elapsedSeconds || 0);
        setVisible(timer, racing);

        if (pb) {
            pb.text = next.personalBestSeconds == null
                ? "PB  --:--.---"
                : "PB  " + formatTime(next.personalBestSeconds);
        }
        setVisible(pb, racing);

        if (split) {
            var total = Number(next.checkpointCount || 0);
            var current = Number(next.nextCheckpoint || 0);
            split.text = total > 0 ? ("CHECKPOINT  " + current + " / " + total) : "";
        }
        setVisible(split, racing && Number(next.checkpointCount || 0) > 0);

        setVisible(rec, next.recording === true);

        var activeAssists = [];
        if (next.stamina) activeAssists.push("∞ STAMINA");
        if (next.cooldowns) activeAssists.push("NO COOLDOWN");
        if (next.ammo) activeAssists.push("∞ AMMO");
        if (assists) assists.text = activeAssists.join("   ·   ");
        setVisible(assists, activeAssists.length > 0);
    }

    function unpack(text) {
        var output = "";
        for (var i = 0; i < text.length; i++) {
            var code = text.charCodeAt(i);
            if (code >= 1 && code <= 127) output += String.fromCharCode(code);
            else if (code >= 0x100 && code <= 0x1ff) output += String.fromCharCode(code - 0x100);
            else return null;
        }
        return output;
    }

    function applyFrame(text) {
        try {
            var json = unpack(text);
            if (json === null) throw new Error("invalid packed bytes");
            applyState(JSON.parse(json));
        } catch (error) {
            $.Msg("[SPLIT][Panorama] invalid HUD frame: " + error);
        }
    }

    function readTitle(title) {
        if (!title.startsWith(titlePrefix)) {
            parts = null;
            lastTitle = "";
            if (title === "") applyState({ active: false });
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
        }
        return false;
    }

    if (!statePanel) {
        $.Msg("[SPLIT][Panorama] bridge HTML panel missing");
        return;
    }

    $.RegisterEventHandler("HTMLStartRequest", statePanel, function () { return false; });
    $.RegisterEventHandler("HTMLTitle", statePanel, function (_, title) {
        return readTitle(title);
    });

    statePanel.SetURL(bridgeURL);
    $.Msg("[SPLIT][Panorama] Freestyle-style HUD bridge started");
})();
