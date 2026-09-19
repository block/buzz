# Frankie through Buzz

Run a local voice conversation through the native llama.cpp Frankie server, Buzz
agent, and this webpage. The browser captures and plays mono PCM16 at 24 kHz.
Buzz owns MCP tools and their permission decisions; llama.cpp loads the single
GGUF and runs the brain, Parakeet, Qwen3-TTS or Breeze, vision, and turn models
through ggml. The package selects the mouth.
No Python inference service or hosted model API is needed.

The page includes live transcripts, interruption, thinking controls, tool
approval, and latency measured from the end of speech to actual playback.

![Frankie voice demo](preview.png)

## Get the model

Obtain a compatible Frankie GGUF separately and save it as
`models/frankie.gguf` outside the source repositories. Both Qwen-TTS and Breeze
v3 packages work with the fork revision below. Model weights and reference
recordings are not distributed through this repository.

Verify the package against the checksum supplied with it using
`shasum -a 256 models/frankie.gguf` (macOS) or `sha256sum` (Linux). Keep its
manifest and model/voice license information with the artifact.

## Build llama.cpp

Prerequisites: Git, CMake, a C/C++ toolchain, ICU development headers/libraries,
and OpenSSL development headers/libraries. For CUDA, install the CUDA toolkit;
for Metal, use macOS with Xcode command-line tools. See the fork's
[Frankie build guide](https://github.com/tlongwell-block/llama.cpp/blob/c622f8f174ba869bebd7f2d4478c79586ff57ebf/tools/frankie/README.md)
for native dependencies and tuning.

Use the Frankie fork, including [PR #1](https://github.com/tlongwell-block/llama.cpp/pull/1).
The pinned revision includes both mouths, custom WAV references, and Breeze
sentence continuity. Stock llama.cpp cannot run this package.

```sh
git clone https://github.com/tlongwell-block/llama.cpp.git llama-frankie
cd llama-frankie
git checkout c622f8f174ba869bebd7f2d4478c79586ff57ebf

# Metal (macOS)
cmake -S . -B build-frankie -DCMAKE_BUILD_TYPE=Release \
  -DLLAMA_BUILD_FRANKIE=ON -DGGML_METAL=ON
cmake --build build-frankie --target llama-frankie-realtime -j4
cd ..
```

For CPU, replace `-DGGML_METAL=ON` with
`-DGGML_METAL=OFF -DGGML_CUDA=OFF`. For CUDA, use
`-DGGML_METAL=OFF -DGGML_CUDA=ON`. If CMake cannot find ICU, add
`-DICU_ROOT=/path/to/icu/prefix` using the prefix of your installation.

## Build Buzz and launch

The example uses Node 24 and the repository's Rust toolchain through Hermit.
Follow the root [contributing guide](../../CONTRIBUTING.md) for any platform
build prerequisites. The page has no npm runtime dependencies and needs no
frontend build.

```sh
git clone https://github.com/block/buzz.git buzz-frankie
cd buzz-frankie
git fetch origin pull/7519/head:frankie-demo
git checkout frankie-demo
. ./bin/activate-hermit
cargo build --release -p buzz-agent -p buzz-dev-mcp

node examples/realtime-audio/launch.mjs \
  --native ../llama-frankie/build-frankie/bin/llama-frankie-realtime \
  --model ../models/frankie.gguf \
  --device gpu
```

Use `--device cpu` with a CPU build. Open the complete `http://127.0.0.1:18796/#…`
URL printed by the launcher in Chrome, click **Start talking**, and allow the
microphone. Model loading and warm-up happen before this URL appears. Stop with
Ctrl-C. The launcher supports macOS and Linux.

Try “What is two plus two?” and then “Run uptime.” By default, a tool request
displays its arguments: choose **Allow once** or **Deny**. The demo advertises one short shell
tool through a thin adapter to the existing `buzz-dev-mcp`; approval and execution
stay in Buzz. Nothing is executed by recognizing speech alone.

The **Tool approval** menu defaults to **Ask each time**. Choose **Automatically
approve** to approve every subsequent shell call, including commands that modify
files. You can switch back while connected; any approval dialog already open
still needs a decision. The choice stays in this page across reconnects and resets
to **Ask each time** on reload. Automatically approved calls are noted in the
transcript.

By default tools run in an empty temporary directory. Add `--cwd /path/to/project`
to choose their working directory. The launcher uses neutral instructions and
disables local hints/skills; it does not load a personal persona. Each connection
gets a new agent session. A disconnect does not replay previous tool actions.

## Voice, thinking, and memory

Add a non-silent WAV up to 20 seconds to replace the bundled voice:

```sh
node examples/realtime-audio/launch.mjs \
  --native ../llama-frankie/build-frankie/bin/llama-frankie-realtime \
  --model ../models/frankie.gguf \
  --voice /path/to/my-voice.wav
```

Both mouths accept `--voice` directly when the package includes its reference
encoder. Qwen3-TTS computes a speaker embedding; Breeze encodes and transcribes
the reference once at startup. Rebuilding a complete v3 GGUF for each voice is
unnecessary. Breeze also retains bounded speech context between chunks within
one reply for more consistent prosody; this is enabled in the native runtime.

For optional Qwen3-TTS ICL conditioning, `--voice-text-file` and `--voice-codes`
must be supplied together with the matching WAV. Codec IDs must already exist;
the native runtime does not generate these optional Qwen ICL codes. Do not pass
Qwen codec IDs to Breeze. See the native guide for mouth-specific options.

Choose **Off**, **Minimal**, **Low**, **Medium**, **High**, **Very high**, or
**Maximum** thinking before connecting. Off is the default. Buzz forwards the
setting and requires the provider to acknowledge it. Reasoning is not spoken;
higher levels can substantially delay the first answer. End and reconnect to
change the level.

The launcher defaults to `--context 131072 --cache-type q4_0 --threads 4`.
`--context`, `--cache-type q4_0|q8_0|f16`, and `--threads` are configurable. File
size is not peak GPU memory: KV/recurrent state and compute buffers also count.
CPU, Metal, and CUDA use the same runtime options. Measure the complete voice
pipeline on the intended device; allocating 128K context alone does not establish
memory capacity or throughput under that workload.

## Operation and troubleshooting

- Buzz allows two minutes of generated speech per live reply, or a smaller
  provider limit. At the cap, it stops that reply and keeps the call open. Each
  subsequent reply receives a fresh budget.

- Only one browser connection can use this demo at a time. Close an existing
  conversation before opening another. The page requires a fresh connection
  after transport errors; tool effects are never automatically replayed.
- Use `--port` and `--native-port` if 18796 or 18793 is already occupied.
- The browser adapter and native server bind to loopback. Keep the printed URL
  private: its random fragment authorizes that local demo. Provider credentials
  stay server-side. There are no external page assets or browser storage writes.
- Temporary logs are printed at startup and retained for troubleshooting.
  The adapter does not record conversations or audio to disk. Normal tool commands
  may read or write files when approved. Do not add runtime logs or models to Git.
- With Bluetooth headphones, selecting their microphone can switch the device
  into lower-quality call audio. Try speakers or a separate microphone if playback
  sounds uniformly muffled. Check Chrome's microphone selection.
- Brief capture stalls are buffered for up to five seconds. Longer stalls close
  the session visibly instead of silently dropping accepted samples. Slow inference
  can still underrun playback; reduce thinking or use a faster backend.
- The launcher expires after 24 hours (`--hours`, up to 168). Each conversation
  also has a one-hour limit. Use `--help` for binary and port overrides.

For a separately managed native server, `server.mjs` can run directly. Configure
`BUZZ_AGENT_PROVIDER=openai`, `OPENAI_COMPAT_API=realtime`,
`OPENAI_COMPAT_MODEL=frankie`, `OPENAI_COMPAT_BASE_URL=ws://127.0.0.1:18793/v1/realtime`,
and `OPENAI_COMPAT_API_KEY` matching the native `FRANKIE_REALTIME_TOKEN`. Set
`BUZZ_AGENT_BIN`, `BUZZ_MCP_BIN` to this directory's executable `mcp-shell.mjs`,
`FRANKIE_DEV_MCP_BIN` to `buzz-dev-mcp`, and `BUZZ_AGENT_NO_HINTS=1`. Run
`node /path/to/server.mjs` from the desired tool working directory. The launcher
is the recommended path because it supplies neutral configuration and owns cleanup.

## Tests

```sh
node --test examples/realtime-audio/*.test.mjs
cargo test -p buzz-agent
```

The Node tests exercise the production capture queue, AudioWorklet, loopback
authentication, permission forwarding, disconnect cleanup, and launcher rejection
paths. Rust tests exercise the actual ACP subprocess and controlled WebSocket/MCP
peers, including stale playback, cancellation, thinking acknowledgment, output
limits, and reconnect isolation. Native model inference is a separate end-to-end
check requiring the GGUF and the fork binary; unit fixtures are not inference
benchmarks. The example tests also run in `just ci` and GitHub CI.

With the repository’s Playwright development dependencies and Chromium installed,
run `node examples/realtime-audio/client.browser.mjs` to check the browser approval
menu against a controlled ACP subprocess. This covers manual and automatic
approval, switching modes, keyboard controls, reload defaults, and narrow screens.
