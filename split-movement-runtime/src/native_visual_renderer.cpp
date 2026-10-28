#include "split_movement/native_visual_renderer.h"

#include <algorithm>
#include <array>
#include <cmath>
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

  using ServerCommandFn = void (*)(void*, const char*);

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
  impl_->engine =
      ResolveInterface(L"engine2.dll", "Source2EngineToServer001", error);
  if (!impl_->engine) return false;

  auto** table = *static_cast<void***>(impl_->engine);
  if (!table || !table[kServerCommandSlot]) {
    error = "Source2EngineToServer001::ServerCommand is unavailable";
    impl_->engine = nullptr;
    return false;
  }

  impl_->installed = true;
  return true;
#endif
}

bool NativeVisualRenderer::Ready() const {
  return impl_->installed && impl_->engine;
}

void NativeVisualRenderer::Invalidate() {
  impl_->last_signature = 0;
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
