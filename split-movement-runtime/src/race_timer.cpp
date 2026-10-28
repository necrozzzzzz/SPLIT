#include "split_movement/race_timer.h"

#include <cmath>
#include <fstream>
#include <iomanip>
#include <utility>

namespace split::movement {
namespace {

void HashBytes(std::uint64_t& hash, const void* data, std::size_t size) {
  constexpr std::uint64_t kPrime = 1099511628211ull;
  const auto* bytes = static_cast<const unsigned char*>(data);
  for (std::size_t i = 0; i < size; ++i) {
    hash ^= bytes[i];
    hash *= kPrime;
  }
}

std::int32_t Quantize(float value) {
  return static_cast<std::int32_t>(std::lround(value * 10.0f));
}

}  // namespace

RaceTimer::RaceTimer(std::filesystem::path storage_directory)
    : storage_directory_(std::move(storage_directory)) {}

bool RaceTimer::Configure(const RaceCourse& course, std::string& error) {
  if (course.map.empty()) {
    error = "race course has no map";
    return false;
  }
  if (!(course.radius >= 32.0f && course.radius <= 512.0f)) {
    error = "race radius must be between 32 and 512 units";
    return false;
  }

  const double dx = static_cast<double>(course.start[0]) - course.finish[0];
  const double dy = static_cast<double>(course.start[1]) - course.finish[1];
  const double dz = static_cast<double>(course.start[2]) - course.finish[2];
  const double separation = std::sqrt(dx * dx + dy * dy + dz * dz);
  if (separation < static_cast<double>(course.radius) * 2.0) {
    error = "route start and finish are too close for timing";
    return false;
  }

  std::scoped_lock lock(mutex_);
  course_ = course;
  status_ = RaceStatus{};
  status_.course_id = CourseId(course);
  was_inside_start_ = false;
  left_start_after_begin_ = false;
  LoadPbLocked();
  return true;
}

bool RaceTimer::Arm(std::string& error) {
  std::scoped_lock lock(mutex_);
  if (!course_) {
    error = "no route is configured";
    return false;
  }

  status_.phase = RaceStatus::Phase::kArmed;
  status_.elapsed_seconds = 0.0;
  status_.last_seconds.reset();
  status_.current_splits.clear();
  status_.next_checkpoint = 0;
  status_.last_was_pb = false;
  was_inside_start_ = false;
  left_start_after_begin_ = false;
  return true;
}

void RaceTimer::Cancel() {
  std::scoped_lock lock(mutex_);
  status_.phase = RaceStatus::Phase::kIdle;
  status_.elapsed_seconds = 0.0;
  status_.current_splits.clear();
  status_.next_checkpoint = 0;
  status_.last_was_pb = false;
  was_inside_start_ = false;
  left_start_after_begin_ = false;
}

RaceEvent RaceTimer::Observe(const std::array<float, 3>& position) {
  std::scoped_lock lock(mutex_);
  RaceEvent event;
  if (!course_) return event;

  const bool in_start = Inside(position, course_->start, course_->radius);
  const auto now = std::chrono::steady_clock::now();

  if (status_.phase == RaceStatus::Phase::kArmed) {
    if (in_start && !was_inside_start_) {
      started_at_ = now;
      status_.phase = RaceStatus::Phase::kRunning;
      status_.elapsed_seconds = 0.0;
      status_.current_splits.clear();
      status_.next_checkpoint = 0;
      left_start_after_begin_ = false;
      event.type = RaceEvent::Type::kStarted;
      event.checkpoint_count = course_->checkpoints.size();
    }
    was_inside_start_ = in_start;
    return event;
  }

  if (status_.phase != RaceStatus::Phase::kRunning) {
    was_inside_start_ = in_start;
    return event;
  }

  status_.elapsed_seconds =
      std::chrono::duration<double>(now - started_at_).count();
  if (!in_start) left_start_after_begin_ = true;

  // Checkpoints are strictly ordered. This prevents cutting across a route and
  // accidentally hitting a later checkpoint first.
  if (status_.next_checkpoint < course_->checkpoints.size()) {
    const auto index = status_.next_checkpoint;
    if (Inside(position, course_->checkpoints[index], course_->radius)) {
      const double split = status_.elapsed_seconds;
      status_.current_splits.push_back(split);
      ++status_.next_checkpoint;

      event.type = RaceEvent::Type::kSplit;
      event.elapsed_seconds = split;
      event.checkpoint_index = index;
      event.checkpoint_count = course_->checkpoints.size();

      if (index < status_.personal_best_splits.size()) {
        event.pb_split_seconds = status_.personal_best_splits[index];
        event.split_delta_seconds =
            split - status_.personal_best_splits[index];
      }
      was_inside_start_ = in_start;
      return event;
    }
  }

  const bool all_checkpoints =
      status_.next_checkpoint >= course_->checkpoints.size();
  const bool in_finish =
      Inside(position, course_->finish, course_->radius);

  if (left_start_after_begin_ && all_checkpoints && in_finish) {
    event.type = RaceEvent::Type::kFinished;
    event.elapsed_seconds = status_.elapsed_seconds;
    event.previous_pb_seconds = status_.personal_best_seconds;
    event.checkpoint_count = course_->checkpoints.size();

    const bool pb = !status_.personal_best_seconds ||
                    status_.elapsed_seconds < *status_.personal_best_seconds;
    if (pb) {
      status_.personal_best_seconds = status_.elapsed_seconds;
      status_.personal_best_splits = status_.current_splits;
      SavePbLocked();
    }

    status_.last_seconds = status_.elapsed_seconds;
    status_.last_was_pb = pb;
    status_.phase = RaceStatus::Phase::kFinished;
    event.personal_best = pb;
  }

  was_inside_start_ = in_start;
  return event;
}

RaceStatus RaceTimer::Status() const {
  std::scoped_lock lock(mutex_);
  auto status = status_;
  if (status.phase == RaceStatus::Phase::kRunning) {
    status.elapsed_seconds =
        std::chrono::duration<double>(
            std::chrono::steady_clock::now() - started_at_)
            .count();
  }
  return status;
}

std::optional<RaceCourse> RaceTimer::Course() const {
  std::scoped_lock lock(mutex_);
  return course_;
}

bool RaceTimer::Inside(const std::array<float, 3>& position,
                       const std::array<float, 3>& center, float radius) {
  const double x = static_cast<double>(position[0]) - center[0];
  const double y = static_cast<double>(position[1]) - center[1];
  const double z = static_cast<double>(position[2]) - center[2];
  return x * x + y * y + z * z <= static_cast<double>(radius) * radius;
}

std::uint64_t RaceTimer::CourseId(const RaceCourse& course) {
  std::uint64_t hash = 1469598103934665603ull;
  HashBytes(hash, course.map.data(), course.map.size());

  const auto hash_point = [&](const std::array<float, 3>& point) {
    for (float value : point) {
      const auto q = Quantize(value);
      HashBytes(hash, &q, sizeof(q));
    }
  };

  hash_point(course.start);
  for (const auto& checkpoint : course.checkpoints) {
    hash_point(checkpoint);
  }
  hash_point(course.finish);
  return hash;
}

void RaceTimer::LoadPbLocked() {
  status_.personal_best_seconds.reset();
  status_.personal_best_splits.clear();

  const auto path =
      storage_directory_ / ("pb-" + std::to_string(status_.course_id) + ".txt");
  std::ifstream input(path);

  double seconds = 0.0;
  if (!(input >> seconds) || !(seconds > 0.0) || !std::isfinite(seconds)) {
    return;
  }
  status_.personal_best_seconds = seconds;

  double split = 0.0;
  while (input >> split) {
    if (split > 0.0 && std::isfinite(split)) {
      status_.personal_best_splits.push_back(split);
    }
  }

  // Old PB files contained only the finish time; they remain valid, just
  // without split comparisons until a new PB is set on this route.
  if (course_ &&
      status_.personal_best_splits.size() != course_->checkpoints.size()) {
    status_.personal_best_splits.clear();
  }
}

void RaceTimer::SavePbLocked() const {
  if (!status_.personal_best_seconds) return;

  std::error_code ec;
  std::filesystem::create_directories(storage_directory_, ec);
  if (ec) return;

  const auto path =
      storage_directory_ / ("pb-" + std::to_string(status_.course_id) + ".txt");
  std::ofstream output(path, std::ios::trunc);
  if (!output) return;

  output << std::fixed << std::setprecision(6)
         << *status_.personal_best_seconds << "\n";
  for (double split : status_.personal_best_splits) {
    output << split << "\n";
  }
}

}  // namespace split::movement
