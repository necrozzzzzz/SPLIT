#include "split_movement/movement_hotkeys.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cwctype>
#include <chrono>
#include <fstream>
#include <map>
#include <mutex>
#include <optional>
#include <sstream>
#include <thread>
#include <utility>
#include <vector>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {
namespace {

struct KeyChord {
  int vk = 0;
  bool ctrl = false;
  bool alt = false;
  bool shift = false;
  std::string display;
};

std::string Trim(std::string value) {
  while (!value.empty() && std::isspace(static_cast<unsigned char>(value.front()))) {
    value.erase(value.begin());
  }
  while (!value.empty() && std::isspace(static_cast<unsigned char>(value.back()))) {
    value.pop_back();
  }
  return value;
}

std::string Upper(std::string value) {
  std::transform(value.begin(), value.end(), value.begin(), [](unsigned char c) {
    return static_cast<char>(std::toupper(c));
  });
  return value;
}

std::string JsonEscape(std::string_view value) {
  std::string out;
  out.reserve(value.size());
  for (char c : value) {
    if (c == '\\' || c == '"') out.push_back('\\');
    out.push_back(c);
  }
  return out;
}

std::optional<int> ParseVirtualKey(std::string_view raw) {
#if !defined(_WIN32)
  (void)raw;
  return std::nullopt;
#else
  const std::string key = Upper(std::string(raw));
  if (key.size() == 1) {
    const unsigned char c = static_cast<unsigned char>(key[0]);
    if ((c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')) {
      return static_cast<int>(c);
    }
  }

  if (key.size() >= 2 && key[0] == 'F') {
    try {
      const int number = std::stoi(key.substr(1));
      if (number >= 1 && number <= 24) return VK_F1 + number - 1;
    } catch (...) {
    }
  }

  static const std::map<std::string, int> kNamed{
      {"SPACE", VK_SPACE}, {"TAB", VK_TAB}, {"ENTER", VK_RETURN},
      {"BACKSPACE", VK_BACK}, {"ESC", VK_ESCAPE}, {"ESCAPE", VK_ESCAPE},
      {"PAGEUP", VK_PRIOR}, {"PAGEDOWN", VK_NEXT}, {"HOME", VK_HOME},
      {"END", VK_END}, {"INSERT", VK_INSERT}, {"DELETE", VK_DELETE},
      {"LEFT", VK_LEFT}, {"RIGHT", VK_RIGHT}, {"UP", VK_UP}, {"DOWN", VK_DOWN},
      {"[", VK_OEM_4}, {"]", VK_OEM_6}, {";", VK_OEM_1}, {"'", VK_OEM_7},
      {",", VK_OEM_COMMA}, {".", VK_OEM_PERIOD}, {"/", VK_OEM_2},
      {"\\", VK_OEM_5}, {"-", VK_OEM_MINUS}, {"=", VK_OEM_PLUS},
  };
  const auto found = kNamed.find(key);
  if (found != kNamed.end()) return found->second;
  return std::nullopt;
#endif
}

std::optional<KeyChord> ParseChord(std::string_view text) {
  std::string input = Trim(std::string(text));
  if (input.empty()) return std::nullopt;

  KeyChord chord;
  std::stringstream stream(input);
  std::string token;
  std::string key_token;
  while (std::getline(stream, token, '+')) {
    token = Trim(token);
    const std::string upper = Upper(token);
    if (upper == "CTRL" || upper == "CONTROL") {
      chord.ctrl = true;
    } else if (upper == "ALT") {
      chord.alt = true;
    } else if (upper == "SHIFT") {
      chord.shift = true;
    } else {
      if (!key_token.empty()) return std::nullopt;
      key_token = token;
    }
  }

  const auto vk = ParseVirtualKey(key_token);
  if (!vk) return std::nullopt;
  chord.vk = *vk;

  std::string display;
  if (chord.ctrl) display += "Ctrl+";
  if (chord.alt) display += "Alt+";
  if (chord.shift) display += "Shift+";
  display += key_token.size() == 1 ? Upper(key_token) : key_token;
  chord.display = display;
  return chord;
}

bool ChordDown(const KeyChord& chord) {
#if !defined(_WIN32)
  (void)chord;
  return false;
#else
  const auto down = [](int vk) {
    return (GetAsyncKeyState(vk) & 0x8000) != 0;
  };
  if (chord.ctrl != (down(VK_CONTROL) || down(VK_LCONTROL) || down(VK_RCONTROL))) {
    if (chord.ctrl || down(VK_CONTROL) || down(VK_LCONTROL) || down(VK_RCONTROL)) return false;
  }
  if (chord.alt != (down(VK_MENU) || down(VK_LMENU) || down(VK_RMENU))) {
    if (chord.alt || down(VK_MENU) || down(VK_LMENU) || down(VK_RMENU)) return false;
  }
  if (chord.shift != (down(VK_SHIFT) || down(VK_LSHIFT) || down(VK_RSHIFT))) {
    if (chord.shift || down(VK_SHIFT) || down(VK_LSHIFT) || down(VK_RSHIFT)) return false;
  }
  return down(chord.vk);
#endif
}

bool DeadlockForeground() {
#if !defined(_WIN32)
  return false;
#else
  const HWND window = GetForegroundWindow();
  if (!window) return false;

  DWORD pid = 0;
  GetWindowThreadProcessId(window, &pid);
  if (pid == 0) return false;

  HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
  if (!process) return false;

  std::array<wchar_t, 1024> path{};
  DWORD length = static_cast<DWORD>(path.size());
  const BOOL ok = QueryFullProcessImageNameW(process, 0, path.data(), &length);
  CloseHandle(process);
  if (!ok || length == 0) return false;

  std::wstring_view full(path.data(), length);
  const std::size_t slash = full.find_last_of(L"\\/");
  std::wstring exe(full.substr(slash == std::wstring_view::npos ? 0 : slash + 1));
  std::transform(exe.begin(), exe.end(), exe.begin(), [](wchar_t c) {
    return static_cast<wchar_t>(std::towlower(c));
  });
  return exe == L"deadlock.exe";
#endif
}

constexpr std::array<std::string_view, 5> kActions{
    "retry", "watch", "stop", "menu", "playlists"};

std::map<std::string, std::string> DefaultBindings() {
  return {
      {"retry", "E"},
      {"watch", "L"},
      {"stop", "I"},
      {"menu", "F6"},
      {"playlists", "9"},
  };
}

}  // namespace

struct MovementHotkeys::Impl {
  std::filesystem::path path;
  Callback callback;
  mutable std::mutex mutex;
  std::map<std::string, std::string> text = DefaultBindings();
  std::map<std::string, KeyChord> chords;
  std::map<std::string, bool> was_down;
  std::atomic<bool> enabled{false};
  std::jthread worker;

