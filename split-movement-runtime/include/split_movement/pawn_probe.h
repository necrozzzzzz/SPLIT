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
  std::uint32_t ground_entity_handle = 0xffffffffu;
  std::array<float, 3> position{};
  std::array<float, 3> velocity{};
  bool grounded = false;

  // Exact movement state read from the player's native movement abilities.
  bool sliding = false;
  bool mantling = false;
  std::optional<float> mantle_start_time;
  std::optional<std::int8_t> consecutive_wall_jumps;
  std::optional<std::array<float, 3>> wall_contact_position;
  std::optional<std::array<float, 3>> wall_jump_normal;
  std::optional<std::uint16_t> wall_jump_facing;
  std::optional<float> zipline_time;
};

class PawnProbe {
 public:
  explicit PawnProbe(void* schema_system);
  ~PawnProbe();

  PawnProbe(const PawnProbe&) = delete;
  PawnProbe& operator=(const PawnProbe&) = delete;

  std::optional<PawnSample> Observe(int slot, std::string& error);
  bool SetGhostVisual(int slot, bool ghost, std::string& error);
  bool Teleport(int slot, const std::array<float, 3>& position,
                const std::array<float, 3>& angles, std::string& error);

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
