#include "split_movement/movement_events.h"

#include <array>
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

constexpr std::string_view kFireModifierEventPattern =
    "40 55 56 41 55 48 8D 6C 24 ?? 48 81 EC ?? ?? ?? ?? 33 F6";
constexpr std::size_t kIdentityOffset = 0x10;
constexpr std::size_t kIdentityEHandleOffset = 0x10;
constexpr std::size_t kIdentityFlagsOffset = 0x30;
constexpr std::uint32_t kEntityIndexBits = 0x7fff;
constexpr std::uint32_t kSerialShift = 15;
constexpr std::uint32_t kSerialBits = 0x1ffff;
constexpr std::uint32_t kEfInvalidEHandle = 0x1;
constexpr std::uint32_t kEfDeleteFlags = 0x10 | 0x200;
constexpr std::uint32_t kUnobserved = 0xffffffffu;

struct EventSpec {
  const char* native_name;
  MovementExecution execution;
};

constexpr std::array<EventSpec, 10> kEventSpecs{{
    {"MODIFIER_EVENT_LANDED_ON_GROUND", MovementExecution::kLanded},
    {"MODIFIER_EVENT_ATTACHED_TO_ZIPLINE", MovementExecution::kZiplineAttached},
    {"MODIFIER_EVENT_PLAYER_GROUND_DASH_STARTED", MovementExecution::kGroundDash},
    {"MODIFIER_EVENT_PLAYER_SLIDE_STARTED", MovementExecution::kSlide},
    {"MODIFIER_EVENT_BOUNCE_PAD_ACTIVATED", MovementExecution::kBouncePad},
    {"MODIFIER_EVENT_DASH_JUMP_EXECUTED", MovementExecution::kDashJump},
    {"MODIFIER_EVENT_AIR_JUMP_EXECUTED", MovementExecution::kAirJump},
    {"MODIFIER_EVENT_WALL_JUMP_EXECUTED", MovementExecution::kWallJump},
    {"MODIFIER_EVENT_AIR_DASH_EXECUTED", MovementExecution::kAirDash},
    {"MODIFIER_EVENT_MELEE_ATTACK_STARTED", MovementExecution::kMeleeStarted},
}};

template <typename T>
T Load(const std::byte* base, std::size_t offset) {
  T value{};
  std::memcpy(&value, base + offset, sizeof(value));
  return value;
}

std::optional<std::uint32_t> ReferenceHandle(void* entity) {
  if (!entity) return std::nullopt;
  auto* bytes = static_cast<std::byte*>(entity);
  auto* identity = Load<std::byte*>(bytes, kIdentityOffset);
  if (!identity || Load<void*>(identity, 0) != entity) return std::nullopt;
  const auto stored = Load<std::uint32_t>(identity, kIdentityEHandleOffset);
  const auto flags = Load<std::uint32_t>(identity, kIdentityFlagsOffset);
  if (stored == 0xffffffffu || (flags & kEfDeleteFlags) != 0) return std::nullopt;
  const auto serial =
      ((stored >> kSerialShift) - (flags & kEfInvalidEHandle)) & kSerialBits;
  return (stored & kEntityIndexBits) | (serial << kSerialShift);
}

