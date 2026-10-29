#include "split_movement/bot_factory.h"
#include "split_movement/control_server.h"
#include "split_movement/engine_bridge.h"
#include "split_movement/engine_runner.h"
#include "split_movement/native_hooks.h"
#include "split_movement/input_capture.h"
#include "split_movement/movement_steering.h"
#include "split_movement/movement_events.h"
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
#include <functional>
#include <iomanip>
#include <iostream>
#include <limits>
#include <mutex>
#include <numbers>
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


struct MovementCue {
  std::string label;
  std::array<float, 3> position{};
  std::uint64_t frame = 0;
};

std::vector<MovementCue> ConsolidateMovementCues(
    const std::vector<MovementCue>& cues) {
  std::vector<bool> hidden(cues.size(), false);

  const auto absorb = [&](std::size_t compound,
                          std::string_view primitive,
                          std::uint64_t before,
                          std::uint64_t after) {
    const auto center = cues[compound].frame;
    for (std::size_t i = 0; i < cues.size(); ++i) {
      if (i == compound || hidden[i] || cues[i].label != primitive) continue;
      const auto at = cues[i].frame;
      if ((at <= center && center - at <= before) ||
          (at > center && at - center <= after)) {
        hidden[i] = true;
      }
    }
  };

  for (std::size_t i = 0; i < cues.size(); ++i) {
    if (cues[i].label == "Slide hop") {
      // A slide hop already communicates the slide primitive.
      absorb(i, "Slide", 16, 4);
    } else if (cues[i].label == "Mantle slide") {
      // The composed technique owns both primitives around the transition.
      absorb(i, "Mantle", 96, 4);
      absorb(i, "Slide", 16, 4);
    } else if (cues[i].label == "Mantle wall jump") {
      // Prefer the strongest technique name over its component labels.
      absorb(i, "Mantle", 96, 8);
      absorb(i, "Wall jump", 12, 12);
    } else if (cues[i].label == "Corner boost" ||
               cues[i].label == "Edge boost") {
      absorb(i, "Wall jump", 12, 12);
    } else if (cues[i].label == "Mantle strafe") {
      absorb(i, "Mantle", 112, 16);
      absorb(i, "Mantle slide", 16, 16);
      absorb(i, "Slide", 16, 8);
    } else if (cues[i].label == "Dash cancel") {
      absorb(i, "Slide", 16, 8);
      absorb(i, "Ground dash", 24, 4);
    } else if (cues[i].label == "Superglide") {
      // Superglide already contains the mantle-exit slide/takeoff sequence.
      // Prefer it over the generic Slide hop and broader mantle/slide labels
      // so the same movement never renders stacked world text.
      absorb(i, "Slide hop", 16, 12);
      absorb(i, "Mantle slide", 16, 8);
      absorb(i, "Mantle", 64, 8);
      absorb(i, "Slide", 16, 8);
    } else if (cues[i].label == "Short hop") {
      // Short hop is the more specific composed technique: a valid dash
      // cancel followed by the quick takeoff. Do not also show its generic
      // Slide hop / Dash cancel / primitive labels on the same sequence.
      absorb(i, "Slide hop", 16, 8);
      absorb(i, "Dash cancel", 24, 8);
      absorb(i, "Slide", 16, 8);
      absorb(i, "Ground dash", 24, 4);
    } else if (cues[i].label == "Heavy melee cancel") {
      absorb(i, "Heavy melee", 96, 8);
      absorb(i, "Light melee", 96, 8);
    } else if (cues[i].label == "Instant air dash" ||
               cues[i].label == "Zipline dash") {
      absorb(i, "Air dash", 4, 4);
    }
  }

  std::vector<MovementCue> out;
  out.reserve(cues.size());
  for (std::size_t i = 0; i < cues.size(); ++i) {
    if (!hidden[i]) out.push_back(cues[i]);
  }
  return out;
}


struct TechniquePrimitive {
  std::string kind;
  std::uint64_t frame = 0;
  std::array<float, 3> position{};
  std::array<float, 3> velocity{};
};

double HorizontalSpeed(const std::array<float, 3>& velocity) {
  return std::hypot(static_cast<double>(velocity[0]),
                    static_cast<double>(velocity[1]));
}

std::optional<double> HeadingTurnDegrees(
    const std::array<float, 3>& from,
    const std::array<float, 3>& to) {
  const double from_speed = HorizontalSpeed(from);
  const double to_speed = HorizontalSpeed(to);
  if (!(from_speed > 0.0) || !(to_speed > 0.0)) return std::nullopt;
  const double cosine = std::clamp(
      (static_cast<double>(from[0]) * to[0] +
       static_cast<double>(from[1]) * to[1]) /
          (from_speed * to_speed),
      -1.0, 1.0);
  return std::acos(cosine) * 180.0 / std::numbers::pi;
}

const split::movement::RecordedTick* RecordedAt(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::uint64_t frame) {
  const auto it = std::lower_bound(
      ticks.begin(), ticks.end(), frame,
      [](const split::movement::RecordedTick& tick, std::uint64_t wanted) {
        return tick.frame < wanted;
      });
  return it != ticks.end() && it->frame == frame ? &*it : nullptr;
}


std::optional<std::array<float, 3>> UsableHorizontalVelocityNear(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::uint64_t frame, std::uint64_t radius = 4) {
  const auto usable = [](const std::array<float, 3>& velocity) {
    return HorizontalSpeed(velocity) > 1.0;
  };

  if (const auto* exact = RecordedAt(ticks, frame);
      exact && usable(exact->pawn.velocity)) {
    return exact->pawn.velocity;
  }

  for (std::uint64_t delta = 1; delta <= radius; ++delta) {
    if (frame >= delta) {
      if (const auto* before = RecordedAt(ticks, frame - delta);
          before && usable(before->pawn.velocity)) {
        return before->pawn.velocity;
      }
    }
    if (const auto* after = RecordedAt(ticks, frame + delta);
        after && usable(after->pawn.velocity)) {
      return after->pawn.velocity;
    }
  }
  return std::nullopt;
}

std::optional<double> PeakRecorded(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::uint64_t from, std::uint64_t to,
    const std::function<double(const split::movement::PawnSample&)>& value) {
  if (from > to) return std::nullopt;
  double peak = -std::numeric_limits<double>::infinity();
  for (std::uint64_t frame = from; frame <= to; ++frame) {
    const auto* tick = RecordedAt(ticks, frame);
    if (!tick) return std::nullopt;
    peak = std::max(peak, value(tick->pawn));
  }
  return peak;
}

