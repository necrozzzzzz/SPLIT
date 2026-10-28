#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "split_movement/recorder.h"

namespace split::movement {

struct SteeringStatus {
  bool active = false;
  int slot = -1;
  std::size_t index = 0;
  std::size_t total = 0;
};

class MovementSteering {
 public:
  explicit MovementSteering(void* schema_system);
  ~MovementSteering();

  MovementSteering(const MovementSteering&) = delete;
  MovementSteering& operator=(const MovementSteering&) = delete;

  bool Install(std::string& error);
  bool Start(int slot, std::vector<RecordedTick> ticks, std::string& error);
  void Stop();
  SteeringStatus Status() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
