# DOKI Checkpoint

## Scope
Checkpoint for audio visualizer work in `scope-studio`.

## Date
2026-06-20

## What’s in this checkpoint
- Added explicit audio theme system with runtime `--theme` support.
- Added decoupled theme/background controls:
  - `t` cycles theme (`original`, `classic`, `neon`, `ocean`, `mono`)
  - `b` cycles background presets
  - `terminal` preset uses terminal background via `Color::Reset` and is the default for `original`.
- Added runtime visual-style switching (`v`):
  - `wave` (legacy line chart)
  - `sonar` (vectorscope-style radial motion)
  - `kale` (radial kaleidoscope vectors)
- Rebuilt `sonar` on correct parametric math with terminal cell aspect correction:
  - `TERMINAL_CELL_ASPECT = 0.5` compensates for 2:1 character cell shape
  - `x(θ) = cx + R·cos(θ)`, `y(θ) = cy + R·sin(θ)·aspect` — standard parametric circle
  - `R_max = min(cx, cy/aspect)·0.90` prevents bleed on either axis
  - Three clean concentric circles per band (no audio modulation yet — baseline geometry only)
- Refined `sonar`/`kale` to render waveform samples directly in circular/radial trajectories.
- Render path now passes explicit background and visual style to the renderer.
- Kept analyzer behavior in-band: 3-band (`bass`, `mid`, `treble`) reconstruction + peak/rms/clip metrics.
- Removed fixed x-axis footer labels (`0`, `1/2`, `end`).
- Kept network analyzer integration untouched and preserved theme/background value behavior.

## Validation
- `cargo fmt`
- `cargo check --bin audio-scope --features audio`
- `cargo check`
- `cargo run --bin audio-scope --features audio -- --list-devices --theme original`

## Files touched in this checkpoint
- `README.md`
- `src/bin/audio.rs`
- `src/lib.rs`
- `src/render.rs`
