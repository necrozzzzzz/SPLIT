#include "split_movement/movement_steering.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <expected>
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

constexpr std::size_t kMoveAngles = 0x08;
constexpr std::size_t kMoveWishAngles = 0xe8;
constexpr std::size_t kMoveImpulses = 0x20;
constexpr std::size_t kMoveAxes = 0x2c;
constexpr std::size_t kMoveVelocity = 0x38;
constexpr std::size_t kMoveOrigin = 0xc8;
constexpr std::size_t kButtonStates = 0x08;
constexpr std::uint32_t kInvalidEHandleIndex = 0xffffffffu;
constexpr std::uint32_t kEntityIndexBits = 0x7fff;

constexpr std::string_view kMovementPattern =
    "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 70 48 8D 05 ?? ?? ?? ?? 48 C7 44 24 28 ?? ?? 00 00 48 89 44 24 20 4C 8D 44 24 40 0F 10 44 24 20 48 8D 05 ?? ?? ?? ?? 48 8B F2";
constexpr std::string_view kSetAbsAnglesPattern =
    "E8 ?? ?? ?? ?? 48 8B 03 48 8B CB FF 90 ?? ?? ?? ?? 0F 28 74 24 ?? 84 C0 75 ?? 48 8D 15 ?? ?? ?? ?? 48 8B CB E8";

namespace schema {
struct MetadataEntry { const char* name; const void* data; };
struct Type { const void* vtable; const char* name; };
struct Field {
  const char* name;
  const Type* type;
  std::int32_t offset;
  std::int32_t metadata_count;
  const MetadataEntry* metadata;
};
struct ClassInfo;
struct BaseClass { std::uint32_t offset; const ClassInfo* info; };
struct ClassInfo {
  const ClassInfo* self;
  const char* name;
  const char* project;
  const char* cpp_name;
  std::int32_t size;
  std::uint16_t field_count;
  std::uint16_t metadata_count;
  std::uint8_t alignment;
  std::uint8_t base_count;
  const Field* fields;
  const BaseClass* bases;
  const void* data_map;
  const MetadataEntry* metadata;
  const void* type_scope;
  const void* declared_class;
  std::uint32_t flags;
  std::uint32_t flags2;
};
}

template <typename T>
T Read(const void* base, std::size_t offset) {
  T value{};
  std::memcpy(&value, static_cast<const std::byte*>(base) + offset, sizeof(T));
  return value;
}

template <typename T>
void Write(void* base, std::size_t offset, const T& value) {
  std::memcpy(static_cast<std::byte*>(base) + offset, &value, sizeof(T));
}

const schema::ClassInfo* FirstBaseOf(const schema::ClassInfo* info) {
  return info && info->base_count && info->bases ? info->bases[0].info : nullptr;
}

std::expected<std::size_t, std::string> FieldOffset(
    const schema::ClassInfo* info, const char* field_name, std::size_t minimum) {
  for (; info; info = FirstBaseOf(info)) {
    for (std::uint16_t i = 0; i < info->field_count; ++i) {
      const auto& field = info->fields[i];
      if (!field.name || std::strcmp(field.name, field_name) != 0) continue;

      std::int32_t boundary = info->size;
      for (std::uint16_t j = 0; j < info->field_count; ++j) {
        const auto& next = info->fields[j];
        if (next.offset > field.offset &&
            (boundary <= field.offset || next.offset < boundary)) {
          boundary = next.offset;
        }
      }

      if (field.offset < 0 || boundary <= field.offset ||
          static_cast<std::size_t>(boundary - field.offset) < minimum) {
        return std::unexpected(std::string(field_name) + " storage is invalid");
      }
      return static_cast<std::size_t>(field.offset);
    }
  }
  return std::unexpected(std::string("schema field not found: ") + field_name);
}

