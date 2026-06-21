#[cfg(feature = "audio")]
use std::fs::File;
#[cfg(feature = "audio")]
use std::io::{self, Read};
#[cfg(feature = "audio")]
use std::os::fd::AsRawFd;
#[cfg(feature = "audio")]
use std::process::{Child, Command, Stdio};
#[cfg(feature = "audio")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "audio")]
use std::thread::{self, JoinHandle};

#[cfg(feature = "audio")]
use crate::{DataSource, Matrix};
#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[cfg(feature = "audio")]
pub enum AudioSource {
    System(SystemCapture),
    Safe(SafeCapture),
}

#[cfg(feature = "audio")]
pub struct SystemCapture {
    samples: Arc<Mutex<Vec<f64>>>,
    _child: Child,
    _reader: Option<JoinHandle<()>>,
    pub history: usize,
}

#[cfg(feature = "audio")]
pub struct SafeCapture {
    samples: Arc<Mutex<Vec<f64>>>,
    _stream: cpal::Stream,
    pub history: usize,
}

#[cfg(feature = "audio")]
impl AudioSource {
    pub fn new(
        device: String,
        sample_rate: u32,
        channels: usize,
        history: usize,
        monitor_preferred: bool,
    ) -> anyhow::Result<Self> {
        if monitor_preferred && Self::supports_system_audio_capture() {
            Ok(Self::System(SystemCapture::new(
                device,
                sample_rate,
                channels,
                history,
            )?))
        } else if monitor_preferred && !Self::supports_system_audio_capture() {
            Ok(Self::Safe(SafeCapture::new(
                device,
                sample_rate,
                channels,
                history,
            )?))
        } else {
            Ok(Self::Safe(SafeCapture::new(
                device,
                sample_rate,
                channels,
                history,
            )?))
        }
    }

    pub fn list_input_devices(system_audio: bool) -> anyhow::Result<()> {
        if system_audio && Self::supports_system_audio_capture() {
            println!("System audio capture (PipeWire capture sink):");
            println!("  pw-cat --record --format f32 --channels 2 --rate 48000 -P stream.capture.sink=true -");
            println!();
        }

        if system_audio && !Self::supports_system_audio_capture() {
            println!("System audio capture is not supported on this platform.");
            println!("This build will use safe input devices by default.");
            println!();
        }

        if Self::supports_system_audio_capture() {
            println!("Safe input devices (use --safe for mic/input capture):");
        } else {
            println!("Safe input devices (default capture mode on this platform):");
        }
        let host = cpal::default_host();
        for (name, default_tag, auto_tag) in list_safe_input_devices(&host)? {
            println!("  {default_tag}{name}{auto_tag}");
        }
        Ok(())
    }

    pub fn supports_system_audio_capture() -> bool {
        cfg!(target_os = "linux")
    }
}

