// Speech input for the Nidoo satellite: the signed UDP messages of
// docs/reference/voice-protocol.md (hub on 4960, this board on 4961).
//
// The board only talks to the hub that greeted it with a valid Hello, streams
// its microphone only inside a window (wake word, or the hub's Listen), and
// accepts a hub message only when its counter rises. Every datagram carries
// the first 8 bytes of HMAC-SHA256(key, everything before the tag).
#pragma once

#include <cstdint>
#include <cstring>
#include <vector>

#include <fcntl.h>
#include <lwip/sockets.h>
#include <mbedtls/md.h>

#include "esphome/core/log.h"

namespace nidoo_voice {

static const char *const TAG = "nidoo_voice";

static const uint8_t NV_HELLO = 0x01, NV_STOP = 0x02, NV_LISTEN = 0x03;
static const uint8_t NV_WAKE = 0x10, NV_AUDIO = 0x11, NV_END = 0x12;
static const size_t NV_HEADER = 16, NV_TAG_LEN = 8, NV_MAX_SAMPLES = 640;

// What nidoo_voice_poll() found.
static const int NV_NOTHING = 0, NV_GOT_HELLO = 1, NV_GOT_STOP = 2, NV_GOT_LISTEN = 3;

static int fd = -1;
static uint8_t key[32];
static bool key_ok = false;
static bool greeted = false;
static char sid[12];
static uint64_t nonce = 0, last_hub_ctr = 0, listen_ctr = 0;
static struct sockaddr_in hub;
static uint32_t session = 0, seq = 0;

inline int hex_digit(char c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'a' && c <= 'f') return c - 'a' + 10;
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}

inline void put_u32(uint8_t *p, uint32_t v) {
  for (int i = 0; i < 4; i++) p[i] = (uint8_t) (v >> (8 * i));
}

inline void put_u64(uint8_t *p, uint64_t v) {
  for (int i = 0; i < 8; i++) p[i] = (uint8_t) (v >> (8 * i));
}

inline uint32_t get_u32(const uint8_t *p) {
  uint32_t v = 0;
  for (int i = 3; i >= 0; i--) v = (v << 8) | p[i];
  return v;
}

inline uint64_t get_u64(const uint8_t *p) {
  uint64_t v = 0;
  for (int i = 7; i >= 0; i--) v = (v << 8) | p[i];
  return v;
}

inline void hmac(const uint8_t *data, size_t n, uint8_t out[NV_TAG_LEN]) {
  uint8_t full[32];
  const mbedtls_md_info_t *info = mbedtls_md_info_from_type(MBEDTLS_MD_SHA256);
  mbedtls_md_hmac(info, key, sizeof(key), data, n, full);
  std::memcpy(out, full, NV_TAG_LEN);
}

// The tag at the end of `d` matches, compared in constant time.
inline bool signed_ok(const uint8_t *d, size_t n) {
  if (n < NV_HEADER + NV_TAG_LEN) return false;
  uint8_t want[NV_TAG_LEN];
  hmac(d, n - NV_TAG_LEN, want);
  uint8_t diff = 0;
  for (size_t i = 0; i < NV_TAG_LEN; i++) diff |= want[i] ^ d[n - NV_TAG_LEN + i];
  return diff == 0;
}

inline size_t header(uint8_t *b, uint8_t type) {
  b[0] = 'N';
  b[1] = 'V';
  b[2] = 1;
  b[3] = type;
  std::memcpy(b + 4, sid, sizeof(sid));
  return NV_HEADER;
}

// Signs the first `n` bytes of `b` and sends them to the hub.
inline void send_signed(uint8_t *b, size_t n) {
  if (fd < 0 || !greeted) return;
  hmac(b, n, b + n);
  ::sendto(fd, b, n + NV_TAG_LEN, 0, (struct sockaddr *) &hub, sizeof(hub));
}

}  // namespace nidoo_voice

// Opens the control socket; `key_hex` is the 64-hex-character voice_key.
inline void nidoo_voice_setup(const char *key_hex, uint16_t board_port) {
  using namespace nidoo_voice;
  key_ok = std::strlen(key_hex) == 64;
  for (int i = 0; key_ok && i < 32; i++) {
    int hi = hex_digit(key_hex[2 * i]), lo = hex_digit(key_hex[2 * i + 1]);
    if (hi < 0 || lo < 0) key_ok = false;
    else key[i] = (uint8_t) (hi * 16 + lo);
  }
  if (!key_ok) {
    ESP_LOGW(TAG, "voice_key is not 64 hex characters: speech input is off");
    return;
  }
  fd = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
  if (fd < 0) return;
  struct sockaddr_in me;
  std::memset(&me, 0, sizeof(me));
  me.sin_family = AF_INET;
  me.sin_port = htons(board_port);
  me.sin_addr.s_addr = htonl(INADDR_ANY);
  if (::bind(fd, (struct sockaddr *) &me, sizeof(me)) < 0) {
    ESP_LOGW(TAG, "cannot listen on UDP %u", board_port);
    ::close(fd);
    fd = -1;
    return;
  }
  ::fcntl(fd, F_SETFL, ::fcntl(fd, F_GETFL, 0) | O_NONBLOCK);
  ESP_LOGI(TAG, "listening for the hub on UDP %u", board_port);
}

