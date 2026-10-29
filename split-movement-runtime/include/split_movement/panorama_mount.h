#pragma once

#include <filesystem>
#include <string>

namespace split::movement {

class PanoramaMount {
 public:
  PanoramaMount() = default;
  ~PanoramaMount();

  PanoramaMount(const PanoramaMount&) = delete;
  PanoramaMount& operator=(const PanoramaMount&) = delete;

  bool Mount(const std::filesystem::path& game_dir,
             const std::filesystem::path& package_vpk,
             std::string& error);
  void Unmount();
  bool Mounted() const { return mounted_; }

 private:
  std::filesystem::path gameinfo_;
  bool mounted_ = false;
};

}  // namespace split::movement
