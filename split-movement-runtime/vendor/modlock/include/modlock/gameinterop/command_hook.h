#pragma once

#include <array>
#include <cstdint>
#include <expected>
#include <functional>
#include <memory>
#include <optional>
#include <string>

#include "modlock/export.h"
#include "modlock/gameinterop/game_symbols.h"

namespace modlock::gameinterop {

// CommandHook reads every hero's commands and replaces a bot's input with a
// session's. Every hero's commands reach movement through
// CBasePlayerController::ProcessUsercmds, a bot's as one command its brain
// built this tick, so a command rewritten there runs as a player's own:
// movement, movement abilities, casts and animation all follow it. Retain it
// until the engine stops. The callback runs on the engine thread and must
// outlive the hook; only one may be installed.
class MODLOCK_API CommandHook {
 public:
  // Command is a player's controls for one command.
  struct Command {
    // Angles are pitch, yaw and roll in degrees.
    std::array<float, 3> angles{};
    // Forward and Left are the movement axes, from -1 to 1.
    float forward = 0;
    float left = 0;
    // Held are the buttons down; Pressed went down in this command, even when
    // already held.
    uint64_t held = 0;
    uint64_t pressed = 0;
  };

  // Decide returns the command for the controller in slot, or nothing to
  // leave its own.
  using Decide = std::function<std::optional<Command>(int32_t slot)>;

  static std::expected<CommandHook, std::string> Install(const ModuleImage& server, Decide decide);

  // CameraPosition is where the camera of the player in slot was when the
  // client built its latest commands: the origin the reticle's ray starts
  // from, shoulder offset included. It is absent for a bot and before the
  // player's first command.
  std::optional<std::array<float, 3>> CameraPosition(int32_t slot) const;
  CommandHook(CommandHook&&) noexcept;
  CommandHook& operator=(CommandHook&&) noexcept;
  ~CommandHook();

 private:
  struct Impl;
  explicit CommandHook(std::unique_ptr<Impl> impl);
  std::unique_ptr<Impl> impl_;
};

}  // namespace modlock::gameinterop
