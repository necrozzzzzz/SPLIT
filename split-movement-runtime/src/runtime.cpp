#include "split_movement/runtime.h"

#include <iostream>

#include "split_movement/engine_bridge.h"

namespace split::movement {

RuntimeStatus ProbeRuntime(const std::filesystem::path& game_dir) {
  RuntimeStatus status;

  EngineBridge bridge;
  const auto engine = bridge.Open(game_dir);

  status.game_layout_valid = engine.game_layout_valid;
  status.engine2_loaded = engine.engine2_loaded;
  status.server_loaded = engine.server_loaded;
  status.schema_module_loaded = engine.schema_module_loaded;
  status.engine_bridge_ready = engine.schema_interface_ready;
  status.error = engine.error;

  // Next phase: install frame/pawn/movement hooks after the engine host is
  // running a world. Keeping these false makes the probe honest.
  return status;
}

void PrintStatus(const RuntimeStatus& status) {
  const auto yes_no = [](bool value) { return value ? "ready" : "not ready"; };

  std::cout << kRuntimeName << " " << kRuntimeVersion << "\n"
            << "  game layout: " << yes_no(status.game_layout_valid) << "\n"
            << "  engine2.dll: " << yes_no(status.engine2_loaded) << "\n"
            << "  server.dll: " << yes_no(status.server_loaded) << "\n"
            << "  schemasystem.dll: " << yes_no(status.schema_module_loaded) << "\n"
            << "  SchemaSystem_001: " << yes_no(status.engine_bridge_ready) << "\n"
            << "  movement hook: " << yes_no(status.movement_hook_ready) << "\n"
            << "  command hook: " << yes_no(status.command_hook_ready) << "\n"
            << "  pawn observer: " << yes_no(status.pawn_observer_ready) << "\n"
            << "  bot control: " << yes_no(status.bot_control_ready) << "\n";

  if (!status.error.empty()) {
    std::cout << "  error: " << status.error << "\n";
  }
}

}  // namespace split::movement
