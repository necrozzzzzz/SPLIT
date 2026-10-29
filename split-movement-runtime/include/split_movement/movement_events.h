#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace split::movement {

enum class MovementExecution {
  kLanded,
  kZiplineAttached,
  kGroundDash,
  kSlide,
  kBouncePad,
  kDashJump,
  kAirJump,
  kWallJump,
  kAirDash,
  kMeleeStarted,
  kAbilityExecuted,
};

const char* MovementExecutionName(MovementExecution execution);

struct MovementExecutedEvent {
  MovementExecution execution{};
  std::uint32_t pawn_handle = 0;
};

class MovementEvents {
 public:
  MovementEvents();
  ~MovementEvents();

  MovementEvents(const MovementEvents&) = delete;
  MovementEvents& operator=(const MovementEvents&) = delete;

  bool Install(std::string& error);
  std::vector<MovementExecutedEvent> Drain(std::uint32_t pawn_handle);
  void Clear();

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
