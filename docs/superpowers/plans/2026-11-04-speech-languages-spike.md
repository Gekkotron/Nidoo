# Spike: per-request normalisation and the memory of a second model

Run 2026-11-04 on a MacBook (Apple Silicon), `gradium/phonon-40m-fr` at commit `d910869`,
`model.q8.gguf`, release build, test `tts_real::normalising_outside_the_builder_matches`.

- **Normalisation outside the builder works.** `Normalize::for_lang(Lang::Fr).apply` turns
  "Il fait 19,5 degrés le 3 novembre." into "Il fait 19 virgule 5 degrés le 3 novembre.". A
  model built with `Normalize::OFF` and fed that text speaks the same 216 960 samples as a model
  built for French, seed 42. The largest sample difference is 2.6e-6 (peak 1.27), the same size as
  the model's own run-to-run difference (2.8e-6), so they are not bit-exact but sound identical.
  **Result: one loaded model can serve several languages; no fallback needed.**
- **A second loaded q8 40m model costs about 125 MB** of resident memory (one model: 134 MB over
  the empty process; two: 260 MB). Keeping every model in use loaded is affordable on the hub host
  (16 GB).