std::expected<const schema::ClassInfo*, std::string> ClassInfoOf(
    void* schema_system, const char* class_name) {
  auto** schema_vtable = *static_cast<void***>(schema_system);
  if (!schema_vtable) return std::unexpected("schema system vtable is null");

  using FindScope = void* (*)(void*, const char*, const char**);
  auto find_scope = reinterpret_cast<FindScope>(schema_vtable[13]);
  void* scope = find_scope(schema_system, "server.dll", nullptr);
  if (!scope) return std::unexpected("server.dll schema scope is unavailable");

  auto** scope_vtable = *static_cast<void***>(scope);
  if (!scope_vtable) return std::unexpected("schema scope vtable is null");

  using FindClass = void (*)(void*, void*, const char*);
  auto find_class = reinterpret_cast<FindClass>(scope_vtable[2]);
  void* info = nullptr;
  find_class(scope, &info, class_name);
  if (!info) return std::unexpected(std::string("schema class not found: ") + class_name);

  const auto* class_info = static_cast<const schema::ClassInfo*>(info);
  if (class_info->self != class_info) {
    return std::unexpected(std::string("invalid schema record: ") + class_name);
  }
  return class_info;
}

struct Layout {
  std::size_t buttons = 0;
  std::size_t chain = 0;
  std::size_t services = 0;
  std::size_t controller = 0;
  std::size_t ground = 0;
  std::size_t eye_angles = 0;
};

std::expected<Layout, std::string> ResolveLayout(void* schema_system) {
  Layout layout;
  struct Wanted {
    const char* cls;
    const char* field;
    std::size_t minimum;
    std::size_t* out;
  };
  const Wanted wanted[] = {
      {"CPlayer_MovementServices", "m_nButtons", 0, &layout.buttons},
      {"CPlayerPawnComponent", "__m_pChainEntity", sizeof(void*), &layout.chain},
      {"CBasePlayerPawn", "m_pMovementServices", sizeof(void*), &layout.services},
      {"CBasePlayerPawn", "m_hController", sizeof(std::uint32_t), &layout.controller},
      {"CBaseEntity", "m_hGroundEntity", sizeof(std::uint32_t), &layout.ground},
      {"CCitadelPlayerPawn", "m_angEyeAngles", sizeof(float) * 3, &layout.eye_angles},
  };

  for (const auto& item : wanted) {
    auto info = ClassInfoOf(schema_system, item.cls);
    if (!info) return std::unexpected(info.error());
    auto offset = FieldOffset(*info, item.field, item.minimum);
    if (!offset) {
      return std::unexpected(std::string(item.cls) + "." + item.field +
                             ": " + offset.error());
    }
    *item.out = *offset;
  }
  return layout;
}

struct PatternByte { std::uint8_t value = 0; bool wildcard = false; };

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

void* FindUnique(HMODULE module, std::string_view pattern, std::string& error) {
  const auto size = ImageSize(module);
  const auto parsed = ParsePattern(pattern);
  if (!size || parsed.empty() || parsed.size() > *size) {
    error = "invalid movement.process signature";
    return nullptr;
  }

  auto* base = reinterpret_cast<const std::uint8_t*>(module);
  void* match = nullptr;
  std::size_t count = 0;
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
    if (++count > 1) break;
  }

  if (count == 0) {
    error = "movement.process signature not found";
    return nullptr;
  }
  if (count != 1) {
    error = "movement.process signature not unique";
    return nullptr;
  }
  return match;
}

struct State {
  std::mutex mutex;
  void* schema_system = nullptr;
  std::optional<Layout> layout;
  bool layout_failed = false;
  int slot = -1;
  std::vector<RecordedTick> ticks;
  std::size_t index = 0;
  bool active = false;
  std::uint64_t last_held = 0;
};

using SetAbsAnglesFn = void (*)(void*, const float*);

State g_state;
safetyhook::InlineHook* g_hook = nullptr;
SetAbsAnglesFn g_set_abs_angles = nullptr;

void* ResolveRelativeCall(void* instruction) {
  auto* bytes = static_cast<std::uint8_t*>(instruction);
  if (bytes[0] != 0xE8) return nullptr;
  std::int32_t displacement = 0;
  std::memcpy(&displacement, bytes + 1, sizeof(displacement));
  return bytes + 5 + displacement;
}

