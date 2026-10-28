#include "split_movement/bot_factory.h"
#include "split_movement/engine_bridge.h"
#include "split_movement/engine_runner.h"
#include "split_movement/native_hooks.h"
#include "split_movement/input_capture.h"
#include "split_movement/movement_steering.h"
#include "split_movement/pawn_probe.h"
#include "split_movement/recorder.h"
#include "split_movement/runtime.h"

#include <array>
#include <atomic>
#include <cstdlib>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <string>
#include <string_view>
#include <thread>

namespace {

void Help() {
  std::cout
      << "SPLIT Movement Runtime\n\n"
      << "Probe only:\n"
      << "  split-movement-runtime --game-dir PATH\n\n"
      << "Run dedicated Movement Lab server:\n"
      << "  split-movement-runtime --game-dir PATH --server [--map dl_midtown] [--port 27067]\n\n"
      << "Runtime commands while the server is running:\n"
      << "  record  start a fresh route recording\n"
      << "  stop    stop recording and print stats\n"
      << "  status  print current recording stats\n"
      << "  save    save the current route as JSON\n"
      << "  replay  spawn SPLIT Ghost and replay current route\n"
      << "  clear   discard the current route\n\n"
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

void PrintStats(const split::movement::RecordingStats& stats) {
  std::cout << std::fixed << std::setprecision(1)
            << "[SPLIT][Recorder] "
            << (stats.recording ? "recording" : "stopped")
            << " ticks=" << stats.ticks
            << " input=" << stats.ticks_with_input << "/" << stats.ticks
            << " distance=" << stats.distance
            << " max_speed=" << stats.max_speed;
  if (stats.ticks > 0) {
    std::cout << " frames=" << stats.first_frame << ".." << stats.last_frame;
  }
  std::cout << "\n";
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
  split::movement::InputCapture input_capture;
  split::movement::MovementRecorder recorder;
  std::atomic<int> human_slot{-1};
  bool pawn_announced = false;
  bool steering_was_active = false;
  std::atomic<bool> replay_requested{false};
  std::string last_pawn_error;
  std::string current_map = launch.map;

  const auto route_directory =
      std::filesystem::current_path() / "routes";

  // Source2Main owns the main thread. Keep a tiny command reader beside it so
  // this prototype can exercise recording without needing Tauri IPC yet.
  std::jthread command_thread([&](std::stop_token stop) {
    std::string command;
    while (!stop.stop_requested() && std::getline(std::cin, command)) {
      if (command == "record") {
        recorder.Start();
        std::cout << "[SPLIT][Recorder] recording started\n";
      } else if (command == "stop") {
        PrintStats(recorder.Stop());
      } else if (command == "status") {
        PrintStats(recorder.Stats());
      } else if (command == "clear") {
        recorder.Clear();
        std::cout << "[SPLIT][Recorder] route cleared\n";
      } else if (command == "replay") {
        replay_requested.store(true, std::memory_order_release);
        std::cout << "[SPLIT][Replay] replay requested\n";
      } else if (command == "save") {
        auto saved = recorder.SaveJson(route_directory, current_map);
        if (saved) {
          std::cout << "[SPLIT][Recorder] saved " << saved->string() << "\n";
        } else {
          std::cout << "[SPLIT][Recorder] save failed: " << saved.error() << "\n";
        }
      } else if (!command.empty()) {
        std::cout << "[SPLIT][Recorder] unknown command: " << command
                  << " (record, stop, status, save, replay, clear)\n";
      }
    }
  });

  std::string input_error;
  if (!input_capture.Install(input_error)) {
    command_thread.request_stop();
    std::cerr << "Input capture installation failed: " << input_error << "\n";
    return 1;
  }
  std::cout << "SPLIT Movement Runtime: ProcessUsercmds input capture installed.\n";

  split::movement::BotFactory bot_factory;
  std::string bot_error;
  if (!bot_factory.Install(bot_error)) {
    command_thread.request_stop();
    std::cerr << "Bot replay installation failed: " << bot_error << "\n";
    return 1;
  }
  std::cout << "SPLIT Movement Runtime: bot factory/replay guard installed.\n";

  split::movement::MovementSteering steering(bridge.SchemaSystem(), &input_capture);
  std::string steering_error;
  if (!steering.Install(steering_error)) {
    command_thread.request_stop();
    std::cerr << "Movement steering installation failed: " << steering_error << "\n";
    return 1;
  }
  std::cout << "SPLIT Movement Runtime: ProcessMovement steering installed.\n";

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

            recorder.Push(frame, *sample, input_capture.Latest(slot));

            if (replay_requested.exchange(false, std::memory_order_acq_rel)) {
              const auto first = recorder.FirstPawn();
              auto ticks = recorder.PlaybackTicks();
              if (!first || ticks.empty()) {
                std::cout << "[SPLIT][Replay] no recorded route is available\n";
              } else if (first->team < 2 || first->team > 3 || first->hero_id == 0) {
                std::cout << "[SPLIT][Replay] route has invalid team/hero metadata"
                          << " team=" << static_cast<int>(first->team)
                          << " hero=" << first->hero_id << "\n";
              } else {
                std::array<bool, 64> occupied_before{};
                for (int i = 0; i < 64; ++i) {
                  occupied_before[i] = hooks.ClientAt(i).has_value();
                }

                std::string replay_error;
                steering.Stop();
                if (!bot_factory.Create("SPLIT Ghost", static_cast<int>(first->team),
                                        first->hero_id, first->position, replay_error)) {
                  std::cout << "[SPLIT][Replay] bot creation failed: "
                            << replay_error << "\n";
                } else {
                  int bot_slot = -1;
                  for (int i = 0; i < 64; ++i) {
                    const auto client = hooks.ClientAt(i);
                    if (!occupied_before[i] && client && client->is_bot &&
                        client->name == "SPLIT Ghost") {
                      bot_slot = i;
                      break;
                    }
                  }

                  if (bot_slot < 0) {
                    std::cout << "[SPLIT][Replay] bot was created but its slot was not observed\n";
                  } else if (!steering.Start(bot_slot, std::move(ticks), replay_error)) {
                    std::cout << "[SPLIT][Replay] steering start failed: "
                              << replay_error << "\n";
                  } else {
                    const auto state = steering.Status();
                    steering_was_active = state.active;
                    std::cout << "[SPLIT][Replay] SPLIT Ghost slot=" << bot_slot
                              << " hero=" << first->hero_id
                              << " team=" << static_cast<int>(first->team)
                              << " steps=" << state.total
                              << " mode=movement-steering\n";
                  }
                }
              }
            }

            const auto steering_status = steering.Status();
            if (steering_status.active && frame % 120 == 0) {
              std::cout << "[SPLIT][Replay] steering "
                        << steering_status.index << "/" << steering_status.total
                        << " slot=" << steering_status.slot << "\n";
            }
            if (steering_was_active && !steering_status.active &&
                steering_status.total > 0 &&
                steering_status.index >= steering_status.total) {
              std::cout << "[SPLIT][Replay] steering complete "
                        << steering_status.index << "/" << steering_status.total << "\n";
              steering_was_active = false;
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
          [&](std::string_view map) {
            current_map.assign(map);
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
              std::cout << "[SPLIT][Recorder] type 'record' here to start a route.\n";
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
    command_thread.request_stop();
    std::cerr << "Native hook installation failed: " << hook_error << "\n";
    return 1;
  }

  std::cout << "SPLIT Movement Runtime: frame/world/client hooks installed.\n"
            << "Starting dedicated server on 127.0.0.1:" << launch.port
            << " with map " << launch.map << ".\n";

  std::string run_error;
  const int code = split::movement::RunDedicatedServer(game_dir, launch, run_error);

  command_thread.request_stop();

  if (!run_error.empty()) {
    std::cerr << "Engine handoff failed: " << run_error << "\n";
    return 1;
  }

  std::cout << "Source 2 exited with code " << code
            << " after " << hooks.FrameCount() << " observed frames.\n";
  return code;
}
