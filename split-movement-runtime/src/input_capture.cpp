#include "split_movement/input_capture.h"

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>

#include <safetyhook.hpp>

namespace split::movement {
namespace {

constexpr std::size_t kIdentityOffset = 0x10;
constexpr std::size_t kIdentityEHandleOffset = 0x10;
constexpr std::size_t kIdentityFlagsOffset = 0x30;
constexpr std::uint32_t kEntityIndexBits = 0x7fff;
constexpr std::uint32_t kSerialShift = 15;
constexpr std::uint32_t kSerialBits = 0x1ffff;
constexpr std::uint32_t kEfInvalidEHandle = 0x1;
constexpr std::uint32_t kEfDeleteFlags = 0x10 | 0x200;
constexpr int kMaxSlots = 64;

constexpr std::size_t kCommandStride = 0xa8;
constexpr std::size_t kCommandHasBits = 0x20;
constexpr std::size_t kCommandBase = 0x48;
constexpr std::size_t kCommandButtons = 0x78;
constexpr std::size_t kBaseHasBits = 0x10;
constexpr std::size_t kBaseAngles = 0x40;
constexpr std::size_t kBaseForward = 0x58;
constexpr std::size_t kBaseLeft = 0x5c;
constexpr std::uint32_t kBaseAxesBits = 0x40 | 0x80;
constexpr std::uint32_t kBaseAnglesBit = 0x4;
constexpr std::size_t kAnglesHasBits = 0x10;
constexpr std::uint32_t kAnglesXyzBits = 0x7;
constexpr std::size_t kAnglesXyz = 0x18;
constexpr std::size_t kAnglesSize = 0x28;

constexpr std::string_view kProcessUsercmdsPattern =
    "48 8B C4 44 88 48 ?? 44 89 40 ?? 48 89 50 ?? 53";

template <typename T>
T Load(const std::byte* base, std::size_t offset) {
  T value{};
  std::memcpy(&value, base + offset, sizeof(value));
  return value;
}

template <typename T>
void Store(std::byte* base, std::size_t offset, const T& value) {
  std::memcpy(base + offset, &value, sizeof(value));
}

std::optional<std::uint32_t> ReferenceHandle(void* entity) {
  if (!entity) return std::nullopt;
  auto* identity = Load<std::byte*>(static_cast<std::byte*>(entity), kIdentityOffset);
  if (!identity || Load<void*>(identity, 0) != entity) return std::nullopt;

  const auto stored = Load<std::uint32_t>(identity, kIdentityEHandleOffset);
  const auto flags = Load<std::uint32_t>(identity, kIdentityFlagsOffset);
  if (stored == 0xffffffffu || (flags & kEfDeleteFlags) != 0) return std::nullopt;

  const auto serial =
      ((stored >> kSerialShift) - (flags & kEfInvalidEHandle)) & kSerialBits;
  return (stored & kEntityIndexBits) | (serial << kSerialShift);
}

int SlotOfController(void* controller) {
  const auto handle = ReferenceHandle(controller);
  if (!handle) return -1;
  const int slot = static_cast<int>(*handle & kEntityIndexBits) - 1;
  return slot >= 0 && slot < kMaxSlots ? slot : -1;
}

struct PatternByte {
  std::uint8_t value = 0;
  bool wildcard = false;
};

std::vector<PatternByte> ParsePattern(std::string_view pattern) {
  std::vector<PatternByte> bytes;
  for (std::size_t i = 0; i < pattern.size();) {
    while (i < pattern.size() && pattern[i] == ' ') ++i;
    if (i >= pattern.size()) break;
    if (pattern[i] == '?') {
      bytes.push_back({0, true});
      while (i < pattern.size() && pattern[i] == '?') ++i;
      continue;
    }
    auto nibble = [](char c) -> int {
      if (c >= '0' && c <= '9') return c - '0';
      if (c >= 'a' && c <= 'f') return c - 'a' + 10;
      if (c >= 'A' && c <= 'F') return c - 'A' + 10;
      return -1;
    };
    if (i + 1 >= pattern.size()) return {};
    const int hi = nibble(pattern[i]);
    const int lo = nibble(pattern[i + 1]);
    if (hi < 0 || lo < 0) return {};
    bytes.push_back({static_cast<std::uint8_t>((hi << 4) | lo), false});
    i += 2;
  }
  return bytes;
}

std::optional<std::size_t> ImageSize(HMODULE module) {
  if (!module) return std::nullopt;
  auto* base = reinterpret_cast<const std::byte*>(module);
  auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
  if (dos->e_magic != IMAGE_DOS_SIGNATURE) return std::nullopt;
  auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
  if (nt->Signature != IMAGE_NT_SIGNATURE) return std::nullopt;
  return static_cast<std::size_t>(nt->OptionalHeader.SizeOfImage);
}

void* FindUniquePattern(HMODULE module, std::string_view pattern, std::string& error) {
  const auto size = ImageSize(module);
  if (!size) {
    error = "could not read server.dll PE image size";
    return nullptr;
  }

  const auto parsed = ParsePattern(pattern);
  if (parsed.empty() || parsed.size() > *size) {
    error = "invalid process-usercmds signature";
    return nullptr;
  }

  auto* base = reinterpret_cast<const std::uint8_t*>(module);
  void* match = nullptr;
  std::size_t matches = 0;
  for (std::size_t at = 0; at + parsed.size() <= *size; ++at) {
    bool ok = true;
    for (std::size_t j = 0; j < parsed.size(); ++j) {
      if (!parsed[j].wildcard && base[at + j] != parsed[j].value) {
        ok = false;
        break;
      }
    }
    if (!ok) continue;
    match = const_cast<std::uint8_t*>(base + at);
    if (++matches > 1) break;
  }

  if (matches == 0) {
    error = "controller.process-usercmds signature was not found";
    return nullptr;
  }
  if (matches != 1) {
    error = "controller.process-usercmds signature was not unique";
    return nullptr;
  }
  return match;
}

struct SlotInput {
  mutable std::mutex mutex;
  std::optional<InputSample> sample;
};

struct DriveState {
  std::mutex mutex;
  int slot = -1;
  std::optional<InputSample> input;
  std::uint64_t last_held = 0;
};

std::array<SlotInput, kMaxSlots> g_slots;
DriveState g_drive;
safetyhook::InlineHook* g_hook = nullptr;
std::atomic<std::uint64_t> g_sequence{0};
const void* g_angles_vtable = nullptr;
alignas(8) std::array<std::byte, kAnglesSize> g_lent{};

void WriteCommand(std::byte* command, const InputSample& drive,
                  std::uint64_t& last_held, std::byte* lent, bool first) {
  // ProcessUsercmds receives the persistent steering command. Like Modlock,
  // presses are not re-fired here on every command; held edges are enough for
  // the game's command bookkeeping. MovementSteering supplies the recorded
  // changed/press state to ProcessMovement itself.
  const std::uint64_t held = drive.held;
  const std::array<std::uint64_t, 3> buttons{
      held,
      first ? (held ^ last_held) : 0,
      first ? drive.scroll : 0};
  Store(command, kCommandButtons, buttons);
  last_held = held;

  auto* base = Load<std::byte*>(command, kCommandBase);
  if (!base) return;

  Store(command, kCommandHasBits,
        Load<std::uint32_t>(command, kCommandHasBits) | 1u);
  Store(base, kBaseHasBits,
        Load<std::uint32_t>(base, kBaseHasBits) | kBaseAxesBits);
  Store(base, kBaseForward, drive.forward);
  Store(base, kBaseLeft, drive.left);

  auto* angles = Load<std::byte*>(base, kBaseAngles);
  if (!angles && lent) {
    angles = lent;
    Store(base, kBaseAngles, angles);
    Store(base, kBaseHasBits,
          Load<std::uint32_t>(base, kBaseHasBits) | kBaseAnglesBit);
  }
  if (angles) {
    Store(angles, kAnglesHasBits,
          Load<std::uint32_t>(angles, kAnglesHasBits) | kAnglesXyzBits);
    Store(angles, kAnglesXyz, drive.angles);
  }
}

int ProcessUsercmdsThunk(void* controller, std::byte* commands, int count,
                         bool paused, float margin) {
  const int slot = SlotOfController(controller);
  std::byte* lent = nullptr;

  if (slot >= 0 && commands && count > 0) {
    // Record the incoming real command before any bot steering rewrite.
    InputSample captured;
    captured.sequence = g_sequence.fetch_add(1, std::memory_order_relaxed) + 1;

    const auto* newest =
        commands + static_cast<std::size_t>(count - 1) * kCommandStride;
    const auto buttons =
        Load<std::array<std::uint64_t, 3>>(newest, kCommandButtons);
    captured.held = buttons[0];
    captured.changed = buttons[1];
    captured.scroll = buttons[2];
    captured.pressed = (captured.changed & captured.held) | captured.scroll;

    if (const auto* base = Load<const std::byte*>(newest, kCommandBase)) {
      captured.forward = Load<float>(base, kBaseForward);
      captured.left = Load<float>(base, kBaseLeft);
      if (const auto* angles = Load<const std::byte*>(base, kBaseAngles)) {
        captured.angles = Load<std::array<float, 3>>(angles, kAnglesXyz);
        if (!g_angles_vtable) g_angles_vtable = Load<const void*>(angles, 0);
      }
    }

    {
      auto& state = g_slots[slot];
      std::scoped_lock lock(state.mutex);
      state.sample = captured;
    }

    std::optional<InputSample> drive;
    {
      std::scoped_lock lock(g_drive.mutex);
      if (g_drive.slot == slot && g_drive.input) {
        drive = g_drive.input;
      }
    }

    if (drive) {
      if (g_angles_vtable) {
        lent = g_lent.data();
        g_lent.fill(std::byte{});
        Store(lent, 0, g_angles_vtable);
      }

      std::scoped_lock lock(g_drive.mutex);
      for (int i = 0; i < count; ++i) {
        WriteCommand(commands + static_cast<std::size_t>(i) * kCommandStride,
                     *drive, g_drive.last_held, lent, i == 0);
      }
    }
  }

  const int result =
      g_hook->call<int>(controller, commands, count, paused, margin);

  // A bot may borrow our angle protobuf message for the native call. Remove
  // the borrowed pointer afterwards so the engine never owns/frees it.
  for (int i = 0; lent && i < count; ++i) {
    auto* command =
        commands + static_cast<std::size_t>(i) * kCommandStride;
    auto* base = Load<std::byte*>(command, kCommandBase);
    if (base && Load<std::byte*>(base, kBaseAngles) == lent) {
      Store(base, kBaseAngles, static_cast<std::byte*>(nullptr));
      Store(base, kBaseHasBits,
            Load<std::uint32_t>(base, kBaseHasBits) & ~kBaseAnglesBit);
    }
  }

  return result;
}

}  // namespace

struct InputCapture::Impl {
  safetyhook::InlineHook hook;

