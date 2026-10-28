cmake_minimum_required(VERSION 3.24)
project(split_movement_runtime VERSION 0.8.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(FetchContent)
set(SAFETYHOOK_FETCH_ZYDIS ON CACHE BOOL "" FORCE)
set(SAFETYHOOK_BUILD_TESTS OFF CACHE BOOL "" FORCE)
FetchContent_Declare(
  safetyhook
  GIT_REPOSITORY https://github.com/cursey/safetyhook.git
  GIT_TAG v0.7.0
  GIT_SHALLOW TRUE
)
FetchContent_MakeAvailable(safetyhook)

add_executable(split-movement-runtime
  src/main.cpp
  src/runtime.cpp
  src/engine_bridge.cpp
  src/engine_runner.cpp
  src/native_hooks.cpp
  src/pawn_probe.cpp
  src/input_capture.cpp
  src/bot_factory.cpp
  src/recorder.cpp
)

target_include_directories(split-movement-runtime PRIVATE include)
target_link_libraries(split-movement-runtime PRIVATE safetyhook::safetyhook)

if(MSVC)
  target_compile_options(split-movement-runtime PRIVATE /W4 /permissive-)
else()
  target_compile_options(split-movement-runtime PRIVATE -Wall -Wextra -Wpedantic)
endif()
