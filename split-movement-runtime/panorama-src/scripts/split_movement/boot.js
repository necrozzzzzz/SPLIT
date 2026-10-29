(function () {
    "use strict";

    var context = $.GetContextPanel();
    if (!context || context.FindChildTraverse("SplitMovementBootstrap")) {
        return;
    }

    // Match Modlock: latest_news only hosts a tiny bootstrap layout.
    var host = $.CreatePanel("Panel", context, "SplitMovementBootstrap");
    host.hittest = false;
    host.BLoadLayout("file://{resources}/layout/split_movement/bootstrap.xml", false, false);
})();
