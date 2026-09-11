# scope-studio

A two-crate workspace with two separate apps:

- `crates/net-scope` — live WLAN RSSI + RX/TX throughput visualization from `/proc/net/*`
- `crates/audio-scope` — live waveform visualization from system audio or an input device
- `crates/scope-core` — shared terminal/runtime primitives (event loop + renderer)

## Build

```bash
cd /home/hinata/hermes/gitrepos/rust/scope-studio
cargo check
```

`cargo check` builds all workspace members (`scope-core`, `audio-scope`, `net-scope`) by default.

## Build + run audio app

```bash
cargo check -p audio-scope
cargo run -p audio-scope
```

`audio-scope` enables audio dependencies via its default `audio` feature, so this compiles out-of-the-box from this workspace.

```bash
# run with args
cargo run -p audio-scope -- --sample-rate 44100 --channels 1 --history 256

# device list
cargo run -p audio-scope -- --list-devices

# explicit non-monitor capture (opt-out)
cargo run -p audio-scope -- --safe

# explicit monitor capture
cargo run -p audio-scope -- --device auto

# explicit device selection
cargo run -p audio-scope -- --device "Built-in Audio Analog Stereo"
```

## Run net app

```bash
cargo run -p net-scope

cargo run -p net-scope -- --interface wlan0 --width 240 --sleep-ms 250 --history 120
```

## Controls

Shared runtime controls:

- `q` or `Ctrl+c` exit
- `space` pause
- `h` hide/show UI chrome
- `r` reset gain/zoom to the default view
- `p` cycle view presets
- `↑/↓` change vertical gain
- `←/→` change zoom / time span

Audio-only controls (`audio-scope`):

- `w` cycle wave count: `3`, `2`, `1`
- `t` cycle theme
- `b` cycle background color: `terminal`, `black`, `classic`, `neon`, `ocean`, `mono`, `indigo` (default: `terminal`)
- `wave` is the default and only audio visual style currently.

## Auto source behavior

`audio-scope --device auto` prefers system audio / monitor sources by default.

- If a monitor/sink source exists, it will use that.
- If no monitor-like source exists, it falls back to host default input and then first available device.
- `--safe` flips the selection to non-monitor inputs.
