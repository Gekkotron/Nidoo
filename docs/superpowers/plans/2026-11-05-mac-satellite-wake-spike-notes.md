# Mac satellite — wake-word spike notes (Task 1)

Date: 2026-11-05. Code: `satellite/mac/spike/` (throwaway, own Cargo workspace,
never built by CI). Mac: Apple Silicon, one core per detector.

## Clips

Real recordings were not possible from the agent session, so the clips are
macOS voices (`say`), 16 kHz mono, kept out of git:

- **near**: 12 voices (5 English-accented, 7 French) say "Hey Jarvis. … allume
  la lumière du salon."
- **far**: the same clips at −18 dB with room reverb and a pink-noise floor.
- **music**: the same clips over a synthetic chord bed at −14 dB.
- **frac**: 8 French voices say "Hé, Djarvisse" (the way a French speaker
  says the English name), 3 spellings each, 24 clips.
- **negatives**: 7 min of French and English talk (news, kitchen talk, "Hé
  Charles", "jardin", "Jacques", "Gervais"), plus a planted English
  near-miss line ("hey Travis, hey Marvin, Jarvik, Harvey's").

## Candidates

| Engine | English voices (near / far / music) | French voices reading "Hey Jarvis" | "Djarvisse" (24) | False triggers | RTF |
|---|---|---|---|---|---|
| sherpa-onnx KWS, gigaspeech 3.3M (Apache-2.0), "HEY JARVIS" + 7 accent spellings | 4–5 / 5 | 0–1 / 7 | — | 0 in 7 min | 0.03 |
| **openWakeWord `hey_jarvis_v0.1`** (threshold 0.5) | **5/5 · 5/5 · 5/5** (scores 0.89–0.999) | 0–1 / 7 (best 0.41) | 7 at 0.5, 16 at 0.3 | 0 in ordinary French talk; **"Hey Charles" / "hey Travis" in English trigger** | **0.022** |

openWakeWord is clearly better on the phrase it was trained for. Both
engines miss the French TTS voices, which read "Hey Jarvis" as French text
("é jarvi", soft *j*). That is a TTS artefact more than a measure of a French
speaker talking to an assistant. The board's microWakeWord `hey_jarvis`,
trained on the same kind of synthetic data, already works with the owner's
real voice at cutoff 0.85 (speech-in spike).

## Decision

- **Engine: openWakeWord `hey_jarvis_v0.1`** through `ort` (ONNX Runtime),
  streaming exactly as openWakeWord does. Per 80 ms (1280 samples): melspectrogram
  of the last 1760 samples (int16 values as f32), frames `x/10 + 2`, one
  embedding from the last 76 mel frames, classifier on the last 16
  embeddings, first 5 scores skipped, 2 s refractory after a detection. See
  `satellite/mac/spike/src/bin/oww.rs`.
- **Threshold:** 0.5 by default, adjustable (`Settings.wake_threshold`).
- **Model files.** Licence **CC BY-NC-SA 4.0**, so they are downloaded at
  first run, never committed:
  - `https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/melspectrogram.onnx`
    `ba2b0e0f8b7b875369a2c89cb13360ff53bac436f2895cced9f479fa65eb176f`
  - `https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/embedding_model.onnx`
    `70d164290c1d095d1d4ee149bc5e00543250a7316b59f31d056cff7bd3075c1f`
  - `https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/hey_jarvis_v0.1.onnx`
    `94a13cfe60075b132f6a472e7e462e8123ee70861bc3fb58434a73712ee0d2cb`
- sherpa-onnx and `ort` must never be linked into one binary (each brings
  its own onnxruntime). The Mac app uses only `ort`.
- Build note: `sccache` fails on sherpa-onnx's static archive ("Archive member
  size is too large"); build the spike with `RUSTC_WRAPPER=`.

## Gate

- Synthetic English pronunciation: **passes** (5/5 near, far and music, 0
  false triggers in ordinary talk, RTF 0.022 < 0.2).
- French-accented pronunciation: **inconclusive** from TTS.
- **Live check with the owner's voice: pending.** Run
  `oww-spike <model dir> 0.5 --live`, say "Hey Jarvis" 10× at the desk and 5×
  from ~3 m, then talk normally for a minute.
