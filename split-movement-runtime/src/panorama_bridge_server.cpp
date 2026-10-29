#include "split_movement/panorama_bridge_server.h"

#include <array>
#include <chrono>
#include <condition_variable>
#include <mutex>
#include <string>
#include <string_view>
#include <thread>
#include <utility>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <winsock2.h>
#include <ws2tcpip.h>
#endif

namespace split::movement {
namespace {

std::string PackFrame(std::uint64_t sequence, const std::string& json) {
  std::string line = "~" + std::to_string(sequence) + ":";
  line.reserve(line.size() + json.size() * 2 + 1);

  auto append_utf8 = [&](unsigned codepoint) {
    if (codepoint <= 0x7f) {
      line.push_back(static_cast<char>(codepoint));
    } else if (codepoint <= 0x7ff) {
      line.push_back(static_cast<char>(0xc0 | ((codepoint >> 6) & 0x1f)));
      line.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
    }
  };

  for (unsigned char byte : json) {
    if (byte > ' ' && byte < 0x7f && byte != '\\') {
      line.push_back(static_cast<char>(byte));
    } else {
      append_utf8(0x100u + byte);
    }
  }
  line.push_back('\n');
  return line;
}

bool SendAll(SOCKET socket, const char* data, std::size_t size) {
  while (size > 0) {
    const int sent = send(socket, data, static_cast<int>(size), 0);
    if (sent <= 0) return false;
    data += sent;
    size -= static_cast<std::size_t>(sent);
  }
  return true;
}

}  // namespace

struct PanoramaBridgeServer::Impl {
  std::jthread worker;
  std::mutex mutex;
  std::condition_variable_any changed;
  std::string state = R"({"active":false})";
  std::uint64_t generation = 1;
#if defined(_WIN32)
  SOCKET listener = INVALID_SOCKET;
#endif
};

PanoramaBridgeServer::PanoramaBridgeServer() : impl_(std::make_unique<Impl>()) {}
PanoramaBridgeServer::~PanoramaBridgeServer() { Stop(); }

bool PanoramaBridgeServer::Start(std::uint16_t port, std::string& error) {
  error.clear();
  Stop();

#if !defined(_WIN32)
  (void)port;
  error = "Panorama bridge server requires Windows";
  return false;
#else
  WSADATA data{};
  if (WSAStartup(MAKEWORD(2, 2), &data) != 0) {
    error = "WSAStartup failed";
    return false;
  }

  SOCKET listener = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
  if (listener == INVALID_SOCKET) {
    error = "could not create Panorama bridge socket";
    WSACleanup();
    return false;
  }

  BOOL reuse = TRUE;
  setsockopt(listener, SOL_SOCKET, SO_REUSEADDR,
             reinterpret_cast<const char*>(&reuse), sizeof(reuse));

  sockaddr_in address{};
  address.sin_family = AF_INET;
  address.sin_port = htons(port);
  address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  if (::bind(listener, reinterpret_cast<const sockaddr*>(&address), sizeof(address)) ==
      SOCKET_ERROR) {
    error = "could not bind Panorama bridge socket";
    closesocket(listener);
    WSACleanup();
    return false;
  }

  if (listen(listener, 4) == SOCKET_ERROR) {
    error = "could not listen on Panorama bridge socket";
    closesocket(listener);
    WSACleanup();
    return false;
  }

  DWORD timeout_ms = 250;
  setsockopt(listener, SOL_SOCKET, SO_RCVTIMEO,
             reinterpret_cast<const char*>(&timeout_ms), sizeof(timeout_ms));

  impl_->listener = listener;
  impl_->worker = std::jthread([this, listener](std::stop_token stop) {
    while (!stop.stop_requested()) {
      sockaddr_in client_address{};
      int client_size = sizeof(client_address);
      SOCKET client =
          accept(listener, reinterpret_cast<sockaddr*>(&client_address), &client_size);
      if (client == INVALID_SOCKET) {
        const int code = WSAGetLastError();
        if (code == WSAETIMEDOUT || code == WSAEWOULDBLOCK) continue;
        if (stop.stop_requested()) break;
        continue;
      }

      DWORD client_timeout = 1000;
      setsockopt(client, SOL_SOCKET, SO_RCVTIMEO,
                 reinterpret_cast<const char*>(&client_timeout), sizeof(client_timeout));

      std::array<char, 4096> request{};
      const int received = recv(client, request.data(),
                                static_cast<int>(request.size() - 1), 0);
      if (received <= 0) {
        closesocket(client);
        continue;
      }

      std::string_view text(request.data(), static_cast<std::size_t>(received));
      const bool options = text.starts_with("OPTIONS ");
      const bool stream = text.starts_with("GET /panorama/stream ");

      if (options) {
        const std::string headers =
            "HTTP/1.1 204 No Content\r\n"
            "Access-Control-Allow-Origin: https://hyperline.gg\r\n"
            "Access-Control-Allow-Private-Network: true\r\n"
            "Access-Control-Allow-Methods: GET\r\n"
            "Cache-Control: no-store\r\n"
            "Content-Length: 0\r\n"
            "Connection: close\r\n\r\n";
        SendAll(client, headers.data(), headers.size());
        closesocket(client);
        continue;
      }

      if (!stream) {
        const std::string response =
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        SendAll(client, response.data(), response.size());
        closesocket(client);
        continue;
      }

      const std::string headers =
          "HTTP/1.1 200 OK\r\n"
          "Content-Type: text/plain; charset=utf-8\r\n"
          "Access-Control-Allow-Origin: https://hyperline.gg\r\n"
          "Access-Control-Allow-Private-Network: true\r\n"
          "Cache-Control: no-store\r\n"
          "Connection: keep-alive\r\n\r\n";
      if (!SendAll(client, headers.data(), headers.size())) {
        closesocket(client);
        continue;
      }

      std::uint64_t sent_generation = 0;
      std::uint64_t sequence = 1;
      while (!stop.stop_requested()) {
        std::string state;
        std::uint64_t generation = 0;
        {
          std::unique_lock lock(impl_->mutex);
          impl_->changed.wait_for(lock, stop, std::chrono::milliseconds(250), [&] {
            return impl_->generation != sent_generation;
          });
          if (stop.stop_requested()) break;
          state = impl_->state;
          generation = impl_->generation;
        }

        if (generation == sent_generation) continue;
        const std::string frame = PackFrame(sequence++, state);
        if (!SendAll(client, frame.data(), frame.size())) break;
        sent_generation = generation;
      }

      shutdown(client, SD_BOTH);
      closesocket(client);
    }
  });

  return true;
#endif
}

void PanoramaBridgeServer::SetState(std::string json) {
  if (!impl_) return;
  {
    std::scoped_lock lock(impl_->mutex);
    if (impl_->state == json) return;
    impl_->state = std::move(json);
    ++impl_->generation;
  }
  impl_->changed.notify_all();
}

void PanoramaBridgeServer::Stop() {
  if (!impl_) return;

  if (impl_->worker.joinable()) {
    impl_->worker.request_stop();
    impl_->changed.notify_all();
#if defined(_WIN32)
    if (impl_->listener != INVALID_SOCKET) shutdown(impl_->listener, SD_BOTH);
#endif
    impl_->worker.join();
  }

#if defined(_WIN32)
  if (impl_->listener != INVALID_SOCKET) {
    closesocket(impl_->listener);
    impl_->listener = INVALID_SOCKET;
    WSACleanup();
  }
#endif
}

}  // namespace split::movement
