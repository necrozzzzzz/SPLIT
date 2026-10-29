(function () {
    "use strict";

    function findGameHud(panel) {
        for (var at = panel; at; at = at.GetParent()) {
            if (at.paneltype === "CitadelHud") {
                return at;
            }
        }
        return panel;
    }

    var context = $.GetContextPanel();
    var game = findGameHud(context);

    if (!game || game.FindChildTraverse("SplitMovementRootHost")) {
        return;
    }

    // Match Modlock's renderer: create the actual UI root on CitadelHud,
    // not inside latest_news/bootstrap.
    var root = $.CreatePanel("Panel", game, "SplitMovementRootHost");
    root.hittest = true;
    root.style.width = "100%";
    root.style.height = "100%";
    root.style.horizontalAlign = "left";
    root.style.verticalAlign = "top";
    root.BLoadLayout("file://{resources}/layout/split_movement/movement_lab.xml", false, false);

    $.Msg("[SPLIT][Panorama] Movement Lab attached to CitadelHud");
})();
