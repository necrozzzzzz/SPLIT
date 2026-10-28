#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace split::movement {

class ControlServer {
 public:
  using Handler = std::function<void(std::string_view)>;

  ControlServer();
  ~ControlServer();

  ControlServer(const ControlServer&) = delete;
  ControlServer& operator=(const ControlServer&) = delete;

  bool Start(std::uint16_t port, Handler handler, std::string& error);
  void Stop();

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
