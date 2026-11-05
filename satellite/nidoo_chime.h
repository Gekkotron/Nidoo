// The wake chime: the satellite heard "Hey Jarvis" and is listening.
//
// The same sound as the Mac satellite (crates/nidoo-satellite/src/playout.rs):
// two rising bell notes, C6 then G6 80 ms later, each a 3 ms fade-in and a
// 40 ms decay, peaking near a third of full scale; 200 ms in all. Made once as
// s16le, 48 kHz, stereo, the DAC's format, and fed to the mixer's chime input
// a piece at a time: a stopped mixer input takes nothing until it has started.
#pragma once

#include <cmath>
#include <cstdint>
#include <vector>

#include "esphome/components/audio/audio.h"
#include "esphome/components/speaker/speaker.h"

static const uint32_t NC_RATE = 48000;
static const uint32_t NC_FRAMES = NC_RATE / 5;  // 200 ms

static std::vector<uint8_t> nc_pcm;
static size_t nc_at = 0;  // bytes of nc_pcm already handed to the speaker

inline void nc_make() {
  static const float NOTES[2][2] = {{1046.5f, 0.0f}, {1568.0f, 0.080f}};
  nc_pcm.resize(NC_FRAMES * 4);
  for (uint32_t i = 0; i < NC_FRAMES; i++) {
    float t = (float) i / NC_RATE, v = 0.0f;
    for (const auto &n : NOTES) {
      if (t < n[1]) continue;
      float dt = t - n[1];
      float attack = dt / 0.003f < 1.0f ? dt / 0.003f : 1.0f;
      v += 0.3f * attack * expf(-dt / 0.040f) * sinf(2.0f * (float) M_PI * n[0] * dt);
    }
    if (v > 1.0f) v = 1.0f;
    if (v < -1.0f) v = -1.0f;
    int16_t s = (int16_t) lrintf(v * 32767.0f);
    uint8_t lo = s & 0xff, hi = (uint16_t) s >> 8;
    uint8_t *p = &nc_pcm[i * 4];
    p[0] = lo, p[1] = hi, p[2] = lo, p[3] = hi;  // same on both sides
  }
}

// Builds the chime once, at boot, so the wake-word callback never pays for it.
inline void nidoo_chime_prepare() {
  if (nc_pcm.empty()) nc_make();
}

// Plays the chime from its start (again, if it is still playing).
inline void nidoo_chime_start() {
  nidoo_chime_prepare();  // lazy fallback
  nc_at = 0;
}

// Feeds what the speaker takes of the rest, without waiting; then lets it
// stop once the chime has played. Called every 20 ms.
inline void nidoo_chime_pump(esphome::speaker::Speaker *spk) {
  if (nc_pcm.empty() || nc_at >= nc_pcm.size()) return;
  nc_at += spk->play(nc_pcm.data() + nc_at, nc_pcm.size() - nc_at, 0);
  if (nc_at >= nc_pcm.size()) spk->finish();
}
