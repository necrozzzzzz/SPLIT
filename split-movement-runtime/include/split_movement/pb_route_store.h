#pragma once

#include <cstdint>
#include <expected>
#include <filesystem>
#include <string>
#include <vector>

#include "split_movement/recorder.h"

namespace split::movement {

class PbRouteStore {
 public:
  explicit PbRouteStore(std::filesystem::path directory);

  std::expected<void, std::string> Save(
      std::uint64_t course_id, const std::vector<RecordedTick>& ticks) const;
  std::expected<std::vector<RecordedTick>, std::string> Load(
      std::uint64_t course_id) const;
  bool Exists(std::uint64_t course_id) const;

 private:
  std::filesystem::path Path(std::uint64_t course_id) const;

  std::filesystem::path directory_;
};

}  // namespace split::movement
