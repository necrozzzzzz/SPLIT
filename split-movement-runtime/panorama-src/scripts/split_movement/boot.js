(function () {
    "use strict";

    var context = $.GetContextPanel();
    if (!context || context.FindChildTraverse("SplitMovementLabHost")) {
        return;
    }

    var host = $.CreatePanel("Panel", context, "SplitMovementLabHost");
    host.hittest = false;
    host.style.width = "100%";
    host.style.height = "100%";
    host.style.horizontalAlign = "left";
    host.style.verticalAlign = "top";
    host.BLoadLayout("file://{resources}/layout/split_movement/movement_lab.xml", false, false);
})();
