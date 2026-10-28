#include "split_movement/recorder.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <fstream>
#include <iomanip>
#include <sstream>

namespace split::movement {

namespace {

double Length3(const std::array<float, 3>& v) {
  return std::sqrt(static_cast<double>(v[0]) * v[0] +
                   static_cast<double>(v[1]) * v[1] +
                   static_cast<double>(v[2]) * v[2]);
}

double Distance3(const std::array<float, 3>& a, const std::array<float, 3>& b) {
  const double x = static_cast<double>(a[0]) - b[0];
  const double y = static_cast<double>(a[1]) - b[1];
  const double z = static_cast<double>(a[2]) - b[2];
  return std::sqrt(x * x + y * y + z * z);
}

std::string SafeMapName(std::string_view map) {
  std::string out;
  out.reserve(map.size());
  for (char c : map) {
    const bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
                    (c >= '0' && c <= '9') || c == '-' || c == '_';
    out.push_back(ok ? c : '_');
  }
  return out.empty() ? "unknown" : out;
}

}  // namespace

void MovementRecorder::Start() {
  std::scoped_lock lock(mutex_);
  ticks_.clear();
  distance_ = 0.0;
  max_speed_ = 0.0;
  recording_ = true;
}

RecordingStats MovementRecorder::Stop() {
  std::scoped_lock lock(mutex_);
  recording_ = false;

  RecordingStats stats;
  stats.recording = false;
  stats.ticks = ticks_.size();
  stats.distance = distance_;
  stats.max_speed = max_speed_;
  if (!ticks_.empty()) {
    stats.first_frame = ticks_.front().frame;
    stats.last_frame = ticks_.back().frame;
  }
  return stats;
}

void MovementRecorder::Clear() {
  std::scoped_lock lock(mutex_);
  recording_ = false;
  ticks_.clear();
  distance_ = 0.0;
  max_speed_ = 0.0;
}

void MovementRecorder::Push(std::uint64_t frame, const PawnSample& sample) {
  std::scoped_lock lock(mutex_);
  if (!recording_) return;

  if (!ticks_.empty()) {
    distance_ += Distance3(ticks_.back().pawn.position, sample.position);
  }
  max_speed_ = std::max(max_speed_, Length3(sample.velocity));
  ticks_.push_back(RecordedTick{frame, sample});
}

RecordingStats MovementRecorder::Stats() const {
  std::scoped_lock lock(mutex_);

  RecordingStats stats;
  stats.recording = recording_;
  stats.ticks = ticks_.size();
  stats.distance = distance_;
  stats.max_speed = max_speed_;
  if (!ticks_.empty()) {
    stats.first_frame = ticks_.front().frame;
    stats.last_frame = ticks_.back().frame;
  }
  return stats;
}

std::expected<std::filesystem::path, std::string> MovementRecorder::SaveJson(
    const std::filesystem::path& directory, std::string_view map_name) const {
  std::vector<RecordedTick> ticks;
  double distance = 0.0;
  double max_speed = 0.0;
  {
    std::scoped_lock lock(mutex_);
    if (ticks_.empty()) {
      return std::unexpected("there is no recorded route to save");
    }
    ticks = ticks_;
    distance = distance_;
    max_speed = max_speed_;
  }

  std::error_code ec;
  std::filesystem::create_directories(directory, ec);
  if (ec) {
    return std::unexpected("could not create route directory: " + ec.message());
  }

  const auto stamp = std::chrono::duration_cast<std::chrono::milliseconds>(
                         std::chrono::system_clock::now().time_since_epoch())
                         .count();

  const auto path = directory /
                    ("route-" + SafeMapName(map_name) + "-" + std::to_string(stamp) + ".json");

  std::ofstream out(path, std::ios::binary);
  if (!out) {
    return std::unexpected("could not open route file for writing");
  }

  out << std::fixed << std::setprecision(4);
  out << "{\n";
  out << "  \"format\": \"split-movement-route\",\n";
  out << "  \"version\": 1,\n";
  out << "  \"map\": \"" << SafeMapName(map_name) << "\",\n";
  out << "  \"tick_count\": " << ticks.size() << ",\n";
  out << "  \"first_frame\": " << ticks.front().frame << ",\n";
  out << "  \"last_frame\": " << ticks.back().frame << ",\n";
  out << "  \"distance\": " << distance << ",\n";
  out << "  \"max_speed\": " << max_speed << ",\n";
  out << "  \"ticks\": [\n";

  for (std::size_t i = 0; i < ticks.size(); ++i) {
    const auto& tick = ticks[i];
    out << "    {\"frame\":" << tick.frame
        << ",\"pos\":[" << tick.pawn.position[0] << "," << tick.pawn.position[1] << ","
        << tick.pawn.position[2] << "]"
        << ",\"vel\":[" << tick.pawn.velocity[0] << "," << tick.pawn.velocity[1] << ","
        << tick.pawn.velocity[2] << "]"
        << ",\"grounded\":" << (tick.pawn.grounded ? "true" : "false")
        << ",\"pawn_handle\":" << tick.pawn.pawn_handle << "}";
    if (i + 1 != ticks.size()) out << ",";
    out << "\n";
  }

  out << "  ]\n";
  out << "}\n";

  if (!out) {
    return std::unexpected("route file write failed");
  }
  return path;
}

}  // namespace split::movement
