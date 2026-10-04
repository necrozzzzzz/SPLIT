# SPLIT Panorama addon

Build the installable addon from the repository root:

```powershell
npm.cmd run build:panorama-addon
```

The build uses the Reduced CSDK `resourcecompiler.exe` and `CSDKCfgVPK.exe`.
Set `SPLIT_CSDK_ROOT` when the CSDK is not installed at the default Downloads
location. It creates fresh, isolated `build_split_panorama` staging folders,
compiles the six SPLIT resources plus the HUD override, validates the exact VPK
directory tree, and removes the CSDK staging folders. The final artifact is:

`artifacts/panorama/SPLIT-Panorama.vpk`

`stock/panorama/layout/citadel_hud_active_player_stats.xml` is the pristine
current Valve layout. The build verifies its checksum and injects only the
style include and the five ordered script includes declared by SPLIT. The old
POC output is neither read nor used as a VPK base.

To install manually, copy the artifact to Deadlock's
`game/citadel/addons` directory and rename it to an unused
`pakNN_dir.vpk` name. Ensure `Game citadel/addons` is present before the stock
`Game citadel` entry in `game/citadel/gameinfo.gi`. No Steam launch option is
required. A compatible mod manager can perform these installation steps too.

The desktop Panorama edition serves the bridge at
`http://127.0.0.1:32146/ipc/state-bit`. State is transferred in CRC-protected
16-byte frames through a reusable pool of Panorama `Image` panels; actions use
the same image-request mechanism. The legacy PNG bridge on port 32145 is not
used by these files.
