#[cfg(all(feature = "audio", unix))]
use std::fs::File;
#[cfg(feature = "audio")]
use std::io;
#[cfg(all(feature = "audio", target_os = "linux"))]
use std::io::Read;
#[cfg(all(feature = "audio", unix))]
use std::os::fd::AsRawFd;
#[cfg(all(feature = "audio", target_os = "linux"))]
use std::process::{Child, Command, Stdio};
#[cfg(feature = "audio")]
use std::sync::{Arc, Mutex};
#[cfg(all(feature = "audio", target_os = "linux"))]
use std::thread::{self, JoinHandle};

#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(feature = "audio")]
use scope_core::{DataSource, Matrix, RingBuffer};

#[cfg(feature = "audio")]
struct ChannelHistory {
    rows: Vec<RingBuffer>,
}

#[cfg(feature = "audio")]
impl ChannelHistory {
    fn new(channels: usize, history: usize) -> Self {
        Self {
            rows: (0..channels).map(|_| RingBuffer::new(history)).collect(),
        }
    }
}
#[cfg(feature = "audio")]
pub enum AudioSource {
    #[cfg(target_os = "linux")]
    System(SystemCapture),
    Safe(SafeCapture),
}

#[cfg(all(feature = "audio", target_os = "linux"))]
pub struct SystemCapture {
    samples: Arc<Mutex<ChannelHistory>>,
    _child: Child,
    _reader: Option<JoinHandle<()>>,
    pub history: usize,
    sample_rate: u32,
    channels: usize,
}

#[cfg(feature = "audio")]
pub struct SafeCapture {
    samples: Arc<Mutex<ChannelHistory>>,
    _stream: cpal::Stream,
    pub history: usize,
    sample_rate: u32,
    channels: usize,
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
        #[cfg(target_os = "linux")]
        if monitor_preferred {
            return Ok(Self::System(SystemCapture::new(
                device,
                sample_rate,
                channels,
                history,
            )?));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = monitor_preferred;
        Ok(Self::Safe(SafeCapture::new(
            device,
            sample_rate,
            channels,
            history,
        )?))
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            #[cfg(target_os = "linux")]
            Self::System(source) => source.sample_rate,
            Self::Safe(source) => source.sample_rate,
        }
    }

    /// Actual captured channels: one mono row or synchronized left/right rows.
    pub fn channels(&self) -> usize {
        match self {
            #[cfg(target_os = "linux")]
            Self::System(source) => source.channels,
            Self::Safe(source) => source.channels,
        }
    }

    pub fn list_input_devices(system_audio: bool) -> anyhow::Result<()> {
        if cfg!(target_os = "linux") && system_audio {
            println!("System audio capture (PipeWire capture sink):");
            println!("  pw-cat --record --raw --format f32 --channels 2 --channel-map FL,FR --rate 48000 -P 'stream.capture.sink=true stream.dont-remix=true channelmix.upmix=false' -");
            println!();
        }

        println!("CPAL input devices (use --safe on Linux for input capture):");
        let host = cpal::default_host();
        for (name, default_tag, auto_tag) in list_safe_input_devices(&host)? {
            println!("  {default_tag}{name}{auto_tag}");
        }
        Ok(())
    }
}

#[cfg(all(feature = "audio", target_os = "linux"))]
impl SystemCapture {
    pub fn new(
        device: String,
        sample_rate: u32,
        channels: usize,
        history: usize,
    ) -> anyhow::Result<Self> {
        let channels = channels.clamp(1, 2);
        let (channels, target) = if channels == 2 {
            pipewire_capture_layout(device.trim())?
        } else {
            (1, device.trim().to_string())
        };
        let samples = Arc::new(Mutex::new(ChannelHistory::new(channels, history)));
        let reader_state = Arc::clone(&samples);
        let history_cap = history;
        let mut cmd = Command::new("pw-cat");
        cmd.arg("--record")
            .arg("--raw")
            .arg("--format")
            .arg("f32")
            .arg("--channels")
            .arg(channels.to_string())
            .arg("--channel-map")
            .arg(if channels == 2 { "FL,FR" } else { "MONO" })
            .arg("--rate")
            .arg(sample_rate.max(8_000).to_string())
            .arg("-P")
            .arg(if channels == 2 {
                "stream.capture.sink=true stream.dont-remix=true channelmix.upmix=false"
            } else {
                "stream.capture.sink=true channelmix.upmix=false"
            });

        if !target.is_empty() && !target.eq_ignore_ascii_case("auto") {
            cmd.arg("--target").arg(&target);
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
            sample_rate: sample_rate.max(8_000),
            channels,
        })
    }
}

