#pragma once

#include <filesystem>
#include <string>
#include <string_view>

namespace split::movement {

inline constexpr std::string_view kRuntimeName = "SPLIT Movement Runtime";
inline constexpr std::string_view kRuntimeVersion = "0.5.42";

struct RuntimeStatus {
  bool game_layout_valid = false;
  bool engine2_loaded = false;
  bool server_loaded = false;
  bool schema_module_loaded = false;
  bool engine_bridge_ready = false;
  bool movement_hook_ready = false;
  bool command_hook_ready = false;
  bool pawn_observer_ready = false;
  bool bot_control_ready = false;
  std::string error;
};

RuntimeStatus ProbeRuntime(const std::filesystem::path& game_dir);
void PrintStatus(const RuntimeStatus& status);

}  // namespace split::movement
