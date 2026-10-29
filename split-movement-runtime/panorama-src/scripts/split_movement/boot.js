(function () {
    "use strict";

    var context = $.GetContextPanel();
    if (!context || context.FindChildTraverse("SplitMovementLabHost")) {
        return;
    }

    var host = $.CreatePanel("Panel", context, "SplitMovementLabHost");
    host.hittest = true;
    host.BLoadLayout("file://{resources}/layout/split_movement/movement_lab.xml", false, false);
})();
