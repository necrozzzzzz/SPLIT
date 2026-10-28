#include "split_movement/bot_factory.h"
#include "split_movement/control_server.h"
#include "split_movement/engine_bridge.h"
#include "split_movement/engine_runner.h"
#include "split_movement/native_hooks.h"
#include "split_movement/input_capture.h"
#include "split_movement/movement_steering.h"
#include "split_movement/native_visual_renderer.h"
#include "split_movement/pawn_probe.h"
#include "split_movement/pb_route_store.h"
#include "split_movement/recorder.h"
#include "split_movement/race_timer.h"
#include "split_movement/runtime.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

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
      << "  replay      spawn SPLIT Ghost and replay current route\n"
      << "  watch       watch the recorded route as a ghost replay\n"
      << "  race        arm a timed run using route start/finish\n"
      << "  racestatus  print timer / PB status\n"
      << "  racecancel  cancel the current timed run\n"
      << "  visualtest  spawn a native Source 2 particle near the player\n"
      << "  tracertest  draw one native beam segment on the recorded route\n"
      << "  visualdiagnostics  print native visual bridge status\n"
      << "  clear       discard the current route\n\n"
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

double RouteDistance(const std::array<float, 3>& a,
                     const std::array<float, 3>& b) {
  const double x = static_cast<double>(a[0]) - b[0];
  const double y = static_cast<double>(a[1]) - b[1];
  const double z = static_cast<double>(a[2]) - b[2];
  return std::sqrt(x * x + y * y + z * z);
}

std::string JsonNumberOrNull(const std::optional<double>& value) {
  if (!value) return "null";
  std::ostringstream out;
  out << std::fixed << std::setprecision(6) << *value;
  return out.str();
}

std::string JsonNumberArray(const std::vector<double>& values) {
  std::ostringstream out;
  out << "[";
  for (std::size_t i = 0; i < values.size(); ++i) {
    if (i > 0) out << ",";
    out << std::fixed << std::setprecision(6) << values[i];
  }
  out << "]";
  return out.str();
}


std::string JsonVec3(const std::array<float, 3>& point) {
  std::ostringstream out;
  out << std::fixed << std::setprecision(2) << "[";
  for (std::size_t i = 0; i < 3; ++i) {
    if (i) out << ",";
    if (std::isfinite(point[i])) out << point[i];
    else out << "null";
  }
  out << "]";
  return out.str();
}

// A bounded, spatially uniform path for a future Source 2 client renderer.
// Keep UDP packets comfortably below the 64 KiB datagram ceiling.
std::string JsonRoutePolyline(const std::vector<split::movement::RecordedTick>& ticks) {
  constexpr std::size_t kMaxPoints = 128;
  std::ostringstream out;
  out << "[";
  if (!ticks.empty()) {
    const std::size_t count = std::min(ticks.size(), kMaxPoints);
    for (std::size_t i = 0; i < count; ++i) {
      if (i) out << ",";
      const std::size_t index = count == 1 ? 0 :
          i * (ticks.size() - 1) / (count - 1);
      out << JsonVec3(ticks[index].pawn.position);
    }
  }
  out << "]";
  return out.str();
}

const char* RacePhaseName(split::movement::RaceStatus::Phase phase) {
  switch (phase) {
    case split::movement::RaceStatus::Phase::kArmed: return "armed";
    case split::movement::RaceStatus::Phase::kRunning: return "running";
    case split::movement::RaceStatus::Phase::kFinished: return "finished";
    default: return "idle";
  }
}

std::vector<std::array<float, 3>> RouteCheckpoints(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::size_t count = 3) {
  std::vector<std::array<float, 3>> checkpoints;
  if (ticks.size() < 2 || count == 0) return checkpoints;

  std::vector<double> cumulative(ticks.size(), 0.0);
  for (std::size_t i = 1; i < ticks.size(); ++i) {
    cumulative[i] = cumulative[i - 1] +
                    RouteDistance(ticks[i - 1].pawn.position,
                                  ticks[i].pawn.position);
  }

  const double total = cumulative.back();
  if (!(total > 0.0)) return checkpoints;

  checkpoints.reserve(count);
  std::size_t cursor = 1;
  for (std::size_t split = 1; split <= count; ++split) {
    const double target =
        total * static_cast<double>(split) /
        static_cast<double>(count + 1);

    while (cursor + 1 < cumulative.size() &&
           cumulative[cursor] < target) {
      ++cursor;
    }
    checkpoints.push_back(ticks[cursor].pawn.position);
  }
  return checkpoints;
}


