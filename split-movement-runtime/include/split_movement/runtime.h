#pragma once

#include <string_view>

namespace split::movement {

inline constexpr std::string_view kRuntimeName = "SPLIT Movement Runtime";
inline constexpr std::string_view kRuntimeVersion = "0.1.0";

struct RuntimeStatus {
  bool engine_bridge_ready = false;
  bool movement_hook_ready = false;
  bool command_hook_ready = false;
  bool pawn_observer_ready = false;
  bool bot_control_ready = false;
};

RuntimeStatus ProbeRuntime();
void PrintStatus(const RuntimeStatus& status);

}  // namespace split::movement
