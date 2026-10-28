#include "split_movement/engine_bridge.h"

#include <filesystem>
#include <sstream>
#include <utility>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#endif

namespace split::movement {

namespace {

#if defined(_WIN32)

using CreateInterfaceFn = void* (*)(const char*, int*);

std::string WindowsError(const char* stage, DWORD code) {
  std::ostringstream out;
  out << stage << " failed with Windows error " << code;
  return out.str();
}

struct Module {
  HMODULE handle = nullptr;

  Module() = default;
  explicit Module(HMODULE value) : handle(value) {}
  Module(const Module&) = delete;
  Module& operator=(const Module&) = delete;

  Module(Module&& other) noexcept : handle(std::exchange(other.handle, nullptr)) {}
  Module& operator=(Module&& other) noexcept {
    if (this != &other) {
      if (handle) FreeLibrary(handle);
      handle = std::exchange(other.handle, nullptr);
    }
    return *this;
  }

  ~Module() {
    if (handle) FreeLibrary(handle);
  }
};

std::pair<Module, std::string> LoadModule(const std::filesystem::path& path) {
  HMODULE module = LoadLibraryExW(path.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
  if (!module) {
    return {Module{}, WindowsError(("LoadLibraryExW " + path.string()).c_str(), GetLastError())};
  }

  // Match the upstream host behavior: once Source 2 has seen a module, keep it
  // mapped for the lifetime of this runtime process.
  HMODULE pinned = nullptr;
  if (!GetModuleHandleExW(
          GET_MODULE_HANDLE_EX_FLAG_PIN | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
          reinterpret_cast<LPCWSTR>(module),
          &pinned)) {
    const DWORD code = GetLastError();
    FreeLibrary(module);
    return {Module{}, WindowsError(("pin " + path.string()).c_str(), code)};
  }

  return {Module{module}, {}};
}

#endif

}  // namespace

struct EngineBridge::Impl {
#if defined(_WIN32)
  std::vector<DLL_DIRECTORY_COOKIE> directories;
  Module engine2;
  Module server;
#endif
  bool opened = false;
  void* schema_system = nullptr;

  ~Impl() {
#if defined(_WIN32)
    for (auto cookie : directories) {
      if (cookie) RemoveDllDirectory(cookie);
    }
#endif
  }
};

EngineBridge::EngineBridge() : impl_(std::make_unique<Impl>()) {}
EngineBridge::~EngineBridge() = default;

bool EngineBridge::Opened() const {
  return impl_->opened;
}

void* EngineBridge::SchemaSystem() const {
  return impl_->schema_system;
}

EngineBridgeStatus EngineBridge::Open(const std::filesystem::path& game_dir) {
  EngineBridgeStatus status;

  if (impl_->opened) {
    status.error = "engine bridge is already open";
    return status;
  }

#if !defined(_WIN32)
  status.error = "SPLIT Movement Runtime engine bridge currently requires Windows";
  return status;
#else
  const auto root = std::filesystem::absolute(game_dir);
  const auto common_bin = root / "game" / "bin" / "win64";
  const auto citadel_bin = root / "game" / "citadel" / "bin" / "win64";
  const auto engine2_path = common_bin / "engine2.dll";
  const auto server_path = citadel_bin / "server.dll";

  if (!std::filesystem::exists(engine2_path)) {
    status.error = "engine2.dll not found at " + engine2_path.string();
    return status;
  }
  if (!std::filesystem::exists(server_path)) {
    status.error = "server.dll not found at " + server_path.string();
    return status;
  }
  status.game_layout_valid = true;

  if (!SetDefaultDllDirectories(
          LOAD_LIBRARY_SEARCH_APPLICATION_DIR |
          LOAD_LIBRARY_SEARCH_SYSTEM32 |
          LOAD_LIBRARY_SEARCH_USER_DIRS)) {
    status.error = WindowsError("SetDefaultDllDirectories", GetLastError());
    return status;
  }

  for (const auto& dir : {common_bin, citadel_bin}) {
    auto cookie = AddDllDirectory(dir.c_str());
    if (!cookie) {
      status.error = "AddDllDirectory failed for " + dir.string();
      return status;
    }
    impl_->directories.push_back(cookie);
  }

  auto [engine2, engine_error] = LoadModule(engine2_path);
  if (!engine_error.empty()) {
    status.error = std::move(engine_error);
    return status;
  }
  impl_->engine2 = std::move(engine2);
  status.engine2_loaded = true;

  auto [server, server_error] = LoadModule(server_path);
  if (!server_error.empty()) {
    status.error = std::move(server_error);
    return status;
  }
  impl_->server = std::move(server);
  status.server_loaded = true;

  HMODULE schema = GetModuleHandleW(L"schemasystem.dll");
  if (!schema) {
    const auto schema_path = common_bin / "schemasystem.dll";
    auto [loaded_schema, schema_error] = LoadModule(schema_path);
    if (!schema_error.empty()) {
      status.error = std::move(schema_error);
      return status;
    }

    // The module is pinned. Re-resolve its process handle before the temporary
    // ownership wrapper releases its ordinary LoadLibrary reference.
    schema = GetModuleHandleW(L"schemasystem.dll");
  }

  if (!schema) {
    status.error = "schemasystem.dll is not mapped after Source 2 module loading";
    return status;
  }
  status.schema_module_loaded = true;

  const auto create_interface =
      reinterpret_cast<CreateInterfaceFn>(GetProcAddress(schema, "CreateInterface"));
  if (!create_interface) {
    status.error = "schemasystem.dll does not export CreateInterface";
    return status;
  }

  int return_code = 0;
  impl_->schema_system = create_interface("SchemaSystem_001", &return_code);
  if (!impl_->schema_system) {
    status.error = "CreateInterface(SchemaSystem_001) returned null";
    return status;
  }

  status.schema_interface_ready = true;
  impl_->opened = true;
  return status;
#endif
}

}  // namespace split::movement
