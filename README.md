# scope-studio

A three-crate workspace with two separate apps:

- `crates/net-scope` — live WLAN RSSI + RX/TX throughput visualization from `/proc/net/*`
- `crates/audio-scope` — live waveform, fading Persistence, dotted Ocean Mist, stereo Lissajous, spectrum bars, scrolling spectrogram, and read-only MPRIS media metadata
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
cargo run -p audio-scope -- --sample-rate 44100 --channels 2 --history 4096

# device list
cargo run -p audio-scope -- --list-devices

# CPAL input capture (explicit opt-out from Linux system audio)
cargo run -p audio-scope -- --safe

# default capture: Linux playback, macOS/other platforms' default input
cargo run -p audio-scope -- --device auto

# explicit device selection
cargo run -p audio-scope -- --device "Built-in Audio Analog Stereo"
```

### Audio views and media

Waveform remains the default (`--view wave`). Its time window follows the combined energy
centroid, centering localized activity horizontally while keeping all bands time-aligned.
At history edges it clamps to captured samples instead of inserting silent padding; in
silence it shows the latest window. Press `v` to cycle **wave → ocean-mist → spectrum →
spectrogram → lissajous → persistence**;
`w` selects three filtered bands, two bands, or the raw waveform in Wave/Ocean Mist/Persistence.

```bash
cargo run -p audio-scope -- --view ocean-mist
cargo run -p audio-scope -- --view spectrum --history 4096
cargo run -p audio-scope -- --view spectrogram --history 4096
cargo run -p audio-scope -- --view lissajous --channels 2
cargo run -p audio-scope -- --view persistence --persistence-window 12

# Synthetic audio, actual Ratatui rendering, and live media polling; no audio capture or raw terminal mode
cargo run -p audio-scope -- --smoke