std::vector<std::array<float, 3>> RoutePolyline(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::size_t max_points = 128) {
  constexpr double kThinUnits = 16.0;
  constexpr double kSmoothingUnits = 128.0;
  constexpr double kInitialTolerance = 2.0;
  constexpr double kMaxTolerance = 1024.0;

  std::vector<std::array<float, 3>> path;
  if (ticks.empty() || max_points < 2) return path;

  // Hyperline polyline.ts thin(path, 16).
  path.reserve(ticks.size());
  path.push_back(ticks.front().pawn.position);
  for (std::size_t i = 1; i + 1 < ticks.size(); ++i) {
    if (RouteDistance(ticks[i].pawn.position, path.back()) >= kThinUnits) {
      path.push_back(ticks[i].pawn.position);
    }
  }
  if (ticks.size() > 1 &&
      (RouteDistance(ticks.back().pawn.position, path.back()) > 0.0 ||
       path.size() == 1)) {
    path.push_back(ticks.back().pawn.position);
  }
  if (path.size() < 2) return path;

  // Hyperline smoothing(path, [0,last], 128): Gaussian over traveled distance.
  std::vector<double> traveled(path.size(), 0.0);
  for (std::size_t i = 1; i < path.size(); ++i) {
    traveled[i] = traveled[i - 1] + RouteDistance(path[i - 1], path[i]);
  }
  const double reach = 3.0 * kSmoothingUnits;
  std::vector<std::array<float, 3>> smoothed = path;
  for (std::size_t i = 1; i + 1 < path.size(); ++i) {
    std::size_t from = i;
    while (from > 0 && traveled[i] - traveled[from - 1] <= reach) --from;
    std::size_t to = i;
    while (to + 1 < path.size() && traveled[to + 1] - traveled[i] <= reach) ++to;

    double weights = 0.0;
    std::array<double, 3> sum{};
    for (std::size_t j = from; j <= to; ++j) {
      const double offset = (traveled[j] - traveled[i]) / kSmoothingUnits;
      const double weight = std::exp(-(offset * offset) / 2.0);
      weights += weight;
      for (std::size_t axis = 0; axis < 3; ++axis) {
        sum[axis] += weight * path[j][axis];
      }
    }
    if (weights > 0.0) {
      for (std::size_t axis = 0; axis < 3; ++axis) {
        smoothed[i][axis] = static_cast<float>(sum[axis] / weights);
      }
    }
  }

  auto chord_distance = [](const std::array<float, 3>& p,
                           const std::array<float, 3>& a,
                           const std::array<float, 3>& b) {
    const double ux = p[0] - a[0], uy = p[1] - a[1], uz = p[2] - a[2];
    const double vx = b[0] - a[0], vy = b[1] - a[1], vz = b[2] - a[2];
    const double length = std::sqrt(vx * vx + vy * vy + vz * vz);
    if (length == 0.0) return std::sqrt(ux * ux + uy * uy + uz * uz);
    const double cx = uy * vz - uz * vy;
    const double cy = uz * vx - ux * vz;
    const double cz = ux * vy - uy * vx;
    return std::sqrt(cx * cx + cy * cy + cz * cz) / length;
  };

  // Hyperline ranking() then progressively raises tolerance until beam budget fits.
  std::vector<double> ranks(smoothed.size(), 0.0);
  ranks.front() = ranks.back() = std::numeric_limits<double>::infinity();
  struct Span { std::size_t from, to; double cap; };
  std::vector<Span> spans{{0, smoothed.size() - 1,
                           std::numeric_limits<double>::infinity()}};
  while (!spans.empty()) {
    const Span span = spans.back();
    spans.pop_back();
    double farthest = 0.0;
    std::size_t index = span.from;
    for (std::size_t middle = span.from + 1; middle < span.to; ++middle) {
      const double away = chord_distance(
          smoothed[middle], smoothed[span.from], smoothed[span.to]);
      if (away > farthest) {
        farthest = away;
        index = middle;
      }
    }
    if (farthest > kInitialTolerance && index > span.from && index < span.to) {
      const double rank = (std::min)(farthest, span.cap);
      ranks[index] = rank;
      spans.push_back({span.from, index, rank});
      spans.push_back({index, span.to, rank});
    }
  }

  for (double tolerance = kInitialTolerance;
       tolerance <= kMaxTolerance; tolerance *= 2.0) {
    std::vector<std::array<float, 3>> line;
    for (std::size_t i = 0; i < path.size(); ++i) {
      if (ranks[i] > tolerance) line.push_back(smoothed[i]);
    }
    if (line.size() >= 2 && line.size() - 1 <= max_points - 1) {
      return line;
    }
  }

  return {smoothed.front(), smoothed.back()};
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
  split::movement::MovementRecorder race_run_recorder;
  std::atomic<int> human_slot{-1};
  bool pawn_announced = false;
  bool steering_was_active = false;
  int pb_ghost_slot = -1;
  std::atomic<bool> replay_requested{false};
  std::atomic<bool> race_requested{false};
  std::atomic<bool> retry_requested{false};
  std::atomic<bool> visual_test_requested{false};
  std::atomic<int> visual_mode{2}; // both, ghost, tracer, off
  std::atomic<bool> show_rings{true};
  std::atomic<bool> show_checkpoints{true};
  std::atomic<bool> show_hud{true};
  std::string last_pawn_error;
  std::string current_map = launch.map;

  const auto route_directory =
      std::filesystem::current_path() / "routes";
  split::movement::RaceTimer race_timer(route_directory);
  split::movement::PbRouteStore pb_routes(route_directory);

  split::movement::NativeVisualRenderer visual_renderer;
  std::string visual_renderer_error;
  if (!visual_renderer.Install(visual_renderer_error)) {
    std::cout << "[SPLIT][Visuals] native renderer unavailable: "
              << visual_renderer_error << "\n";
  } else {
    std::cout << "[SPLIT][Visuals] native Source 2 particle core ready";
    if (visual_renderer.ManifestReady()) std::cout << " + manifest";
    if (visual_renderer.TransmitReady()) std::cout << " + TransmitAlways";
    if (visual_renderer.Ready()) std::cout << " + debug draw bridge";
    std::cout << "\n";
    if (!visual_renderer.OptionalError().empty()) {
      std::cout << "[SPLIT][Visuals] optional setup warning: "
                << visual_renderer.OptionalError() << "\n";
    }
  }

  auto handle_command = [&](std::string_view command) -> std::string {
    if (command == "visualstate") {
      const auto course = race_timer.Course();
      const auto race = race_timer.Status();
      const auto mode = visual_mode.load(std::memory_order_relaxed);
      const bool trail_enabled = mode == 0 || mode == 2;
      const bool rings_enabled = show_rings.load(std::memory_order_relaxed);
      const bool checkpoints_enabled = show_checkpoints.load(std::memory_order_relaxed);
      const bool hud_enabled = show_hud.load(std::memory_order_relaxed);
      std::ostringstream out;
      out << "{"
          << "\"schemaVersion\":1,"
          << "\"renderedInGame\":false,"
          << "\"visualMode\":\"" << (mode == 1 ? "ghost" : mode == 2 ? "tracer" :
                                       mode == 3 ? "off" : "both") << "\","
          << "\"tracer\":" << (trail_enabled ? JsonRoutePolyline(recorder.PlaybackTicks()) : "[]") << ","
          << "\"rings\":";
      if (course && rings_enabled) {
        out << "{\"start\":" << JsonVec3(course->start)
            << ",\"finish\":" << JsonVec3(course->finish)
            << ",\"radius\":" << course->radius << "}";
      } else {
        out << "null";
      }
      out << ",\"checkpoints\":[";
      if (course && checkpoints_enabled) {
        for (std::size_t i = 0; i < course->checkpoints.size(); ++i) {
          if (i) out << ",";
          out << JsonVec3(course->checkpoints[i]);
        }
      }
      out << "],\"hud\":";
      if (hud_enabled) {
        out << "{\"phase\":\"" << RacePhaseName(race.phase)
            << "\",\"elapsedSeconds\":" << race.elapsed_seconds
            << ",\"personalBestSeconds\":" << JsonNumberOrNull(race.personal_best_seconds)
            << ",\"nextCheckpoint\":" << race.next_checkpoint
            << ",\"currentSplits\":" << JsonNumberArray(race.current_splits)
            << ",\"personalBestSplits\":" << JsonNumberArray(race.personal_best_splits)
            << "}";
      } else {
        out << "null";
      }
      out << "}";
      return out.str();
    }
    // Report an available command bridge separately from actual client rendering.
    // Interface resolution alone cannot prove that debug shapes reach Deadlock.
    if (command == "visualtest") {
      if (!visual_renderer.ParticleReady()) {
        return "{\"queued\":false,\"reason\":\"particle-renderer-unavailable\"}";
      }
      if (human_slot.load(std::memory_order_relaxed) < 0) {
        return "{\"queued\":false,\"reason\":\"player-not-connected\"}";
      }
      visual_test_requested.store(true, std::memory_order_release);
      return "{\"queued\":true,\"note\":\"spawn-Modlock-style-native-particle-near-player\"}";
    }
    if (command == "tracertest") {
      const auto ticks = recorder.PlaybackTicks();
      if (ticks.size() < 2) {
        return "{\"queued\":false,\"reason\":\"no-recorded-route\"}";
      }

      const auto line = RoutePolyline(ticks, 128);
      if (line.size() < 2) {
        return "{\"queued\":false,\"reason\":\"route-has-no-line\"}";
      }

      visual_renderer.Clear();
      std::size_t created = 0;
      std::string beam_error;
      for (std::size_t i = 1; i < line.size(); ++i) {
        if (!visual_renderer.DrawBeamSegment(line[i - 1], line[i], beam_error)) {
          std::ostringstream out;
          out << "{\"queued\":false,\"reason\":\"hyperline-beam-failed\","
              << "\"segment\":" << (i - 1)
              << ",\"created\":" << created
              << ",\"detail\":\"" << beam_error << "\"}";
          return out.str();
        }
        ++created;
      }

      std::ostringstream out;
      out << "{\"queued\":true,\"note\":\"hyperline-route-line-created\","
          << "\"points\":" << line.size()
          << ",\"segments\":" << created << "}";
      return out.str();
    }
    if (command == "visualdiagnostics") {
      auto escape_json = [](std::string_view value) {
        std::string escaped;
        escaped.reserve(value.size() + 8);
        for (const char c : value) {
          switch (c) {
            case '\\': escaped += "\\\\"; break;
            case '"': escaped += "\\\""; break;
            case '\n': escaped += "\\n"; break;
            case '\r': escaped += "\\r"; break;
            case '\t': escaped += "\\t"; break;
            default: escaped += c; break;
          }
        }
        return escaped;
      };
      std::ostringstream out;
      out << "{"
          << "\"rendererInterfaceReady\":" << (visual_renderer.Ready() ? "true" : "false")
          << ",\"particleCoreReady\":" << (visual_renderer.ParticleReady() ? "true" : "false")
          << ",\"manifestReady\":" << (visual_renderer.ManifestReady() ? "true" : "false")
          << ",\"transmitAlwaysReady\":" << (visual_renderer.TransmitReady() ? "true" : "false")
          << ",\"optionalError\":\"" << escape_json(visual_renderer.OptionalError()) << "\""
          << ",\"clientRenderVerified\":false"
          << ",\"hudImplemented\":false"
          << "}";
      return out.str();
    }
    if (command == "state") {
      const auto stats = recorder.Stats();
      const auto race = race_timer.Status();
      const auto course = race_timer.Course();
      const auto first = recorder.FirstPawn();
      const int display_mode = visual_mode.load(std::memory_order_relaxed);
      const char* display_mode_name =
          display_mode == 1 ? "ghost" : display_mode == 2 ? "tracer" :
          display_mode == 3 ? "off" : "both";
      std::ostringstream out;
      out << "{"
          << "\"version\":\"" << split::movement::kRuntimeVersion << "\","
          << "\"playerConnected\":" << (human_slot.load(std::memory_order_relaxed) >= 0 ? "true" : "false") << ","
          << "\"humanSlot\":" << human_slot.load(std::memory_order_relaxed) << ","
          << "\"recording\":" << (stats.recording ? "true" : "false") << ","
          << "\"ticks\":" << stats.ticks << ","
          << "\"hasRoute\":" << (stats.ticks > 0 ? "true" : "false") << ","
          << "\"heroId\":" << (first ? first->hero_id : 0) << ","
          << "\"racePhase\":\"" << RacePhaseName(race.phase) << "\","
          << "\"elapsedSeconds\":" << race.elapsed_seconds << ","
          << "\"lastSeconds\":" << JsonNumberOrNull(race.last_seconds) << ","
          << "\"personalBestSeconds\":" << JsonNumberOrNull(race.personal_best_seconds) << ","
          << "\"lastWasPb\":" << (race.last_was_pb ? "true" : "false") << ","
          << "\"lastDeltaSeconds\":" << JsonNumberOrNull(race.last_delta_seconds) << ","
          << "\"lastSplitDeltas\":" << JsonNumberArray(race.last_split_deltas) << ","
          << "\"currentSplits\":" << JsonNumberArray(race.current_splits) << ","
          << "\"personalBestSplits\":" << JsonNumberArray(race.personal_best_splits) << ","
          << "\"nextCheckpoint\":" << race.next_checkpoint << ","
          << "\"checkpointCount\":" << (course ? course->checkpoints.size() : 0) << ","
          << "\"visualMode\":\"" << display_mode_name << "\","
          << "\"showRings\":" << (show_rings.load() ? "true" : "false") << ","
          << "\"showCheckpoints\":" << (show_checkpoints.load() ? "true" : "false") << ","
          << "\"showHud\":" << (show_hud.load() ? "true" : "false") << ","
          << "\"inGameVisualsReady\":false"
          << "}";
      return out.str();
    }
    if (command == "visual both" || command == "visual ghost" ||
        command == "visual tracer" || command == "visual off") {
      visual_mode.store(command == "visual ghost" ? 1 :
                        command == "visual tracer" ? 2 :
                        command == "visual off" ? 3 : 0);
      std::cout << "[SPLIT][Display] visual=" << command.substr(7) << "\n";
    } else if (command == "rings on" || command == "rings off") {
      show_rings.store(command == "rings on");
    } else if (command == "checkpoints on" || command == "checkpoints off") {
      show_checkpoints.store(command == "checkpoints on");
    } else if (command == "hud on" || command == "hud off") {
      show_hud.store(command == "hud on");
    } else if (command == "record") {
      recorder.Start();
      std::cout << "[SPLIT][Recorder] recording started\n";
    } else if (command == "recordtoggle") {
      if (recorder.Stats().recording) {
        PrintStats(recorder.Stop());
      } else {
        recorder.Start();
        std::cout << "[SPLIT][Recorder] recording started\n";
      }
    } else if (command == "stop") {
      PrintStats(recorder.Stop());
    } else if (command == "status") {
      PrintStats(recorder.Stats());
    } else if (command == "clear") {
      recorder.Clear();
      visual_renderer.Clear();
      std::cout << "[SPLIT][Recorder] route cleared\n";
    } else if (command == "replay" || command == "watch") {
      replay_requested.store(true, std::memory_order_release);
      std::cout << (command == "watch" ? "[SPLIT][Watch] watch requested\n"
                                      : "[SPLIT][Replay] replay requested\n");
    } else if (command == "race") {
      race_requested.store(true, std::memory_order_release);
      std::cout << "[SPLIT][Race] arming route timer\n";
    } else if (command == "retry") {
      retry_requested.store(true, std::memory_order_release);
      std::cout << "[SPLIT][Race] retry requested\n";
    } else if (command == "racecancel") {
      race_timer.Cancel();
      race_run_recorder.Clear();
      visual_renderer.Clear();
      std::cout << "[SPLIT][Race] cancelled\n";
    } else if (command == "racestatus") {
      const auto race = race_timer.Status();
      const auto course = race_timer.Course();
      const char* phase = "idle";
      if (race.phase == split::movement::RaceStatus::Phase::kArmed) phase = "armed";
      if (race.phase == split::movement::RaceStatus::Phase::kRunning) phase = "running";
      if (race.phase == split::movement::RaceStatus::Phase::kFinished) phase = "finished";
      std::cout << std::fixed << std::setprecision(3)
                << "[SPLIT][Race] " << phase
                << " elapsed=" << race.elapsed_seconds << "s";
      if (race.personal_best_seconds) {
        std::cout << " pb=" << *race.personal_best_seconds << "s";
      }
      if (race.last_seconds) {
        std::cout << " last=" << *race.last_seconds << "s";
      }
      std::cout << " split=" << race.next_checkpoint
                << "/" << (course ? course->checkpoints.size() : 0);
      std::cout << "\n";
    } else if (command == "save") {
      auto saved = recorder.SaveJson(route_directory, current_map);
      if (saved) {
        std::cout << "[SPLIT][Recorder] saved " << saved->string() << "\n";
      } else {
        std::cout << "[SPLIT][Recorder] save failed: " << saved.error() << "\n";
      }
    } else if (!command.empty()) {
      std::cout << "[SPLIT][Recorder] unknown command: " << command
                << " (record, recordtoggle, stop, status, state, save, replay, watch, race, retry, racestatus, racecancel, visualtest, tracertest, visualdiagnostics, clear)\n";
    }
    return {};
  };

  // Keep stdin as a debug fallback, but normal Movement Lab control now also
  // arrives from SPLIT over a localhost-only UDP socket.
  std::jthread command_thread([&](std::stop_token stop) {
    std::string command;
    while (!stop.stop_requested() && std::getline(std::cin, command)) {
      const std::string response = handle_command(command);
      if (!response.empty()) {
        std::cout << response << "\n";
      }
    }
  });

  split::movement::ControlServer control_server;
  std::string control_error;
  if (!control_server.Start(27168, handle_command, control_error)) {
    command_thread.request_stop();
    std::cerr << "Movement control server failed: " << control_error << "\n";
    return 1;
  }
  std::cout << "SPLIT Movement Runtime: local control listening on 127.0.0.1:27168.\n";

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

            if (visual_test_requested.exchange(false, std::memory_order_acq_rel)) {
              std::string visual_test_error;
              if (visual_renderer.DrawTestParticle(sample->position, visual_test_error)) {
                std::cout << "[SPLIT][Visuals] Modlock-style info_particle_system spawned; check the game client\n";
              } else {
                std::cout << "[SPLIT][Visuals] native particle test failed: "
                          << visual_test_error << "\n";
              }
            }
            last_pawn_error.clear();
            if (!pawn_announced) {
              pawn_announced = true;
              std::cout << "[SPLIT][Movement] pawn attached slot=" << sample->slot
                        << " handle=0x" << std::hex << sample->pawn_handle << std::dec << "\n";
            }

            const auto latest_input = input_capture.Latest(slot);
            recorder.Push(frame, *sample, latest_input);
            race_run_recorder.Push(frame, *sample, latest_input);

            if (frame % 60 == 0 && !race_timer.Course() &&
                (visual_renderer.ParticleReady() || visual_renderer.Ready())) {
              const int mode = visual_mode.load(std::memory_order_relaxed);
              split::movement::NativeVisualSettings visual_settings;
              visual_settings.tracer = mode == 0 || mode == 2;
              visual_settings.rings =
                  show_rings.load(std::memory_order_relaxed);
              visual_settings.checkpoints =
                  show_checkpoints.load(std::memory_order_relaxed);

              auto visual_route = recorder.PlaybackTicks();
              auto visual_course = visual_route.empty()
                  ? std::optional<split::movement::RaceCourse>{}
                  : race_timer.Course();
              visual_renderer.Render(visual_route, visual_course,
                                     visual_settings);
            }

            if (retry_requested.exchange(false, std::memory_order_acq_rel)) {
              auto ticks = recorder.PlaybackTicks();
              if (ticks.empty()) {
                std::cout << "[SPLIT][Race] retry failed: no recorded route is available\n";
              } else {
                steering.Stop();
                race_timer.Cancel();
                race_run_recorder.Clear();

                std::array<float, 3> angles{};
                if (ticks.front().input) {
                  angles = ticks.front().input->angles;
                }

                std::string teleport_error;
                if (!pawn_probe.Teleport(slot, ticks.front().pawn.position,
                                         angles, teleport_error)) {
                  std::cout << "[SPLIT][Race] retry teleport failed: "
                            << teleport_error << "\n";
                } else {
                  std::cout << "[SPLIT][Race] RETRY teleported to route start\n";
                  race_requested.store(true, std::memory_order_release);
                }
              }
            }

            if (race_requested.exchange(false, std::memory_order_acq_rel)) {
              const auto first = recorder.FirstPawn();
              const auto last = recorder.LastPawn();
              const auto route_ticks = recorder.PlaybackTicks();
              if (!first || !last || route_ticks.empty()) {
                std::cout << "[SPLIT][Race] no recorded route is available\n";
              } else {
                split::movement::RaceCourse course;
                course.start = first->position;
                course.finish = last->position;
                course.checkpoints = RouteCheckpoints(route_ticks, 3);
                course.radius = 120.0f;
                course.map = current_map;
                course.name = "Recorded Route";
                course.difficulty = "Unrated";
                course.hero_id = first->hero_id;

                std::string race_error;
                if (!race_timer.Configure(course, race_error)) {
                  std::cout << "[SPLIT][Race] configure failed: "
                            << race_error << "\n";
                } else if (!race_timer.Arm(race_error)) {
                  std::cout << "[SPLIT][Race] arm failed: "
                            << race_error << "\n";
                } else {
                  // Modlock-style ownership: create persistent world
                  // entities once when the course is armed. They live until
                  // racecancel/clear/world reset; the legacy Render() loop is
                  // deliberately not involved.
                  visual_renderer.Clear();

                  std::string marker_error;
                  if (show_rings.load(std::memory_order_relaxed)) {
                    auto route_title_position = course.start;
                    route_title_position[2] += 110.0f;
                    if (!visual_renderer.DrawWorldTextMarker(
                            route_title_position, course.name.c_str(),
                            5.5f, 0xffff5adcu, marker_error)) {
                      std::cout << "[SPLIT][WorldEffects] route title failed: "
                                << marker_error << "\n";
                    }

                    auto route_meta_position = course.start;
                    route_meta_position[2] += 78.0f;
                    const std::string route_meta =
                        course.difficulty + "  ·  Hero " +
                        std::to_string(course.hero_id);
                    marker_error.clear();
                    if (!visual_renderer.DrawWorldTextMarker(
                            route_meta_position, route_meta.c_str(),
                            2.7f, 0xffffffffu, marker_error)) {
                      std::cout << "[SPLIT][WorldEffects] route metadata failed: "
                                << marker_error << "\n";
                    }

                    marker_error.clear();
                    if (visual_renderer.DrawWorldTextMarker(
                            course.start, "START", 3.4f, 0xff78dc40u,
                            marker_error)) {
                      std::cout << "[SPLIT][WorldEffects] START world text created\n";
                    } else {
                      std::cout << "[SPLIT][WorldEffects] START failed: "
                                << marker_error << "\n";
                    }

                    marker_error.clear();
                    if (visual_renderer.DrawWorldTextMarker(
                            course.finish, "FINISH", 3.4f, 0xff5078ffu,
                            marker_error)) {
                      std::cout << "[SPLIT][WorldEffects] FINISH world text created\n";
                    } else {
                      std::cout << "[SPLIT][WorldEffects] FINISH failed: "
                                << marker_error << "\n";
                    }
                  }

                  if (show_checkpoints.load(std::memory_order_relaxed)) {
                    for (std::size_t checkpoint_index = 0;
                         checkpoint_index < course.checkpoints.size();
                         ++checkpoint_index) {
                      const std::string label =
                          "CP " + std::to_string(checkpoint_index + 1);
                      marker_error.clear();
                      if (!visual_renderer.DrawWorldTextMarker(
                              course.checkpoints[checkpoint_index],
                              label.c_str(), 3.0f, 0xffffaa5au,
                              marker_error)) {
                        std::cout << "[SPLIT][WorldEffects] " << label
                                  << " failed: " << marker_error << "\n";
                      }
                    }
                  }

                  std::cout << "[SPLIT][WorldEffects] persistent race objects created"
                            << " startFinish="
                            << (show_rings.load(std::memory_order_relaxed) ? "on" : "off")
                            << " checkpoints="
                            << (show_checkpoints.load(std::memory_order_relaxed) ? "on" : "off")
                            << "\n";

                  const auto status = race_timer.Status();
                  std::cout << std::fixed << std::setprecision(1)
                            << "[SPLIT][Race] armed"
                            << " start=(" << course.start[0] << ", "
                                         << course.start[1] << ", "
                                         << course.start[2] << ")"
                            << " finish=(" << course.finish[0] << ", "
                                          << course.finish[1] << ", "
                                          << course.finish[2] << ")"
                            << " radius=" << course.radius
                            << " checkpoints=" << course.checkpoints.size();
                  if (status.personal_best_seconds) {
                    std::cout << std::setprecision(3)
                              << " pb=" << *status.personal_best_seconds << "s";
                  }
                  std::cout << "\n";
                }
              }
            }

            const auto race_event = race_timer.Observe(sample->position);
            if (race_event.type == split::movement::RaceEvent::Type::kStarted) {
              race_run_recorder.Start();
              steering.Stop();
              std::cout << "[SPLIT][Race] GO checkpoints="
                        << race_event.checkpoint_count
                        << " mode=tracer-first"
                        << " (use 'watch' for ghost replay)\n";
            } else if (race_event.type == split::movement::RaceEvent::Type::kSplit) {
              std::cout << std::fixed << std::setprecision(3)
                        << "[SPLIT][Race] SPLIT "
                        << (race_event.checkpoint_index + 1)
                        << "/" << race_event.checkpoint_count
                        << " " << race_event.elapsed_seconds << "s";
              if (race_event.split_delta_seconds) {
                const double delta = *race_event.split_delta_seconds;
                std::cout << (delta <= 0.0 ? " (" : " (+");
                if (delta <= 0.0) {
                  std::cout << delta;
                } else {
                  std::cout << delta;
                }
                std::cout << "s vs PB split)";
              }
              std::cout << "\n";
            } else if (race_event.type == split::movement::RaceEvent::Type::kFinished) {
              race_run_recorder.Stop();
              const auto finished_status = race_timer.Status();
              if (race_event.personal_best) {
                const auto pb_ticks = race_run_recorder.PlaybackTicks();
                auto saved_pb = pb_routes.Save(finished_status.course_id, pb_ticks);
                if (saved_pb) {
                  std::cout << "[SPLIT][PB Ghost] saved "
                            << pb_ticks.size() << " steps for course "
                            << finished_status.course_id << "\n";
                } else {
                  std::cout << "[SPLIT][PB Ghost] save failed: "
                            << saved_pb.error() << "\n";
                }
              }

              std::cout << std::fixed << std::setprecision(3)
                        << "[SPLIT][Race] FINISH "
                        << race_event.elapsed_seconds << "s";
              if (race_event.personal_best) {
                if (race_event.previous_pb_seconds) {
                  std::cout << " PB (-"
                            << (*race_event.previous_pb_seconds -
                                race_event.elapsed_seconds)
                            << "s)";
                } else {
                  std::cout << " PB";
                }
              } else if (race_event.previous_pb_seconds) {
                std::cout << " (+" << (race_event.elapsed_seconds -
                                        *race_event.previous_pb_seconds)
                          << "s vs PB)";
              }
              std::cout << "\n";
            }

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
            visual_renderer.Clear();
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
            if (pb_ghost_slot == slot) {
              pb_ghost_slot = -1;
            }
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