#[cfg(all(feature = "audio", target_os = "linux"))]
impl Drop for SystemCapture {
    fn drop(&mut self) {
        let _ = self._child.kill();
        let _ = self._child.wait();
        if let Some(handle) = self._reader.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(all(feature = "audio", target_os = "linux"))]
fn pipewire_capture_layout(requested: &str) -> anyhow::Result<(usize, String)> {
    // Inspect, never reconfigure, the selected sink before requesting its monitor.
    // Without its layout a mono source could be upmixed into a fictitious R channel.
    let output = Command::new("pw-dump").output().map_err(|err| {
        anyhow::anyhow!("failed to inspect PipeWire sink layout with pw-dump: {err}")
    })?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "pw-dump failed to inspect PipeWire sink layout"
        ));
    }
    let objects: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| anyhow::anyhow!("invalid pw-dump sink layout: {err}"))?;
    pipewire_layout_from_dump(&objects, requested)
}

#[cfg(all(feature = "audio", target_os = "linux"))]
fn pipewire_layout_from_dump(
    dump: &serde_json::Value,
    requested: &str,
) -> anyhow::Result<(usize, String)> {
    let objects = dump
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("pw-dump is not an array"))?;
    let automatic = requested.is_empty() || requested.eq_ignore_ascii_case("auto");
    let default_sink = objects.iter().find_map(|object| {
        if object["props"]["metadata.name"] != "default" {
            return None;
        }
        object["metadata"].as_array()?.iter().find_map(|entry| {
            if entry["key"] != "default.audio.sink" {
                return None;
            }
            if let Some(value) = entry["value"].as_str() {
                let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
                parsed["name"].as_str().map(str::to_owned)
            } else {
                entry["value"]["name"].as_str().map(str::to_owned)
            }
        })
    });
    let target = if automatic {
        default_sink.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "cannot discover default PipeWire sink; select --device or request --channels 1"
            )
        })?
    } else {
        requested
    };
    let numeric_target = target.parse::<u64>().ok();
    let node = objects
        .iter()
        .find(|object| {
            let props = &object["info"]["props"];
            object["type"] == "PipeWire:Interface:Node"
                && (props["node.name"].as_str() == Some(target)
                    || props["object.serial"].as_str() == Some(target)
                    || (numeric_target.is_some()
                        && props["object.serial"].as_u64() == numeric_target))
        })
        .ok_or_else(|| anyhow::anyhow!("cannot inspect PipeWire target '{target}'"))?;
    let props = &node["info"]["props"];
    let mut positions = Vec::new();
    if let Some(array) = props["audio.position"].as_array() {
        positions.extend(
            array
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::to_owned),
        );
    } else if let Some(layout) = props["audio.position"].as_str() {
        positions.extend(
            layout
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .filter(|part| !part.is_empty())
                .map(str::to_owned),
        );
    }
    if positions.is_empty() {
        if let Some(formats) = node["info"]["params"]["Format"].as_array() {
            for format in formats {
                if let Some(array) = format["position"].as_array() {
                    positions.extend(
                        array
                            .iter()
                            .filter_map(|value| value.as_str())
                            .map(str::to_owned),
                    );
                }
            }
        }
    }
    if positions.is_empty() {
        // Port labels describe the active layout, including multichannel sinks.
        for object in objects {
            let port = &object["info"]["props"];
            if object["type"] == "PipeWire:Interface:Port"
                && (port["node.id"] == node["id"]
                    || port["node.id"]
                        .as_str()
                        .and_then(|id| id.parse::<u64>().ok())
                        == node["id"].as_u64())
                && (object["info"]["direction"] == "output"
                    || port["port.direction"] == "out"
                    || port["port.monitor"] == true)
            {
                if let Some(position) = port["audio.channel"].as_str() {
                    positions.push(position.to_string());
                }
            }
        }
    }
    let channels = if positions.iter().any(|position| position == "FL")
        && positions.iter().any(|position| position == "FR")
    {
        2
    } else if positions.iter().all(|position| position == "MONO")
        && (!positions.is_empty()
            || props["audio.channels"].as_u64() == Some(1)
            || props["audio.channels"].as_str() == Some("1"))
    {
        1
    } else {
        return Err(anyhow::anyhow!(
            "PipeWire target '{target}' has no identifiable FL/FR or mono layout; request --channels 1 for scalar capture"
        ));
    };
    // Pin automatic selection to the inspected sink so startup cannot race a
    // default-sink change and apply the wrong channel layout.
    Ok((channels, target.to_string()))
}

