#include "split_movement/bot_factory.h"

#include <cstddef>
#include <cstdint>
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

constexpr std::string_view kCreatePattern =
    "40 53 55 41 54 41 55 48 83 EC 38 4C 8B E1 49 8B E9 48 8B 0D ?? ?? ?? ?? 41 8B D8 44 8B EA";
constexpr std::string_view kDuplicateHeroPattern =
    "84 C0 0F 84 ?? ?? ?? ?? 48 8D 94 24 88 00 00 00 48 8D 4C 24 70";

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

void AllowDuplicateHero(safetyhook::Context& context) {
  context.rax &= ~std::uintptr_t{0xff};
}

}  // namespace

struct BotFactory::Impl {
  using CreateFn = void* (*)(const char*, int, std::uint32_t, const float*);

  CreateFn create = nullptr;
  safetyhook::MidHook duplicate_hero;
};

BotFactory::BotFactory() : impl_(std::make_unique<Impl>()) {}
BotFactory::~BotFactory() = default;

bool BotFactory::Install(std::string& error) {
  HMODULE server = GetModuleHandleW(L"server.dll");
  if (!server) {
    error = "server.dll is not mapped";
    return false;
  }

  void* create = FindUnique(server, kCreatePattern, "bot.create", error);
  if (!create) return false;

  void* guard =
      FindUnique(server, kDuplicateHeroPattern, "bot.duplicate-hero-check", error);
  if (!guard) return false;

  auto mid = safetyhook::create_mid(
      guard, &AllowDuplicateHero, safetyhook::MidHook::StartDisabled);
  if (!mid) {
    error = "could not create duplicate-hero guard hook";
    return false;
  }

  impl_->duplicate_hero = std::move(mid);
  if (auto enabled = impl_->duplicate_hero.enable(); !enabled) {
    impl_->duplicate_hero.reset();
    error = "could not enable duplicate-hero guard hook";
    return false;
  }

  impl_->create = reinterpret_cast<Impl::CreateFn>(create);
  return true;
}

bool BotFactory::Create(const char* name, int team, std::uint32_t hero_id,
                        const std::array<float, 3>& position,
                        std::string& error) const {
  if (!impl_->create) {
    error = "bot factory is not installed";
    return false;
  }
  if (!name || !*name || team < 2 || team > 3 || hero_id == 0) {
    error = "bot creation requires a name, team 2/3 and nonzero hero id";
    return false;
  }

  impl_->create(name, team, hero_id, position.data());
  return true;
}

}  // namespace split::movement