  bool Save(std::string& error) {
    std::ofstream out(path, std::ios::trunc);
    if (!out) {
      error = "could not write " + path.string();
      return false;
    }
    out << "# Movement Lab hotkeys\n"
        << "# Format: action=Key or Ctrl+Key / Alt+Key / Shift+Key\n";
    for (const auto action : kActions) {
      out << action << "=" << text[std::string(action)] << "\n";
    }
    return static_cast<bool>(out);
  }

  bool Rebuild(std::string& error) {
    chords.clear();
    for (const auto action : kActions) {
      const std::string name(action);
      const auto parsed = ParseChord(text[name]);
      if (!parsed) {
        error = "invalid hotkey for " + name + ": " + text[name];
        return false;
      }
      chords[name] = *parsed;
      was_down[name] = false;
    }
    return true;
  }
};

MovementHotkeys::MovementHotkeys() : impl_(std::make_unique<Impl>()) {}
MovementHotkeys::~MovementHotkeys() { Stop(); }

bool MovementHotkeys::Start(const std::filesystem::path& config_path,
                            Callback callback, std::string& error) {
  error.clear();
  Stop();
  impl_->path = config_path;
  impl_->callback = std::move(callback);
  impl_->text = DefaultBindings();

  if (std::ifstream input(config_path); input) {
    std::string line;
    while (std::getline(input, line)) {
      line = Trim(line);
      if (line.empty() || line[0] == '#') continue;
      const auto equals = line.find('=');
      if (equals == std::string::npos) continue;
      const std::string action = Upper(Trim(line.substr(0, equals)));
      const std::string value = Trim(line.substr(equals + 1));
      for (const auto known : kActions) {
        if (Upper(std::string(known)) == action && !value.empty()) {
          impl_->text[std::string(known)] = value;
        }
      }
    }
  }

  {
    std::scoped_lock lock(impl_->mutex);
    if (!impl_->Rebuild(error)) return false;
    if (!std::filesystem::exists(config_path) && !impl_->Save(error)) return false;
  }

  impl_->worker = std::jthread([this](std::stop_token stop) {
    using namespace std::chrono_literals;
    while (!stop.stop_requested()) {
      if (!impl_->enabled.load(std::memory_order_acquire) || !DeadlockForeground()) {
        std::scoped_lock lock(impl_->mutex);
        for (auto& [_, down] : impl_->was_down) down = false;
        std::this_thread::sleep_for(20ms);
        continue;
      }

      std::vector<std::string> fired;
      {
        std::scoped_lock lock(impl_->mutex);
        for (const auto& [action, chord] : impl_->chords) {
          const bool down = ChordDown(chord);
          if (down && !impl_->was_down[action]) fired.push_back(action);
          impl_->was_down[action] = down;
        }
      }

      for (const auto& action : fired) {
        if (impl_->callback) impl_->callback(action);
      }
      std::this_thread::sleep_for(8ms);
    }
  });

  return true;
}

void MovementHotkeys::Stop() {
  if (!impl_) return;
  impl_->enabled.store(false, std::memory_order_release);
  if (impl_->worker.joinable()) {
    impl_->worker.request_stop();
    impl_->worker.join();
  }
}

void MovementHotkeys::SetEnabled(bool enabled) {
  if (impl_) impl_->enabled.store(enabled, std::memory_order_release);
}

bool MovementHotkeys::SetBinding(std::string_view action,
                                 std::string_view binding,
                                 std::string& error) {
  error.clear();
  const std::string name = Upper(std::string(action));
  std::string canonical;
  for (const auto known : kActions) {
    if (Upper(std::string(known)) == name) canonical = std::string(known);
  }
  if (canonical.empty()) {
    error = "unknown Movement Lab hotkey action";
    return false;
  }

  const auto parsed = ParseChord(binding);
  if (!parsed) {
    error = "invalid hotkey; examples: E, F6, PageUp, Ctrl+R";
    return false;
  }

  std::scoped_lock lock(impl_->mutex);
  for (const auto& [other, chord] : impl_->chords) {
    if (other != canonical && chord.display == parsed->display) {
      error = parsed->display + " is already assigned to " + other;
      return false;
    }
  }

  impl_->text[canonical] = parsed->display;
  impl_->chords[canonical] = *parsed;
  impl_->was_down[canonical] = false;
  return impl_->Save(error);
}

std::string MovementHotkeys::Binding(std::string_view action) const {
  if (!impl_) return {};
  std::scoped_lock lock(impl_->mutex);
  const auto found = impl_->text.find(std::string(action));
  return found == impl_->text.end() ? std::string{} : found->second;
}

std::string MovementHotkeys::BindingsJson() const {
  if (!impl_) return "{}";
  std::scoped_lock lock(impl_->mutex);
  std::ostringstream out;
  out << "{";
  bool first = true;
  for (const auto action : kActions) {
    if (!first) out << ",";
    first = false;
    const auto found = impl_->text.find(std::string(action));
    const std::string value = found == impl_->text.end() ? "" : found->second;
    out << "\"" << action << "\":\"" << JsonEscape(value) << "\"";
  }
  out << "}";
  return out.str();
}

}  // namespace split::movement
