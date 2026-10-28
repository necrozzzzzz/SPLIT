#pragma once

#include <cstdint>
#include <expected>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "modlock/export.h"

namespace modlock::gameinterop {

// Signature is a parsed byte pattern. bytes and wildcard have the same length;
// each wildcard marks a byte that can match any value.
struct Signature {
  // Caller-chosen identity from the signature database entry.
  std::string id;
  // Concrete bytes; wildcards carry an unspecified value here.
  std::vector<uint8_t> bytes;
  // wildcard[i] is true when bytes[i] came from a run of question marks.
  std::vector<bool> wildcard;
};

// ParseSignature reads a signature-database pattern string such as
// "48 8B ?? ?? 74" into a Signature. Hex byte pairs may be adjacent or separated
// by whitespace. Each run of question marks marks one wildcard byte; other
// tokens return an error naming the signature.
[[nodiscard]] MODLOCK_API std::expected<Signature, std::string> ParseSignature(
    std::string id, std::string_view pattern);

// SignatureScan reports every offset in data where sig matches. Wildcard
// bytes match anything. Candidates that would run past the end of data never
// match, so a pattern truncated by the blob boundary yields no hit. The caller
// supplies a valid Signature and keeps the borrowed bytes alive during the scan.
[[nodiscard]] MODLOCK_API std::vector<size_t> SignatureScan(std::span<const uint8_t> data,
                                                            const Signature& sig);

}  // namespace modlock::gameinterop
