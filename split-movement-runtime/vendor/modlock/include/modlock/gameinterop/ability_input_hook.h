#pragma once

#include <array>
#include <cstdint>
#include <expected>
#include <functional>
#include <memory>
#include <string>

#include "modlock/export.h"
#include "modlock/gameinterop/game_symbols.h"
#include "modlock/gameinterop/native_memory.h"

namespace modlock::gameinterop {

// AbilityInputHook filters selected buttons before native ability execution.
// Installation, callbacks and destruction belong to the engine thread. The
// handler can remap button states and returns the bits it consumes afterward.
class MODLOCK_API AbilityInputHook {
 public:
  struct Input {
    int32_t slot = -1;
    uint32_t controller_handle = 0;
    uint64_t steam_id = 0;
    uint32_t session_generation = 0;
    std::array<uint64_t, 3> buttons{};
  };

  using Handler = std::function<uint64_t(Input&)>;

  struct Layout {
    size_t controller = 0;
    size_t movement = 0;
    size_t buttons = 0;
  };

  // Process filters held, changed and scroll states together through the
  // schema-resolved layout. Non-player pawns do not call the handler.
  static std::expected<void, std::string> Process(void* pawn, const Layout& layout,
                                                  const BoundedReader& read,
                                                  const BoundedWriter& write,
                                                  const Handler& handler);

  // Install includes native bot commands only when include_bots is true.
  // Several owners may install at once and share one detour. Each handler sees
  // the buttons earlier owners remapped, and their consumed bits combine.
  static std::expected<AbilityInputHook, std::string> Install(
      const ModuleImage& server, void* schema_system, Handler handler,
      std::function<void(std::string)> failure, bool include_bots = false);

  AbilityInputHook(AbilityInputHook&&) noexcept;
  AbilityInputHook& operator=(AbilityInputHook&&) noexcept;
  ~AbilityInputHook();
  AbilityInputHook(const AbilityInputHook&) = delete;
  AbilityInputHook& operator=(const AbilityInputHook&) = delete;

 private:
  struct Impl;
  explicit AbilityInputHook(std::unique_ptr<Impl> impl);
  std::unique_ptr<Impl> impl_;
};

}  // namespace modlock::gameinterop