# Same non-capture smoke, with RGB wave/Persistence counts 1/2/3 and all six views
cargo run -p audio-scope -- --theme rgb --smoke
```

`ocean-mist` renders one to three filtered wave layers as spaced dot ribbons with a faint previous-frame dot trail; RGB keeps one global hue
and varies only layer brightness.

`persistence` overlays recent **time-domain** waveform snapshots, oldest first and newest last;
it is separate from the stereo X–Y view and scrolling FFT spectrogram. `--persistence-window`
sets the retained frame count from **1 to 120**, default **12**; `{` / `}` decrease/increase it
within those bounds. This is a frame window, not a fixed duration: its time span follows
the configured frame cadence. One retained snapshot is full brightness; multiple snapshots
fade linearly from 20% for the oldest to 100% for the newest. Only current bands appear in the legend.

Persistence recycles waveform buffers after the window fills and streams interpolation into
one Braille Canvas instead of cloning/resampling the entire history into temporary vectors.
Shrinking the window keeps the newest fitting snapshots. Changing views or wave count clears
incompatible history; empty/malformed input clears stale trails. Valid silence appends zero
waveforms, aging prior signals out until only the centered baseline remains. Resize, zoom,
and gain reproject retained raw samples without clearing them. Theme changes recolor retained
snapshots; RGB uses the same current 45-second hue across every age, differing only in brightness.
Original/Mono use fixed standard ANSI RGB equivalents for fading, so terminal-custom named
colors cannot affect Persistence's intensity ramp.

`lissajous` is a true stereo X–Y plot: each dot is `(left[n], right[n])` from
the same recent `--width`-frame window. It does not use time as an axis, FFT bands,
delayed mono, or a synthesized second channel. Equal channels form a positive
diagonal; opposite-polarity channels form a negative diagonal even when their mono
sum cancels. Quadrature tones form a circle or amplitude-dependent ellipse.
Sensitivity applies equally to both axes around a fixed zero center; there is no
per-channel normalization or connecting-line interpolation. Out-of-view points are
omitted, not clamped into false edge patterns. Actual silence is one centered dot.
Missing stereo, mismatched channel windows, and empty input show an explanatory
message instead of a substitute waveform.

X–Y uses equal physical axis scaling: terminal pixel/cell dimensions are sampled
at startup and resize, with a 1:2 cell width:height fallback when pixel metrics are
unavailable. The longer viewport dimension expands its range rather than stretching
the curve. Wave also uses a shared full-plot Braille Canvas with symmetric amplitude
bounds across all palettes; headers and legend overlays do not shift its zero center.

Spectrum and spectrogram use 64 logarithmic frequency bins from 20 Hz to the capture sample rate's
Nyquist frequency, with intensity mapped from −80 to 0 dBFS. The capture ring defaults to 4096
samples; spectral views analyze up to the latest 2048 independently of waveform `--width`. Bands
narrower than one FFT bin use the nearest positive-frequency bin, preventing empty display gaps
without inventing signal in silent ranges. FFT plans,
Hann windows, frequency ranges, scratch space, and waveform outputs are cached/reused.
Spectrum uses eighth-cell vertical blocks for fractional level detail, with a dim
full-width zero baseline and bin-start guides so silent frequency regions still show
the plot's extent. At default sensitivity, frame-local display normalization maps
the strongest live level at/above −40 dBFS to 65% of plot height:
`display_multiplier = 0.65 / max(live_peak, 0.5)` on the existing −80..0 dBFS levels.
The multiplier is capped at 1.3×, so weak noise is not expanded to full height.
Neighboring levels retain their ratios rather than all clipping at the top; gain
still applies after normalization. This changes display only, not DSP/audio levels.
Held peaks use the same multiplier but do not set the reference. Their subdued marks
remain above falling bars, holding for 600 ms before decaying at 28 dB/s.
Both spectral views show sparse logarithmic frequency ticks (20, 100, 1k, 10k Hz
where space and the actual Nyquist limit permit).

The spectrogram retains at most 120 rows, recycles row buffers, and clears history
when changing views. The newest fitting rows are displayed: oldest at top (`old`),
newest at bottom (`now`), advancing upward as frames arrive. Labels describe row
order, not a fixed seconds-per-row rate. Its fixed contrast mapping suppresses
levels at/below −65 dBFS, maps −65..0 dBFS into brightness, and lifts mid-level
detail with a 0.7 power curve. Gain is applied before contrast mapping.
Original uses a dark-blue-to-white heat palette; Mono uses grayscale.
Gain applies to all views; zoom/time-span controls apply to Wave, Ocean Mist, and Persistence.

Themes (`--theme`, or cycle with `t`) are **original → rgb → mono**:

- **Original** (default): the existing cyan/green/blue waveform palette and cyan spectrum bars.
- **RGB**: one saturated global hue drifts smoothly through red, green, and blue,
  completing a cycle every 45 seconds of monotonic application time. It is uniform
  across each frame and shared by all views; there are no position/frequency hue
  gradients or trace phase offsets. Waveform traces differ only in brightness
  (100%, 75%, 50%). Switching themes/views does not reset the hue clock; retained
  spectrogram and Persistence history is recolored with the current frame hue. Waveforms use one
  shared Braille Canvas with bounded legend overlays. Spectrum foregrounds
  and spectrogram backgrounds encode level by brightness of that same hue.
  Silence retains the selected background.
- **Mono**: grayscale waveform traces, spectrum bars, and spectrogram intensity.

Backgrounds (`b`) are **terminal → black → midnight**; midnight is RGB `(0, 8, 24)`.
Original and RGB initially use the terminal background. Mono initially uses black,
replacing its removed dark-gray preset. Theme cycling does not reset the background.
Other display defaults remain unchanged: Original theme, terminal background, waveform view,
and three traces. Capture now prefers two real channels. True-color terminal support is needed for the intended RGB appearance;
limited-color terminals may quantize the slowly changing hue and intensity shades.

`--smoke` honors `--theme` and `--persistence-window`, renders Wave and Persistence counts
1/2/3, Ocean Mist, spectrum, spectrogram, and an explicitly synthetic stereo X–Y fixture
through Ratatui's TestBackend. It checks Persistence's bounded chronological trail,
fading, and current-only legend, emits true-color foreground/background output, and reports
distinct RGB colors (intensity shades, not spatial hue gradients). RGB smoke fails
if any Wave/Persistence count or stereo X–Y fixture lacks RGB stroke foregrounds, spectrum lacks
colored bar foregrounds, or spectrogram lacks colored signal backgrounds. Synthetic frames use
an injected clock for repeatable color snapshots. Spectrogram RGB foreground counts
may remain zero: its color is encoded in backgrounds.
It still polls read-only MPRIS without opening audio capture.

The first header line shows player, playback status, optional artist, and title. A background worker
rediscovers MPRIS players on the session D-Bus after each one-second polling delay and prefers
Playing over Paused over Stopped (bus-name order breaks ties). Rendering reads a nonblocking
cache. This uses read-only `busctl` queries with 250 ms per-request timeouts and disables service
activation and interactive authorization; it never sends playback controls and does not use
`playerctl`. Missing `busctl`, an unavailable session bus, vanished players, or invalid replies
fall back to `No MPRIS player`; empty artists do not hide titles. Poll duration scales with player
count, and updates become visible when the next unpaused frame renders.

A running binary is not hot-reloaded by a build. Keep an existing capture running until you choose
to replace it; use `--smoke` to inspect the new views without opening another capture.

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
- `↑/↓` change vertical scale; `↑` reduces spectrum-bar height/sensitivity
- `←/→` change zoom / time span

Rendering, source snapshots, and DSP share a fixed frame deadline independent of
mouse/key event volume. The period is `2 * --sleep-ms`: the default 16 means about
32 ms per frame (31 FPS), preserving the previous idle cadence. Input is handled
between frames without restarting the deadline; missed frames do not trigger a
catch-up burst. `--sleep-ms 0` uses a 1 ms minimum period.

Audio-only controls (`audio-scope`):

- `w` cycle wave count: `3`, `2`, `1`
- `v` cycle view: `wave`, `ocean-mist`, `spectrum`, `spectrogram`, `lissajous`, `persistence`
- `t` cycle theme: `original`, `rgb`, `mono` (`--theme` sets the initial theme)
- `b` cycle background: `terminal`, `black`, `midnight` (default: `terminal`)
- `{` / `}` decrease/increase Persistence's retained frame window (1–120; `--persistence-window`, default 12)
- `wave` is the default view; `--view` selects the initial view.

## Auto source behavior

- **Linux, without `--safe`:** capture playback using PipeWire's `pw-cat` capture-sink
  backend. Stereo capture requires `pw-cat`, read-only `pw-dump` layout discovery,
  and a working PipeWire server. `--device auto` selects and pins the inspected
  default playback sink at startup; an explicit device is passed as `pw-cat --target`.
  Stereo uses raw F32 with explicit FL/FR mapping and remix/upmix disabled. Native
  mono remains mono; unidentifiable layouts fail explicitly rather than inventing
  stereo. `--channels 1` retains scalar auto/target capture without layout discovery.
  Backend failure does not silently fall back to microphone capture.
- **macOS and other platforms:** use CPAL's default input device, not system playback.
  `--device` selects an exact CPAL input name from `--list-devices`.
- **`--safe`:** always use CPAL. On Linux, automatic selection avoids monitor-like
  inputs; explicitly named inputs are still allowed.

On macOS, playback capture requires a virtual loopback input such as
[BlackHole](https://github.com/ExistentialAudio/BlackHole) and routing playback into it.
Select that input explicitly with `--device "BlackHole 2ch"` (use its actual listed name),
or make it the default input. A microphone/default input is not system-output capture.

`--channels` defaults to 2. CPAL prefers a supported configuration with that channel
count, keeping the default sample rate where supported, otherwise using a supported
rate or genuine mono fallback. Arbitrary multichannel prefixes are not assumed to be
L/R. `--sample-rate` configures PipeWire; DSP receives the actual capture rate.
All ten scalar formats exposed by CPAL 0.15 are handled: F32/F64, I8/I16/I32/I64,
and U8/U16/U32/U64. Additional formats use CPAL's normalized conversion; existing
F32/I16/I32/U16 scaling is retained.

Both backends retain channel-separated samples in preallocated circular buffers.
`--history` counts synchronized audio frames per channel, not interleaved scalars.
One mutex publishes batches and snapshots across all channels atomically; partial
PipeWire float/frame bytes are retained until complete. Conversion uses bounded
reusable scratch outside the lock and skips frames that cannot survive the batch.
Snapshots allocate before locking and copy chronological ring slices; callbacks
still use a mutex, not a lock-free/hard-realtime guarantee. Zero history discards
incoming samples and returns empty channel windows, without synthetic startup data.

Wave, Ocean Mist, spectrum, and spectrogram intentionally downmix complete channel
frames as `clamp(sum(channels) / channel_count, -1, 1)` before their existing DSP,
using reusable scratch. Lissajous bypasses that destructive mix and applies a display-only
rotation of one turn per 60 seconds; it still plots only measured left/right pairs. A mono
source cannot provide stereo X–Y; use a real stereo playback monitor or two-channel input.
