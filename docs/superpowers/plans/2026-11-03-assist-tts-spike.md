# Assist text-to-speech: speed spike notes

Engine: `ptts` 0.6.0 (Phonon), CPU only, `default-features = false, features = ["hf"]`, behind the `tts` feature of the `nidoo` crate.

## API facts for Task 6

- Imports: `ptts::checkpoint::{Checkpoint, ResolveOptions}`, `ptts::preprocess::Lang`, `ptts::synth::{DeviceKind, Quant, SpeechOptions}`.
- Load: `Checkpoint::resolve(dir, ResolveOptions { quant, weights: Some("model.q8.gguf") })?`, then `checkpoint.builder(Lang::Fr).device(DeviceKind::Cpu).build()?` returns a `ptts::synth::Synth`, then `checkpoint.register_voices(&mut synth)`.
- The methods `sample_rate`, `voices`, `languages`, `add_voice_file(&mut self, name, path)`, `say`, `say_with`, `stream`, `stream_with(text, &SpeechOptions)` live on the sealed trait **`ptts::synth::SynthApi`**. `Synth` derefs to `dyn SynthApi` (and `DerefMut` for `add_voice_file`), so on a concrete `Synth` value **no trait import is needed** (the spike test imports none; importing it gives an unused-import warning). Import `ptts::synth::SynthApi` only for generic code or a `&dyn SynthApi` parameter. `SessionApi` (`synth.session()`, with `say`/`stream`) is a second trait for a primed per-connection session; not needed.
- `stream_with` returns `ptts::synth::SpeechStream`, an `Iterator<Item = ptts::Result<Vec<f32>>>`. It is blocking ("use a blocking thread in async code"); dropping it joins its worker threads. It spawns 2 worker threads on the CPU (flow LM + Mimi decoder). One generation at a time per `Synth` (use one session per connection for concurrency).
- `Synth` is `Send + Sync` (trait bound), `say*`/`stream*` take `&self`.
- Quant: `Quant::Q80` for `model.q8.gguf`, `Quant::Q4k` for `model.q4k.gguf`.
- Sample rate: 24000 Hz, voices after `register_voices` on this model: `["default"]`.

## CPU thread count (xn)

- `ptts` runs on `xn` 0.2.13. `xn::get_num_threads()`: the value set by `xn::set_num_threads(n)` if non-zero, else the env var `RAYON_NUM_THREADS` (same as rayon), else `num_cpus::get()` (all logical cores).
- `xn::set_num_threads(n)` also does `std::env::set_var("RAYON_NUM_THREADS", n)` (unsafe, process-wide). The xn thread pool is **fixed at first use**, like rayon's global pool: set the limit before the first synthesis (or before loading), later calls only change chunking and cannot grow the pool. `XN_THREADPOOL=0` turns xn's own pool off (falls back to rayon's width); `XN_SPIN_BUDGET` tunes worker spinning.
- So a limit is possible two ways: set `RAYON_NUM_THREADS=<n>` in the hub's environment (no code, no extra dependency), or add `xn` as a direct dependency and call `xn::set_num_threads(n)` once at startup. Since `nidoo` does not depend on `xn`, the env var is the cheap route.
- Measured on the Mac (12 logical cores), warm round, French q8 sentence 2 (4.40 s of audio): default threads 0.38 s, `RAYON_NUM_THREADS=2` 0.30 s, `RAYON_NUM_THREADS=1` 0.52 s (RTF 0.12). The model is small enough that 1 to 2 threads are already well under real time.

## Mac numbers (this machine, Apple Silicon, 12 cores, release build)

Model: public `gradium/phonon-31m-en` (English, 31M parameters), `Quant::Q80`, CPU. Test: `crates/nidoo/tests/tts_real.rs`. The model accepted `Lang::Fr` normalisation and synthesised the French sentences (pronunciation quality of an English model on French text was not judged; only speed matters here).

| Run | Sentence | First chunk | Total | Audio | RTF |
|---|---|---|---|---|---|
| load | `model.q8.gguf` | - | 46 ms (warm file cache) | - | - |
| cold | Salon allumé (3 lumières). | 27 ms | 249 ms | 3.12 s | 0.08 |
| cold | Il fait 17,5 °C dans le salon. | 28 ms | 425 ms | 4.40 s | 0.10 |
| warm | Salon allumé (3 lumières). | 26 ms | 283 ms | 3.12 s | 0.09 |
| warm | Il fait 17,5 °C dans le salon. | 27 ms | 377 ms | 4.40 s | 0.09 |

English control (`Lang::En`, "The living room light is on."): first chunk 8 ms, total 131 ms warm, RTF 0.06.

Target from the spec: first audio under 1 s, faster than real time. The Mac is far inside it (first audio about 30 ms, RTF about 0.1). An Apple Silicon Mac is much faster than a mini PC, so these numbers do not settle the question for the hub host.

## q4k

`model.q4k.gguf` (`Quant::Q4k`) **loads and synthesises on CPU**: load 49 ms, warm total 236 ms and 386 ms, RTF 0.08 to 0.09, same speed as q8. Keep it in `WEIGHT_FILES`. File sizes: q8 64.5 MB, q4k 55.1 MB.

## Build cost

- Dev build of `nidoo` from an empty shared target (everything compiled, `ptts` among it): 2 m 10 s with the feature; the same crate with `--no-default-features` afterwards: 41 s (incremental on top of the shared deps, so not a clean measure). `ptts` plus `xn` and their dependencies add roughly a minute of clean-build CPU time.
- Release build of `nidoo`, workspace deps warm: 1 m 42 s with `tts`, 1 m 31 s without (about 10 s more).
- Release binary size (`CARGO_PROFILE_RELEASE_DEBUG=0`): 42 929 792 bytes with `tts`, 42 280 176 without, a difference of 0.65 MB. This is small only because nothing calls `ptts` yet; Task 6 will pull in the real code, so re-measure then.

## Mini PC (x86_64, the hub host): pending, the controller asks the user

Not measured. To fill in: on the hub host, release `ptts` (or `docker run ghcr.io/gradium-ai/ptts-openai-server:0.6.0`) with `HF_TOKEN` set, `time ptts --model hf://gradium/phonon-40m-fr --quant q8 --lang fr "Il fait 17,5 °C dans le salon." -o /tmp/t.wav`, twice (second run = warm cache). Gate: first audio under 1 s and synthesis faster than real time. Note the target model is the 40M French one, slightly larger than the 31M English one measured here.