std::optional<std::size_t> ImageSize(HMODULE module) {
  if (!module) return std::nullopt;
  auto* base = reinterpret_cast<const std::byte*>(module);
  const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
  if (dos->e_magic != IMAGE_DOS_SIGNATURE) return std::nullopt;
  const auto* nt =
      reinterpret_cast<const IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
  if (nt->Signature != IMAGE_NT_SIGNATURE) return std::nullopt;
  return static_cast<std::size_t>(nt->OptionalHeader.SizeOfImage);
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

void* FindUnique(HMODULE module, std::string_view pattern, std::string& error) {
  const auto size = ImageSize(module);
  const auto parsed = ParsePattern(pattern);
  if (!size || parsed.empty() || parsed.size() > *size) {
    error = "invalid FireModifierEvent scan";
    return nullptr;
  }
  auto* base = reinterpret_cast<const std::uint8_t*>(module);
  void* hit = nullptr;
  std::size_t hits = 0;
  for (std::size_t at = 0; at + parsed.size() <= *size; ++at) {
    bool match = true;
    for (std::size_t i = 0; i < parsed.size(); ++i) {
      if (!parsed[i].wildcard && base[at + i] != parsed[i].value) {
        match = false;
        break;
      }
    }
    if (!match) continue;
    hit = const_cast<std::uint8_t*>(base + at);
    if (++hits > 1) break;
  }
  if (hits != 1) {
    error = hits == 0 ? "FireModifierEvent signature not found"
                      : "FireModifierEvent signature not unique";
    return nullptr;
  }
  return hit;
}

std::optional<std::uint32_t> EnumeratorValue(
    HMODULE module, std::string_view name) {
  const auto size = ImageSize(module);
  if (!size) return std::nullopt;
  const auto base = reinterpret_cast<std::uintptr_t>(module);
  const auto* bytes = reinterpret_cast<const char*>(module);
  const std::string_view view(bytes, *size);

  std::string needle;
  needle.push_back('\0');
  needle.append(name);
  needle.push_back('\0');
  const auto at = view.find(needle);
  if (at == std::string_view::npos) return std::nullopt;

  const std::uint64_t address = static_cast<std::uint64_t>(base + at + 1);
  char pointer[sizeof(address)]{};
  std::memcpy(pointer, &address, sizeof(address));
  const std::string_view pointer_view(pointer, sizeof(pointer));

  std::optional<std::uint32_t> value;
  for (auto pos = view.find(pointer_view); pos != std::string_view::npos;
       pos = view.find(pointer_view, pos + 1)) {
    if (pos % alignof(std::uint64_t) != 0 ||
        pos + 2 * sizeof(std::uint64_t) > view.size()) {
      continue;
    }
    std::int64_t candidate = -1;
    std::memcpy(&candidate, view.data() + pos + sizeof(std::uint64_t),
                sizeof(candidate));
    if (candidate < 0 || candidate >= 1024) continue;
    const auto converted = static_cast<std::uint32_t>(candidate);
    if (value && *value != converted) return std::nullopt;
    value = converted;
  }
  return value;
}

struct QueuedEvent {
  MovementExecution execution{};
  std::uint32_t pawn_handle = 0;
};

std::mutex g_mutex;
std::vector<QueuedEvent> g_events;
std::array<std::pair<std::uint32_t, MovementExecution>, kEventSpecs.size()> g_ids{};
safetyhook::InlineHook* g_hook = nullptr;

__int64 __fastcall FireModifierEventThunk(
    std::uint32_t event, void* caster, void* target,
    void* cast_entity, void* event_data) {
  for (const auto& [id, execution] : g_ids) {
    if (id != kUnobserved && event == id) {
      if (const auto handle = ReferenceHandle(caster)) {
        std::scoped_lock lock(g_mutex);
        g_events.push_back({execution, *handle});
      }
      break;
    }
  }
  return g_hook->call<__int64>(
      event, caster, target, cast_entity, event_data);
}

}  // namespace

struct MovementEvents::Impl {
  safetyhook::InlineHook hook;

  ~Impl() {
    if (g_hook == &hook) {
      hook.reset();
      g_hook = nullptr;
      std::scoped_lock lock(g_mutex);
      g_events.clear();
      for (auto& entry : g_ids) entry.first = kUnobserved;
    }
  }
};

MovementEvents::MovementEvents() : impl_(std::make_unique<Impl>()) {}
MovementEvents::~MovementEvents() = default;

const char* MovementExecutionName(MovementExecution execution) {
  switch (execution) {
    case MovementExecution::kLanded: return "Landed";
    case MovementExecution::kZiplineAttached: return "Zipline attached";
    case MovementExecution::kGroundDash: return "Ground dash";
    case MovementExecution::kSlide: return "Slide";
    case MovementExecution::kBouncePad: return "Bounce pad";
    case MovementExecution::kDashJump: return "Dash jump";
    case MovementExecution::kAirJump: return "Air jump";
    case MovementExecution::kWallJump: return "Wall jump";
    case MovementExecution::kAirDash: return "Air dash";
    case MovementExecution::kMeleeStarted: return "Heavy melee";
  }
  return "Movement";
}

bool MovementEvents::Install(std::string& error) {
  if (g_hook) {
    error = "movement event hook already installed";
    return false;
  }
  HMODULE server = GetModuleHandleW(L"server.dll");
  if (!server) {
    error = "server.dll is not mapped";
    return false;
  }

  void* target = FindUnique(server, kFireModifierEventPattern, error);
  if (!target) return false;

  for (std::size_t i = 0; i < kEventSpecs.size(); ++i) {
    const auto resolved = EnumeratorValue(server, kEventSpecs[i].native_name);
    g_ids[i] = {resolved.value_or(kUnobserved), kEventSpecs[i].execution};
  }

  auto created = safetyhook::create_inline(
      target, reinterpret_cast<void*>(&FireModifierEventThunk),
      safetyhook::InlineHook::StartDisabled);
  if (!created) {
    error = "could not create FireModifierEvent hook";
    return false;
  }

  impl_->hook = std::move(created);
  g_hook = &impl_->hook;
  if (auto enabled = impl_->hook.enable(); !enabled) {
    g_hook = nullptr;
    impl_->hook.reset();
    error = "could not enable FireModifierEvent hook";
    return false;
  }
  return true;
}

std::vector<MovementExecutedEvent> MovementEvents::Drain(
    std::uint32_t pawn_handle) {
  std::vector<MovementExecutedEvent> out;
  std::scoped_lock lock(g_mutex);
  auto write = g_events.begin();
  for (auto it = g_events.begin(); it != g_events.end(); ++it) {
    if (it->pawn_handle == pawn_handle) {
      out.push_back({it->execution, it->pawn_handle});
    } else {
      if (write != it) *write = *it;
      ++write;
    }
  }
  g_events.erase(write, g_events.end());
  return out;
}

void MovementEvents::Clear() {
  std::scoped_lock lock(g_mutex);
  g_events.clear();
}

}  // namespace split::movement