std::vector<TechniquePrimitive> BuildTechniquePrimitives(
    const std::vector<split::movement::RecordedTick>& ticks,
    const std::vector<MovementCue>& raw_cues) {
  std::vector<TechniquePrimitive> out;
  out.reserve(raw_cues.size() + ticks.size() / 8);

  const auto push = [&](std::string kind, const split::movement::RecordedTick& tick) {
    if (!out.empty() && out.back().kind == kind &&
        out.back().frame == tick.frame) {
      return;
    }
    out.push_back({std::move(kind), tick.frame, tick.pawn.position,
                   tick.pawn.velocity});
  };

  for (const auto& cue : raw_cues) {
    std::string kind;
    if (cue.label == "Ground dash") kind = "groundDash";
    else if (cue.label == "Slide") kind = "slideStart";
    else if (cue.label == "Wall jump") kind = "wallJump";
    else if (cue.label == "Dash jump") kind = "dashJump";
    else if (cue.label == "Air jump") kind = "airJump";
    else if (cue.label == "Air dash") kind = "airDash";
    else if (cue.label == "Bounce pad") kind = "bouncePad";
    else if (cue.label == "Melee started") kind = "meleeStarted";
    else if (cue.label == "Ability executed") kind = "abilityExecuted";
    else continue;

    if (const auto* tick = RecordedAt(ticks, cue.frame)) {
      auto velocity = tick->pawn.velocity;
      if (kind == "groundDash" && HorizontalSpeed(velocity) <= 1.0) {
        if (const auto nearby = UsableHorizontalVelocityNear(ticks, cue.frame, 4)) {
          velocity = *nearby;
        }
      }
      out.push_back({kind, cue.frame, cue.position, velocity});
    }
  }

  constexpr std::uint64_t kCrouchButton = std::uint64_t{1} << 2;
  constexpr std::uint64_t kCrouchToggleButton = std::uint64_t{1} << 59;

  for (std::size_t i = 1; i < ticks.size(); ++i) {
    const auto& before = ticks[i - 1];
    const auto& now = ticks[i];

    const bool fresh_input =
        now.input &&
        (!before.input || before.input->sequence != now.input->sequence);
    if (fresh_input &&
        (now.input->pressed & (kCrouchButton | kCrouchToggleButton)) != 0) {
      push("crouchPress", now);
    }

    if (before.pawn.grounded && !now.pawn.grounded) push("takeoff", now);
    if (!before.pawn.grounded && now.pawn.grounded) push("touchdown", now);

    bool mantle_restarted = false;
    if (now.pawn.mantle_start_time) {
      mantle_restarted =
          !before.pawn.mantle_start_time ||
          std::fabs(*now.pawn.mantle_start_time -
                    *before.pawn.mantle_start_time) > 0.0001f;
    }
    if (mantle_restarted ||
        (!before.pawn.mantling && now.pawn.mantling)) {
      push("mantleStart", now);
    }
    if (before.pawn.mantling && !now.pawn.mantling) {
      push("mantleEnd", now);
    }

    if (!before.pawn.sliding && now.pawn.sliding) push("slideStart", now);
    if (before.pawn.sliding && !now.pawn.sliding) push("slideEnd", now);

    const bool before_riding =
        before.pawn.zipline_time && now.pawn.zipline_time &&
        *now.pawn.zipline_time > *before.pawn.zipline_time;
    if (i >= 2) {
      const auto& older = ticks[i - 2];
      const bool older_riding =
          older.pawn.zipline_time && before.pawn.zipline_time &&
          *before.pawn.zipline_time > *older.pawn.zipline_time;
      if (older_riding && !before_riding && now.pawn.zipline_time) {
        push("ziplineLeft", now);
      }
    }
  }

  std::sort(out.begin(), out.end(),
            [](const TechniquePrimitive& a, const TechniquePrimitive& b) {
              if (a.frame != b.frame) return a.frame < b.frame;
              return a.kind < b.kind;
            });
  out.erase(std::unique(out.begin(), out.end(),
                        [](const TechniquePrimitive& a,
                           const TechniquePrimitive& b) {
                          return a.frame == b.frame && a.kind == b.kind;
                        }),
            out.end());

  // A wheel-bound crouch/slide is delivered through the command scroll
  // mask. On this Deadlock build it can cancel a dash without surfacing the
  // native Slide event/state that keyboard crouch produces. When a real
  // crouch press occurs during the same 24-tick Ground Dash window, treat that
  // press as the slideStart anchor. The normal dash-cancel speed/takeoff rules
  // still decide whether it qualifies as a technique.
  {
    std::vector<TechniquePrimitive> inferred;
    for (const auto& primitive : out) {
      if (primitive.kind != "crouchPress") continue;
      const auto from = primitive.frame > 24 ? primitive.frame - 24 : 0;

      const TechniquePrimitive* dash = nullptr;
      bool native_slide_nearby = false;
      bool slide_already_started_for_dash = false;
      for (auto it = out.rbegin(); it != out.rend(); ++it) {
        if (it->frame > primitive.frame) continue;
        if (it->frame < from) break;

        if (it->kind == "groundDash") {
          if (!dash) dash = &*it;
          break;
        }

        if (it->kind == "slideStart") {
          slide_already_started_for_dash = true;
          if (primitive.frame >= it->frame &&
              primitive.frame - it->frame <= 2) {
            native_slide_nearby = true;
          }
        }

        if (it->kind == "slideEnd" || it->kind == "touchdown") {
          slide_already_started_for_dash = false;
        }
      }

      const bool inferred_for_same_dash = std::any_of(
          inferred.begin(), inferred.end(),
          [&](const TechniquePrimitive& candidate) {
            return candidate.kind == "slideStart" &&
                   candidate.frame >= from &&
                   candidate.frame <= primitive.frame;
          });

      if (dash && !native_slide_nearby &&
          !slide_already_started_for_dash &&
          !inferred_for_same_dash) {
        inferred.push_back({"slideStart", primitive.frame,
                            primitive.position, primitive.velocity});
      }
    }
    out.insert(out.end(), inferred.begin(), inferred.end());
    std::sort(out.begin(), out.end(),
              [](const TechniquePrimitive& a, const TechniquePrimitive& b) {
                if (a.frame != b.frame) return a.frame < b.frame;
                return a.kind < b.kind;
              });
    out.erase(std::unique(out.begin(), out.end(),
                          [](const TechniquePrimitive& a,
                             const TechniquePrimitive& b) {
                            return a.frame == b.frame && a.kind == b.kind;
                          }),
              out.end());
  }

  // The native Slide event/state can surface 1-2 server frames after the
  // actual crouch press that starts the dash cancel. Hyperline evaluates the
  // slide-start sample itself, so preserve its 500 u/s threshold by anchoring
  // a confirmed native slide back to the immediately preceding crouch press.
  for (auto& primitive : out) {
    if (primitive.kind != "slideStart") continue;

    const auto from = primitive.frame > 2 ? primitive.frame - 2 : 0;
    const TechniquePrimitive* crouch = nullptr;
    for (auto it = out.rbegin(); it != out.rend(); ++it) {
      if (it->kind != "crouchPress" || it->frame < from ||
          it->frame >= primitive.frame) {
        continue;
      }
      crouch = &*it;
      break;
    }
    if (!crouch) continue;

    primitive.frame = crouch->frame;
    primitive.position = crouch->position;
    primitive.velocity = crouch->velocity;
  }

  std::sort(out.begin(), out.end(),
            [](const TechniquePrimitive& a, const TechniquePrimitive& b) {
              if (a.frame != b.frame) return a.frame < b.frame;
              return a.kind < b.kind;
            });
  out.erase(std::unique(out.begin(), out.end(),
                        [](const TechniquePrimitive& a,
                           const TechniquePrimitive& b) {
                          return a.frame == b.frame && a.kind == b.kind;
                        }),
            out.end());
  return out;
}

const TechniquePrimitive* LastPrimitive(
    const std::vector<TechniquePrimitive>& recent, std::string_view kind,
    std::uint64_t from, std::uint64_t to,
    const TechniquePrimitive* skip = nullptr) {
  for (auto it = recent.rbegin(); it != recent.rend(); ++it) {
    if (&*it == skip || it->kind != kind || it->frame < from ||
        it->frame > to) {
      continue;
    }
    return &*it;
  }
  return nullptr;
}

bool AnyPrimitive(const std::vector<TechniquePrimitive>& recent,
                  std::string_view kind, std::uint64_t from,
                  std::uint64_t to) {
  return LastPrimitive(recent, kind, from, to) != nullptr;
}

std::optional<bool> WallContactFollowed(
    const std::vector<split::movement::RecordedTick>& ticks,
    std::uint64_t frame) {
  if (frame == 0) return std::nullopt;
  const auto* now = RecordedAt(ticks, frame);
  const auto* before = RecordedAt(ticks, frame - 1);
  if (!now || !before || !now->pawn.wall_contact_position ||
      !before->pawn.wall_contact_position) {
    return std::nullopt;
  }
  return *now->pawn.wall_contact_position !=
         *before->pawn.wall_contact_position;
}

