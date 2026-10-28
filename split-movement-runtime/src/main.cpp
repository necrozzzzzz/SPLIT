#include "split_movement/runtime.h"

#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <string_view>

namespace {

void Help() {
  std::cout
      << "SPLIT Movement Runtime probe\n\n"
      << "Usage:\n"
      << "  split-movement-runtime --game-dir PATH\n"
      << "  set DEADLOCK_DIR=PATH && split-movement-runtime\n";
}

}  // namespace

int main(int argc, char** argv) {
  std::filesystem::path game_dir;

  if (const char* environment = std::getenv("DEADLOCK_DIR")) {
    game_dir = environment;
  }

  for (int i = 1; i < argc; ++i) {
    const std::string_view arg(argv[i]);
    if (arg == "--help") {
      Help();
      return 0;
    }
    if (arg == "--game-dir" && i + 1 < argc) {
      game_dir = argv[++i];
      continue;
    }

    std::cerr << "Unknown or incomplete argument: " << arg << "\n";
    Help();
    return 2;
  }

  if (game_dir.empty()) {
    std::cerr << "Set DEADLOCK_DIR or pass --game-dir with the Deadlock install root.\n";
    return 2;
  }

  const auto status = split::movement::ProbeRuntime(game_dir);
  split::movement::PrintStatus(status);
  return status.engine_bridge_ready ? 0 : 1;
}