#[cfg(feature = "audio")]
impl SystemCapture {
    pub fn new(
        device: String,
        sample_rate: u32,
        channels: usize,
        history: usize,
    ) -> anyhow::Result<Self> {
        let channels = channels.clamp(1, 2);
        let samples = Arc::new(Mutex::new(Vec::new()));
        let reader_state = Arc::clone(&samples);
        let history_cap = history;
        let monitor = device.trim();
        let mut cmd = Command::new("pw-cat");
        cmd.arg("--record")
            .arg("--format")
            .arg("f32")
            .arg("--channels")
            .arg(channels.to_string())
            .arg("--rate")
            .arg(sample_rate.max(8_000).to_string())
            .arg("-P")
            .arg("stream.capture.sink=true");

        if !monitor.is_empty() && !monitor.eq_ignore_ascii_case("auto") {
            cmd.arg("--target").arg(monitor);
        }

        cmd.arg("-").stdout(Stdio::piped()).stderr(Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|err| anyhow::anyhow!("failed to spawn pw-cat capture backend: {err}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("pw-cat stdout pipe not available"))?;

        let reader =
            thread::spawn(move || read_pwcat_stdout(stdout, channels, reader_state, history_cap));

        Ok(Self {
            samples,
            _child: child,
            _reader: Some(reader),
            history,
        })
    }
}

#[cfg(feature = "audio")]
impl Drop for SystemCapture {
    fn drop(&mut self) {
        let _ = self._child.kill();
        let _ = self._child.wait();
        if let Some(handle) = self._reader.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(feature = "audio")]
impl SafeCapture {
    pub fn new(
        device: String,
        _sample_rate: u32,
        _channels: usize,
        history: usize,
    ) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = Self::resolve_device(&host, device.as_str())?;
        let cfg = with_suppressed_backend_noise(|| device.default_input_config())
            .map_err(|err| anyhow::anyhow!("failed to inspect default input config: {err}"))?;
        let stream_config = cfg.config();
        let channels = stream_config.channels as usize;
        let device_label = with_suppressed_backend_noise(|| device.name().ok())
            .unwrap_or_else(|| "<unknown>".to_string());
        let stream_err = |err| {
            anyhow::anyhow!(
                "failed to open input stream on '{device_label}' with {} ch @ {} Hz: {err}",
                stream_config.channels,
                stream_config.sample_rate.0
            )
        };

        let shared = Arc::new(Mutex::new(Vec::new()));
        let callback_state = Arc::clone(&shared);

        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| sample_cb_f32(data, channels, &callback_state, history),
                    |err| eprintln!("audio stream error: {err}"),
                    None,
                )
                .map_err(|err| stream_err(err))?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| sample_cb_i16(data, channels, &callback_state, history),
                    |err| eprintln!("audio stream error: {err}"),
                    None,
                )
                .map_err(|err| stream_err(err))?,
            cpal::SampleFormat::I32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i32], _| sample_cb_i32(data, channels, &callback_state, history),
                    |err| eprintln!("audio stream error: {err}"),
                    None,
                )
                .map_err(|err| stream_err(err))?,
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| sample_cb_u16(data, channels, &callback_state, history),
                    |err| eprintln!("audio stream error: {err}"),
                    None,
                )
                .map_err(|err| stream_err(err))?,
            _ => return Err(anyhow::anyhow!("unsupported sample format")),
        };

        stream
            .play()
            .map_err(|err| anyhow::anyhow!("failed to start input stream: {err}"))?;

        Ok(Self {
            samples: shared,
            _stream: stream,
            history,
        })
    }

    fn resolve_device(host: &cpal::Host, requested: &str) -> anyhow::Result<cpal::Device> {
        let mut candidates = with_suppressed_backend_noise(|| collect_input_devices(host))?;
        if candidates.is_empty() {
            return Err(anyhow::anyhow!("no input device found"));
        }

        let requested = requested.trim();
        if !requested.is_empty() && !requested.eq_ignore_ascii_case("auto") {
            if let Some((idx, _)) = candidates
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name.eq_ignore_ascii_case(requested))
            {
                return Ok(candidates.swap_remove(idx).1);
            }

            return Err(anyhow::anyhow!(
                "requested audio device '{requested}' not found; use --list-devices to inspect names"
            ));
        }

        let default_name = default_input_name(host);

        if let Some(default_name) = default_name.as_deref() {
            if let Some((idx, _)) = candidates.iter().enumerate().find(|(_, (name, _))| {
                name == &default_name && !is_likely_system_output_capture(name)
            }) {
                return Ok(candidates.swap_remove(idx).1);
            }
        }

        if let Some((idx, _)) = candidates
            .iter()
            .enumerate()
            .find(|(_, (name, _))| !is_likely_system_output_capture(name))
        {
            return Ok(candidates.swap_remove(idx).1);
        }

        Err(anyhow::anyhow!(
            "only monitor-like input sources were found; rerun with system audio capture instead"
        ))
    }
}

#[cfg(feature = "audio")]
impl DataSource for AudioSource {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        match self {
            AudioSource::System(src) => src.next_frame(),
            AudioSource::Safe(src) => src.next_frame(),
        }
    }
}

#[cfg(feature = "audio")]
fn list_safe_input_devices(
    host: &cpal::Host,
) -> anyhow::Result<Vec<(String, &'static str, String)>> {
    let input_devices = with_suppressed_backend_noise(|| host.input_devices())
        .map_err(|err| anyhow::anyhow!("failed to enumerate input devices: {err}"))?;

    let default_name = default_input_name(host);
    let devices = input_devices
        .filter_map(|device| device.name().ok().map(|name| (name, device)))
        .map(|(name, _device)| {
            let default_tag = if default_name.as_deref() == Some(name.as_str()) {
                "* "
            } else {
                "  "
            };
            let auto_tag = if is_likely_system_output_capture(&name) {
                " (monitor-like)"
            } else {
                ""
            };
            (name, default_tag, auto_tag.to_string())
        })
        .collect();

    Ok(devices)
}

#[cfg(feature = "audio")]
impl DataSource for SystemCapture {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        let state = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_empty() {
            return Ok(Some(vec![vec![0.0; 2]]));
        }

        let window = if state.len() > self.history {
            state[state.len() - self.history..].to_vec()
        } else {
            state.clone()
        };
        Ok(Some(vec![window]))
    }
}

#[cfg(feature = "audio")]
impl DataSource for SafeCapture {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        let state = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_empty() {
            return Ok(Some(vec![vec![0.0; 2]]));
        }

        let window = if state.len() > self.history {
            state[state.len() - self.history..].to_vec()
        } else {
            state.clone()
        };
        Ok(Some(vec![window]))
    }
}