void* PawnOf(void* services, const Layout& layout) {
  void* pawn = Read<void*>(services, layout.chain);
  if (!pawn) return nullptr;
  return Read<void*>(pawn, layout.services) == services ? pawn : nullptr;
}

bool NotifyEntityStateChanged(void* entity) {
  if (!entity) return false;

  // Current game data: CEntityInstance::NetworkStateChanged = vtable slot 28.
  constexpr std::size_t kNetworkStateChangedSlot = 28;
  auto*** object = reinterpret_cast<void***>(entity);
  if (!object || !*object || !(*object)[kNetworkStateChangedSlot]) return false;

  struct alignas(8) FullChange {
    std::uint32_t flag = 0;
    std::uint32_t padding = 0;
    std::byte offsets_and_names[40]{};
    std::int32_t unknown = -1;
    std::int32_t array_index = -1;
    std::int32_t path_index = -1;
    std::int16_t mode = 0;
    std::int16_t tail = 0;
  } changed;

  static_assert(sizeof(FullChange) == 64);
  using Notify = void (*)(void*, const FullChange&);
  reinterpret_cast<Notify>((*object)[kNetworkStateChangedSlot])(entity, changed);
  return true;
}

void MovementThunk(void* services, void* data) {
  if (!g_state.layout && !g_state.layout_failed) {
    auto resolved = ResolveLayout(g_state.schema_system);
    if (resolved) g_state.layout = *resolved;
    else g_state.layout_failed = true;
  }

  if (!g_state.layout) {
    g_hook->call<void>(services, data);
    return;
  }

  void* pawn = PawnOf(services, *g_state.layout);
  int slot = -1;
  if (pawn) {
    const auto controller =
        Read<std::uint32_t>(pawn, g_state.layout->controller);
    if (controller != kInvalidEHandleIndex) {
      slot = static_cast<int>(controller & kEntityIndexBits) - 1;
    }
  }

  std::optional<RecordedTick> tick;
  std::uint64_t previous_held = 0;
  {
    std::scoped_lock lock(g_state.mutex);
    if (g_state.active && slot == g_state.slot && g_state.index < g_state.ticks.size()) {
      tick = g_state.ticks[g_state.index++];
      previous_held = g_state.last_held;
      if (tick->input) {
        g_state.last_held = tick->input->held | tick->input->pressed;
      }
      if (g_state.index >= g_state.ticks.size()) g_state.active = false;
    }
  }

  if (!tick) {
    g_hook->call<void>(services, data);
    return;
  }

  const auto ground_before = pawn
      ? Read<std::uint32_t>(pawn, g_state.layout->ground)
      : kInvalidEHandleIndex;

  if (tick->input) {
    // Match Modlock's Face(): the recorded view drives both the pawn aim and
    // the body's absolute yaw. CMoveData angles alone leave a bot's model at
    // its own brain yaw, which makes it appear to run backwards/sideways.
    if (pawn) {
      Write(pawn, g_state.layout->eye_angles, tick->input->angles);
      // Eye angles are networked; publish the changed pawn state so remote
      // clients actually update aim/animation state.
      static_cast<void>(NotifyEntityStateChanged(pawn));
      if (g_set_abs_angles) {
        const std::array<float, 3> body{0.0f, tick->input->angles[1], 0.0f};
        g_set_abs_angles(pawn, body.data());
      }
    }

    auto buttons =
        Read<std::array<std::uint64_t, 3>>(services, g_state.layout->buttons + kButtonStates);
    const std::uint64_t held = tick->input->held | tick->input->pressed;
    buttons[0] = held;
    buttons[1] = (held ^ previous_held) | tick->input->pressed;
    buttons[2] = 0;
    Write(services, g_state.layout->buttons + kButtonStates, buttons);

    Write(data, kMoveAngles, tick->input->angles);
    Write(data, kMoveWishAngles, tick->input->angles);
    const std::array<float, 3> axes{tick->input->forward, tick->input->left, 0.0f};
    Write(data, kMoveImpulses, axes);
    Write(data, kMoveAxes, axes);
  }

  g_hook->call<void>(services, data);

  Write(data, kMoveOrigin, tick->pawn.position);
  Write(data, kMoveVelocity, tick->pawn.velocity);
  if (pawn) {
    Write(pawn, g_state.layout->ground, tick->pawn.ground_entity_handle);
    if (ground_before != tick->pawn.ground_entity_handle) {
      static_cast<void>(NotifyEntityStateChanged(pawn));
    }
  }
}

}  // namespace

