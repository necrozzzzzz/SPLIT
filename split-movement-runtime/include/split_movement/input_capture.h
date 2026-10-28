#pragma once

#include <array>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace split::movement {

struct InputSample {
  std::array<float, 3> angles{};
  float forward = 0.0f;
  float left = 0.0f;
  std::uint64_t held = 0;
  std::uint64_t pressed = 0;
  std::uint64_t sequence = 0;
};

struct PlaybackStatus {
  bool active = false;
  int slot = -1;
  std::size_t index = 0;
  std::size_t total = 0;
};

class InputCapture {
 public:
  InputCapture();
  ~InputCapture();

  InputCapture(const InputCapture&) = delete;
  InputCapture& operator=(const InputCapture&) = delete;

  bool Install(std::string& error);
  std::optional<InputSample> Latest(int slot) const;

  bool StartPlayback(int slot, std::vector<InputSample> inputs, std::string& error);
  void StopPlayback();
  PlaybackStatus Playback() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
