#include "split_movement/native_visual_renderer.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstddef>
#include <cstring>
#include <optional>
#include <cstdint>
#include <iomanip>
#include <sstream>
#include <string>
#include <string_view>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {
namespace {

#if defined(_WIN32)
using CreateInterfaceFn = void* (*)(const char*, int*);

void* ResolveInterface(const wchar_t* module_name, const char* version,
                       std::string& error) {
  HMODULE module = GetModuleHandleW(module_name);
  if (!module) {
    error = "module is not mapped";
    return nullptr;
  }
  auto factory =
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
#endif

// Source2EngineToServer001 / IVEngineServer2. The current Source 2 SDK layout
// places ServerCommand at vtable slot 45 (11 IAppSystem + 7 ISource2Engine +
// 27 IVEngineServer2 methods before ServerCommand).
constexpr std::size_t kServerCommandSlot = 45;
constexpr std::size_t kMaxTracerPoints = 64;
constexpr std::size_t kRingSegments = 24;

constexpr std::size_t kIdentityOffset = 0x10;
constexpr std::size_t kResourceServiceEntitySystemOffset = 0x58;
constexpr std::size_t kEntityKeyValuesSize = 56;

constexpr std::string_view kCreateEntityPattern =
    "48 83 EC ?? 48 8B 0D ?? ?? ?? ?? 41 8B C0";
constexpr std::string_view kQueueSpawnPattern =
    "40 56 57 41 56 48 83 EC ?? F7 42";
constexpr std::string_view kExecuteQueuedPattern =
    "48 89 5C 24 ?? 57 48 81 EC ?? ?? ?? ?? FF 81 ?? ?? ?? ?? 48 8D 44 24";
constexpr std::string_view kAcceptInputPattern =
    "48 89 5C 24 ?? 48 89 6C 24 ?? 56 57 41 56 48 81 EC ?? ?? ?? ?? 4D 8B F0 48 8B F1";
constexpr std::string_view kKvAllocatePattern =
    "48 8B 05 ?? ?? ?? ?? 48 8B D1 48 8B 08 48 8B 01 48 FF 60 08 CC CC CC CC CC CC CC CC CC CC CC CC 48 8B 05 ?? ?? ?? ?? 4C 8B C9 4C 8B C2 49 8B D1";
constexpr std::string_view kKvConstructPattern =
    "48 89 5C 24 08 57 48 83 EC 20 33 FF 48 8B D9 48 89 79 28 48 89 79 30 44 88 41 25 48 85 D2 74 ??";
constexpr std::string_view kKvSetPattern =
    "40 53 55 56 48 83 EC 30 66 83 79 22 00 41 0F B6 E8 48 8B F2 48 8B D9 7E ??";

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

#if defined(_WIN32)
std::optional<std::size_t> ImageSize(HMODULE module) {
  if (!module) return std::nullopt;
  auto* base = reinterpret_cast<const std::byte*>(module);
  auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
  if (dos->e_magic != IMAGE_DOS_SIGNATURE) return std::nullopt;
  auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
  if (nt->Signature != IMAGE_NT_SIGNATURE) return std::nullopt;
  return static_cast<std::size_t>(nt->OptionalHeader.SizeOfImage);
}

void* FindUnique(HMODULE module, std::string_view pattern, const char* name,
                 std::string& error) {
  const auto size = ImageSize(module);
  const auto parsed = ParsePattern(pattern);
  if (!size || parsed.empty() || parsed.size() > *size) {
    error = std::string("invalid signature for ") + name;
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
    error = std::string(name) + " signature not found";
    return nullptr;
  }
  if (count != 1) {
    error = std::string(name) + " signature not unique";
    return nullptr;
  }
  return match;
}

#endif

std::uint32_t MurmurHash2(const unsigned char* data, std::size_t length,
                          std::uint32_t seed) {
  constexpr std::uint32_t m = 0x5bd1e995u;
  constexpr int r = 24;
  std::uint32_t h = seed ^ static_cast<std::uint32_t>(length);
  std::size_t position = 0;
  while (length - position >= 4) {
    std::uint32_t k = 0;
    std::memcpy(&k, data + position, sizeof(k));
    k *= m;
    k ^= k >> r;
    k *= m;
    h *= m;
    h ^= k;
    position += 4;
  }
  const auto remaining = length - position;
  if (remaining == 3) h ^= static_cast<std::uint32_t>(data[position + 2]) << 16;
  if (remaining >= 2) h ^= static_cast<std::uint32_t>(data[position + 1]) << 8;
  if (remaining >= 1) {
    h ^= data[position];
    h *= m;
  }
  h ^= h >> 13;
  h *= m;
  h ^= h >> 15;
  return h;
}

struct MemberName {
  std::uint32_t hash = 0;
  std::uint32_t symbol = 0xffffffffu;
  const char* string = nullptr;
};

MemberName MakeMemberName(std::string_view key) {
  std::string lowered(key);
  for (char& c : lowered) {
    c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  }
  return MemberName{
      MurmurHash2(reinterpret_cast<const unsigned char*>(lowered.data()),
                  lowered.size(), 0x31415926u),
      0xffffffffu,
      key.data()};
}

struct RawKeyValues3 {
  std::uint64_t metadata = 0;
  std::uint64_t data = 0;
};

void SetKvString(RawKeyValues3& value, const char* text) {
  constexpr std::uint64_t kTypeMask = 0xffull << 2;
  constexpr std::uint64_t kSubtypeMask = 0xffull << 10;
  constexpr std::uint64_t kArrayCountMask = 0x1full << 34;
  value.metadata &= ~(kTypeMask | kSubtypeMask | kArrayCountMask | (1ull << 1));
  value.metadata |= static_cast<std::uint64_t>(0x26u) << 2;
  value.metadata |= static_cast<std::uint64_t>(26u) << 10;
  value.data = reinterpret_cast<std::uintptr_t>(text);
}

void SetKvBool(RawKeyValues3& value, bool enabled) {
  constexpr std::uint64_t kTypeMask = 0xffull << 2;
  constexpr std::uint64_t kSubtypeMask = 0xffull << 10;
  constexpr std::uint64_t kArrayCountMask = 0x1full << 34;
  value.metadata &= ~(kTypeMask | kSubtypeMask | kArrayCountMask | (1ull << 1));
  value.metadata |= static_cast<std::uint64_t>(2u) << 2;
  value.metadata |= static_cast<std::uint64_t>(13u) << 10;
  value.data = enabled ? 1u : 0u;
}

void* IdentityOf(void* entity) {
  void* identity = nullptr;
  if (entity) {
    std::memcpy(&identity, static_cast<std::byte*>(entity) + kIdentityOffset,
                sizeof(identity));
  }
  return identity;
}

std::uint64_t HashMix(std::uint64_t hash, std::uint64_t value) {
  constexpr std::uint64_t kPrime = 1099511628211ull;
  hash ^= value;
  hash *= kPrime;
  return hash;
}

std::uint64_t Quantized(float value) {
  if (!std::isfinite(value)) return 0;
  return static_cast<std::uint64_t>(
      static_cast<std::int64_t>(std::llround(value * 10.0f)));
}

std::uint64_t PointHash(std::uint64_t hash, const std::array<float, 3>& point) {
  for (float value : point) hash = HashMix(hash, Quantized(value));
  return hash;
}

std::vector<std::array<float, 3>> SampleRoute(
    const std::vector<RecordedTick>& route) {
  std::vector<std::array<float, 3>> result;
  if (route.empty()) return result;

  const std::size_t count = std::min(route.size(), kMaxTracerPoints);
  result.reserve(count);
  for (std::size_t i = 0; i < count; ++i) {
    const std::size_t index =
        count == 1 ? 0 : i * (route.size() - 1) / (count - 1);
    auto point = route[index].pawn.position;
    point[2] += 8.0f;
    result.push_back(point);
  }
  return result;
}

std::string LineCommand(const std::array<float, 3>& a,
                        const std::array<float, 3>& b) {
  std::ostringstream out;
  out << std::fixed << std::setprecision(2)
      << "drawline "
      << a[0] << ' ' << a[1] << ' ' << a[2] << ' '
      << b[0] << ' ' << b[1] << ' ' << b[2] << "\n";
  return out.str();
}

std::string AxisCommand(const std::array<float, 3>& point,
                        int r, int g, int b) {
  std::ostringstream out;
  out << std::fixed << std::setprecision(2)
      << "axis " << point[0] << ' ' << point[1] << ' ' << point[2]
      << " 0 0 0 10 " << r << ' ' << g << ' ' << b << " 255\n";
  return out.str();
}

}  // namespace

struct NativeVisualRenderer::Impl {
  void* engine = nullptr;
  std::uint64_t last_signature = 0;
  bool installed = false;
  bool cheats_requested = false;
  bool particle_ready = false;

