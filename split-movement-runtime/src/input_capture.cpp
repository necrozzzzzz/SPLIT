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

// Current Deadlock CUserCmd layout, from the vendored 2026-10-28 engine data.
constexpr std::size_t kCommandStride = 0xa8;
constexpr std::size_t kCommandBase = 0x48;
constexpr std::size_t kCommandButtons = 0x78;
constexpr std::size_t kBaseAngles = 0x40;
constexpr std::size_t kBaseForward = 0x58;
constexpr std::size_t kBaseLeft = 0x5c;
constexpr std::size_t kAnglesXyz = 0x18;

constexpr std::string_view kProcessUsercmdsPattern =
    "48 8B C4 44 88 48 ?? 44 89 40 ?? 48 89 50 ?? 53";

template <typename T>
T Load(const std::byte* base, std::size_t offset) {
  T value{};
  std::memcpy(&value, base + offset, sizeof(value));
  return value;
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
    ++matches;
    if (matches > 1) break;
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

std::array<SlotInput, kMaxSlots> g_slots;
safetyhook::InlineHook* g_hook = nullptr;
std::atomic<std::uint64_t> g_sequence{0};

int ProcessUsercmdsThunk(void* controller, std::byte* commands, int count, bool paused,
                         float margin) {
  const int slot = SlotOfController(controller);

  if (slot >= 0 && commands && count > 0) {
    // The newest command in the batch is the state closest to the movement
    // simulation that follows this call.
    const auto* command =
        commands + static_cast<std::size_t>(count - 1) * kCommandStride;

    InputSample sample;
    sample.sequence = g_sequence.fetch_add(1, std::memory_order_relaxed) + 1;

    const auto buttons =
        Load<std::array<std::uint64_t, 3>>(command, kCommandButtons);
    sample.held = buttons[0];
    // "changed" contains both rising and falling edges. Keep only rising
    // edges so the recorder has a stable "pressed this command" mask.
    sample.pressed = buttons[1] & buttons[0];

    if (const auto* base = Load<const std::byte*>(command, kCommandBase)) {
      sample.forward = Load<float>(base, kBaseForward);
      sample.left = Load<float>(base, kBaseLeft);

      if (const auto* angles = Load<const std::byte*>(base, kBaseAngles)) {
        sample.angles = Load<std::array<float, 3>>(angles, kAnglesXyz);
      }
    }

    auto& state = g_slots[slot];
    std::scoped_lock lock(state.mutex);
    state.sample = sample;
  }

  return g_hook->call<int>(controller, commands, count, paused, margin);
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

}  // namespace split::movement
