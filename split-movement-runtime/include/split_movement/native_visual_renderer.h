#pragma once

#include <array>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "split_movement/race_timer.h"
#include "split_movement/recorder.h"

namespace split::movement {

struct NativeVisualSettings {
  bool tracer = true;
  bool rings = true;
  bool checkpoints = true;
};

class NativeVisualRenderer {
 public:
  NativeVisualRenderer();
  ~NativeVisualRenderer();

  NativeVisualRenderer(const NativeVisualRenderer&) = delete;
  NativeVisualRenderer& operator=(const NativeVisualRenderer&) = delete;

  bool Install(std::string& error);
  bool Ready() const;
  bool ParticleReady() const;
  bool ManifestReady() const;
  bool TransmitReady() const;
  const std::string& OptionalError() const;
  void Invalidate();
  bool DrawTestParticle(const std::array<float, 3>& position, std::string& error);
  bool DrawVisibleMarker(const std::array<float, 3>& position, std::string& error);
  bool DrawWorldTextMarker(const std::array<float, 3>& position,
                           const char* message, std::string& error);
  void Clear();
  void Render(const std::vector<RecordedTick>& route,
              const std::optional<RaceCourse>& course,
              const NativeVisualSettings& settings);

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace split::movement
