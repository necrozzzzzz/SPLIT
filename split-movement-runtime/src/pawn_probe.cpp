#include "split_movement/pawn_probe.h"

#include <cmath>
#include <cstddef>
#include <cstring>
#include <expected>
#include <string>
#include <string_view>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {

namespace {

constexpr std::size_t kIdentityOffset = 0x10;
constexpr std::size_t kResourceServiceEntitySystemOffset = 0x58;
constexpr std::size_t kEntityListOffset = 0x10;
constexpr std::uint32_t kMaxEntitiesInList = 512;
constexpr std::uint32_t kMaxEntityLists = 64;
constexpr std::size_t kIdentitySize = 0x70;
constexpr std::size_t kIdentityInstanceOffset = 0x00;
constexpr std::size_t kIdentityEHandleOffset = 0x10;
constexpr std::size_t kIdentityFlagsOffset = 0x30;
constexpr std::uint32_t kEntityIndexBits = 0x7FFF;
constexpr std::uint32_t kSerialShift = 15;
constexpr std::uint32_t kSerialBits = 0x1FFFF;
constexpr std::uint32_t kInvalidEHandleIndex = 0xFFFFFFFF;
constexpr std::uint32_t kEfInvalidEHandle = 0x1;
constexpr std::uint32_t kEfDeleteFlags = 0x10 | 0x200;

namespace schema {

struct MetadataEntry {
  const char* name;
  const void* data;
};

struct Type {
  const void* vtable;
  const char* name;
};

struct Field {
  const char* name;
  const Type* type;
  std::int32_t offset;
  std::int32_t metadata_count;
  const MetadataEntry* metadata;
};

struct ClassInfo;

struct BaseClass {
  std::uint32_t offset;
  const ClassInfo* info;
};

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

}  // namespace schema

static_assert(sizeof(schema::Field) == 32);
static_assert(sizeof(schema::BaseClass) == 16);
static_assert(offsetof(schema::ClassInfo, size) == 0x20);
static_assert(offsetof(schema::ClassInfo, base_count) == 0x29);
static_assert(offsetof(schema::ClassInfo, fields) == 0x30);

using CreateInterfaceFn = void* (*)(const char*, int*);

void* ResolveInterface(const wchar_t* module_name, const char* version, std::string& error) {
#if !defined(_WIN32)
  (void)module_name;
  (void)version;
  error = "engine interfaces require Windows";
  return nullptr;
#else
  HMODULE module = GetModuleHandleW(module_name);
  if (!module) {
    error = "module is not mapped";
    return nullptr;
  }
  auto factory = reinterpret_cast<CreateInterfaceFn>(GetProcAddress(module, "CreateInterface"));
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
#endif
}

template <typename T>
T Read(const void* base, std::size_t offset) {
  T value{};
  std::memcpy(&value, static_cast<const std::byte*>(base) + offset, sizeof(T));
  return value;
}

std::uint32_t EntityIndexOf(std::uint32_t handle) {
  return handle & kEntityIndexBits;
}

std::optional<std::uint32_t> LiveHandleOf(std::uint32_t stored, std::uint32_t flags) {
  if (stored == kInvalidEHandleIndex || (flags & kEfDeleteFlags) != 0) {
    return std::nullopt;
  }
  const std::uint32_t serial =
      ((stored >> kSerialShift) - (flags & kEfInvalidEHandle)) & kSerialBits;
  return EntityIndexOf(stored) | (serial << kSerialShift);
}

void* IdentityOf(void* entity) {
  return entity ? Read<void*>(entity, kIdentityOffset) : nullptr;
}

unsigned char* IdentityOfIndex(unsigned char* entity_list, std::uint32_t index) {
  if (!entity_list || index >= kMaxEntitiesInList * kMaxEntityLists) return nullptr;

  auto* chunks = reinterpret_cast<unsigned char**>(entity_list);
  unsigned char* chunk = chunks[index / kMaxEntitiesInList];
  if (!chunk) return nullptr;

  unsigned char* identity = chunk + (index % kMaxEntitiesInList) * kIdentitySize;
  const auto ehandle = Read<std::uint32_t>(identity, kIdentityEHandleOffset);
  if (EntityIndexOf(ehandle) != index) return nullptr;

  void* instance = Read<void*>(identity, kIdentityInstanceOffset);
  if (!instance || IdentityOf(instance) != identity) return nullptr;
  return identity;
}

unsigned char* IdentityOfHandle(unsigned char* entity_list, std::uint32_t handle) {
  if (handle == kInvalidEHandleIndex) return nullptr;

  auto* identity = IdentityOfIndex(entity_list, EntityIndexOf(handle));
  if (!identity) return nullptr;

  const auto stored = Read<std::uint32_t>(identity, kIdentityEHandleOffset);
  const auto flags = Read<std::uint32_t>(identity, kIdentityFlagsOffset);
  const auto live = LiveHandleOf(stored, flags);
  return live && *live == handle ? identity : nullptr;
}

void* InstanceOf(unsigned char* identity) {
  return identity ? Read<void*>(identity, kIdentityInstanceOffset) : nullptr;
}

const schema::ClassInfo* FirstBaseOf(const schema::ClassInfo* info) {
  return info && info->base_count > 0 && info->bases ? info->bases[0].info : nullptr;
}

std::expected<std::size_t, std::string> FieldOffset(const schema::ClassInfo* info,
                                                   const char* field_name,
                                                   std::size_t minimum_size) {
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
          static_cast<std::size_t>(boundary - field.offset) < minimum_size) {
        return std::unexpected(std::string(field_name) + " storage span is invalid");
      }
      return static_cast<std::size_t>(field.offset);
    }
  }
  return std::unexpected(std::string("schema field not found: ") + field_name);
}

