#include "split_movement/engine_bridge.h"
#include "split_movement/engine_runner.h"
#include "split_movement/native_hooks.h"
#include "split_movement/pawn_probe.h"
#include "split_movement/runtime.h"

#include <atomic>
#include <cstdlib>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <string>
#include <string_view>

namespace {

void Help() {
  std::cout
      << "SPLIT Movement Runtime\n\n"
      << "Probe only:\n"
      << "  split-movement-runtime --game-dir PATH\n\n"
      << "Run dedicated Movement Lab server:\n"
      << "  split-movement-runtime --game-dir PATH --server [--map dl_midtown] [--port 27067]\n\n"
      << "DEADLOCK_DIR can replace --game-dir.\n";
}

void PrintClient(const char* stage, const split::movement::ClientConnection& client) {
  std::cout << "[SPLIT][Movement] " << stage
            << " slot=" << client.slot
            << " xuid=" << client.xuid
            << " bot=" << (client.is_bot ? "yes" : "no")
            << " ready=" << (client.fully_connected ? "yes" : "no")
            << " name=\"" << client.name << "\"\n";
}

}  // namespace

int main(int argc, char** argv) {
  std::filesystem::path game_dir;
  bool run_server = false;
  split::movement::ServerLaunch launch;

  if (const char* environment = std::getenv("DEADLOCK_DIR")) {
    game_dir = environment;
  }

  for (int i = 1; i < argc; ++i) {
    const std::string_view arg(argv[i]);

    if (arg == "--help") {
      Help();
      return 0;
    }
    if (arg == "--server") {
      run_server = true;
      continue;
    }
    if (arg == "--game-dir" && i + 1 < argc) {
      game_dir = argv[++i];
      continue;
    }
    if (arg == "--map" && i + 1 < argc) {
      launch.map = argv[++i];
      continue;
    }
    if (arg == "--port" && i + 1 < argc) {
      const auto parsed = std::stoi(argv[++i]);
      if (parsed < 1 || parsed > 65535) {
        std::cerr << "Port must be between 1 and 65535.\n";
        return 2;
      }
      launch.port = static_cast<std::uint16_t>(parsed);
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

  if (!run_server) {
    const auto status = split::movement::ProbeRuntime(game_dir);
    split::movement::PrintStatus(status);
    return status.engine_bridge_ready ? 0 : 1;
  }

  split::movement::EngineBridge bridge;
  const auto engine = bridge.Open(game_dir);
  if (!engine.schema_interface_ready) {
    std::cerr << "Engine bridge failed: " << engine.error << "\n";
    return 1;
  }

  std::cout << "SPLIT Movement Runtime: engine bridge ready.\n";

  split::movement::PawnProbe pawn_probe(bridge.SchemaSystem());
  std::atomic<int> human_slot{-1};
  bool pawn_announced = false;
  std::string last_pawn_error;

  split::movement::NativeHooks hooks;
  std::string hook_error;
  if (!hooks.Install(
          [&](std::uint64_t frame) {
            if (frame == 1 || frame % 300 == 0) {
              std::cout << "[SPLIT][Movement] server frame " << frame << "\n";
            }

            const int slot = human_slot.load(std::memory_order_relaxed);
            if (slot < 0) return;

            std::string error;
            auto sample = pawn_probe.Observe(slot, error);
            if (!sample) {
              if (!error.empty() && error != last_pawn_error) {
                last_pawn_error = error;
                std::cout << "[SPLIT][Movement] pawn probe waiting: " << error << "\n";
              }
              return;
            }

            last_pawn_error.clear();
            if (!pawn_announced) {
              pawn_announced = true;
              std::cout << "[SPLIT][Movement] pawn attached slot=" << sample->slot
                        << " handle=0x" << std::hex << sample->pawn_handle << std::dec << "\n";
            }

            if (frame % 120 == 0) {
              std::cout << std::fixed << std::setprecision(1)
                        << "[SPLIT][Movement] pose"
                        << " pos=(" << sample->position[0] << ", "
                                    << sample->position[1] << ", "
                                    << sample->position[2] << ")"
                        << " vel=(" << sample->velocity[0] << ", "
                                    << sample->velocity[1] << ", "
                                    << sample->velocity[2] << ")"
                        << " grounded=" << (sample->grounded ? "yes" : "no")
                        << "\n";
            }
          },
          [](std::string_view map) {
            std::cout << "[SPLIT][Movement] loading world: " << map << "\n";
          },
          [](std::string_view map) {
            std::cout << "[SPLIT][Movement] world ready: " << map << "\n";
          },
          [](const split::movement::ClientConnection& client) {
            PrintClient("client entered", client);
          },
          [&](const split::movement::ClientConnection& client) {
            PrintClient("client fully connected", client);
            if (!client.is_bot) {
              human_slot.store(client.slot, std::memory_order_relaxed);
              std::cout << "[SPLIT][Movement] human session ready on slot "
                        << client.slot
                        << "; attaching pawn probe.\n";
            }
          },
          [&](int slot, std::uint64_t xuid) {
            std::cout << "[SPLIT][Movement] client disconnecting slot="
                      << slot << " xuid=" << xuid << "\n";
            if (human_slot.load(std::memory_order_relaxed) == slot) {
              human_slot.store(-1, std::memory_order_relaxed);
              pawn_announced = false;
            }
          },
          hook_error)) {
    std::cerr << "Native hook installation failed: " << hook_error << "\n";
    return 1;
  }

  std::cout << "SPLIT Movement Runtime: frame/world/client hooks installed.\n"
            << "Starting dedicated server on 127.0.0.1:" << launch.port
            << " with map " << launch.map << ".\n";

  std::string run_error;
  const int code = split::movement::RunDedicatedServer(game_dir, launch, run_error);
  if (!run_error.empty()) {
    std::cerr << "Engine handoff failed: " << run_error << "\n";
    return 1;
  }

  std::cout << "Source 2 exited with code " << code
            << " after " << hooks.FrameCount() << " observed frames.\n";
  return code;
}
