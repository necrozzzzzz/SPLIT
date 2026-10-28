#pragma once

#include <array>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>

namespace split::movement {

struct PawnSample {
  int slot = -1;
  std::uint32_t pawn_handle = 0;
  std::uint32_t hero_id = 0;
  std::uint8_t team = 0;
  std::array<float, 3> position{};
  std::array<float, 3> velocity{};
  bool grounded = false;
};

class PawnProbe {
 public:
  explicit PawnProbe(void* schema_system);
  ~PawnProbe();

  PawnProbe(const PawnProbe&) = delete;
  PawnProbe& operator=(const PawnProbe&) = delete;

  std::optional<PawnSample> Observe(int slot, std::string& error);

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
