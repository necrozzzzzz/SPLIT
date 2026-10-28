#include "split_movement/race_timer.h"

#include <cmath>
#include <fstream>
#include <iomanip>
#include <sstream>

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
  status_.last_was_pb = false;
  was_inside_start_ = false;
  left_start_after_begin_ = false;
  return true;
}

void RaceTimer::Cancel() {
  std::scoped_lock lock(mutex_);
  status_.phase = RaceStatus::Phase::kIdle;
  status_.elapsed_seconds = 0.0;
  status_.last_was_pb = false;
  was_inside_start_ = false;
  left_start_after_begin_ = false;
}

RaceEvent RaceTimer::Observe(const std::array<float, 3>& position) {
  std::scoped_lock lock(mutex_);
  RaceEvent event;
  if (!course_) return event;

  const bool in_start = Inside(position, course_->start, course_->radius);
  const bool in_finish = Inside(position, course_->finish, course_->radius);
  const auto now = std::chrono::steady_clock::now();

  if (status_.phase == RaceStatus::Phase::kArmed) {
    // Start on entry into the start sphere. Tracking the previous state avoids
    // repeatedly starting while the player stands inside it.
    if (in_start && !was_inside_start_) {
      started_at_ = now;
      status_.phase = RaceStatus::Phase::kRunning;
      status_.elapsed_seconds = 0.0;
      left_start_after_begin_ = false;
      event.type = RaceEvent::Type::kStarted;
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

  // Require leaving the start area first, so a route whose finish volume
  // happens to overlap nearby geometry cannot instantly complete.
  if (left_start_after_begin_ && in_finish) {
    event.type = RaceEvent::Type::kFinished;
    event.elapsed_seconds = status_.elapsed_seconds;
    event.previous_pb_seconds = status_.personal_best_seconds;

    const bool pb = !status_.personal_best_seconds ||
                    status_.elapsed_seconds < *status_.personal_best_seconds;
    if (pb) {
      status_.personal_best_seconds = status_.elapsed_seconds;
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
        std::chrono::duration<double>(std::chrono::steady_clock::now() - started_at_).count();
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
  for (float value : course.start) {
    const auto q = Quantize(value);
    HashBytes(hash, &q, sizeof(q));
  }
  for (float value : course.finish) {
    const auto q = Quantize(value);
    HashBytes(hash, &q, sizeof(q));
  }
  return hash;
}

void RaceTimer::LoadPbLocked() {
  status_.personal_best_seconds.reset();
  const auto path =
      storage_directory_ / ("pb-" + std::to_string(status_.course_id) + ".txt");
  std::ifstream input(path);
  double seconds = 0.0;
  if (input >> seconds && seconds > 0.0 && std::isfinite(seconds)) {
    status_.personal_best_seconds = seconds;
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
}

}  // namespace split::movement
