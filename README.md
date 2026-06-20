# scope-studio

Two small TUI apps built on a shared runtime pattern:

- `net-scope` — live WLAN RSSI + RX/TX throughput visualization from `/proc/net/*`
- `audio-scope` — live waveform visualization from system audio or an input device (requires `audio` feature)

## Current status

- `net-scope` builds and runs from default feature set.
- `audio-scope` is implemented behind `--features audio`.
- Audio capture defaults to PipeWire capture-sink via `pw-cat` on Linux; `--safe` keeps a CPAL input-device fallback.

## Build

```bash
cd /home/hinata/hermes/gitrepos/rust/scope-studio
cargo check
```

## Install runtime deps (audio)

```bash
sudo apt-get update
sudo apt-get install -y libasound2-dev pkg-config
```

## Run

```bash
# Network analyzer
cargo run --bin net-scope

# Audio visualizer (system audio by default)
cargo run --bin audio-scope --features audio
```

You can pass args:

```bash
# network
cargo run --bin net-scope -- --interface wlan0 --width 240 --sleep-ms 250 --history 120

# audio
cargo run --bin audio-scope --features audio -- --sample-rate 44100 --channels 1 --history 256

# quick device discovery
cargo run --bin audio-scope --features audio -- --list-devices

# explicit non-monitor capture (opt-out)
cargo run --bin audio-scope --features audio -- --safe

# explicit monitor capture
cargo run --bin audio-scope --features audio -- --device auto

# explicit device selection
cargo run --bin audio-scope --features audio -- --device "Built-in Audio Analog Stereo"
```

Controls:

- `q` or `Ctrl+c` exit
- `space` pause
- `h` hide/show UI chrome
- `↑/↓` change vertical gain
- `←/→` change zoom / time span
- `t` cycle theme
- `b` cycle background color: `terminal`, `black`, `classic`, `neon`, `ocean`, `mono`, `indigo` (default: `terminal`)
- `v` cycle visual style: `wave`, `sonar`, `kale`

## CLI flags

### net-scope

```bash
-i, --interface    wireless interface (auto-detect)
-w, --width        points in x-axis [default: 240]
-s, --sleep-ms     poll interval in ms [default: 250]
-n, --history      sample history depth [default: 120]
```

### audio-scope

```bash
-r, --sample-rate  sample rate [default: 44100]
-c, --channels     input channels (1-2) [default: 1]
-d, --device       input device name from system (default: auto)
--safe            prefer safe non-monitor inputs (opt-out)
-s, --sleep-ms     render refresh interval in ms [default: 16]
-l, --list-devices print available input devices and capture hints
-n, --history      sample history depth [default: 256]
-w, --width        points in x-axis [default: 512]
-t, --theme        visualization theme: original | classic | neon | ocean | mono [default: original]
```

## Auto source behavior
`audio-scope --device auto` now prefers system audio / monitor sources by default.

- If a monitor/sink source exists, it will use that.
- If no monitor-like source exists, it falls back to the host default input and then the first available device.
- `--safe` flips the selection to non-monitor inputs.

## Notes

- The scope uses a minimal shared architecture:
  - `src/lib.rs` handles event loop + terminal lifecycle
  - `src/data/*` contains data source modules
  - `src/bin/*` contains app-specific renderer + CLI
  - `src/render.rs` shared chart rendering
- Linux auto-detect for audio is heuristic; it is intentionally conservative unless `--monitor` is passed.
