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
    var lastDrillGeneration = -1;
    var feedbackToken = 0;
    var hiddenGamePanels = [];

    function findGameHud(panel) {
        for (var at = panel; at; at = at.GetParent()) {
            if (at.paneltype === "CitadelHud") return at;
        }
        return null;
    }

    function walkPanels(panel, visit) {
        if (!panel || !panel.IsValid()) return;
        visit(panel);
        var children = panel.Children();
        for (var i = 0; i < children.length; i++) {
            walkPanels(children[i], visit);
        }
    }

    function setNativeMatchHudHidden(hidden) {
        var game = findGameHud(context);
        if (!game) return;

        if (hidden) {
            if (hiddenGamePanels.length > 0) return;
            walkPanels(game, function (panel) {
                // Freestyle keeps gameplay HUD (crosshair, minimap, abilities)
                // but removes the stock match/team top bar.
                if (panel.paneltype === "CitadelHudTopBar") {
                    hiddenGamePanels.push({
                        panel: panel,
                        visibility: panel.style.visibility
                    });
                    panel.style.visibility = "collapse";
                    $.Msg("[SPLIT][Panorama] stock CitadelHudTopBar hidden");
                }
            });
            return;
        }

        for (var i = 0; i < hiddenGamePanels.length; i++) {
            var item = hiddenGamePanels[i];
            if (item.panel && item.panel.IsValid()) {
                item.panel.style.visibility = item.visibility || "visible";
            }
        }
        if (hiddenGamePanels.length > 0) {
            $.Msg("[SPLIT][Panorama] stock CitadelHudTopBar restored");
        }
        hiddenGamePanels = [];
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

    function showFeedback(text) {
        var feedback = label("SplitHudFeedback");
        if (!feedback) return;
        feedbackToken++;
        var token = feedbackToken;
        feedback.text = text;
        feedback.style.visibility = "visible";
        $.Schedule(1.25, function () {
            if (token === feedbackToken && feedback && feedback.IsValid()) {
                feedback.style.visibility = "collapse";
            }
        });
    }

    function signedDelta(seconds) {
        if (typeof seconds !== "number" || !isFinite(seconds)) return "";
        return (seconds > 0 ? "+" : "") + seconds.toFixed(3);
    }

    function applyState(next) {
        if (next.active !== true) {
            setNativeMatchHudHidden(false);
            if (root && root.IsValid()) root.DeleteAsync(0);
            root = null;
            return;
        }

        setNativeMatchHudHidden(true);
        ensureHud();

        var phase = next.racePhase || "idle";
        var title = label("RouteTitle");
        var you = label("RouteYou");
        var delta = label("RouteDelta");
        var reference = label("RouteReference");
        var instruction = label("RouteInstruction");

        if (title) {
            var routeName = (next.routeName || "Recorded Route").toUpperCase();
            var difficulty = (next.difficulty || "Unrated").toUpperCase();
            title.text = routeName + " · " + difficulty;
        }

        if (you) {
            you.text = phase === "running" || phase === "finished"
                ? Number(next.elapsedSeconds || 0).toFixed(2)
                : "0.00";
        }

        if (reference) {
            reference.text = typeof next.referenceSeconds === "number"
                ? Number(next.referenceSeconds).toFixed(2)
                : "—";
        }

        if (delta) {
            if (phase === "armed") {
                delta.text = "READY";
                delta.style.color = "#f06a64";
            } else if (phase === "running" && typeof next.referenceSeconds === "number") {
                var runningDelta = Number(next.elapsedSeconds || 0) - Number(next.referenceSeconds);
                delta.text = (runningDelta > 0 ? "+" : "") + runningDelta.toFixed(2);
                delta.style.color = runningDelta <= 0 ? "#55d79a" : "#f06a64";
            } else if (phase === "finished" && typeof next.referenceSeconds === "number") {
                var finishDelta = Number(next.elapsedSeconds || 0) - Number(next.referenceSeconds);
                delta.text = (finishDelta > 0 ? "+" : "") + finishDelta.toFixed(2);
                delta.style.color = finishDelta <= 0 ? "#55d79a" : "#f06a64";
            } else {
                delta.text = "—";
                delta.style.color = "#f06a64";
            }
        }

        if (instruction) {
            instruction.text = phase === "armed" ? "Leave the start to go" :
                               phase === "running" ? "" :
                               phase === "finished" ? "Run finished" :
                               next.hasRoute ? "Go to the start" :
                               "Record a route to begin";
            instruction.style.visibility = instruction.text ? "visible" : "collapse";
        }

        var generation = Number(next.drillGeneration || 0);
        if (generation !== lastDrillGeneration) {
            if (lastDrillGeneration >= 0) {
                if (next.drillFeedback === "saved") showFeedback("POSITION SAVED");
                else if (next.drillFeedback === "returned") showFeedback("RETURNED");
                else if (next.drillFeedback === "empty") showFeedback("NO POSITION SAVED");
            }
            lastDrillGeneration = generation;
        }
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