  using ServerCommandFn = void (*)(void*, const char*);
  using CreateEntityFn = void* (*)(void*, const char*, int);
  using QueueSpawnFn = void (*)(void*, void*, void*);
  using ExecuteQueuedFn = void (*)(void*);
  using AcceptInputFn = bool (*)(void*, const char*, void*, void*, void*, int, void*);
  using KvAllocateFn = void* (*)(std::size_t);
  using KvConstructFn = void* (*)(void*, void*, unsigned char);
  using KvSetFn = void* (*)(void*, const MemberName*, unsigned char);

  CreateEntityFn create_entity = nullptr;
  QueueSpawnFn queue_spawn = nullptr;
  ExecuteQueuedFn execute_queued = nullptr;
  AcceptInputFn accept_input = nullptr;
  KvAllocateFn kv_allocate = nullptr;
  KvConstructFn kv_construct = nullptr;
  KvSetFn kv_set = nullptr;

  void Command(std::string_view command) {
    if (!engine) return;
    auto** table = *static_cast<void***>(engine);
    if (!table || !table[kServerCommandSlot]) return;
    auto fn = reinterpret_cast<ServerCommandFn>(table[kServerCommandSlot]);
    std::string owned(command);
    fn(engine, owned.c_str());
  }

  void EnsureCheats() {
    if (cheats_requested) return;
    Command("sv_cheats 1\n");
    cheats_requested = true;
  }
};

NativeVisualRenderer::NativeVisualRenderer() : impl_(std::make_unique<Impl>()) {}
NativeVisualRenderer::~NativeVisualRenderer() = default;

bool NativeVisualRenderer::Install(std::string& error) {
#if !defined(_WIN32)
  error = "native visual renderer currently requires Windows";
  return false;
#else
  error.clear();

  // The Modlock-style particle path is independent from Source2EngineToServer001.
  // Keep the old debug-command bridge only as an optional fallback for the
  // pre-existing drawline renderer.
  std::string debug_error;
  impl_->engine =
      ResolveInterface(L"engine2.dll", "Source2EngineToServer001", debug_error);
  if (impl_->engine) {
    auto** table = *static_cast<void***>(impl_->engine);
    if (!table || !table[kServerCommandSlot]) {
      impl_->engine = nullptr;
    }
  }

  HMODULE server = GetModuleHandleW(L"server.dll");
  if (!server) {
    error = "server.dll is not mapped";
    return false;
  }

  impl_->create_entity = reinterpret_cast<Impl::CreateEntityFn>(
      FindUnique(server, kCreateEntityPattern, "entity-system.create-entity-by-name", error));
  if (!impl_->create_entity) return false;
  impl_->queue_spawn = reinterpret_cast<Impl::QueueSpawnFn>(
      FindUnique(server, kQueueSpawnPattern, "entity-system.queue-spawn-entity", error));
  if (!impl_->queue_spawn) return false;
  impl_->execute_queued = reinterpret_cast<Impl::ExecuteQueuedFn>(
      FindUnique(server, kExecuteQueuedPattern, "entity-system.execute-queued-creation", error));
  if (!impl_->execute_queued) return false;
  impl_->accept_input = reinterpret_cast<Impl::AcceptInputFn>(
      FindUnique(server, kAcceptInputPattern, "entity-instance.accept-input", error));
  if (!impl_->accept_input) return false;
  impl_->kv_allocate = reinterpret_cast<Impl::KvAllocateFn>(
      FindUnique(server, kKvAllocatePattern, "entity-keyvalues.allocate", error));
  if (!impl_->kv_allocate) return false;
  impl_->kv_construct = reinterpret_cast<Impl::KvConstructFn>(
      FindUnique(server, kKvConstructPattern, "entity-keyvalues.construct", error));
  if (!impl_->kv_construct) return false;
  impl_->kv_set = reinterpret_cast<Impl::KvSetFn>(
      FindUnique(server, kKvSetPattern, "entity-keyvalues.set-key-value", error));
  if (!impl_->kv_set) return false;

  impl_->particle_ready = true;
  impl_->installed = true;
  return true;
#endif
}

bool NativeVisualRenderer::Ready() const {
  return impl_->installed && impl_->engine;
}

bool NativeVisualRenderer::ParticleReady() const {
  return impl_->installed && impl_->particle_ready;
}

void NativeVisualRenderer::Invalidate() {
  impl_->last_signature = 0;
}

bool NativeVisualRenderer::DrawTestParticle(
    const std::array<float, 3>& position, std::string& error) {
#if !defined(_WIN32)
  (void)position;
  error = "native particle renderer currently requires Windows";
  return false;
#else
  error.clear();
  if (!ParticleReady()) {
    error = "native particle ABI is not ready";
    return false;
  }
  for (float coordinate : position) {
    if (!std::isfinite(coordinate)) {
      error = "test particle position is not finite";
      return false;
    }
  }

  std::string interface_error;
  void* resource_service = ResolveInterface(
      L"engine2.dll", "GameResourceServiceServerV001", interface_error);
  if (!resource_service) {
    error = "GameResourceServiceServerV001: " + interface_error;
    return false;
  }

  void* entity_system = nullptr;
  std::memcpy(&entity_system,
              static_cast<std::byte*>(resource_service) +
                  kResourceServiceEntitySystemOffset,
              sizeof(entity_system));
  if (!entity_system) {
    error = "CGameResourceService has no live entity system";
    return false;
  }

  void* particle = impl_->create_entity(nullptr, "info_particle_system", -1);
  if (!particle) {
    error = "CreateEntityByName(info_particle_system) returned null";
    return false;
  }

  void* identity = IdentityOf(particle);
  if (!identity) {
    error = "info_particle_system has no entity identity";
    return false;
  }

  void* storage = impl_->kv_allocate(kEntityKeyValuesSize);
  if (!storage) {
    error = "CEntityKeyValues allocation returned null";
    return false;
  }
  void* key_values = impl_->kv_construct(storage, nullptr, 0);
  if (!key_values) {
    error = "CEntityKeyValues construction returned null";
    return false;
  }

  static constexpr char kEffect[] =
      "particles/environment/powerup_spawner_ambient_ring.vpcf";
  const MemberName effect_name = MakeMemberName("effect_name");
  auto* effect_value = static_cast<RawKeyValues3*>(
      impl_->kv_set(key_values, &effect_name, 0));
  if (!effect_value) {
    error = "could not create effect_name keyvalue";
    return false;
  }
  SetKvString(*effect_value, kEffect);

  const MemberName start_active = MakeMemberName("start_active");
  auto* active_value = static_cast<RawKeyValues3*>(
      impl_->kv_set(key_values, &start_active, 0));
  if (!active_value) {
    error = "could not create start_active keyvalue";
    return false;
  }
  SetKvBool(*active_value, true);

  auto origin = position;
  origin[2] += 48.0f;
  constexpr std::size_t kTeleportSlot = 163;
  auto*** object = reinterpret_cast<void***>(particle);
  if (!object || !*object || !(*object)[kTeleportSlot]) {
    error = "CBaseEntity::Teleport is unavailable for info_particle_system";
    return false;
  }
  using TeleportFn = void (*)(void*, const float*, const float*, const float*);
  const std::array<float, 3> angles{};
  const std::array<float, 3> velocity{};
  reinterpret_cast<TeleportFn>((*object)[kTeleportSlot])(
      particle, origin.data(), angles.data(), velocity.data());

  impl_->queue_spawn(entity_system, identity, key_values);
  impl_->execute_queued(entity_system);

  struct alignas(8) Variant {
    const void* pointer = nullptr;
    std::uint8_t type = 30;
    std::uint8_t alignment = 0;
    std::uint16_t flags = 0;
  };
  static_assert(sizeof(Variant) == 16);
  static constexpr char kEmpty[] = "";
  Variant value{.pointer = kEmpty};
  if (!impl_->accept_input(
          particle, "Start", nullptr, nullptr, &value, 0, nullptr)) {
    error = "info_particle_system rejected Start input";
    return false;
  }

  return true;
#endif
}

void NativeVisualRenderer::Clear() {
  if (!Ready()) return;
  impl_->EnsureCheats();
  impl_->Command("cleardraw\n");
  impl_->last_signature = 0;
}

void NativeVisualRenderer::Render(
    const std::vector<RecordedTick>& route,
    const std::optional<RaceCourse>& course,
    const NativeVisualSettings& settings) {
  if (!Ready()) return;

  std::uint64_t signature = 1469598103934665603ull;
  signature = HashMix(signature, settings.tracer ? 1u : 0u);
  signature = HashMix(signature, settings.rings ? 1u : 0u);
  signature = HashMix(signature, settings.checkpoints ? 1u : 0u);
  signature = HashMix(signature, route.size());

  if (!route.empty()) {
    signature = PointHash(signature, route.front().pawn.position);
    signature = PointHash(signature, route.back().pawn.position);
  }
  if (course) {
    signature = PointHash(signature, course->start);
    signature = PointHash(signature, course->finish);
    signature = HashMix(signature, Quantized(course->radius));
    signature = HashMix(signature, course->checkpoints.size());
    for (const auto& checkpoint : course->checkpoints) {
      signature = PointHash(signature, checkpoint);
    }
  }

  if (impl_->last_signature == signature) return;

  impl_->EnsureCheats();
  impl_->Command("cleardraw\n");

  if (settings.tracer) {
    const auto points = SampleRoute(route);
    for (std::size_t i = 1; i < points.size(); ++i) {
      impl_->Command(LineCommand(points[i - 1], points[i]));
    }
  }

  auto draw_ring = [&](const std::array<float, 3>& center, float radius) {
    constexpr double kTau = 6.28318530717958647692;
    for (std::size_t i = 0; i < kRingSegments; ++i) {
      const double a0 = kTau * static_cast<double>(i) / kRingSegments;
      const double a1 = kTau * static_cast<double>(i + 1) / kRingSegments;
      std::array<float, 3> p0{
          center[0] + radius * static_cast<float>(std::cos(a0)),
          center[1] + radius * static_cast<float>(std::sin(a0)),
          center[2] + 8.0f};
      std::array<float, 3> p1{
          center[0] + radius * static_cast<float>(std::cos(a1)),
          center[1] + radius * static_cast<float>(std::sin(a1)),
          center[2] + 8.0f};
      impl_->Command(LineCommand(p0, p1));
    }
  };

  if (course && settings.rings) {
    draw_ring(course->start, course->radius);
    draw_ring(course->finish, course->radius);
    auto start = course->start;
    auto finish = course->finish;
    start[2] += 8.0f;
    finish[2] += 8.0f;
    impl_->Command(AxisCommand(start, 64, 220, 120));
    impl_->Command(AxisCommand(finish, 255, 120, 80));
  }

  if (course && settings.checkpoints) {
    for (const auto& checkpoint : course->checkpoints) {
      draw_ring(checkpoint, course->radius * 0.72f);
      auto marker = checkpoint;
      marker[2] += 8.0f;
      impl_->Command(AxisCommand(marker, 90, 170, 255));
    }
  }

  impl_->last_signature = signature;
}

}  // namespace split::movement
