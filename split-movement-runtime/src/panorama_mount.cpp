#include "split_movement/panorama_mount.h"

#include <fstream>
#include <sstream>

namespace split::movement {
namespace {

constexpr const char* kBegin = "// SPLIT MOVEMENT BEGIN";
constexpr const char* kEnd = "// SPLIT MOVEMENT END";

std::string ReadText(const std::filesystem::path& path, std::string& error) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    error = "cannot open " + path.string();
    return {};
  }
  std::ostringstream out;
  out << input.rdbuf();
  return out.str();
}

bool WriteText(const std::filesystem::path& path, const std::string& text,
               std::string& error) {
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  if (!output) {
    error = "cannot write " + path.string();
    return false;
  }
  output.write(text.data(), static_cast<std::streamsize>(text.size()));
  if (!output) {
    error = "short write to " + path.string();
    return false;
  }
  return true;
}

bool RemoveBlock(std::string& text, std::string& error) {
  const auto begin = text.find(kBegin);
  const auto end = text.find(kEnd);
  if (begin == std::string::npos && end == std::string::npos) return true;
  if (begin == std::string::npos || end == std::string::npos || end < begin) {
    error = "gameinfo.gi contains an incomplete SPLIT MOVEMENT mount block";
    return false;
  }

  auto line_begin = text.rfind('\n', begin);
  line_begin = line_begin == std::string::npos ? 0 : line_begin + 1;
  auto line_end = text.find('\n', end);
  line_end = line_end == std::string::npos ? text.size() : line_end + 1;
  text.erase(line_begin, line_end - line_begin);
  return true;
}

bool AddBlock(std::string& text, std::string& error) {
  const auto search = text.find("SearchPaths");
  if (search == std::string::npos) {
    error = "gameinfo.gi has no SearchPaths block";
    return false;
  }
  const auto open = text.find('{', search);
  if (open == std::string::npos) {
    error = "gameinfo.gi SearchPaths block has no opening brace";
    return false;
  }

  const bool crlf = text.find("\r\n") != std::string::npos;
  const std::string nl = crlf ? "\r\n" : "\n";
  const std::string block =
      nl + "\t\t\t" + kBegin +
      nl + "\t\t\tGame\t\t\tcitadel/split_movement_runtime" +
      nl + "\t\t\tMod\t\t\t\"citadel\"" +
      nl + "\t\t\tWrite\t\t\t\"citadel\"" +
      nl + "\t\t\t" + kEnd;

  text.insert(open + 1, block);
  return true;
}

}  // namespace

PanoramaMount::~PanoramaMount() {
  Unmount();
}

bool PanoramaMount::Mount(const std::filesystem::path& game_dir,
                          const std::filesystem::path& package_vpk,
                          std::string& error) {
  error.clear();
  if (mounted_) return true;

  if (!std::filesystem::exists(package_vpk)) {
    error = "Panorama package missing: " + package_vpk.string();
    return false;
  }

  const auto package_dir =
      game_dir / "game" / "citadel" / "split_movement_runtime";
  std::error_code ec;
  std::filesystem::create_directories(package_dir, ec);
  if (ec) {
    error = "cannot create Panorama package directory: " + ec.message();
    return false;
  }

  std::filesystem::copy_file(
      package_vpk,
      package_dir / "pak01_dir.vpk",
      std::filesystem::copy_options::overwrite_existing,
      ec);
  if (ec) {
    error = "cannot install Panorama package: " + ec.message();
    return false;
  }

  gameinfo_ = game_dir / "game" / "citadel" / "gameinfo.gi";
  auto text = ReadText(gameinfo_, error);
  if (!error.empty()) return false;

  if (!RemoveBlock(text, error) || !AddBlock(text, error)) return false;
  if (!WriteText(gameinfo_, text, error)) return false;

  mounted_ = true;
  return true;
}

void PanoramaMount::Unmount() {
  if (!mounted_ || gameinfo_.empty()) return;

  std::string error;
  auto text = ReadText(gameinfo_, error);
  if (error.empty() && RemoveBlock(text, error)) {
    WriteText(gameinfo_, text, error);
  }
  mounted_ = false;
}

}  // namespace split::movement
