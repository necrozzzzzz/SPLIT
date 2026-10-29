# SPLIT Movement Lab Panorama

This directory is the in-game UI source. It is intentionally separate from the Tauri/React SPLIT desktop UI.

Architecture follows Modlock's current Deadlock integration:

1. A temporary session package is mounted ahead of `citadel` in `gameinfo.gi`.
2. The package shadows a stock Panorama layout that Deadlock keeps loaded.
3. `boot.js` creates `SplitMovementLabHost` and loads `movement_lab.xml`.
4. Buttons dispatch `CitadelConCommand` commands with a `split_*` prefix.
5. The runtime hooks `Source2GameClients001::ClientCommand` (slot 17), consumes those commands before Deadlock rejects them, and routes them to Movement Lab.

The player must not manually install or remove a VPK. Mount/unmount belongs to the SPLIT runtime session.
