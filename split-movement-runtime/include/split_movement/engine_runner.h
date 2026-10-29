#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>

namespace split::movement {

struct ServerLaunch {
  std::uint16_t port = 27067;
  std::string map = "dl_midtown";
  std::string extra_args;
};

int RunDedicatedServer(const std::filesystem::path& game_dir, const ServerLaunch& launch,
                       std::string& error);

// Run one command through IVEngineServer2::ServerCommand after Source 2 has
// started. This is the same server-console path Modlock uses for world setup.
bool RunServerCommand(std::string_view command, std::string& error);

}  // namespace split::movement