#[cfg(feature = "audio")]
fn select_input_config(
    default: cpal::SupportedStreamConfig,
    supported: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
    requested: usize,
) -> anyhow::Result<cpal::SupportedStreamConfig> {
    let requested = requested.clamp(1, 2) as u16;
    let mut preferred = None;
    let mut mono = None;
    for range in supported {
        if range.channels() != requested && range.channels() != 1 {
            // CPAL does not expose channel positions: never call the first two
            // channels of an arbitrary multichannel configuration L/R.
            continue;
        }
        let rate = default
            .sample_rate()
            .0
            .clamp(range.min_sample_rate().0, range.max_sample_rate().0);
        let config = range.with_sample_rate(cpal::SampleRate(rate));
        let slot = if config.channels() == requested {
            &mut preferred
        } else {
            &mut mono
        };
        if slot
            .as_ref()
            .is_none_or(|previous: &cpal::SupportedStreamConfig| {
                previous.sample_format() != default.sample_format()
                    && config.sample_format() == default.sample_format()
            })
        {
            *slot = Some(config);
        }
    }
    preferred
        .or(mono)
        .or_else(|| (default.channels() == 1).then_some(default))
        .ok_or_else(|| anyhow::anyhow!("input device has no supported mono/stereo configuration"))
}

#[cfg(feature = "audio")]
impl SafeCapture {
    pub fn new(
        device: String,
        _sample_rate: u32,
        channels: usize,
        history: usize,
    ) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = Self::resolve_device(&host, device.as_str())?;
        let cfg = with_suppressed_backend_noise(|| device.default_input_config())
            .map_err(|err| anyhow::anyhow!("failed to inspect default input config: {err}"))?;
        let cfg = if usize::from(cfg.channels()) == channels.clamp(1, 2) {
            cfg
        } else {
            let supported = with_suppressed_backend_noise(|| device.supported_input_configs())
                .map_err(|err| {
                    anyhow::anyhow!("failed to inspect supported input configs: {err}")
                })?;
            select_input_config(cfg, supported, channels)?
        };
        let stream_config = cfg.config();
        let device_label = with_suppressed_backend_noise(|| device.name().ok())
            .unwrap_or_else(|| "<unknown>".to_string());
        let stream_err = |err| {
            anyhow::anyhow!(
                "failed to open input stream on '{device_label}' with {} ch @ {} Hz: {err}",
                stream_config.channels,
                stream_config.sample_rate.0
            )
        };

        let channels = usize::from(stream_config.channels);
        let shared = Arc::new(Mutex::new(ChannelHistory::new(channels, history)));
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => build_sample_stream::<f32>,
            cpal::SampleFormat::F64 => build_sample_stream::<f64>,
            cpal::SampleFormat::I8 => build_sample_stream::<i8>,
            cpal::SampleFormat::I16 => build_sample_stream::<i16>,
            cpal::SampleFormat::I32 => build_sample_stream::<i32>,
            cpal::SampleFormat::I64 => build_sample_stream::<i64>,
            cpal::SampleFormat::U8 => build_sample_stream::<u8>,
            cpal::SampleFormat::U16 => build_sample_stream::<u16>,
            cpal::SampleFormat::U32 => build_sample_stream::<u32>,
            cpal::SampleFormat::U64 => build_sample_stream::<u64>,
            format => return Err(anyhow::anyhow!("unsupported input sample format: {format}")),
        }(&device, &stream_config, Arc::clone(&shared), history)
        .map_err(stream_err)?;

        stream
            .play()
            .map_err(|err| anyhow::anyhow!("failed to start input stream: {err}"))?;

        Ok(Self {
            samples: shared,
            _stream: stream,
            history,
            sample_rate: stream_config.sample_rate.0,
            channels,
        })
    }

    fn resolve_device(host: &cpal::Host, requested: &str) -> anyhow::Result<cpal::Device> {
        let requested = requested.trim();
        if !cfg!(target_os = "linux")
            && (requested.is_empty() || requested.eq_ignore_ascii_case("auto"))
        {
            return with_suppressed_backend_noise(|| host.default_input_device())
                .ok_or_else(|| anyhow::anyhow!("no default input device found"));
        }

        let mut candidates = with_suppressed_backend_noise(|| collect_input_devices(host))?;
        if candidates.is_empty() {
            return Err(anyhow::anyhow!("no input device found"));
        }

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
                name == default_name && !is_likely_system_output_capture(name)
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
            "only monitor-like input sources were found; select one explicitly with --device or use system audio capture"
        ))
    }
}