std::vector<MovementCue> RecognizeRecordedTechniques(
    const std::vector<split::movement::RecordedTick>& ticks,
    const std::vector<MovementCue>& raw_cues,
    bool diagnostics) {
  constexpr std::uint64_t kWallJumpRiseTicks = 6;
  constexpr std::uint64_t kWallJumpSettleTicks = 8;
  constexpr std::uint64_t kMantleStrafeTicks = 12;
  constexpr std::uint64_t kMantleExitTicks = 16;
  constexpr std::uint64_t kMantleWallJumpTicks = 96;
  constexpr std::uint64_t kDashCancelTicks = 24;
  constexpr std::uint64_t kSuperglideSettleTicks = 2;
  constexpr std::uint64_t kZiplineDashTicks = 90;
  constexpr std::uint64_t kMeleeSettleTicks = 96;
  constexpr std::uint64_t kCancelSettleTicks = 8;
  constexpr double kCornerBoostMinVerticalSpeed = 600.0;
  constexpr double kEdgeBoostMinEntrySpeed = 150.0;
  constexpr std::uint64_t kEdgeBoostMaxDetachedTicks = 16;
  constexpr double kDashCancelMinSpeed = 500.0;
  constexpr double kSuperglideMinSpeed = 500.0;
  constexpr double kMantleWallJumpMinSpeed = 500.0;
  constexpr double kMantleStrafeMinTurnDegrees = 45.0;
  constexpr double kMantleStrafeMinRetention = 0.8;
  constexpr double kHeavyMeleeMinSpeedGain = 300.0;
  constexpr double kHeavyMeleeCancelMinSpeed = 500.0;
  constexpr double kHeavyMeleeCancelMinRetention = 0.8;

  const auto primitives = BuildTechniquePrimitives(ticks, raw_cues);
  if (diagnostics) {
    for (const auto& primitive : primitives) {
      if (primitive.kind == "groundDash" ||
          primitive.kind == "crouchPress" ||
          primitive.kind == "mantleStart" ||
          primitive.kind == "mantleEnd" ||
          primitive.kind == "slideStart" ||
          primitive.kind == "slideEnd" ||
          primitive.kind == "takeoff" ||
          primitive.kind == "touchdown" ||
          primitive.kind == "meleeStarted" ||
          primitive.kind == "abilityExecuted") {
        std::cout << "[SPLIT][TechDiag][Primitive] frame="
                  << primitive.frame
                  << " kind=" << primitive.kind;
        if (primitive.kind == "slideStart") {
          const auto* same_crouch =
              LastPrimitive(primitives, "crouchPress",
                            primitive.frame, primitive.frame);
          std::cout << " source="
                    << (same_crouch ? "crouchPress" : "native");
        }
        std::cout << " speed=" << std::fixed << std::setprecision(1)
                  << HorizontalSpeed(primitive.velocity) << "\n";
      }
    }
  }
  std::vector<MovementCue> recognized;

  const auto add = [&](std::string label, const TechniquePrimitive& anchor) {
    recognized.push_back(
        {std::move(label), anchor.position, anchor.frame});
  };
  const auto diag = [&](std::string_view technique,
                        const TechniquePrimitive& anchor,
                        std::string_view detail) {
    if (!diagnostics) return;
    std::cout << "[SPLIT][TechDiag][" << technique << "] frame="
              << anchor.frame << " " << detail << "\n";
  };

  const auto dash_cancelled = [&](const TechniquePrimitive& slide) {
    const auto from =
        slide.frame > kDashCancelTicks ? slide.frame - kDashCancelTicks : 0;
    const auto* dash =
        LastPrimitive(primitives, "groundDash", from, slide.frame);
    const auto* nearest_dash =
        LastPrimitive(primitives, "groundDash", 0, slide.frame);
    const bool airborne =
        dash && AnyPrimitive(primitives, "takeoff", dash->frame, slide.frame);
    const double slide_speed = HorizontalSpeed(slide.velocity);

    std::optional<double> dash_speed;
    std::optional<double> peak_speed;
    std::optional<std::uint64_t> dash_delta;
    if (dash) {
      dash_speed = HorizontalSpeed(dash->velocity);
      dash_delta = slide.frame - dash->frame;
      peak_speed = PeakRecorded(
          ticks, dash->frame, slide.frame,
          [](const split::movement::PawnSample& pawn) {
            return HorizontalSpeed(pawn.velocity);
          });
    }

    const bool ok =
        dash && !airborne && slide_speed >= kDashCancelMinSpeed;

    if (diagnostics) {
      std::ostringstream detail;
      detail << "accepted=" << (ok ? "yes" : "no")
             << " groundDash=" << (dash ? "yes" : "no")
             << " effectiveSlideFrame=" << slide.frame;
      if (!dash && nearest_dash) {
        detail << " nearestGroundDashDelta="
               << (slide.frame - nearest_dash->frame);
      }
      if (dash_delta) detail << " dashToSlide=" << *dash_delta << "/24";

      const auto* crouch_press =
          LastPrimitive(primitives, "crouchPress",
                        dash ? dash->frame : from, slide.frame);
      detail << " crouchPress=" << (crouch_press ? "yes" : "no");
      if (crouch_press) {
        detail << " dashToCrouch="
               << (dash ? crouch_press->frame - dash->frame : 0)
               << " crouchToSlide="
               << (slide.frame - crouch_press->frame)
               << " crouchSpeed=" << std::fixed << std::setprecision(1)
               << HorizontalSpeed(crouch_press->velocity);
      }

      detail << " takeoffBetween=" << (airborne ? "yes" : "no")
             << " dashSpeed=";
      if (dash_speed) detail << std::fixed << std::setprecision(1) << *dash_speed;
      else detail << "n/a";
      detail << " peakDashToSlide=";
      if (peak_speed) detail << std::fixed << std::setprecision(1) << *peak_speed;
      else detail << "n/a";
      detail << " slideSpeed=" << std::fixed << std::setprecision(1)
             << slide_speed << "/" << kDashCancelMinSpeed;
      diag("Dash cancel", slide, detail.str());
    }
    return ok;
  };

  for (const auto& anchor : primitives) {
    if (anchor.kind == "wallJump") {
      bool classified = false;

      // Exact Hyperline mantleWallJump rule.
      const auto from =
          anchor.frame > kMantleWallJumpTicks
              ? anchor.frame - kMantleWallJumpTicks
              : 0;
      if (const auto* mantle_end =
              LastPrimitive(primitives, "mantleEnd", from, anchor.frame)) {
        const auto* exit = static_cast<const TechniquePrimitive*>(nullptr);
        for (const auto& candidate : primitives) {
          if (candidate.kind == "takeoff" &&
              candidate.frame >= mantle_end->frame &&
              candidate.frame <= anchor.frame) {
            exit = &candidate;
            break;
          }
        }
        if (!exit) exit = mantle_end;
        if (exit->frame >= mantle_end->frame &&
            exit->frame - mantle_end->frame <= kMantleExitTicks &&
            !AnyPrimitive(primitives, "touchdown", exit->frame + 1,
                          anchor.frame)) {
          const auto peak = PeakRecorded(
              ticks, exit->frame,
              std::max(exit->frame,
                       anchor.frame > 0 ? anchor.frame - 1 : anchor.frame),
              [](const split::movement::PawnSample& pawn) {
                return HorizontalSpeed(pawn.velocity);
              });
          if (peak && *peak >= kMantleWallJumpMinSpeed) {
            add("Mantle wall jump", anchor);
            classified = true;
          }
        }
      }
      if (classified) continue;

      // Exact Hyperline cornerBoost rule.
      if (const auto peak = PeakRecorded(
              ticks, anchor.frame, anchor.frame + kWallJumpRiseTicks,
              [](const split::movement::PawnSample& pawn) {
                return static_cast<double>(pawn.velocity[2]);
              });
          peak && *peak >= kCornerBoostMinVerticalSpeed) {
        add("Corner boost", anchor);
        continue;
      }

      // Exact Hyperline edgeBoost rule.
      if (anchor.frame >= 2) {
        const auto* entry = RecordedAt(ticks, anchor.frame - 1);
        const auto followed_entry =
            WallContactFollowed(ticks, anchor.frame - 1);
        if (entry &&
            HorizontalSpeed(entry->pawn.velocity) >= kEdgeBoostMinEntrySpeed &&
            followed_entry && !*followed_entry) {
          bool edge = false;
          for (std::uint64_t back = 2;
               back <= kEdgeBoostMaxDetachedTicks + 1 &&
               back < anchor.frame;
               ++back) {
            const auto followed =
                WallContactFollowed(ticks, anchor.frame - back);
            if (!followed) continue;
            edge = *followed;
            break;
          }
          if (edge) {
            add("Edge boost", anchor);
            continue;
          }
        }
      }
      continue;
    }

    if (anchor.kind == "slideStart") {
      // Mantle strafe has priority over mantle slide / dash cancel / slide.
      const auto mantle_from = anchor.frame > 112 ? anchor.frame - 112 : 0;
      if (const auto* mantle =
              LastPrimitive(primitives, "mantleStart", mantle_from,
                            anchor.frame)) {
        const auto* start = RecordedAt(ticks, anchor.frame);
        const auto* later =
            RecordedAt(ticks, anchor.frame + kMantleStrafeTicks);
        if (start && later) {
          const auto mantle_velocity =
              UsableHorizontalVelocityNear(ticks, mantle->frame, 6);
          const auto turn =
              mantle_velocity
                  ? HeadingTurnDegrees(*mantle_velocity, later->pawn.velocity)
                  : std::nullopt;
          const double speed = HorizontalSpeed(start->pawn.velocity);
          const double later_speed = HorizontalSpeed(later->pawn.velocity);
          const bool ok =
              turn && *turn >= kMantleStrafeMinTurnDegrees && speed > 0.0 &&
              later_speed >= kMantleStrafeMinRetention * speed;
          if (ok) {
            add("Mantle strafe", anchor);
            continue;
          }
          if (diagnostics) {
            std::ostringstream detail;
            detail << "accepted=" << (ok ? "yes" : "no")
                   << " mantleStart=yes"
                   << " mantleToSlide=" << (anchor.frame - mantle->frame)
                   << "/112"
                   << " settle=" << kMantleStrafeTicks
                   << " mantleVelocity="
                   << (mantle_velocity ? "yes" : "no")
                   << " turn=";
            if (turn) detail << std::fixed << std::setprecision(1) << *turn;
            else detail << "n/a";
            detail << "/" << kMantleStrafeMinTurnDegrees
                   << " speed=" << std::fixed << std::setprecision(1)
                   << speed
                   << " laterSpeed=" << later_speed
                   << " minRetention="
                   << (kMantleStrafeMinRetention * speed);
            diag("Mantle strafe", anchor, detail.str());
          }
        } else if (diagnostics) {
          std::ostringstream detail;
          detail << "accepted=no"
                 << " mantleStart=yes"
                 << " mantleToSlide=" << (anchor.frame - mantle->frame)
                 << "/112"
                 << " startSample=" << (start ? "yes" : "no")
                 << " sampleAt+12=" << (later ? "yes" : "no");
          diag("Mantle strafe", anchor, detail.str());
        }
      }

      // Exact two Hyperline mantleSlide requirements.
      const auto start_from = anchor.frame > 96 ? anchor.frame - 96 : 0;
      if (const auto* mantle_start =
              LastPrimitive(primitives, "mantleStart", start_from,
                            anchor.frame)) {
        if (!AnyPrimitive(primitives, "mantleEnd",
                          mantle_start->frame + 1,
                          anchor.frame > 0 ? anchor.frame - 1
                                           : anchor.frame)) {
          add("Mantle slide", anchor);
          continue;
        }
      }
      const auto end_from = anchor.frame > 16 ? anchor.frame - 16 : 0;
      if (LastPrimitive(primitives, "mantleEnd", end_from, anchor.frame)) {
        add("Mantle slide", anchor);
        continue;
      }

      if (dash_cancelled(anchor)) {
        add("Dash cancel", anchor);
        continue;
      }
      continue;
    }

    if (anchor.kind == "takeoff") {
      // Exact Hyperline superglide requirement + settle peak.
      const auto mantle_from = anchor.frame > 4 ? anchor.frame - 4 : 0;
      const auto* mantle_end =
          LastPrimitive(primitives, "mantleEnd", mantle_from, anchor.frame);

      // On this Deadlock build m_bChanneling can fail to expose mantleEnd even
      // though m_flStartTime reliably exposes mantleStart. Keep Hyperline's
      // exact mantleEnd<=4 path first, then use a conservative inferred-end
      // fallback: takeoff must still belong to the same recent mantle sequence.
      const auto mantle_start_from =
          anchor.frame > 96 ? anchor.frame - 96 : 0;
      const auto* recent_mantle_start =
          LastPrimitive(primitives, "mantleStart", mantle_start_from,
                        anchor.frame);

      const bool mantle_context =
          mantle_end != nullptr || recent_mantle_start != nullptr;

      if (mantle_context) {
        const auto peak = PeakRecorded(
            ticks, anchor.frame,
            anchor.frame + kSuperglideSettleTicks,
            [](const split::movement::PawnSample& pawn) {
              return HorizontalSpeed(pawn.velocity);
            });

        const bool exact_timing =
            mantle_end &&
            anchor.frame >= mantle_end->frame &&
            anchor.frame - mantle_end->frame <= 4;

        const bool inferred_timing =
            !mantle_end && recent_mantle_start &&
            anchor.frame >= recent_mantle_start->frame &&
            anchor.frame - recent_mantle_start->frame >= 24 &&
            anchor.frame - recent_mantle_start->frame <= 64;

        if ((exact_timing || inferred_timing) &&
            peak && *peak >= kSuperglideMinSpeed) {
          add("Superglide", anchor);
          if (diagnostics && inferred_timing) {
            std::ostringstream detail;
            detail << "accepted via inferred mantleEnd"
                   << " startDelta="
                   << (anchor.frame - recent_mantle_start->frame)
                   << "/64 peakSpeed=" << std::fixed << std::setprecision(1)
                   << *peak << "/" << kSuperglideMinSpeed;
            diag("Superglide", anchor, detail.str());
          }
          continue;
        }

        if (diagnostics) {
          std::ostringstream detail;
          detail << "mantleEnd=" << (mantle_end ? "yes" : "no");
          if (mantle_end) {
            detail << " delta=" << (anchor.frame - mantle_end->frame) << "/4";
          } else if (recent_mantle_start) {
            detail << " inferredStartDelta="
                   << (anchor.frame - recent_mantle_start->frame)
                   << " window=24..64";
          }
          detail << " peakSpeed=";
          if (peak) detail << std::fixed << std::setprecision(1) << *peak;
          else detail << "n/a";
          detail << "/" << kSuperglideMinSpeed;
          diag("Superglide", anchor, detail.str());
        }
      } else if (diagnostics) {
        diag("Superglide", anchor,
             "mantleEnd=no mantleStart=no recentSequence");
      }

      // Exact Hyperline shortHop: recent slideStart that itself qualifies as
      // a dash cancel, with no touchdown between it and takeoff.
      const auto slide_from = anchor.frame > 8 ? anchor.frame - 8 : 0;
      if (const auto* slide =
              LastPrimitive(primitives, "slideStart", slide_from,
                            anchor.frame)) {
        const bool touchdown =
            AnyPrimitive(primitives, "touchdown", slide->frame + 1,
                         anchor.frame);
        const bool cancelled = dash_cancelled(*slide);
        const auto slide_delta = anchor.frame - slide->frame;
        const auto crouch_from = slide->frame > 16 ? slide->frame - 16 : 0;
        const auto* crouch_press =
            LastPrimitive(primitives, "crouchPress", crouch_from, slide->frame);

        if (!touchdown && cancelled) {
          add("Short hop", anchor);
          if (diagnostics) {
            std::ostringstream detail;
            detail << "accepted=yes"
                   << " slideToTakeoff=" << slide_delta << "/8"
                   << " touchdownBetween=no"
                   << " dashCancel=yes";
            diag("Short hop", anchor, detail.str());
          }
          continue;
        }

        if (diagnostics) {
          std::ostringstream detail;
          detail << "accepted=no"
                 << " slideStart=yes"
                 << " slideToTakeoff=" << slide_delta << "/8"
                 << " touchdownBetween=" << (touchdown ? "yes" : "no")
                 << " dashCancel=" << (cancelled ? "yes" : "no")
                 << " crouchPress=" << (crouch_press ? "yes" : "no");
          if (crouch_press) {
            detail << " crouchToSlide="
                   << (slide->frame - crouch_press->frame)
                   << " crouchSpeed=" << std::fixed << std::setprecision(1)
                   << HorizontalSpeed(crouch_press->velocity);
          }
          detail << " takeoffSpeed=" << std::fixed << std::setprecision(1)
                 << HorizontalSpeed(anchor.velocity);
          diag("Short hop", anchor, detail.str());
        }
      } else if (diagnostics) {
        const auto* nearest_slide =
            LastPrimitive(primitives, "slideStart", 0, anchor.frame);
        std::ostringstream detail;
        detail << "accepted=no"
               << " slideStart=no within=8";
        if (nearest_slide) {
          detail << " nearestSlideDelta="
                 << (anchor.frame - nearest_slide->frame);
          const auto* nearest_dash =
              LastPrimitive(primitives, "groundDash", 0,
                            nearest_slide->frame);
          if (nearest_dash) {
            detail << " nearestDashToSlide="
                   << (nearest_slide->frame - nearest_dash->frame);
          } else {
            detail << " nearestDashToSlide=n/a";
          }
        } else {
          detail << " nearestSlideDelta=n/a";
        }
        detail << " takeoffSpeed=" << std::fixed << std::setprecision(1)
               << HorizontalSpeed(anchor.velocity);
        diag("Short hop", anchor, detail.str());
      }
      continue;
    }

    if (anchor.kind == "meleeStarted") {
      // Hyperline heavyMelee: a melee start is considered heavy only when
      // horizontal speed gains at least 300 u/s during the following 96 ticks.
      // A mantle start truncates the observation window.
      bool duplicate_heavy = false;
      for (const auto& earlier : recognized) {
        if (earlier.label == "Heavy melee" &&
            earlier.frame < anchor.frame &&
            anchor.frame <= earlier.frame + kMeleeSettleTicks) {
          duplicate_heavy = true;
          break;
        }
      }

      std::uint64_t end = anchor.frame + kMeleeSettleTicks;
      for (const auto& candidate : primitives) {
        if (candidate.kind == "mantleStart" &&
            candidate.frame >= anchor.frame &&
            candidate.frame <= end) {
          end = candidate.frame > anchor.frame
              ? candidate.frame - 1
              : anchor.frame;
          break;
        }
      }

      const auto* start = RecordedAt(ticks, anchor.frame);
      const auto peak = PeakRecorded(
          ticks, anchor.frame, end,
          [](const split::movement::PawnSample& pawn) {
            return HorizontalSpeed(pawn.velocity);
          });
      const double start_speed =
          start ? HorizontalSpeed(start->pawn.velocity) : 0.0;
      const double gain = peak ? *peak - start_speed : 0.0;
      const bool heavy =
          !duplicate_heavy && start && peak &&
          gain >= kHeavyMeleeMinSpeedGain;

      if (diagnostics) {
        std::ostringstream detail;
        detail << "classified=" << (heavy ? "heavy" : "light")
               << " startSpeed=" << std::fixed << std::setprecision(1)
               << start_speed
               << " peakSpeed=";
        if (peak) detail << *peak;
        else detail << "n/a";
        detail << " gain=";
        if (peak) detail << gain;
        else detail << "n/a";
        detail << "/" << kHeavyMeleeMinSpeedGain
               << " window=" << (end - anchor.frame)
               << "/" << kMeleeSettleTicks
               << " duplicateHeavy="
               << (duplicate_heavy ? "yes" : "no");
        diag("Melee", anchor, detail.str());
      }

      add(heavy ? "Heavy melee" : "Light melee", anchor);
      continue;
    }

    if (anchor.kind == "abilityExecuted") {
      // Exact Hyperline heavyMeleeCancel: an ability executed within 96 ticks
      // of melee start must happen at >=500 u/s and retain >=80% of that
      // horizontal speed eight ticks later, with no mantle transition involved.
      const auto from =
          anchor.frame > kMeleeSettleTicks
              ? anchor.frame - kMeleeSettleTicks
              : 0;
      const auto* melee =
          LastPrimitive(primitives, "meleeStarted", from, anchor.frame);

      if (melee) {
        const auto* cast = RecordedAt(ticks, anchor.frame);
        const auto* after =
            RecordedAt(ticks, anchor.frame + kCancelSettleTicks);
        const bool mantle =
            AnyPrimitive(primitives, "mantleStart", melee->frame,
                         anchor.frame + kCancelSettleTicks) ||
            AnyPrimitive(primitives, "mantleEnd", melee->frame,
                         anchor.frame + kCancelSettleTicks);

        const double cast_speed =
            cast ? HorizontalSpeed(cast->pawn.velocity) : 0.0;
        const double after_speed =
            after ? HorizontalSpeed(after->pawn.velocity) : 0.0;
        const bool ok =
            cast && after && !mantle &&
            cast_speed >= kHeavyMeleeCancelMinSpeed &&
            after_speed >= kHeavyMeleeCancelMinRetention * cast_speed;

        if (diagnostics) {
          std::ostringstream detail;
          detail << "accepted=" << (ok ? "yes" : "no")
                 << " meleeDelta=" << (anchor.frame - melee->frame)
                 << "/" << kMeleeSettleTicks
                 << " mantle=" << (mantle ? "yes" : "no")
                 << " castSpeed=" << std::fixed << std::setprecision(1)
                 << cast_speed << "/" << kHeavyMeleeCancelMinSpeed
                 << " after8=" << after_speed
                 << " minRetention="
                 << (kHeavyMeleeCancelMinRetention * cast_speed);
          diag("Heavy melee cancel", anchor, detail.str());
        }

        if (ok) {
          add("Heavy melee cancel", anchor);
          continue;
        }
      }
    }

    if (anchor.kind == "airDash") {
      const auto takeoff_from = anchor.frame > 4 ? anchor.frame - 4 : 0;
      if (const auto* takeoff =
              LastPrimitive(primitives, "takeoff", takeoff_from,
                            anchor.frame)) {
        const bool touchdown =
            AnyPrimitive(primitives, "touchdown", takeoff->frame + 1,
                         anchor.frame > 0 ? anchor.frame - 1
                                          : anchor.frame);
        const auto dash_jump_from =
            takeoff->frame > 2 ? takeoff->frame - 2 : 0;
        const bool dash_jump =
            AnyPrimitive(primitives, "dashJump", dash_jump_from,
                         anchor.frame);
        if (!touchdown && !dash_jump) {
          add("Instant air dash", anchor);
          continue;
        }
      }

      const auto zip_from =
          anchor.frame > kZiplineDashTicks
              ? anchor.frame - kZiplineDashTicks
              : 0;
      if (const auto* left =
              LastPrimitive(primitives, "ziplineLeft", zip_from,
                            anchor.frame)) {
        if (!AnyPrimitive(primitives, "touchdown", left->frame + 1,
                          anchor.frame > 0 ? anchor.frame - 1
                                           : anchor.frame)) {
          add("Zipline dash", anchor);
          continue;
        }
      }
    }
  }

  return recognized;
}

