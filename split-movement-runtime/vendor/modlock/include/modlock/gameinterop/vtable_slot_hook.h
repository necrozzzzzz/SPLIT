#pragma once

#include <cstddef>
#include <expected>
#include <string>

#include "modlock/export.h"

namespace modlock::gameinterop {

// VtableSlotHook redirects one entry of a live dispatch table to a replacement
// target and reports the displaced original. It patches the entry inside the
// table the object already dispatches through, so slots modlock never names keep
// dispatching exactly as the module built them - including slots past any
// index modlock knows about. The table belongs to the class, so every instance
// sharing it dispatches through the replacement; the engine interfaces modlock
// hooks are process singletons. Destruction writes the original entry back.
// The module's table must outlive the hook. Hooks stacked on the same slot
// must be destroyed in reverse installation order; mutation is not synchronized.
class MODLOCK_API VtableSlotHook {
 public:
  // Install patches slot of instance's dispatch table to replacement. The
  // slot index comes from the recorded memory database; a dispatch table
  // carries no length, so an out-of-range index cannot be detected here and
  // would corrupt the module. The error names the rejected shape.
  static std::expected<VtableSlotHook, std::string> Install(void* instance, size_t slot,
                                                            void* replacement);

  VtableSlotHook(VtableSlotHook&& other) noexcept;
  VtableSlotHook& operator=(VtableSlotHook&& other) noexcept;
  ~VtableSlotHook();

  // Original returns the displaced target for the replacement's call-through.
  [[nodiscard]] void* Original() const { return original_; }

  // Restore writes the original entry back into the table. Idempotent.
  void Restore();

 private:
  VtableSlotHook() = default;

  void** slot_ = nullptr;
  void* original_ = nullptr;
};

}  // namespace modlock::gameinterop