#[cfg(feature = "audio")]
impl DataSource for AudioSource {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        match self {
            #[cfg(target_os = "linux")]
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

#[cfg(all(feature = "audio", target_os = "linux"))]
impl DataSource for SystemCapture {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        Ok(Some(snapshot_samples(
            &self.samples,
            self.history,
            self.channels,
        )))
    }
}

#[cfg(feature = "audio")]
impl DataSource for SafeCapture {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        Ok(Some(snapshot_samples(
            &self.samples,
            self.history,
            self.channels,
        )))
    }
}

#[cfg(feature = "audio")]
fn snapshot_samples(state: &Mutex<ChannelHistory>, history: usize, channels: usize) -> Matrix {
    // Allocate before locking; the callback only waits for bounded slice copies.
    let mut rows: Matrix = (0..channels).map(|_| Vec::with_capacity(history)).collect();
    {
        let guard = state.lock().unwrap_or_else(|err| err.into_inner());
        for (window, ring) in rows.iter_mut().zip(&guard.rows) {
            let (first, second) = ring.data.as_slices();
            window.extend_from_slice(first);
            window.extend_from_slice(second);
        }
    }
    rows
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

#[cfg(all(feature = "audio", unix))]
struct StderrSilencer {
    saved_fd: i32,
    _null_file: File,
}

#[cfg(all(feature = "audio", unix))]
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

#[cfg(all(feature = "audio", unix))]
impl Drop for StderrSilencer {
    fn drop(&mut self) {
        const STDERR_FILENO: i32 = 2;
        unsafe {
            let _ = dup2(self.saved_fd, STDERR_FILENO);
            let _ = close(self.saved_fd);
        }
    }
}

#[cfg(all(feature = "audio", unix))]
unsafe extern "C" {
    fn dup(fd: i32) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn close(fd: i32) -> i32;
}

#[cfg(all(feature = "audio", target_os = "linux"))]
fn read_pwcat_stdout(
    mut stdout: impl Read + Send + 'static,
    channels: usize,
    state: Arc<Mutex<ChannelHistory>>,
    history: usize,
) {
    let mut pending = Vec::<u8>::with_capacity(8192 + channels * 4);
    let mut buf = [0u8; 8192];
    let mut staging = channel_staging(channels, history);

    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        pending.extend_from_slice(&buf[..n]);
        let frame_bytes = channels * 4;
        let usable = pending.len() / frame_bytes * frame_bytes;
        if usable == 0 {
            continue;
        }

        push_channels(
            pending[..usable]
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64),
            channels,
            &state,
            history,
            &mut staging,
        );
        pending.drain(..usable);
    }
}

#[cfg(feature = "audio")]
trait CaptureSample: cpal::SizedSample {
    fn normalized(self) -> f64;
}

#[cfg(feature = "audio")]
macro_rules! impl_capture_samples {
    ($($sample:ty => $convert:expr),+ $(,)?) => {
        $(
            impl CaptureSample for $sample {
                fn normalized(self) -> f64 {
                    ($convert)(self)
                }
            }
        )+
    };
}

#[cfg(feature = "audio")]
impl_capture_samples!(
    // Preserve the scaling used by the existing four input formats.
    f32 => |sample: f32| f64::from(sample),
    i16 => |sample: i16| f64::from(sample) / f64::from(i16::MAX),
    i32 => |sample: i32| f64::from(sample) / f64::from(i32::MAX),
    u16 => |sample: u16| (f64::from(sample) / f64::from(u16::MAX)) * 2.0 - 1.0,
    // Additional formats use CPAL's conversion, including exact unsigned silence.
    f64 => |sample: f64| sample,
    i8 => |sample: i8| cpal::Sample::to_sample::<f64>(sample),
    i64 => |sample: i64| cpal::Sample::to_sample::<f64>(sample),
    u8 => |sample: u8| cpal::Sample::to_sample::<f64>(sample),
    u32 => |sample: u32| cpal::Sample::to_sample::<f64>(sample),
    u64 => |sample: u64| cpal::Sample::to_sample::<f64>(sample),
);

