# doki

`doki` is the primary terminal audio visualizer binary in this repo.
`audio-scope` is a compatibility alias that keeps existing workflows working.

Both apps share the same Rust runtime; only the binary name changed to make the project naming clear.

## Included binaries

- `doki` (`audio-scope`) — waveform TUI from system audio or an input device (requires `audio` feature)
- `net-scope` — WLAN RSSI + RX/TX throughput visualizer from `/proc/net/*`

## Install

### Install `doki`

```bash
cargo install --git https://github.com/highdeftant/doki.git \
  --branch main \
  --features audio \
  --locked \
  --bin doki \
  --force
```

### Local install (recommended for dev)

```bash
cd /home/hinata/hermes/gitrepos/rust/scope-studio-audio
./scripts/install.sh
```

This installs `doki` by default and keeps `audio-scope` available for backward compatibility.

## Quick run

```bash
# Installed binary
doki

# Direct source run
cargo run --features audio --bin doki -- --help

# Network app
cargo run --bin net-scope
```

## System dependencies

### Linux

```bash
sudo apt-get update
sudo apt-get install -y libasound2-dev pkg-config
```

### macOS

```bash
brew install pkg-config
```

## Audio run options

```bash
cargo run --bin audio-scope --features audio -- --sample-rate 44100 --channels 1 --history 256
cargo run --bin audio-scope --features audio -- --list-devices
cargo run --bin audio-scope --features audio -- --safe           # prefer non-monitor inputs
cargo run --bin audio-scope --features audio -- --device auto     # explicit auto-selection
cargo run --bin audio-scope --features audio -- --device "Built-in Audio Analog Stereo"
```

## Runtime controls

- `q` or `Ctrl+c` exit
- `space` pause
- `h` hide/show UI chrome
- `r` reset gain/zoom
- `p` cycle presets
- `w` cycle wave count: `3`, `2`, `1`
- `t` cycle theme
- `b` cycle background color
- `↑/↓` vertical gain
- `←/→` zoom / time span

### Audio CLI flags

```bash
-r, --sample-rate  sample rate [default: 44100]
-c, --channels     input channels (1-2) [default: 1]
-d, --device       input device name (default: auto)
--safe             prefer non-monitor inputs
-s, --sleep-ms     render interval in ms [default: 16]
-l, --list-devices print available input devices
-n, --history      sample history depth [default: 256]
-w, --width        points in x-axis [default: 512]
-t, --theme        original | classic | neon | ocean | mono | doki [default: original]
```

### net-scope flags

```bash
-i, --interface    wireless interface (auto-detect)
-w, --width        points in x-axis [default: 240]
-s, --sleep-ms     poll interval in ms [default: 250]
-n, --history      sample history depth [default: 120]
```

## Auto-capture behavior

`--device auto` prefers system/monitor audio sources by default on Linux. If no monitor is found, it falls back to input sources. Use `--safe` to force non-monitor capture.

On macOS, system playback capture usually still requires a virtual loopback input (such as BlackHole/Soundflower).