std::expected<const schema::ClassInfo*, std::string> ClassInfoOf(
    void* schema_system, const char* class_name) {
  if (!schema_system) return std::unexpected("SchemaSystem_001 is null");

  // Current game data: ISchemaSystem::FindTypeScopeForModule = 13.
  auto** schema_vtable = *static_cast<void***>(schema_system);
  if (!schema_vtable) return std::unexpected("schema system vtable is null");

  using FindTypeScopeFn = void* (*)(void*, const char*, const char**);
  auto find_scope = reinterpret_cast<FindTypeScopeFn>(schema_vtable[13]);
  void* scope = find_scope(schema_system, "server.dll", nullptr);
  if (!scope) return std::unexpected("server.dll schema type scope is not open");

  // Current game data: ISchemaSystemTypeScope::FindDeclaredClass = 2.
  auto** scope_vtable = *static_cast<void***>(scope);
  if (!scope_vtable) return std::unexpected("schema type scope vtable is null");

  using FindClassFn = void (*)(void*, void*, const char*);
  auto find_class = reinterpret_cast<FindClassFn>(scope_vtable[2]);
  void* info = nullptr;
  find_class(scope, &info, class_name);
  if (!info) return std::unexpected(std::string("schema class not found: ") + class_name);

  const auto* class_info = static_cast<const schema::ClassInfo*>(info);
  if (class_info->self != class_info) {
    return std::unexpected(std::string("schema class record invalid: ") + class_name);
  }
  return class_info;
}

bool Finite3(const std::array<float, 3>& value) {
  return std::isfinite(value[0]) && std::isfinite(value[1]) && std::isfinite(value[2]);
}