#[cfg(feature = "audio")]
fn build_sample_stream<T: CaptureSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<Mutex<ChannelHistory>>,
    history: usize,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels);
    let mut staging = channel_staging(channels, history);
    device.build_input_stream(
        config,
        move |data: &[T], _| sample_cb(data, channels, &state, history, &mut staging),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )
}

#[cfg(feature = "audio")]
fn sample_cb<T: CaptureSample>(
    data: &[T],
    channels: usize,
    state: &Mutex<ChannelHistory>,
    history: usize,
    staging: &mut [Vec<f64>],
) {
    push_channels(
        data.iter().copied().map(CaptureSample::normalized),
        channels,
        state,
        history,
        staging,
    );
}

#[cfg(feature = "audio")]
fn channel_staging(channels: usize, history: usize) -> Vec<Vec<f64>> {
    (0..channels).map(|_| Vec::with_capacity(history)).collect()
}

#[cfg(feature = "audio")]
fn push_channels(
    samples: impl ExactSizeIterator<Item = f64>,
    channels: usize,
    state: &Mutex<ChannelHistory>,
    history: usize,
    staging: &mut [Vec<f64>],
) {
    for row in staging.iter_mut() {
        row.clear();
    }
    if channels == 0 || history == 0 {
        return;
    }

    // Older complete frames cannot survive this batch; do not convert them.
    let frames = samples.len() / channels;
    let skip = frames.saturating_sub(history) * channels;
    let usable = frames * channels - skip;
    for (index, sample) in samples.skip(skip).take(usable).enumerate() {
        // Keep raw normalized values: scalar views clamp only after downmixing.
        staging[index % channels].push(sample);
    }

    if !staging[0].is_empty() {
        // A single lock publishes every channel in the batch atomically.
        let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
        for (ring, row) in guard.rows.iter_mut().zip(staging) {
            for &sample in row.iter() {
                ring.push(sample);
            }
        }
    }
}

