#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace split::movement {

struct ClientConnection {
  int slot = -1;
  std::uint64_t xuid = 0;
  bool is_bot = false;
  bool fully_connected = false;
  std::string name;
};

class NativeHooks {
 public:
  using FrameCallback = std::function<void(std::uint64_t)>;
  using WorldCallback = std::function<void(std::string_view)>;
  using ClientCallback = std::function<void(const ClientConnection&)>;
  using DisconnectCallback = std::function<void(int, std::uint64_t)>;

  NativeHooks();
  ~NativeHooks();

  NativeHooks(const NativeHooks&) = delete;
  NativeHooks& operator=(const NativeHooks&) = delete;

  bool Install(FrameCallback on_frame, WorldCallback before_world, WorldCallback after_world,
               ClientCallback on_connected, ClientCallback on_fully_connected,
               DisconnectCallback on_disconnecting, std::string& error);
  bool Installed() const;
  std::uint64_t FrameCount() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
