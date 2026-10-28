#pragma once

#include <cstdint>
#include <filesystem>
#include <string>

namespace split::movement {

struct ServerLaunch {
  std::uint16_t port = 27067;
  std::string map = "dl_midtown";
  std::string extra_args;
};

int RunDedicatedServer(const std::filesystem::path& game_dir, const ServerLaunch& launch,
                       std::string& error);

}  // namespace split::movement
