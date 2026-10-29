#pragma once

#include <atomic>
#include <filesystem>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace split::movement {

class MovementHotkeys {
 public:
  using Callback = std::function<void(std::string_view action)>;

  MovementHotkeys();
  ~MovementHotkeys();

  MovementHotkeys(const MovementHotkeys&) = delete;
  MovementHotkeys& operator=(const MovementHotkeys&) = delete;

  bool Start(const std::filesystem::path& config_path, Callback callback,
             std::string& error);
  void Stop();

  void SetEnabled(bool enabled);
  bool SetBinding(std::string_view action, std::string_view binding,
                  std::string& error);

  std::string Binding(std::string_view action) const;
  std::string BindingsJson() const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