bool NotifyEntityStateChanged(void* entity) {
  if (!entity) return false;
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

struct Layout {
  std::size_t pawn_handle = 0;
  std::size_t body_component = 0;
  std::size_t scene_node = 0;
  std::size_t abs_origin = 0;
  std::size_t abs_velocity = 0;
  std::size_t ground_entity = 0;
  std::size_t team = 0;
  std::size_t hero_component = 0;
  std::size_t spawned_hero = 0;
  std::size_t hero_id = 0;
  std::size_t render_mode = 0;
  std::size_t render_color = 0;
};

std::expected<Layout, std::string> ResolveLayout(void* schema_system) {
  Layout layout;

  struct Wanted {
    const char* class_name;
    const char* field_name;
    std::size_t minimum;
    std::size_t* target;
  };

  const Wanted wanted[] = {
      {"CBasePlayerController", "m_hPawn", sizeof(std::uint32_t), &layout.pawn_handle},
      {"CBaseEntity", "m_CBodyComponent", sizeof(void*), &layout.body_component},
      {"CBodyComponent", "m_pSceneNode", sizeof(void*), &layout.scene_node},
      {"CGameSceneNode", "m_vecAbsOrigin", sizeof(float) * 3, &layout.abs_origin},
      {"CBaseEntity", "m_vecAbsVelocity", sizeof(float) * 3, &layout.abs_velocity},
      {"CBaseEntity", "m_hGroundEntity", sizeof(std::uint32_t), &layout.ground_entity},
      {"CBaseEntity", "m_iTeamNum", sizeof(std::uint8_t), &layout.team},
      {"CCitadelPlayerPawn", "m_CCitadelHeroComponent", 1, &layout.hero_component},
      {"CCitadelHeroComponent", "m_spawnedHero", 1, &layout.spawned_hero},
      {"CitadelHeroSpawnData_t", "m_nHeroID", sizeof(std::uint32_t), &layout.hero_id},
      {"CBaseModelEntity", "m_nRenderMode", sizeof(std::uint8_t), &layout.render_mode},
      {"CBaseModelEntity", "m_clrRender", sizeof(std::uint32_t), &layout.render_color},
  };

  for (const auto& field : wanted) {
    auto info = ClassInfoOf(schema_system, field.class_name);
    if (!info) return std::unexpected(info.error());

    auto offset = FieldOffset(*info, field.field_name, field.minimum);
    if (!offset) {
      return std::unexpected(std::string(field.class_name) + "." + field.field_name +
                             ": " + offset.error());
    }
    *field.target = *offset;
  }
  return layout;
}

}  // namespace

struct PawnProbe::Impl {
  explicit Impl(void* schema) : schema_system(schema) {}

  void* schema_system = nullptr;
  void* resource_service = nullptr;
  std::optional<Layout> layout;
};

PawnProbe::PawnProbe(void* schema_system) : impl_(std::make_unique<Impl>(schema_system)) {}
PawnProbe::~PawnProbe() = default;

std::optional<PawnSample> PawnProbe::Observe(int slot, std::string& error) {
  error.clear();
  if (slot < 0 || slot >= 64) {
    error = "player slot is outside the Deadlock slot range";
    return std::nullopt;
  }

  if (!impl_->layout) {
    auto resolved = ResolveLayout(impl_->schema_system);
    if (!resolved) {
      error = resolved.error();
      return std::nullopt;
    }
    impl_->layout = *resolved;
  }

  if (!impl_->resource_service) {
    impl_->resource_service =
        ResolveInterface(L"engine2.dll", "GameResourceServiceServerV001", error);
    if (!impl_->resource_service) return std::nullopt;
  }

  void* entity_system =
      Read<void*>(impl_->resource_service, kResourceServiceEntitySystemOffset);
  if (!entity_system) {
    error = "CGameResourceService has no live entity system";
    return std::nullopt;
  }

  auto* entity_list =
      static_cast<unsigned char*>(entity_system) + kEntityListOffset;

  // Source player controllers are entity index slot + 1.
  auto* controller_identity =
      IdentityOfIndex(entity_list, static_cast<std::uint32_t>(slot + 1));
  void* controller = InstanceOf(controller_identity);
  if (!controller) return std::nullopt;

  const auto pawn_handle =
      Read<std::uint32_t>(controller, impl_->layout->pawn_handle);
  auto* pawn_identity = IdentityOfHandle(entity_list, pawn_handle);
  void* pawn = InstanceOf(pawn_identity);
  if (!pawn) return std::nullopt;

  void* body = Read<void*>(pawn, impl_->layout->body_component);
  if (!body) return std::nullopt;

  void* scene = Read<void*>(body, impl_->layout->scene_node);
  if (!scene) return std::nullopt;

  PawnSample sample;
  sample.slot = slot;
  sample.pawn_handle = pawn_handle;
  sample.team = Read<std::uint8_t>(pawn, impl_->layout->team);
  sample.hero_id = Read<std::uint32_t>(
      pawn, impl_->layout->hero_component + impl_->layout->spawned_hero +
                impl_->layout->hero_id);
  std::memcpy(sample.position.data(),
              static_cast<const std::byte*>(scene) + impl_->layout->abs_origin,
              sizeof(sample.position));
  std::memcpy(sample.velocity.data(),
              static_cast<const std::byte*>(pawn) + impl_->layout->abs_velocity,
              sizeof(sample.velocity));

  const auto ground =
      Read<std::uint32_t>(pawn, impl_->layout->ground_entity);
  sample.ground_entity_handle = ground;
  sample.grounded = ground != kInvalidEHandleIndex;

  if (!Finite3(sample.position) || !Finite3(sample.velocity)) {
    error = "pawn pose contains non-finite values";
    return std::nullopt;
  }

  return sample;
}


