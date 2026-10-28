cmake_minimum_required(VERSION 3.24)
project(split_movement_runtime VERSION 0.3.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(split-movement-runtime
  src/main.cpp
  src/runtime.cpp
  src/engine_bridge.cpp
  src/engine_runner.cpp
  src/native_hooks.cpp
)

target_include_directories(split-movement-runtime PRIVATE include)

if(MSVC)
  target_compile_options(split-movement-runtime PRIVATE /W4 /permissive-)
else()
  target_compile_options(split-movement-runtime PRIVATE -Wall -Wextra -Wpedantic)
endif()
