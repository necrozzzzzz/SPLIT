#include "split_movement/runtime.h"

#include <iostream>

namespace split::movement {

RuntimeStatus ProbeRuntime() {
  // Phase 1 intentionally does not install native hooks yet.
  // The Modlock MIT gameinterop snapshot is vendored beside this runtime.
  // Each capability will flip to true as we wire and validate it against
  // Deadlock independently.
  return {};
}

void PrintStatus(const RuntimeStatus& status) {
  const auto yes_no = [](bool value) { return value ? "ready" : "not wired"; };

  std::cout << kRuntimeName << " " << kRuntimeVersion << "\n"
            << "  engine bridge: " << yes_no(status.engine_bridge_ready) << "\n"
            << "  movement hook: " << yes_no(status.movement_hook_ready) << "\n"
            << "  command hook: " << yes_no(status.command_hook_ready) << "\n"
            << "  pawn observer: " << yes_no(status.pawn_observer_ready) << "\n"
            << "  bot control: " << yes_no(status.bot_control_ready) << "\n";
}

}  // namespace split::movement