  ~Impl() {
    if (g_hook == &hook) {
      hook.reset();
      g_hook = nullptr;
      for (auto& slot : g_slots) {
        std::scoped_lock lock(slot.mutex);
        slot.sample.reset();
      }
      std::scoped_lock lock(g_drive.mutex);
      g_drive.slot = -1;
      g_drive.input.reset();
      g_drive.last_held = 0;
    }
  }
};

InputCapture::InputCapture() : impl_(std::make_unique<Impl>()) {}
InputCapture::~InputCapture() = default;

bool InputCapture::Install(std::string& error) {
  if (g_hook) {
    error = "input capture is already installed";
    return false;
  }

  HMODULE server = GetModuleHandleW(L"server.dll");
  if (!server) {
    error = "server.dll is not mapped";
    return false;
  }

  void* target = FindUniquePattern(server, kProcessUsercmdsPattern, error);
  if (!target) return false;

  auto hook = safetyhook::create_inline(
      target, reinterpret_cast<void*>(&ProcessUsercmdsThunk),
      safetyhook::InlineHook::StartDisabled);
  if (!hook) {
    error = "could not create ProcessUsercmds inline hook";
    return false;
  }

  impl_->hook = std::move(hook);
  g_hook = &impl_->hook;

  if (auto enabled = impl_->hook.enable(); !enabled) {
    g_hook = nullptr;
    impl_->hook.reset();
    error = "could not enable ProcessUsercmds inline hook";
    return false;
  }
  return true;
}

std::optional<InputSample> InputCapture::Latest(int slot) const {
  if (slot < 0 || slot >= kMaxSlots) return std::nullopt;
  const auto& state = g_slots[slot];
  std::scoped_lock lock(state.mutex);
  return state.sample;
}

bool InputCapture::SetDrive(int slot, const InputSample& input, std::string& error) {
  if (slot < 0 || slot >= kMaxSlots) {
    error = "drive slot is invalid";
    return false;
  }
  if (!g_angles_vtable) {
    error = "no human view-angle message has been observed yet";
    return false;
  }

  std::scoped_lock lock(g_drive.mutex);
  if (g_drive.slot != slot) g_drive.last_held = 0;
  g_drive.slot = slot;
  g_drive.input = input;
  return true;
}

void InputCapture::ClearDrive() {
  std::scoped_lock lock(g_drive.mutex);
  g_drive.slot = -1;
  g_drive.input.reset();
  g_drive.last_held = 0;
}

}  // namespace split::movement
