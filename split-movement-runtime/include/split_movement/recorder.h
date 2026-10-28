#pragma once

#include <cstdint>
#include <expected>
#include <filesystem>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "split_movement/input_capture.h"
#include "split_movement/pawn_probe.h"

namespace split::movement {

struct RecordedTick {
  std::uint64_t frame = 0;
  PawnSample pawn;
  std::optional<InputSample> input;
};

struct RecordingStats {
  bool recording = false;
  std::size_t ticks = 0;
  std::size_t ticks_with_input = 0;
  double distance = 0.0;
  double max_speed = 0.0;
  std::uint64_t first_frame = 0;
  std::uint64_t last_frame = 0;
};

class MovementRecorder {
 public:
  void Start();
  RecordingStats Stop();
  void Clear();

  void Push(std::uint64_t frame, const PawnSample& sample,
            std::optional<InputSample> input);

  RecordingStats Stats() const;
  std::expected<std::filesystem::path, std::string> SaveJson(
      const std::filesystem::path& directory, std::string_view map_name) const;

 private:
  mutable std::mutex mutex_;
  bool recording_ = false;
  std::vector<RecordedTick> ticks_;
  double distance_ = 0.0;
  double max_speed_ = 0.0;
};
}  // namespace split::movement
