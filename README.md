# doki

`doki` is the primary terminal audio visualizer binary in this repo.
`audio-scope` is kept as a compatibility alias so existing workflows keep working.

Both binaries share the same Rust runtime; `doki` is the preferred name going forward.

## Included binaries

- `doki` (`audio-scope`) — live waveform visualization from system audio or an input device

## Install

```bash
cargo install --git https://github.com/highdeftant/doki.git \
  --branch main \
  --features audio \
  --locked \
  --bin doki \
  --force
```

### Local install (recommended for contributors)

```bash
cd /home/hinata/hermes/gitrepos/rust/scope-studio-audio
./scripts/install.sh
```

## Run

```bash
doki

# source runs
cargo run --features audio --bin doki -- --help
cargo run --features audio --bin audio-scope -- --help
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

## Audio usage examples

```bash
cargo run --bin audio-scope --features audio -- --sample-rate 44100 --channels 1 --history 256
cargo run --bin audio-scope --features audio -- --list-devices
cargo run --bin audio-scope --features audio -- --safe
cargo run --bin audio-scope --features audio -- --device auto
cargo run --bin audio-scope --features audio -- --device "Built-in Audio Analog Stereo"
```

## Controls

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

## Auto-capture behavior

`--device auto` prefers system/monitor audio sources by default on Linux. If no monitor source is found, it falls back to input sources.
Use `--safe` to force non-monitor capture.

On macOS, system playback capture generally still requires a virtual loopback device (such as BlackHole or Soundflower).