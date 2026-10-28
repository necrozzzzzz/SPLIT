#include "split_movement/engine_runner.h"

#include <filesystem>
#include <string>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {

int RunDedicatedServer(const std::filesystem::path& game_dir, const ServerLaunch& launch,
                       std::string& error) {
#if !defined(_WIN32)
  (void)game_dir;
  (void)launch;
  error = "Source 2 engine handoff currently requires Windows";
  return 1;
#else
  HMODULE engine = GetModuleHandleW(L"engine2.dll");
  if (!engine) {
    error = "engine2.dll is not mapped";
    return 1;
  }

  using Source2MainFn = int (*)(void*, void*, const char*, int, const char*, const char*);
  const auto source2_main =
      reinterpret_cast<Source2MainFn>(GetProcAddress(engine, "Source2Main"));
  if (!source2_main) {
    error = "engine2.dll does not export Source2Main";
    return 1;
  }

  const auto root = std::filesystem::absolute(game_dir);
  const auto base_dir = root / "game";

  if (!std::filesystem::exists(base_dir / "citadel")) {
    error = "citadel game directory not found at " + (base_dir / "citadel").string();
    return 1;
  }

  std::string command =
      "-dedicated -nodedicatedconsole -novconsole -dev -insecure "
      "-allow_no_lobby_connect -playtest "
      "+tv_citadel_auto_record 0 +spec_replay_enable 0 +tv_enable 0 "
      "+citadel_upload_replay_enabled 0";
  command += " +hostport " + std::to_string(launch.port);
  command += " +ip 127.0.0.1";
  command += " +map " + launch.map;
  command += " -game \"" + (base_dir / "citadel").string() + "\"";
  if (!launch.extra_args.empty()) {
    command += " " + launch.extra_args;
  }

  // Prevent Source 2 assertion popups from trapping the engine thread.
  SetEnvironmentVariableW(L"NO_GAME_ASSERT_DIALOG", L"1");

  return source2_main(nullptr, nullptr, command.c_str(), 0, base_dir.string().c_str(), "citadel");
#endif
}

}  // namespace split::movement
