#include "split_movement/pb_route_store.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <fstream>
#include <limits>

namespace split::movement {
namespace {

constexpr std::array<char, 8> kMagic{'S', 'P', 'L', 'I', 'T', 'P', 'B', '1'};
constexpr std::uint32_t kVersion = 1;
constexpr std::uint64_t kMaxTicks = 60ull * 60ull * 30ull;

template <typename T>
bool Write(std::ofstream& out, const T& value) {
  out.write(reinterpret_cast<const char*>(&value), sizeof(value));
  return static_cast<bool>(out);
}

template <typename T>
bool Read(std::ifstream& in, T& value) {
  in.read(reinterpret_cast<char*>(&value), sizeof(value));
  return static_cast<bool>(in);
}

bool WriteTick(std::ofstream& out, const RecordedTick& tick) {
  if (!Write(out, tick.frame) ||
      !Write(out, tick.pawn.slot) ||
      !Write(out, tick.pawn.pawn_handle) ||
      !Write(out, tick.pawn.hero_id) ||
      !Write(out, tick.pawn.team) ||
      !Write(out, tick.pawn.ground_entity_handle)) {
    return false;
  }

  for (float value : tick.pawn.position) if (!Write(out, value)) return false;
  for (float value : tick.pawn.velocity) if (!Write(out, value)) return false;

  const std::uint8_t grounded = tick.pawn.grounded ? 1 : 0;
  const std::uint8_t has_input = tick.input ? 1 : 0;
  if (!Write(out, grounded) || !Write(out, has_input)) return false;

  if (tick.input) {
    for (float value : tick.input->angles) if (!Write(out, value)) return false;
    if (!Write(out, tick.input->forward) ||
        !Write(out, tick.input->left) ||
        !Write(out, tick.input->held) ||
        !Write(out, tick.input->pressed) ||
        !Write(out, tick.input->sequence)) {
      return false;
    }
  }
  return true;
}

bool ReadTick(std::ifstream& in, RecordedTick& tick) {
  if (!Read(in, tick.frame) ||
      !Read(in, tick.pawn.slot) ||
      !Read(in, tick.pawn.pawn_handle) ||
      !Read(in, tick.pawn.hero_id) ||
      !Read(in, tick.pawn.team) ||
      !Read(in, tick.pawn.ground_entity_handle)) {
    return false;
  }

  for (float& value : tick.pawn.position) if (!Read(in, value)) return false;
  for (float& value : tick.pawn.velocity) if (!Read(in, value)) return false;

  std::uint8_t grounded = 0;
  std::uint8_t has_input = 0;
  if (!Read(in, grounded) || !Read(in, has_input)) return false;
  tick.pawn.grounded = grounded != 0;

  if (has_input) {
    InputSample input;
    for (float& value : input.angles) if (!Read(in, value)) return false;
    if (!Read(in, input.forward) ||
        !Read(in, input.left) ||
        !Read(in, input.held) ||
        !Read(in, input.pressed) ||
        !Read(in, input.sequence)) {
      return false;
    }
    tick.input = input;
  } else {
    tick.input.reset();
  }
  return true;
}

}  // namespace

PbRouteStore::PbRouteStore(std::filesystem::path directory)
    : directory_(std::move(directory)) {}

std::filesystem::path PbRouteStore::Path(std::uint64_t course_id) const {
  return directory_ / ("pb-route-" + std::to_string(course_id) + ".bin");
}

bool PbRouteStore::Exists(std::uint64_t course_id) const {
  std::error_code ec;
  return std::filesystem::is_regular_file(Path(course_id), ec) && !ec;
}

std::expected<void, std::string> PbRouteStore::Save(
    std::uint64_t course_id, const std::vector<RecordedTick>& ticks) const {
  if (ticks.empty()) return std::unexpected("PB route is empty");
  if (ticks.size() > kMaxTicks) return std::unexpected("PB route is too large");

  std::error_code ec;
  std::filesystem::create_directories(directory_, ec);
  if (ec) {
    return std::unexpected("could not create PB route directory: " + ec.message());
  }

  std::ofstream out(Path(course_id), std::ios::binary | std::ios::trunc);
  if (!out) return std::unexpected("could not open PB route file");

  out.write(kMagic.data(), static_cast<std::streamsize>(kMagic.size()));
  const auto count = static_cast<std::uint64_t>(ticks.size());
  if (!Write(out, kVersion) || !Write(out, count)) {
    return std::unexpected("could not write PB route header");
  }

  for (const auto& tick : ticks) {
    if (!WriteTick(out, tick)) {
      return std::unexpected("could not write PB route tick");
    }
  }
  return {};
}

std::expected<std::vector<RecordedTick>, std::string> PbRouteStore::Load(
    std::uint64_t course_id) const {
  std::ifstream in(Path(course_id), std::ios::binary);
  if (!in) return std::unexpected("no saved PB ghost");

  std::array<char, 8> magic{};
  in.read(magic.data(), static_cast<std::streamsize>(magic.size()));
  if (!in || magic != kMagic) {
    return std::unexpected("PB route file has invalid magic");
  }

  std::uint32_t version = 0;
  std::uint64_t count = 0;
  if (!Read(in, version) || !Read(in, count)) {
    return std::unexpected("PB route header is truncated");
  }
  if (version != kVersion) {
    return std::unexpected("PB route version is unsupported");
  }
  if (count == 0 || count > kMaxTicks ||
      count > static_cast<std::uint64_t>(std::numeric_limits<std::size_t>::max())) {
    return std::unexpected("PB route tick count is invalid");
  }

  std::vector<RecordedTick> ticks(static_cast<std::size_t>(count));
  for (auto& tick : ticks) {
    if (!ReadTick(in, tick)) {
      return std::unexpected("PB route file is truncated");
    }
  }
  return ticks;
}

}  // namespace split::movement