bool PawnProbe::SetGhostVisual(int slot, bool ghost, std::string& error) {
  error.clear();
  if (slot < 0 || slot >= 64) {
    error = "player slot is outside the Deadlock slot range";
    return false;
  }

  if (!impl_->layout) {
    auto resolved = ResolveLayout(impl_->schema_system);
    if (!resolved) {
      error = resolved.error();
      return false;
    }
    impl_->layout = *resolved;
  }

  if (!impl_->resource_service) {
    impl_->resource_service =
        ResolveInterface(L"engine2.dll", "GameResourceServiceServerV001", error);
    if (!impl_->resource_service) return false;
  }

  void* entity_system =
      Read<void*>(impl_->resource_service, kResourceServiceEntitySystemOffset);
  if (!entity_system) {
    error = "CGameResourceService has no live entity system";
    return false;
  }

  auto* entity_list =
      static_cast<unsigned char*>(entity_system) + kEntityListOffset;
  auto* controller_identity =
      IdentityOfIndex(entity_list, static_cast<std::uint32_t>(slot + 1));
  void* controller = InstanceOf(controller_identity);
  if (!controller) {
    error = "controller is not live";
    return false;
  }

  const auto pawn_handle =
      Read<std::uint32_t>(controller, impl_->layout->pawn_handle);
  auto* pawn_identity = IdentityOfHandle(entity_list, pawn_handle);
  void* pawn = InstanceOf(pawn_identity);
  if (!pawn) {
    error = "pawn is not live";
    return false;
  }

  constexpr std::uint8_t kRenderNormal = 0;
  constexpr std::uint8_t kRenderTransColor = 1;
  constexpr std::uint8_t kGhostAlpha = 110;
  const std::uint8_t mode = ghost ? kRenderTransColor : kRenderNormal;
  const std::uint32_t alpha = ghost ? kGhostAlpha : 255u;
  const std::uint32_t color = 0x00ffffffu | (alpha << 24);

  // Mirror Modlock's writeField() behavior exactly: each networked field
  // write is published immediately, rather than batching both into one notify.
  std::memcpy(static_cast<std::byte*>(pawn) + impl_->layout->render_mode,
              &mode, sizeof(mode));
  if (!NotifyEntityStateChanged(pawn)) {
    error = "ghost render mode could not be replicated";
    return false;
  }

  std::memcpy(static_cast<std::byte*>(pawn) + impl_->layout->render_color,
              &color, sizeof(color));
  if (!NotifyEntityStateChanged(pawn)) {
    error = "ghost render color could not be replicated";
    return false;
  }

  const auto applied_mode =
      Read<std::uint8_t>(pawn, impl_->layout->render_mode);
  const auto applied_color =
      Read<std::uint32_t>(pawn, impl_->layout->render_color);
  if (applied_mode != mode || applied_color != color) {
    error = "ghost render values were overwritten immediately";
    return false;
  }
  return true;
}

}  // namespace split::movement
