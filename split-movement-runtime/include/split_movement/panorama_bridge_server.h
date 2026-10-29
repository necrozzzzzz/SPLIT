#pragma once

#include <cstdint>
#include <memory>
#include <string>

namespace split::movement {

class PanoramaBridgeServer {
 public:
  PanoramaBridgeServer();
  ~PanoramaBridgeServer();

  PanoramaBridgeServer(const PanoramaBridgeServer&) = delete;
  PanoramaBridgeServer& operator=(const PanoramaBridgeServer&) = delete;

  bool Start(std::uint16_t port, std::string& error);
  void SetState(std::string json);
  void Stop();

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
