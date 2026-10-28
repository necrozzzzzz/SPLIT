#pragma once

#include <filesystem>
#include <memory>
#include <string>
#include <vector>

namespace split::movement {

struct EngineBridgeStatus {
  bool game_layout_valid = false;
  bool engine2_loaded = false;
  bool server_loaded = false;
  bool schema_module_loaded = false;
  bool schema_interface_ready = false;
  std::string error;
};

class EngineBridge {
 public:
  EngineBridge();
  ~EngineBridge();

  EngineBridge(const EngineBridge&) = delete;
  EngineBridge& operator=(const EngineBridge&) = delete;

  EngineBridgeStatus Open(const std::filesystem::path& game_dir);
  bool Opened() const;
  void* SchemaSystem() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
