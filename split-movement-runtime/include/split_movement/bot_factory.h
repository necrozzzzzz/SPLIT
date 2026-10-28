#pragma once

#include <array>
#include <cstdint>
#include <memory>
#include <string>

namespace split::movement {

class BotFactory {
 public:
  BotFactory();
  ~BotFactory();

  BotFactory(const BotFactory&) = delete;
  BotFactory& operator=(const BotFactory&) = delete;

  bool Install(std::string& error);
  bool Create(const char* name, int team, std::uint32_t hero_id,
              const std::array<float, 3>& position, std::string& error) const;

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
