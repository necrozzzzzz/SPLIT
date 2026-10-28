#pragma once

#include <expected>
#include <functional>
#include <string>

#include "modlock/export.h"
#include "modlock/gameinterop/thunk_owner.h"
#include "modlock/gameinterop/vtable_slot_hook.h"

namespace modlock::gameinterop {

// FrameCallback runs on the engine's server thread once per server frame,
// after ISource2Server::GameFrame returns.
using FrameCallback = std::function<void()>;

// EngineFrameHook joins modlock's frame work to the engine-owned frame clock.
// After the Source2Main handoff the engine drives frames, so plugin ticking
// rides the server's ISource2Server::GameFrame virtual. The hook resolves
// the interface through server.dll's CreateInterface export and patches its
// GameFrame entry to a thunk that runs
// the original first, then the callback. One hook may exist at a time; the
// move and destruction ordering is ThunkOwner's contract.
class MODLOCK_API EngineFrameHook {
 public:
  // Server.dll's ISource2Server interface version string.
  static constexpr char kServerInterfaceVersion[] = "Source2Server001";

  // Install resolves the mapped server.dll interface and hooks its GameFrame
  // slot. Failure names the missing piece (module, export, interface).
  static std::expected<EngineFrameHook, std::string> Install(FrameCallback on_frame);

  EngineFrameHook(EngineFrameHook&& other) noexcept = default;
  EngineFrameHook& operator=(EngineFrameHook&& other) noexcept = default;
  ~EngineFrameHook() = default;

 private:
  // Private by construction: only Install builds one, around a live slot hook.
  explicit EngineFrameHook(ThunkOwner owner);

  ThunkOwner owner_;
};

}  // namespace modlock::gameinterop
