#pragma once

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace split::movement {

struct RaceCourse {
  std::array<float, 3> start{};
  std::array<float, 3> finish{};
  std::vector<std::array<float, 3>> checkpoints;
  float radius = 120.0f;
  std::string map;
  std::string name = "Recorded Route";
  std::string difficulty = "Unrated";
  std::uint32_t hero_id = 0;
  std::optional<double> reference_seconds;
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
  std::vector<double> current_splits;
  std::vector<double> personal_best_splits;
  std::size_t next_checkpoint = 0;
  bool last_was_pb = false;
  std::optional<double> last_delta_seconds;
  std::vector<double> last_split_deltas;
  std::uint64_t course_id = 0;
};

struct RaceEvent {
  enum class Type {
    kNone,
    kStarted,
    kSplit,
    kFinished,
  };

  Type type = Type::kNone;
  double elapsed_seconds = 0.0;
  std::size_t checkpoint_index = 0;
  std::size_t checkpoint_count = 0;
  std::optional<double> previous_pb_seconds;
  std::optional<double> pb_split_seconds;
  std::optional<double> split_delta_seconds;
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
