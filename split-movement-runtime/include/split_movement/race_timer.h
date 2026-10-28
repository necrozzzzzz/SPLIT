#pragma once

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>

namespace split::movement {

struct RaceCourse {
  std::array<float, 3> start{};
  std::array<float, 3> finish{};
  float radius = 120.0f;
  std::string map;
};

struct RaceStatus {
  enum class Phase {
    kIdle,
    kArmed,
    kRunning,
    kFinished,
  };

  Phase phase = Phase::kIdle;
  double elapsed_seconds = 0.0;
  std::optional<double> last_seconds;
  std::optional<double> personal_best_seconds;
  bool last_was_pb = false;
  std::uint64_t course_id = 0;
};

struct RaceEvent {
  enum class Type {
    kNone,
    kStarted,
    kFinished,
  };

  Type type = Type::kNone;
  double elapsed_seconds = 0.0;
  std::optional<double> previous_pb_seconds;
  bool personal_best = false;
};

class RaceTimer {
 public:
  explicit RaceTimer(std::filesystem::path storage_directory);

  bool Configure(const RaceCourse& course, std::string& error);
  bool Arm(std::string& error);
  void Cancel();

  RaceEvent Observe(const std::array<float, 3>& position);
  RaceStatus Status() const;
  std::optional<RaceCourse> Course() const;

 private:
  static bool Inside(const std::array<float, 3>& position,
                     const std::array<float, 3>& center, float radius);
  static std::uint64_t CourseId(const RaceCourse& course);

  void LoadPbLocked();
  void SavePbLocked() const;

  mutable std::mutex mutex_;
  std::filesystem::path storage_directory_;
  std::optional<RaceCourse> course_;
  RaceStatus status_;
  bool was_inside_start_ = false;
  bool left_start_after_begin_ = false;
  std::chrono::steady_clock::time_point started_at_{};
};

}  // namespace split::movement
