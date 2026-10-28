#include "split_movement/native_hooks.h"

#include <atomic>
#include <utility>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {

namespace {

#if defined(_WIN32)

using CreateInterfaceFn = void* (*)(const char*, int*);

void* ResolveInterface(const wchar_t* module_name, const char* version, std::string& error) {
  HMODULE module = GetModuleHandleW(module_name);
  if (!module) {
    error = "module is not mapped";
    return nullptr;
  }

  const auto factory =
      reinterpret_cast<CreateInterfaceFn>(GetProcAddress(module, "CreateInterface"));
  if (!factory) {
    error = "CreateInterface export is missing";
    return nullptr;
  }

  int code = 0;
  void* value = factory(version, &code);
  if (!value) {
    error = std::string("CreateInterface(") + version + ") returned null";
  }
  return value;
}

class VtablePatch {
 public:
  VtablePatch() = default;
  ~VtablePatch() { Reset(); }

  VtablePatch(const VtablePatch&) = delete;
  VtablePatch& operator=(const VtablePatch&) = delete;

  bool Install(void* object, std::size_t slot, void* replacement, std::string& error) {
    if (!object) {
      error = "cannot hook a null interface";
      return false;
    }

    auto*** object_ptr = reinterpret_cast<void***>(object);
    void** table = *object_ptr;
    if (!table) {
      error = "interface has a null vtable";
      return false;
    }

    slot_ = &table[slot];
    DWORD old_protection = 0;
    if (!VirtualProtect(slot_, sizeof(void*), PAGE_EXECUTE_READWRITE, &old_protection)) {
      error = "VirtualProtect failed with Windows error " + std::to_string(GetLastError());
      slot_ = nullptr;
      return false;
    }

    original_ = *slot_;
    *slot_ = replacement;
    FlushInstructionCache(GetCurrentProcess(), slot_, sizeof(void*));

    DWORD ignored = 0;
    VirtualProtect(slot_, sizeof(void*), old_protection, &ignored);
    return true;
  }

  void* Original() const { return original_; }

  void Reset() {
    if (!slot_ || !original_) return;

    DWORD old_protection = 0;
    if (VirtualProtect(slot_, sizeof(void*), PAGE_EXECUTE_READWRITE, &old_protection)) {
      *slot_ = original_;
      FlushInstructionCache(GetCurrentProcess(), slot_, sizeof(void*));
      DWORD ignored = 0;
      VirtualProtect(slot_, sizeof(void*), old_protection, &ignored);
    }

    slot_ = nullptr;
    original_ = nullptr;
  }

 private:
  void** slot_ = nullptr;
  void* original_ = nullptr;
};

using GameFrameFn = void (*)(void*, bool, bool, bool);
using StartupServerFn = void (*)(void*, const void*, void*, const char*);

GameFrameFn g_game_frame_original = nullptr;
StartupServerFn g_startup_original = nullptr;
NativeHooks::FrameCallback g_frame_callback;
NativeHooks::WorldCallback g_before_world;
NativeHooks::WorldCallback g_after_world;
std::atomic<std::uint64_t>* g_frame_count = nullptr;

void GameFrameThunk(void* self, bool simulating, bool first_tick, bool last_tick) {
  if (g_game_frame_original) {
    g_game_frame_original(self, simulating, first_tick, last_tick);
  }

  if (g_frame_count) {
    const auto count = g_frame_count->fetch_add(1, std::memory_order_relaxed) + 1;
    if (g_frame_callback) g_frame_callback(count);
  }
}

void StartupServerThunk(void* self, const void* config, void* world_session, const char* map_name) {
  const std::string_view map = map_name ? std::string_view(map_name) : std::string_view();

  if (g_before_world) g_before_world(map);
  if (g_startup_original) g_startup_original(self, config, world_session, map_name);
  if (g_after_world) g_after_world(map);
}

#endif

}  // namespace

struct NativeHooks::Impl {
#if defined(_WIN32)
  VtablePatch frame;
  VtablePatch startup;
#endif
  std::atomic<std::uint64_t> frames{0};
  bool installed = false;
};

NativeHooks::NativeHooks() : impl_(std::make_unique<Impl>()) {}

NativeHooks::~NativeHooks() {
#if defined(_WIN32)
  // Restore dispatch entries before clearing the thunk state.
  impl_->startup.Reset();
  impl_->frame.Reset();

  g_game_frame_original = nullptr;
  g_startup_original = nullptr;
  g_frame_callback = {};
  g_before_world = {};
  g_after_world = {};
  g_frame_count = nullptr;
#endif
}

bool NativeHooks::Install(FrameCallback on_frame, WorldCallback before_world,
                          WorldCallback after_world, std::string& error) {
  if (impl_->installed) {
    error = "native hooks are already installed";
    return false;
  }

#if !defined(_WIN32)
  error = "native hooks currently require Windows";
  return false;
#else
  // These slot numbers are pinned by the vendored 2026-10-28 game data.
  // Later we will resolve/verify them from SPLIT's own signature database.
  constexpr std::size_t kGameFrameSlot = 19;
  constexpr std::size_t kStartupServerSlot = 26;

  std::string interface_error;
  void* server = ResolveInterface(L"server.dll", "Source2Server001", interface_error);
  if (!server) {
    error = "Source2Server001: " + interface_error;
    return false;
  }

  void* network =
      ResolveInterface(L"engine2.dll", "NetworkServerService_001", interface_error);
  if (!network) {
    error = "NetworkServerService_001: " + interface_error;
    return false;
  }

  if (!impl_->frame.Install(server, kGameFrameSlot,
                            reinterpret_cast<void*>(&GameFrameThunk), error)) {
    error = "GameFrame hook: " + error;
    return false;
  }
  g_game_frame_original = reinterpret_cast<GameFrameFn>(impl_->frame.Original());

  if (!impl_->startup.Install(network, kStartupServerSlot,
                              reinterpret_cast<void*>(&StartupServerThunk), error)) {
    impl_->frame.Reset();
    g_game_frame_original = nullptr;
    error = "StartupServer hook: " + error;
    return false;
  }
  g_startup_original = reinterpret_cast<StartupServerFn>(impl_->startup.Original());

  g_frame_callback = std::move(on_frame);
  g_before_world = std::move(before_world);
  g_after_world = std::move(after_world);
  g_frame_count = &impl_->frames;

  impl_->installed = true;
  return true;
#endif
}

bool NativeHooks::Installed() const {
  return impl_->installed;
}

std::uint64_t NativeHooks::FrameCount() const {
  return impl_->frames.load(std::memory_order_relaxed);
}

}  // namespace split::movement
