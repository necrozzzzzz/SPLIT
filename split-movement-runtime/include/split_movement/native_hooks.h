#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace split::movement {

class NativeHooks {
 public:
  using FrameCallback = std::function<void(std::uint64_t)>;
  using WorldCallback = std::function<void(std::string_view)>;

  NativeHooks();
  ~NativeHooks();

  NativeHooks(const NativeHooks&) = delete;
  NativeHooks& operator=(const NativeHooks&) = delete;

  bool Install(FrameCallback on_frame, WorldCallback before_world, WorldCallback after_world,
               std::string& error);
  bool Installed() const;
  std::uint64_t FrameCount() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