#[cfg(all(test, feature = "audio"))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    struct Chunked {
        bytes: Vec<u8>,
        position: usize,
        chunk: usize,
    }

    #[cfg(target_os = "linux")]
    impl Read for Chunked {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let count = self
                .chunk
                .min(self.bytes.len() - self.position)
                .min(output.len());
            output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fragmented_stereo_capture_preserves_channel_alignment_and_history() {
        for chunk in [1, 3, 5, 7, 8192] {
            let mut bytes: Vec<u8> = [0.2_f32, 0.6, -0.8, -0.4, 0.5, -0.5]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect();
            // Neither an incomplete stereo frame nor an incomplete float is audio.
            bytes.extend_from_slice(&0.9_f32.to_le_bytes());
            bytes.extend_from_slice(&[0, 1]);
            let state = Arc::new(Mutex::new(ChannelHistory::new(2, 2)));
            read_pwcat_stdout(
                Chunked {
                    bytes,
                    position: 0,
                    chunk,
                },
                2,
                Arc::clone(&state),
                2,
            );
            let frame = snapshot_samples(&state, 2, 2);
            assert_eq!(frame.len(), 2);
            assert_eq!(frame[0].len(), 2);
            assert_eq!(frame[1].len(), 2);
            assert!((frame[0][0] + 0.8).abs() < 1e-6);
            assert!((frame[1][0] + 0.4).abs() < 1e-6);
            assert_eq!(frame[0][1], 0.5);
            assert_eq!(frame[1][1], -0.5);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_large_reads_keep_latest_antiphase_pairs() {
        let bytes = (0..2051)
            .flat_map(|i| [i as f32, -(i as f32)])
            .flat_map(f32::to_le_bytes)
            .collect();
        let state = Arc::new(Mutex::new(ChannelHistory::new(2, 3)));
        read_pwcat_stdout(
            Chunked {
                bytes,
                position: 0,
                chunk: 8192,
            },
            2,
            Arc::clone(&state),
            3,
        );
        assert_eq!(
            snapshot_samples(&state, 3, 2),
            vec![
                vec![2048.0, 2049.0, 2050.0],
                vec![-2048.0, -2049.0, -2050.0]
            ]
        );
    }

    #[test]
    fn integer_input_retains_latest_separate_channels() {
        let state = Mutex::new(ChannelHistory::new(2, 2));
        let mut staging = channel_staging(2, 2);
        sample_cb(
            &[i16::MIN, i16::MAX, 0, i16::MIN, i16::MAX, 0],
            2,
            &state,
            2,
            &mut staging,
        );
        assert_eq!(
            snapshot_samples(&state, 2, 2),
            vec![
                vec![0.0, 1.0],
                vec![f64::from(i16::MIN) / f64::from(i16::MAX), 0.0]
            ]
        );
    }

    fn assert_conversion<T: CaptureSample>(input: &[T], expected: &[f64]) {
        let state = Mutex::new(ChannelHistory::new(1, input.len()));
        let mut staging = channel_staging(1, input.len());
        sample_cb(input, 1, &state, input.len(), &mut staging);
        let frame = snapshot_samples(&state, input.len(), 1);
        assert_eq!(frame.len(), 1);
        assert_eq!(frame[0].len(), expected.len());
        for (&actual, &expected) in frame[0].iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 1e-12,
                "{}: {actual} != {expected}",
                T::FORMAT
            );
        }
        // Every supported scalar PCM format must preserve unequal stereo channels too.
        let stereo: Vec<T> = input
            .iter()
            .copied()
            .zip(input.iter().rev().copied())
            .flat_map(|(left, right)| [left, right])
            .collect();
        let state = Mutex::new(ChannelHistory::new(2, input.len()));
        let mut staging = channel_staging(2, input.len());
        sample_cb(&stereo, 2, &state, input.len(), &mut staging);
        let frame = snapshot_samples(&state, input.len(), 2);
        for (actual, expected) in frame[0]
            .iter()
            .zip(expected)
            .chain(frame[1].iter().zip(expected.iter().rev()))
        {
            assert!((actual - expected).abs() < 1e-12, "{}", T::FORMAT);
        }
    }

    #[test]
    fn scalar_formats_preserve_silence_polarity_and_unclamped_normalization() {
        assert_conversion(&[-2.0_f32, 0.0, 0.5, 2.0], &[-2.0, 0.0, 0.5, 2.0]);
        assert_conversion(&[-2.0_f64, 0.0, 0.5, 2.0], &[-2.0, 0.0, 0.5, 2.0]);
        assert_conversion(&[i8::MIN, 0, i8::MAX], &[-1.0, 0.0, 127.0 / 128.0]);
        assert_conversion(&[i16::MIN, 0, i16::MAX], &[-32768.0 / 32767.0, 0.0, 1.0]);
        assert_conversion(
            &[i32::MIN, 0, i32::MAX],
            &[-2147483648.0 / 2147483647.0, 0.0, 1.0],
        );
        assert_conversion(&[i64::MIN, 0, i64::MAX], &[-1.0, 0.0, 1.0]);
        assert_conversion(&[u8::MIN, 128, u8::MAX], &[-1.0, 0.0, 127.0 / 128.0]);
        // Preserve the existing U16 midpoint convention instead of changing old streams.
        assert_conversion(&[u16::MIN, 32_768, u16::MAX], &[-1.0, 1.0 / 65_535.0, 1.0]);
        assert_conversion(
            &[u32::MIN, 1 << 31, u32::MAX],
            &[-1.0, 0.0, 1.0 - 1.0 / 2_147_483_648.0],
        );
        // f64 precision rounds the 64-bit positive endpoint to 1.0.
        assert_conversion(&[u64::MIN, 1 << 63, u64::MAX], &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn raw_channels_allow_legacy_clamp_after_average_and_ignore_partial_frames() {
        let state = Mutex::new(ChannelHistory::new(2, 4));
        let mut staging = channel_staging(2, 4);
        sample_cb(
            &[2.0_f64, 0.0, 2.0, -2.0, 0.75, -0.25, 0.9],
            2,
            &state,
            4,
            &mut staging,
        );
        let expected = vec![vec![2.0, 2.0, 0.75], vec![0.0, -2.0, -0.25]];
        let actual = snapshot_samples(&state, 4, 2);
        assert_eq!(actual, expected);
        let mono: Vec<_> = actual[0]
            .iter()
            .zip(&actual[1])
            .map(|(left, right)| ((left + right) / 2.0).clamp(-1.0, 1.0))
            .collect();
        assert_eq!(mono, [1.0, 0.0, 0.25]);
        sample_cb::<f64>(&[], 2, &state, 4, &mut staging);
        sample_cb(&[0.5], 2, &state, 4, &mut staging);
        assert_eq!(snapshot_samples(&state, 4, 2), expected);
    }

    #[test]
    fn circular_history_preserves_pairs_across_wrap_and_oversized_batches_without_growth() {
        for history in [0, 1, 3, 256] {
            let state = Mutex::new(ChannelHistory::new(2, history));
            let mut staging = channel_staging(2, history);
            let capacities: Vec<_> = staging.iter().map(Vec::capacity).collect();
            let mut expected = vec![Vec::new(), Vec::new()];
            let mut offset = 0;
            for frames in [0, 1, 2, 1, 257, 2, 1, 1024, 1] {
                let mut input = Vec::new();
                for i in 0..frames {
                    let value = ((offset + i) % 65) as f64 / 64.0;
                    input.extend_from_slice(&[value, -value]);
                    expected[0].push(value);
                    expected[1].push(-value);
                }
                offset += frames;
                input.push(0.9); // A partial frame must never enter history.
                sample_cb(&input, 2, &state, history, &mut staging);
                for row in &mut expected {
                    let excess = row.len().saturating_sub(history);
                    row.drain(..excess);
                }
                assert_eq!(snapshot_samples(&state, history, 2), expected);
                assert_eq!(
                    staging.iter().map(Vec::capacity).collect::<Vec<_>>(),
                    capacities
                );
            }
        }
    }

    #[test]
    fn empty_and_zero_history_do_not_invent_samples_and_mono_stays_mono() {
        for channels in [1, 2] {
            let state = Mutex::new(ChannelHistory::new(channels, 3));
            assert_eq!(
                snapshot_samples(&state, 3, channels),
                vec![Vec::<f64>::new(); channels]
            );
            let empty = Mutex::new(ChannelHistory::new(channels, 0));
            let mut staging = channel_staging(channels, 0);
            sample_cb(&[0.0_f64; 4], channels, &empty, 0, &mut staging);
            assert_eq!(
                snapshot_samples(&empty, 0, channels),
                vec![Vec::<f64>::new(); channels]
            );
        }
        assert_conversion(&[0.0_f32, 0.0, 0.0], &[0.0, 0.0, 0.0]);
    }

    fn input_range(channels: u16, rate: u32) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            cpal::SampleRate(rate),
            cpal::SampleRate(rate),
            cpal::SupportedBufferSize::Unknown,
            cpal::SampleFormat::F32,
        )
    }

    #[test]
    fn cpal_selects_supported_stereo_or_genuine_mono_with_actual_rate() {
        let mono = input_range(1, 48000).with_max_sample_rate();
        let stereo = select_input_config(
            mono.clone(),
            [input_range(1, 48000), input_range(2, 44100)].into_iter(),
            2,
        )
        .expect("supported stereo configuration");
        assert_eq!(stereo.channels(), 2);
        assert_eq!(stereo.sample_rate().0, 44100);
        let fallback = select_input_config(mono, [input_range(1, 48000)].into_iter(), 2)
            .expect("native mono fallback");
        assert_eq!(fallback.channels(), 1);
        let multichannel = input_range(6, 48000).with_max_sample_rate();
        assert!(select_input_config(multichannel, [input_range(6, 48000)].into_iter(), 2).is_err());
    }

    #[cfg(target_os = "linux")]
    fn sink_dump(props: serde_json::Value) -> serde_json::Value {
        serde_json::json!([
            {
                "id": 1, "type": "PipeWire:Interface:Metadata",
                "props": {"metadata.name": "default"},
                "metadata": [{"key": "default.audio.sink", "value": {"name": "sink.test"}}]
            },
            {"id": 10, "type": "PipeWire:Interface:Node", "info": {"props": props}}
        ])
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_discovers_mono_and_explicit_front_left_right_without_fake_upmix() {
        let mono = sink_dump(serde_json::json!({
            "node.name": "sink.test", "audio.channels": 1, "audio.position": "[ MONO ]"
        }));
        assert_eq!(
            pipewire_layout_from_dump(&mono, "auto").expect("native mono layout"),
            (1, "sink.test".into())
        );
        let stereo = sink_dump(serde_json::json!({
            "node.name": "sink.test", "object.serial": 123, "audio.position": ["FL", "FR"]
        }));
        assert_eq!(
            pipewire_layout_from_dump(&stereo, "123").expect("explicit stereo layout"),
            (2, "123".into())
        );
        // pw-cat numeric targets are object serials, not transient graph IDs.
        assert!(pipewire_layout_from_dump(&stereo, "10").is_err());
        let mut collision = stereo.clone();
        collision.as_array_mut().expect("array fixture").insert(
            0,
            serde_json::json!({
                "id": 123,
                "type": "PipeWire:Interface:Node",
                "info": {"props": {
                    "node.name": "unrelated.mono", "object.serial": 456,
                    "audio.position": ["MONO"]
                }}
            }),
        );
        assert_eq!(
            pipewire_layout_from_dump(&collision, "123")
                .expect("serial selects the intended stereo sink")
                .0,
            2
        );
        let mut string_metadata = stereo.clone();
        string_metadata[0]["metadata"][0]["value"] = serde_json::json!("{\"name\":\"sink.test\"}");
        assert_eq!(
            pipewire_layout_from_dump(&string_metadata, "auto")
                .expect("string metadata")
                .0,
            2
        );
        let unknown = sink_dump(serde_json::json!({
            "node.name": "sink.test", "audio.channels": 2, "audio.position": "[ AUX0 AUX1 ]"
        }));
        assert!(pipewire_layout_from_dump(&unknown, "auto").is_err());
        assert!(pipewire_layout_from_dump(&stereo, "missing").is_err());
        assert!(pipewire_layout_from_dump(&serde_json::json!([]), "auto").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_discovers_layout_from_active_port_labels() {
        let mut dump = sink_dump(serde_json::json!({"node.name": "sink.test"}));
        for channel in ["FL", "FR", "FC"] {
            dump.as_array_mut().expect("array fixture").push(serde_json::json!({
                "type": "PipeWire:Interface:Port",
                "info": {"direction": "output", "props": {"node.id": 10, "audio.channel": channel}}
            }));
        }
        assert_eq!(
            pipewire_layout_from_dump(&dump, "auto")
                .expect("monitor stereo ports")
                .0,
            2
        );
        for object in dump
            .as_array_mut()
            .expect("array fixture")
            .iter_mut()
            .skip(2)
        {
            object["info"]["direction"] = serde_json::json!("input");
        }
        assert!(
            pipewire_layout_from_dump(&dump, "auto").is_err(),
            "playback ports alone do not establish monitor channels"
        );
        dump.as_array_mut()
            .expect("array fixture")
            .push(serde_json::json!({
                "type": "PipeWire:Interface:Port",
                "info": {"direction": "output", "props": {"node.id": 10, "audio.channel": "MONO"}}
            }));
        assert_eq!(
            pipewire_layout_from_dump(&dump, "auto")
                .expect("native mono monitor")
                .0,
            1
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_fragmented_mono_and_empty_input_remain_honest() {
        for values in [vec![], vec![0.0_f32, 0.5, -0.5]] {
            let state = Arc::new(Mutex::new(ChannelHistory::new(1, 3)));
            let bytes = values
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            read_pwcat_stdout(
                Chunked {
                    bytes,
                    position: 0,
                    chunk: 3,
                },
                1,
                Arc::clone(&state),
                3,
            );
            assert_eq!(
                snapshot_samples(&state, 3, 1),
                vec![values.into_iter().map(f64::from).collect::<Vec<_>>()]
            );
        }
        let mut dump = sink_dump(serde_json::json!({
            "node.name": "sink.test", "audio.channels": "1"
        }));
        assert_eq!(
            pipewire_layout_from_dump(&dump, "auto")
                .expect("mono channel count")
                .0,
            1
        );
        dump[1]["info"]["params"] = serde_json::json!({
            "Format": [{"position": ["FL", "FR"]}]
        });
        assert_eq!(
            pipewire_layout_from_dump(&dump, "auto")
                .expect("active stereo format")
                .0,
            2
        );
    }
}