// Reads the waiting hub messages up to the first Stop or Listen (one per call,
// so none is lost when two arrive together); Hellos are taken on the way.
inline int nidoo_voice_poll() {
  using namespace nidoo_voice;
  if (fd < 0) return NV_NOTHING;
  int found = NV_NOTHING;
  uint8_t b[128];
  for (;;) {
    ssize_t n = ::recvfrom(fd, b, sizeof(b), 0, nullptr, nullptr);
    if (n <= 0) break;
    size_t len = (size_t) n;
    if (len < NV_HEADER + NV_TAG_LEN + 8 || b[0] != 'N' || b[1] != 'V' || b[2] != 1) continue;
    if (!signed_ok(b, len)) continue;
    uint8_t type = b[3];
    size_t body = len - NV_HEADER - NV_TAG_LEN;
    uint64_t ctr = 0;
    if (type == NV_HELLO && body == 22) ctr = get_u64(b + 24);
    else if (type == NV_STOP && body == 12) ctr = get_u64(b + 16);
    else if (type == NV_LISTEN && body == 8) ctr = get_u64(b + 16);
    else continue;
    if (ctr <= last_hub_ctr) continue;  // replayed or out of order
    last_hub_ctr = ctr;
    if (type == NV_HELLO) {
      std::memcpy(sid, b + 4, sizeof(sid));
      nonce = get_u64(b + 16);
      std::memset(&hub, 0, sizeof(hub));
      hub.sin_family = AF_INET;
      std::memcpy(&hub.sin_addr.s_addr, b + 32, 4);  // address order
      hub.sin_port = htons((uint16_t) (b[36] | (b[37] << 8)));
      if (!greeted) ESP_LOGI(TAG, "greeted by the hub");
      greeted = true;
      if (found == NV_NOTHING) found = NV_GOT_HELLO;
    } else if (type == NV_STOP) {
      if (get_u32(b + 24) == session) return NV_GOT_STOP;
    } else {
      listen_ctr = ctr;
      return NV_GOT_LISTEN;
    }
  }
  return found;
}

// A hub greeted this board: a window may stream.
inline bool nidoo_voice_ready() { return nidoo_voice::greeted; }

// Opens a session: 0 after the wake word, 1 for a follow-up the hub asked for.
inline void nidoo_voice_wake(uint8_t kind) {
  using namespace nidoo_voice;
  session++;
  seq = 0;
  uint8_t b[NV_HEADER + 21 + NV_TAG_LEN];
  size_t n = header(b, NV_WAKE);
  put_u64(b + n, nonce);
  put_u32(b + n + 8, session);
  b[n + 12] = kind;
  put_u64(b + n + 13, kind == 1 ? listen_ctr : 0);
  send_signed(b, n + 21);
}

// Sends one microphone buffer (32-bit I2S samples, top 16 bits kept), at most
// 640 samples per datagram.
inline void nidoo_voice_audio(const std::vector<uint8_t> &x) {
  using namespace nidoo_voice;
  const int32_t *s = reinterpret_cast<const int32_t *>(x.data());
  size_t total = x.size() / 4;
  static uint8_t b[NV_HEADER + 8 + NV_MAX_SAMPLES * 2 + NV_TAG_LEN];
  for (size_t i = 0; i < total;) {
    size_t k = total - i < NV_MAX_SAMPLES ? total - i : NV_MAX_SAMPLES;
    size_t n = header(b, NV_AUDIO);
    put_u32(b + n, session);
    put_u32(b + n + 4, ++seq);
    int16_t *out = reinterpret_cast<int16_t *>(b + n + 8);
    for (size_t j = 0; j < k; j++) out[j] = (int16_t) (s[i + j] >> 16);
    send_signed(b, n + 8 + k * 2);
    i += k;
  }
}

// The board closed the window itself (its own time limit).
inline void nidoo_voice_end() {
  using namespace nidoo_voice;
  uint8_t b[NV_HEADER + 4 + NV_TAG_LEN];
  size_t n = header(b, NV_END);
  put_u32(b + n, session);
  send_signed(b, n + 4);
}
