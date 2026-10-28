#pragma once

#include <array>
#include <expected>
#include <string>

#include "modlock/export.h"
#include "modlock/gameinterop/game_symbols.h"

namespace modlock::gameinterop {

// BotCreation borrows the server's native bot factory until the module unloads.
// Calls run on the engine thread with ConnectionTracker installed.
class MODLOCK_API BotCreation {
 public:
  static std::expected<BotCreation, std::string> Resolve(const ModuleImage& server);

  // Create returns the newly published bot connection slot. Hero selection can
  // finish on a later frame; the caller must observe it before claiming a pawn.
  // The initial position prevents a newly created bot from appearing at world
  // origin before the first playback sample is applied.
  std::expected<int32_t, std::string> Create(const char* name, int32_t team, uint32_t hero_id,
                                             const std::array<float, 3>& position = {}) const;

 private:
  void* (*create_)(const char*, int32_t, uint32_t, const float*) = nullptr;
};

}  // namespace modlock::gameinterop
