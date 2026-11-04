---
title: Voice protocol
sidebar_position: 8
description: The signed UDP messages between a speaker satellite and the hub: layout, replay rules and test vectors.
---

# Voice protocol (satellite ↔ hub)

A speaker satellite sends its microphone to the hub over UDP, only inside a
*window*: after the wake word, or after the hub's **Listen** (a follow-up
answer). The hub listens on UDP **4960** (Assistant → Speech settings → Speech
input), each board on UDP **4961**. Messages are **authenticated, not
encrypted**: a device without the shared key can neither inject audio nor send
a board's microphone elsewhere, but someone on the network can hear a window's
audio.

## Datagram

Every datagram, in both directions, all numbers little-endian:

| Bytes | Field |
|-------|-------|
| 0–1 | `"NV"` |
| 2 | version, `1` |
| 3 | type |
| 4–15 | the satellite's short id, 12 lowercase hex characters (the `<sid>` of `nidoo_sound_<sid>`) |
| 16… | body (below) |
| last 8 | tag: the first 8 bytes of `HMAC-SHA256(key, every byte before the tag)` |

The key is 32 bytes, written as 64 hex characters (`voice_key` in the board's
`secrets.yaml`). A datagram with a wrong tag is dropped before its body is read.

## Messages

Hub → board:

| Type | Name | Body |
|------|------|------|
| `0x01` | Hello | `nonce` u64, `hub_ctr` u64, hub IPv4 (4 bytes, in address order), hub port u16 |
| `0x02` | Stop | `hub_ctr` u64, `session` u32 |
| `0x03` | Listen | `hub_ctr` u64 |

Board → hub:

| Type | Name | Body |
|------|------|------|
| `0x10` | Wake | `nonce` u64, `session` u32, `kind` u8 (`0` wake word, `1` follow-up), `listen_ctr` u64 (the Listen's `hub_ctr`, `0` for a wake word) |
| `0x11` | Audio | `session` u32, `seq` u32, 1 to 640 samples, 16 kHz mono s16le |
| `0x12` | End | `session` u32 (the board closed the window itself) |

- The hub greets every Allowed, connected satellite with a **Hello** every
  5 s, at the address the sound service reports. The board streams to the
  address *inside* the Hello, never to the datagram's source. A board that has
  had no Hello sends nothing.
- **Wake** opens a session; **Audio** follows with a rising `seq`; the hub
  sends **Stop** as soon as the phrase is over. The board closes a window by
  itself after 8 s (wake word) or 6 s (follow-up) and sends **End**.
- **Listen** is sent only once the reply has finished playing on that board
  (plus a margin, 300 ms by default), so the board does not hear itself.

## Replay rules

- Each Hello carries a fresh random `nonce`. A Wake must echo one of the last
  3 nonces the hub sent to that satellite, with a `session` above every session
  already opened under that nonce (sessions start at 1 and rise during one
  boot).
- Audio is accepted only for the open session, with a `seq` above the last one.
- `hub_ctr` rises with every hub message, across hub restarts (it follows the
  wall clock in µs). A board accepts a hub message only when its `hub_ctr` is
  above the last one it accepted since boot.

## Test vectors

Key bytes `00 01 02 … 1f`, satellite `0123456789ab`:

- Hello `nonce 1`, `hub_ctr 2`, hub `192.0.2.10:4960`:
  `4e56010130313233343536373839616201000000000000000200000000000000c000020a6013eebff1b9d2213d37`
- Audio `session 1`, `seq 7`, samples `[0, 1, -1, 32767]`:
  `4e560111303132333435363738396162010000000700000000000100ffffff7f0c9b91e10bae7b2e`
