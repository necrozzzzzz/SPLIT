#include "split_movement/control_server.h"

#include <array>
#include <atomic>
#include <chrono>
#include <thread>
#include <utility>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
#endif

namespace split::movement {

struct ControlServer::Impl {
  std::jthread worker;
#if defined(_WIN32)
  SOCKET socket = INVALID_SOCKET;
#endif
};

ControlServer::ControlServer() : impl_(std::make_unique<Impl>()) {}
ControlServer::~ControlServer() { Stop(); }

bool ControlServer::Start(std::uint16_t port, Handler handler, std::string& error) {
  error.clear();
  Stop();

#if !defined(_WIN32)
  (void)port;
  (void)handler;
  error = "movement control server requires Windows";
  return false;
#else
  WSADATA data{};
  if (WSAStartup(MAKEWORD(2, 2), &data) != 0) {
    error = "WSAStartup failed";
    return false;
  }

  SOCKET socket = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
  if (socket == INVALID_SOCKET) {
    error = "could not create movement control socket";
    WSACleanup();
    return false;
  }

  sockaddr_in address{};
  address.sin_family = AF_INET;
  address.sin_port = htons(port);
  address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  if (::bind(socket, reinterpret_cast<const sockaddr*>(&address), sizeof(address)) ==
      SOCKET_ERROR) {
    error = "could not bind movement control socket";
    closesocket(socket);
    WSACleanup();
    return false;
  }

  DWORD timeout_ms = 200;
  setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO,
             reinterpret_cast<const char*>(&timeout_ms), sizeof(timeout_ms));

  impl_->socket = socket;
  impl_->worker = std::jthread(
      [socket, handler = std::move(handler)](std::stop_token stop) mutable {
        std::array<char, 128> buffer{};
        while (!stop.stop_requested()) {
          sockaddr_in sender{};
          int sender_size = sizeof(sender);
          const int received =
              recvfrom(socket, buffer.data(), static_cast<int>(buffer.size() - 1), 0,
                       reinterpret_cast<sockaddr*>(&sender), &sender_size);
          if (received == SOCKET_ERROR) {
            const int code = WSAGetLastError();
            if (code == WSAETIMEDOUT || code == WSAEWOULDBLOCK) continue;
            if (stop.stop_requested()) break;
            continue;
          }
          if (received <= 0) continue;

          std::string_view command(buffer.data(), static_cast<std::size_t>(received));
          while (!command.empty() &&
                 (command.back() == '\r' || command.back() == '\n' ||
                  command.back() == ' ' || command.back() == '\t')) {
            command.remove_suffix(1);
          }
          while (!command.empty() &&
                 (command.front() == ' ' || command.front() == '\t')) {
            command.remove_prefix(1);
          }
          if (!command.empty()) handler(command);
        }
      });

  return true;
#endif
}

void ControlServer::Stop() {
  if (!impl_) return;
  if (impl_->worker.joinable()) {
    impl_->worker.request_stop();
#if defined(_WIN32)
    if (impl_->socket != INVALID_SOCKET) {
      shutdown(impl_->socket, SD_BOTH);
    }
#endif
    impl_->worker.join();
  }

#if defined(_WIN32)
  if (impl_->socket != INVALID_SOCKET) {
    closesocket(impl_->socket);
    impl_->socket = INVALID_SOCKET;
    WSACleanup();
  }
#endif
}

}  // namespace split::movement