#[cfg(feature = "audio")]
fn collect_input_devices(host: &cpal::Host) -> anyhow::Result<Vec<(String, cpal::Device)>> {
    let input_devices = host
        .input_devices()
        .map_err(|err| anyhow::anyhow!("failed to enumerate input devices: {err}"))?;

    let devices: Vec<(String, cpal::Device)> = input_devices
        .filter_map(|device| device.name().ok().map(|name| (name, device)))
        .collect();

    Ok(devices)
}

#[cfg(feature = "audio")]
fn default_input_name(host: &cpal::Host) -> Option<String> {
    with_suppressed_backend_noise(|| {
        host.default_input_device()
            .and_then(|device| device.name().ok())
    })
}

#[cfg(feature = "audio")]
fn is_likely_system_output_capture(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    [
        "monitor",
        ".monitor",
        "monitor of",
        "alsa_output",
        "sink",
        "loopback",
        "output",
        "speaker",
        "headphone",
    ]
    .iter()
    .any(|hint| normalized.contains(hint))
}

#[cfg(feature = "audio")]
fn with_suppressed_backend_noise<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(unix)]
    let _guard = StderrSilencer::new().ok();
    f()
}

#[cfg(unix)]
struct StderrSilencer {
    saved_fd: i32,
    _null_file: File,
}

#[cfg(unix)]
impl StderrSilencer {
    fn new() -> io::Result<Self> {
        const STDERR_FILENO: i32 = 2;

        let null_file = File::options().write(true).open("/dev/null")?;
        let saved_fd = unsafe { dup(STDERR_FILENO) };
        if saved_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        if unsafe { dup2(null_file.as_raw_fd(), STDERR_FILENO) } < 0 {
            let err = io::Error::last_os_error();
            unsafe {
                close(saved_fd);
            }
            return Err(err);
        }

        Ok(Self {
            saved_fd,
            _null_file: null_file,
        })
    }
}

#[cfg(unix)]
impl Drop for StderrSilencer {
    fn drop(&mut self) {
        const STDERR_FILENO: i32 = 2;
        unsafe {
            let _ = dup2(self.saved_fd, STDERR_FILENO);
            let _ = close(self.saved_fd);
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn dup(fd: i32) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn close(fd: i32) -> i32;
}

#[cfg(feature = "audio")]
fn read_pwcat_stdout(
    mut stdout: impl Read + Send + 'static,
    channels: usize,
    state: Arc<Mutex<Vec<f64>>>,
    history: usize,
) {
    let mut pending = Vec::<u8>::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        pending.extend_from_slice(&buf[..n]);
        let usable = pending.len() / 4 * 4;
        if usable == 0 {
            continue;
        }

        let mut frame = Vec::with_capacity(usable / 4);
        for chunk in pending[..usable].chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            frame.push(sample as f64);
        }
        pending.drain(..usable);
        push_averages(frame.into_iter(), channels, &state, history);
    }
}

#[cfg(feature = "audio")]
fn sample_cb_f32(data: &[f32], channels: usize, state: &Arc<Mutex<Vec<f64>>>, history: usize) {
    push_averages(data.iter().map(|s| *s as f64), channels, state, history);
}

#[cfg(feature = "audio")]
fn sample_cb_i16(data: &[i16], channels: usize, state: &Arc<Mutex<Vec<f64>>>, history: usize) {
    push_averages(
        data.iter().map(|s| *s as f64 / i16::MAX as f64),
        channels,
        state,
        history,
    );
}

#[cfg(feature = "audio")]
fn sample_cb_i32(data: &[i32], channels: usize, state: &Arc<Mutex<Vec<f64>>>, history: usize) {
    push_averages(
        data.iter().map(|s| *s as f64 / i32::MAX as f64),
        channels,
        state,
        history,
    );
}

#[cfg(feature = "audio")]
fn sample_cb_u16(data: &[u16], channels: usize, state: &Arc<Mutex<Vec<f64>>>, history: usize) {
    push_averages(
        data.iter()
            .map(|s| (*s as f64 / u16::MAX as f64) * 2.0 - 1.0),
        channels,
        state,
        history,
    );
}

#[cfg(feature = "audio")]
fn push_averages(
    samples: impl Iterator<Item = f64>,
    channels: usize,
    state: &Arc<Mutex<Vec<f64>>>,
    history: usize,
) {
    let mut frame = Vec::new();
    let mut acc = 0.0;
    let mut count = 0usize;

    for sample in samples {
        acc += sample;
        count += 1;
        if count == channels {
            frame.push((acc / channels as f64).clamp(-1.0, 1.0));
            acc = 0.0;
            count = 0;
        }
    }

    if frame.is_empty() {
        return;
    }

    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.extend(frame);
    if guard.len() > history {
        let excess = guard.len() - history;
        guard.drain(0..excess);
    }
}