std::vector<MovementCue> MergeRecognizedTechniques(
    const std::vector<MovementCue>& raw,
    const std::vector<MovementCue>& recognized) {
  std::vector<MovementCue> merged;
  merged.reserve(raw.size() + recognized.size());

  for (const auto& cue : raw) {
    if (cue.label == "Melee started" ||
        cue.label == "Ability executed") {
      continue;
    }
    const bool superseded = std::any_of(
        recognized.begin(), recognized.end(),
        [&](const MovementCue& exact) {
          if (exact.label != cue.label) return false;
          const auto delta = exact.frame > cue.frame
              ? exact.frame - cue.frame
              : cue.frame - exact.frame;
          return delta <= 128;
        });
    if (!superseded) merged.push_back(cue);
  }

  merged.insert(merged.end(), recognized.begin(), recognized.end());
  std::sort(merged.begin(), merged.end(),
            [](const MovementCue& a, const MovementCue& b) {
              if (a.frame != b.frame) return a.frame < b.frame;
              return a.label < b.label;
            });
  merged.erase(std::unique(merged.begin(), merged.end(),
                           [](const MovementCue& a, const MovementCue& b) {
                             return a.frame == b.frame &&
                                    a.label == b.label;
                           }),
               merged.end());
  return merged;
}

bool IsRouteCallout(split::movement::MovementExecution execution) {
  using E = split::movement::MovementExecution;
  switch (execution) {
    case E::kGroundDash:
    case E::kSlide:
    case E::kBouncePad:
    case E::kDashJump:
    case E::kAirJump:
    case E::kWallJump:
    case E::kAirDash:
    case E::kMeleeStarted:
    case E::kAbilityExecuted:
      return true;
    default:
      return false;
  }
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
  split::movement::MovementEvents movement_events;
  split::movement::MovementRecorder recorder;
  split::movement::MovementRecorder race_run_recorder;
  std::atomic<int> human_slot{-1};
  bool pawn_announced = false;
  bool practice_shop_ready = false;
  int pre_spawn_cleanup_passes_remaining = 0;
  std::uint64_t next_pre_spawn_cleanup_frame = 0;
  bool steering_was_active = false;
  int pb_ghost_slot = -1;
  std::atomic<bool> replay_requested{false};
  std::atomic<bool> race_requested{false};
  std::atomic<bool> retry_requested{false};
  std::atomic<bool> visual_test_requested{false};
  std::atomic<bool> native_route_line_active{false};
  std::atomic<bool> reset_movement_recognizer{true};
  std::atomic<bool> technique_diagnostics{false};
  std::atomic<int> visual_mode{2}; // both, ghost, tracer, off
  std::atomic<bool> show_rings{true};
  std::atomic<bool> show_checkpoints{true};
  std::atomic<bool> show_hud{true};
  std::atomic<bool> practice_infinite_stamina{false};
  std::atomic<bool> practice_no_cooldowns{false};
  std::atomic<bool> practice_infinite_ammo{false};
  std::string last_pawn_error;
  std::string last_practice_stamina_error;
  std::string last_practice_cooldown_error;
  std::string current_map = launch.map;
  std::mutex route_cues_mutex;
  std::vector<MovementCue> route_cues;
  std::mutex movement_sample_mutex;
  std::optional<split::movement::PawnSample> latest_movement_sample;
  std::optional<split::movement::PawnSample> previous_movement_sample;
  std::optional<std::uint64_t> last_mantle_start_frame;
  std::optional<std::uint64_t> last_mantle_end_frame;
  std::optional<std::uint64_t> last_slide_start_frame;
  std::optional<std::uint64_t> last_slide_end_frame;
  std::optional<std::uint64_t> last_jump_press_frame;
  std::optional<std::uint64_t> techdiag_dash_window_until;
  std::uint64_t techdiag_last_input_held = 0;

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
    if (command == "techdiag on") {
      technique_diagnostics.store(true, std::memory_order_release);
      return "{\"techniqueDiagnostics\":true}";
    }
    if (command == "techdiag off") {
      technique_diagnostics.store(false, std::memory_order_release);
      return "{\"techniqueDiagnostics\":false}";
    }
    if (command == "techdiag") {
      return technique_diagnostics.load(std::memory_order_acquire)
          ? "{\"techniqueDiagnostics\":true}"
          : "{\"techniqueDiagnostics\":false}";
    }
    if (command == "movementstate") {
      std::optional<split::movement::PawnSample> snapshot;
      {
        std::scoped_lock lock(movement_sample_mutex);
        snapshot = latest_movement_sample;
      }
      if (!snapshot) return "{\"ready\":false}";
      std::ostringstream out;
      out << "{"
          << "\"ready\":true,"
          << "\"mantling\":" << (snapshot->mantling ? "true" : "false") << ","
          << "\"sliding\":" << (snapshot->sliding ? "true" : "false") << ","
          << "\"mantleStart\":";
      if (snapshot->mantle_start_time) out << *snapshot->mantle_start_time;
      else out << "null";
      out << ",\"wallJumps\":";
      if (snapshot->consecutive_wall_jumps) out << static_cast<int>(*snapshot->consecutive_wall_jumps);
      else out << "null";
      out << ",\"wallFacing\":";
      if (snapshot->wall_jump_facing) out << *snapshot->wall_jump_facing;
      else out << "null";
      out << ",\"ziplineTime\":";
      if (snapshot->zipline_time) out << *snapshot->zipline_time;
      else out << "null";
      out << "}";
      return out.str();
    }
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
      if (native_route_line_active.load(std::memory_order_acquire)) {
        return "{\"queued\":true,\"note\":\"hyperline-route-line-already-active\","
               "\"created\":0}";
      }

      const auto ticks = recorder.PlaybackTicks();
      if (ticks.size() < 2) {
        return "{\"queued\":false,\"reason\":\"no-recorded-route\"}";
      }

      const auto line = RoutePolyline(ticks, 128);
      if (line.size() < 2) {
        return "{\"queued\":false,\"reason\":\"route-has-no-line\"}";
      }

      visual_renderer.Clear();
      native_route_line_active.store(true, std::memory_order_release);
      std::size_t created = 0;
      std::string beam_error;
      for (std::size_t i = 1; i < line.size(); ++i) {
        if (!visual_renderer.DrawBeamSegment(line[i - 1], line[i], beam_error)) {
          std::ostringstream out;
          out << "{\"queued\":false,\"reason\":\"hyperline-beam-failed\","
              << "\"segment\":" << (i - 1)
              << ",\"created\":" << created
              << ",\"detail\":\"" << beam_error << "\"}";
          native_route_line_active.store(false, std::memory_order_release);
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
          << "\"practiceInfiniteStamina\":" << (practice_infinite_stamina.load(std::memory_order_acquire) ? "true" : "false") << ","
          << "\"practiceNoCooldowns\":" << (practice_no_cooldowns.load(std::memory_order_acquire) ? "true" : "false") << ","
          << "\"practiceInfiniteAmmo\":" << (practice_infinite_ammo.load(std::memory_order_acquire) ? "true" : "false") << ","
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
    } else if (command == "practice stamina on" ||
               command == "practice stamina off") {
      const bool on = command == "practice stamina on";
      practice_infinite_stamina.store(on, std::memory_order_release);
      last_practice_stamina_error.clear();
      std::cout << "[SPLIT][Practice] infinite stamina="
                << (on ? "on" : "off") << "\n";
    } else if (command == "practice cooldowns on" ||
               command == "practice cooldowns off") {
      const bool on = command == "practice cooldowns on";
      practice_no_cooldowns.store(on, std::memory_order_release);
      last_practice_cooldown_error.clear();
      std::cout << "[SPLIT][Practice] no cooldowns="
                << (on ? "on" : "off") << "\n";
    } else if (command == "practice ammo on" ||
               command == "practice ammo off") {
      const bool on = command == "practice ammo on";
      std::string command_error;
      const std::string server_command =
          std::string("sv_infinite_ammo ") + (on ? "1" : "0");
      if (!split::movement::RunServerCommand(server_command, command_error)) {
        std::cout << "[SPLIT][Practice] infinite ammo unavailable: "
                  << command_error << "\n";
      } else {
        practice_infinite_ammo.store(on, std::memory_order_release);
        std::cout << "[SPLIT][Practice] infinite ammo="
                  << (on ? "on" : "off") << "\n";
      }
    } else if (command == "record") {
      {
        std::scoped_lock lock(route_cues_mutex);
        route_cues.clear();
      }
      movement_events.Clear();
      reset_movement_recognizer.store(true, std::memory_order_release);
      recorder.Start();
      std::cout << "[SPLIT][Recorder] recording started\n";
    } else if (command == "recordtoggle") {
      if (recorder.Stats().recording) {
        PrintStats(recorder.Stop());
      } else {
        {
          std::scoped_lock lock(route_cues_mutex);
          route_cues.clear();
        }
        movement_events.Clear();
        reset_movement_recognizer.store(true, std::memory_order_release);
        recorder.Start();
        std::cout << "[SPLIT][Recorder] recording started\n";
      }
    } else if (command == "stop") {
      PrintStats(recorder.Stop());
    } else if (command == "status") {
      PrintStats(recorder.Stats());
    } else if (command == "clear") {
      recorder.Clear();
      {
        std::scoped_lock lock(route_cues_mutex);
        route_cues.clear();
      }
      movement_events.Clear();
      reset_movement_recognizer.store(true, std::memory_order_release);
      native_route_line_active.store(false, std::memory_order_release);
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
      native_route_line_active.store(false, std::memory_order_release);
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
                << " (record, recordtoggle, stop, status, state, save, replay, watch, race, retry, racestatus, racecancel, visualtest, tracertest, visualdiagnostics, movementstate, techdiag [on|off], practice stamina [on|off], practice cooldowns [on|off], practice ammo [on|off], clear)\n";
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
  std::string movement_event_error;
  if (!movement_events.Install(movement_event_error)) {
    std::cout << "[SPLIT][MovementEvents] unavailable: "
              << movement_event_error << "\n";
  } else {
    std::cout << "[SPLIT][MovementEvents] native modifier events ready\n";
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
            if (pre_spawn_cleanup_passes_remaining > 0 &&
                frame >= next_pre_spawn_cleanup_frame &&
                !pawn_announced) {
              static constexpr std::array<std::string_view, 8> kMapNpcClasses{
                  "npc_trooper",
                  "npc_trooper_neutral",
                  "npc_neutral",
                  "npc_super_neutral",
                  "npc_neutral_sinners_sacrifice",
                  "npc_trooper_boss",
                  "npc_boss_tier1",
                  "npc_boss_tier2",
              };

              bool all_ok = true;
              for (const auto class_name : kMapNpcClasses) {
                std::string command_error;
                const std::string command =
                    std::string("ent_remove_all ") + std::string(class_name);
                if (!split::movement::RunServerCommand(command, command_error)) {
                  all_ok = false;
                  std::cout << "[SPLIT][World] silent remove failed: "
                            << command << " (" << command_error << ")\n";
                }
              }
              if (all_ok) {
                std::cout << "[SPLIT][World] silent pre-spawn cleanup pass "
                          << (4 - pre_spawn_cleanup_passes_remaining)
                          << "/3 complete\n";
              }
              --pre_spawn_cleanup_passes_remaining;
              next_pre_spawn_cleanup_frame = frame + 30;
            }

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

            if (!practice_shop_ready) {
              // Use Deadlock's own Hero Testing purchase path rather than
              // inflating CCitadelPlayerPawn::m_nCurrencies. Modlock exposes
              // citadel_hero_testing_money as the buffer used for item
              // purchases, and Deadlock's stock Hero Testing HUD drives
              // citadel_hero_testing_infinite_money.
              //
              // Crucially this is enabled only after Observe() proved the hero
              // pawn is already live. Enabling hero testing in the launch
              // command previously interfered with SPLIT's normal spawn flow.
              static constexpr std::array<std::string_view, 4> kPracticeShopCommands{
                  "citadel_hero_testing_enabled 1",
                  "citadel_hero_testing_money 1000000",
                  "citadel_hero_testing_infinite_money 1",
                  "citadel_allow_purchasing_anywhere 1",
              };

              bool shop_ok = true;
              for (const auto command : kPracticeShopCommands) {
                std::string command_error;
                if (!split::movement::RunServerCommand(command, command_error)) {
                  shop_ok = false;
                  std::cout << "[SPLIT][Practice] shop command failed: "
                            << command << " (" << command_error << ")\n";
                }
              }
              if (shop_ok) {
                practice_shop_ready = true;
                std::cout << "[SPLIT][Practice] Deadlock Hero Testing shop enabled "
                             "(infinite money + buy anywhere)\n";
              }
            }

            // Practice mutations deliberately run only after Observe() proved
            // that this slot owns a live pawn. This mirrors Hyperline's
            // per-frame player policy and avoids changing server startup flow.
            if (practice_infinite_stamina.load(std::memory_order_acquire)) {
              std::string practice_error;
              if (!pawn_probe.RestorePracticeStamina(slot, practice_error)) {
                if (!practice_error.empty() &&
                    practice_error != last_practice_stamina_error) {
                  last_practice_stamina_error = practice_error;
                  std::cout << "[SPLIT][Practice] stamina unavailable: "
                            << practice_error << "\n";
                }
              } else {
                last_practice_stamina_error.clear();
              }
            }
            if (practice_no_cooldowns.load(std::memory_order_acquire)) {
              std::string practice_error;
              if (!pawn_probe.RefreshPracticeCooldowns(slot, practice_error)) {
                if (!practice_error.empty() &&
                    practice_error != last_practice_cooldown_error) {
                  last_practice_cooldown_error = practice_error;
                  std::cout << "[SPLIT][Practice] cooldown refresh unavailable: "
                            << practice_error << "\n";
                }
              } else {
                last_practice_cooldown_error.clear();
              }
            }

            const auto latest_input = input_capture.Latest(slot);
            constexpr std::uint64_t kJumpButton = std::uint64_t{1} << 48;
            if (latest_input && (latest_input->pressed & kJumpButton) != 0) {
              last_jump_press_frame = frame;
            }

            recorder.Push(frame, *sample, latest_input);
            race_run_recorder.Push(frame, *sample, latest_input);

            bool native_slide_started = false;
            bool native_wall_jump = false;
            for (const auto& movement : movement_events.Drain(sample->pawn_handle)) {
              const char* label =
                  split::movement::MovementExecutionName(movement.execution);
              std::cout << "[SPLIT][MovementEvents] " << label
                        << " at (" << sample->position[0] << ", "
                        << sample->position[1] << ", "
                        << sample->position[2] << ")\n";

              if (movement.execution == split::movement::MovementExecution::kGroundDash &&
                  technique_diagnostics.load(std::memory_order_acquire)) {
                techdiag_dash_window_until = frame + 40;
              }
              if (movement.execution == split::movement::MovementExecution::kSlide) {
                native_slide_started = true;
              }
              if (movement.execution == split::movement::MovementExecution::kWallJump) {
                native_wall_jump = true;
              }

              if (recorder.Stats().recording && IsRouteCallout(movement.execution)) {
                std::scoped_lock lock(route_cues_mutex);
                const bool semantic_event =
                    movement.execution == split::movement::MovementExecution::kMeleeStarted ||
                    movement.execution == split::movement::MovementExecution::kAbilityExecuted;
                const bool duplicate =
                    !route_cues.empty() &&
                    route_cues.back().label == label &&
                    (semantic_event
                         ? (frame >= route_cues.back().frame &&
                            frame - route_cues.back().frame <= 2)
                         : RouteDistance(route_cues.back().position,
                                         sample->position) < 48.0);
                if (!duplicate) {
                  route_cues.push_back({label, sample->position, frame});
                }
              }
            }

            if (reset_movement_recognizer.exchange(false, std::memory_order_acq_rel)) {
              previous_movement_sample.reset();
              last_mantle_start_frame.reset();
              last_mantle_end_frame.reset();
              last_slide_start_frame.reset();
              last_slide_end_frame.reset();
              last_jump_press_frame.reset();
              techdiag_dash_window_until.reset();
              techdiag_last_input_held = 0;
            }

            {
              std::scoped_lock lock(movement_sample_mutex);
              latest_movement_sample = *sample;
            }

            // Hyperline treats a changed mantleStart timestamp as a mantle
            // restart even when channeling never visibly drops between frames.
            bool mantle_started = false;
            if (previous_movement_sample) {
              mantle_started =
                  (!previous_movement_sample->mantling && sample->mantling);
              if (sample->mantle_start_time) {
                const float now = *sample->mantle_start_time;
                const bool new_timestamp =
                    !previous_movement_sample->mantle_start_time ||
                    std::fabs(now - *previous_movement_sample->mantle_start_time) > 0.0001f;
                mantle_started = mantle_started || (new_timestamp && now > 0.0f);
              }
            } else if (sample->mantle_start_time && *sample->mantle_start_time > 0.0f) {
              mantle_started = sample->mantling;
            }

            if (mantle_started) {
              std::cout << "[SPLIT][MovementState] Mantle start at ("
                        << sample->position[0] << ", "
                        << sample->position[1] << ", "
                        << sample->position[2] << ")"
                        << " startTime="
                        << (sample->mantle_start_time
                                ? std::to_string(*sample->mantle_start_time)
                                : std::string("n/a"))
                        << "\n";
              if (recorder.Stats().recording) {
                std::scoped_lock lock(route_cues_mutex);
                const bool duplicate =
                    !route_cues.empty() &&
                    route_cues.back().label == "Mantle" &&
                    RouteDistance(route_cues.back().position, sample->position) < 48.0;
                if (!duplicate) {
                  route_cues.push_back({"Mantle", sample->position, frame});
                }
              }
            }
            if (mantle_started) {
              last_mantle_start_frame = frame;
            }

            const bool mantle_ended =
                previous_movement_sample &&
                previous_movement_sample->mantling && !sample->mantling;
            if (mantle_ended) {
              last_mantle_end_frame = frame;
            }

            if (technique_diagnostics.load(std::memory_order_acquire) &&
                techdiag_dash_window_until &&
                frame <= *techdiag_dash_window_until) {
              const std::uint64_t held = latest_input ? latest_input->held : 0;
              const std::uint64_t changed = latest_input ? latest_input->changed : 0;
              const std::uint64_t scroll = latest_input ? latest_input->scroll : 0;
              const std::uint64_t pressed = latest_input ? latest_input->pressed : 0;
              const bool held_changed = held != techdiag_last_input_held;
              const bool state_changed =
                  !previous_movement_sample ||
                  previous_movement_sample->grounded != sample->grounded ||
                  previous_movement_sample->sliding != sample->sliding;
              if (held_changed || pressed != 0 || state_changed ||
                  native_slide_started) {
                std::cout << "[SPLIT][TechDiag][DashFrame] frame=" << frame
                          << " grounded=" << (sample->grounded ? "1" : "0")
                          << " sliding=" << (sample->sliding ? "1" : "0")
                          << " nativeSlide=" << (native_slide_started ? "1" : "0")
                          << " speed=" << std::fixed << std::setprecision(1)
                          << HorizontalSpeed(sample->velocity)
                          << " held=0x" << std::hex << held
                          << " changed=0x" << changed
                          << " scroll=0x" << scroll
                          << " pressed=0x" << pressed << std::dec
                          << "\n";
              }
              techdiag_last_input_held = held;
              if (frame == *techdiag_dash_window_until) {
                techdiag_dash_window_until.reset();
              }
            }

            const bool slide_started =
                native_slide_started ||
                (previous_movement_sample &&
                 !previous_movement_sample->sliding && sample->sliding);
            if (slide_started) {
              last_slide_start_frame = frame;

              // Hyperline's composed mantle/slide techniques use the mantle
              // exit neighborhood. Keep the raw Slide event as evidence, but
              // add the stronger composed label when the transition is close.
              constexpr std::uint64_t kMantleExitTicks = 16;
              constexpr std::uint64_t kMantleSequenceTicks = 96;
              const auto near_mantle = [&](const std::optional<std::uint64_t>& at,
                                           std::uint64_t window) {
                return at && frame >= *at && frame - *at <= window;
              };
              // m_bChanneling can be missed on this build while m_flStartTime
              // is reliable. A slide shortly after the validated mantle start
              // is therefore also considered part of the same mantle sequence.
              if (near_mantle(last_mantle_end_frame, kMantleExitTicks) ||
                  near_mantle(last_mantle_start_frame, kMantleSequenceTicks)) {
                std::cout << "[SPLIT][Recognizer] Mantle slide at ("
                          << sample->position[0] << ", "
                          << sample->position[1] << ", "
                          << sample->position[2] << ")\n";
                if (recorder.Stats().recording) {
                  std::scoped_lock lock(route_cues_mutex);
                  route_cues.push_back({"Mantle slide", sample->position, frame});
                }
              }
            }

            const bool slide_ended =
                previous_movement_sample &&
                previous_movement_sample->sliding && !sample->sliding;
            if (slide_ended) {
              last_slide_end_frame = frame;
            }

            const bool takeoff =
                previous_movement_sample &&
                previous_movement_sample->grounded && !sample->grounded;
            if (takeoff) {
              constexpr std::uint64_t kCoalesceTicks = 2;
              const bool from_slide =
                  previous_movement_sample->sliding ||
                  (last_slide_end_frame && frame >= *last_slide_end_frame &&
                   frame - *last_slide_end_frame <= kCoalesceTicks);

              // A physical launch (vent, knockback, bounce, etc.) also causes
              // grounded -> airborne. Hyperline's slide hop is an authored
              // jump technique, so require the actual IN_MANTLE/Jump press.
              const bool jumped =
                  last_jump_press_frame &&
                  frame >= *last_jump_press_frame &&
                  frame - *last_jump_press_frame <= kCoalesceTicks;

              if (from_slide && jumped) {
                std::cout << "[SPLIT][Recognizer] Slide hop at ("
                          << sample->position[0] << ", "
                          << sample->position[1] << ", "
                          << sample->position[2] << ")\n";
                if (recorder.Stats().recording) {
                  std::scoped_lock lock(route_cues_mutex);
                  route_cues.push_back({"Slide hop", sample->position, frame});
                }
              } else if (from_slide && !jumped) {
                std::cout << "[SPLIT][Recognizer] slide takeoff ignored: no Jump press\n";
              }
            }

            if (native_wall_jump && last_mantle_start_frame) {
              constexpr std::uint64_t kMantleWallJumpTicks = 96;
              constexpr double kMantleWallJumpMinSpeed = 500.0;
              const double horizontal_speed =
                  std::hypot(static_cast<double>(sample->velocity[0]),
                             static_cast<double>(sample->velocity[1]));
              if (frame >= *last_mantle_start_frame &&
                  frame - *last_mantle_start_frame <= kMantleWallJumpTicks &&
                  horizontal_speed >= kMantleWallJumpMinSpeed) {
                std::cout << "[SPLIT][Recognizer] Mantle wall jump at ("
                          << sample->position[0] << ", "
                          << sample->position[1] << ", "
                          << sample->position[2] << ") speed="
                          << horizontal_speed << "\n";
                if (recorder.Stats().recording) {
                  std::scoped_lock lock(route_cues_mutex);
                  route_cues.push_back({"Mantle wall jump", sample->position, frame});
                }
              }
            }

            previous_movement_sample = *sample;

            if (frame % 60 == 0 && !race_timer.Course() &&
                !native_route_line_active.load(std::memory_order_acquire) &&
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
                  native_route_line_active.store(true, std::memory_order_release);

                  // Hyperline-style RouteLine: fit the recorded trajectory once
                  // and own its native particle segments for the whole race scene.
                  const auto route_line = RoutePolyline(route_ticks, 128);
                  std::size_t tracer_segments = 0;
                  std::string tracer_error;
                  if (route_line.size() >= 2) {
                    for (std::size_t i = 1; i < route_line.size(); ++i) {
                      tracer_error.clear();
                      if (!visual_renderer.DrawBeamSegment(
                              route_line[i - 1], route_line[i], tracer_error)) {
                        std::cout << "[SPLIT][RouteLine] segment "
                                  << (i - 1) << " failed: "
                                  << tracer_error << "\n";
                        break;
                      }
                      ++tracer_segments;
                    }
                  }

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

                  // Direct native movement facts become the first reliable
                  // Hyperline-style route callouts. Compound techniques are
                  // added later by the recognizer, not guessed here.
                  std::vector<MovementCue> callouts;
                  {
                    std::scoped_lock lock(route_cues_mutex);
                    const auto exact =
                        RecognizeRecordedTechniques(
                            route_ticks, route_cues,
                            technique_diagnostics.load(std::memory_order_acquire));
                    callouts = ConsolidateMovementCues(
                        MergeRecognizedTechniques(route_cues, exact));
                  }
                  std::size_t callout_count = 0;
                  for (const auto& cue : callouts) {
                    marker_error.clear();
                    if (visual_renderer.DrawWorldTextMarker(
                            cue.position, cue.label.c_str(),
                            3.0f, 0xffff4dd2u, marker_error)) {
                      ++callout_count;
                    } else {
                      std::cout << "[SPLIT][Callouts] " << cue.label
                                << " failed: " << marker_error << "\n";
                    }
                  }

                  // Checkpoints remain internal timing/split anchors. Hyperline
                  // does not expose generic CP1/CP2 labels on the route line.
                  std::cout << "[SPLIT][WorldEffects] persistent race objects created"
                            << " startFinish="
                            << (show_rings.load(std::memory_order_relaxed) ? "on" : "off")
                            << " tracerSegments=" << tracer_segments
                            << " callouts=" << callout_count
                            << " internalSplits=" << course.checkpoints.size()
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
            pawn_announced = false;
            practice_shop_ready = false;
            pre_spawn_cleanup_passes_remaining = 0;
            next_pre_spawn_cleanup_frame = 0;
            std::cout << "[SPLIT][Movement] loading world: " << map << "\n";
          },
          [&](std::string_view map) {
            // Hyperline removes map NPC entities directly rather than killing
            // them. Arm a few silent ent_remove_all passes while no player pawn
            // exists yet, so objectives never visibly explode on entry.
            pre_spawn_cleanup_passes_remaining = 3;
            next_pre_spawn_cleanup_frame = hooks.FrameCount() + 1;
            std::cout << "[SPLIT][Movement] world ready: " << map
                      << " (silent cleanup armed)\n";
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
              practice_shop_ready = false;
              pre_spawn_cleanup_passes_remaining = 0;
              next_pre_spawn_cleanup_frame = 0;
            }
          },
          [&](int slot, std::string_view command) {
            if (slot != human_slot.load(std::memory_order_relaxed)) return false;
            auto dispatch = [&](std::string_view runtime_command) {
              const auto response = handle_command(runtime_command);
              if (!response.empty()) {
                std::cout << "[SPLIT][Panorama] " << response << "\n";
              }
              return true;
            };
            if (command == "split_record") return dispatch("recordtoggle");
            if (command == "split_retry") return dispatch("retry");
            if (command == "split_replay") return dispatch("replay");
            if (command == "split_race") return dispatch("race");
            if (command == "split_practice_stamina 1") return dispatch("practice stamina on");
            if (command == "split_practice_stamina 0") return dispatch("practice stamina off");
            if (command == "split_practice_cooldowns 1") return dispatch("practice cooldowns on");
            if (command == "split_practice_cooldowns 0") return dispatch("practice cooldowns off");
            if (command == "split_practice_ammo 1") return dispatch("practice ammo on");
            if (command == "split_practice_ammo 0") return dispatch("practice ammo off");
            return false;
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