struct MovementSteering::Impl {
  explicit Impl(void* schema) { g_state.schema_system = schema; }
  safetyhook::InlineHook hook;

  ~Impl() {
    if (g_hook == &hook) {
      hook.reset();
      g_hook = nullptr;
      std::scoped_lock lock(g_state.mutex);
      g_state.slot = -1;
      g_state.ticks.clear();
      g_state.index = 0;
      g_state.active = false;
      g_state.last_held = 0;
      g_state.layout.reset();
      g_state.layout_failed = false;
      g_state.schema_system = nullptr;
      g_set_abs_angles = nullptr;
    }
  }
};

MovementSteering::MovementSteering(void* schema_system)
    : impl_(std::make_unique<Impl>(schema_system)) {}
MovementSteering::~MovementSteering() = default;

bool MovementSteering::Install(std::string& error) {
  if (g_hook) {
    error = "movement steering is already installed";
    return false;
  }

  HMODULE server = GetModuleHandleW(L"server.dll");
  if (!server) {
    error = "server.dll is not mapped";
    return false;
  }
  void* target = FindUnique(server, kMovementPattern, error);
  if (!target) return false;

  void* angle_site = FindUnique(server, kSetAbsAnglesPattern, error);
  if (!angle_site) return false;
  // Upstream game data pins the SetAbsAngles relative CALL at pattern + 0x24.
  g_set_abs_angles = reinterpret_cast<SetAbsAnglesFn>(
      ResolveRelativeCall(static_cast<std::uint8_t*>(angle_site) + 0x24));
  if (!g_set_abs_angles) {
    error = "entity.set-abs-angles call target could not be resolved";
    return false;
  }

  auto hook = safetyhook::create_inline(
      target, reinterpret_cast<void*>(&MovementThunk),
      safetyhook::InlineHook::StartDisabled);
  if (!hook) {
    error = "could not create movement.process hook";
    return false;
  }

  impl_->hook = std::move(hook);
  g_hook = &impl_->hook;
  if (auto enabled = impl_->hook.enable(); !enabled) {
    g_hook = nullptr;
    impl_->hook.reset();
    error = "could not enable movement.process hook";
    return false;
  }
  return true;
}

bool MovementSteering::Start(int slot, std::vector<RecordedTick> ticks,
                             std::string& error) {
  if (slot < 0 || slot >= 64) {
    error = "steering slot is invalid";
    return false;
  }
  if (ticks.empty()) {
    error = "steering route is empty";
    return false;
  }

  std::scoped_lock lock(g_state.mutex);
  g_state.slot = slot;
  g_state.ticks = std::move(ticks);
  g_state.index = 0;
  g_state.active = true;
  g_state.last_held = 0;
  return true;
}

void MovementSteering::Stop() {
  std::scoped_lock lock(g_state.mutex);
  g_state.slot = -1;
  g_state.ticks.clear();
  g_state.index = 0;
  g_state.active = false;
  g_state.last_held = 0;
}

SteeringStatus MovementSteering::Status() const {
  std::scoped_lock lock(g_state.mutex);
  return SteeringStatus{
      .active = g_state.active,
      .slot = g_state.slot,
      .index = g_state.index,
      .total = g_state.ticks.size(),
  };
}

}  // namespace split::movement
