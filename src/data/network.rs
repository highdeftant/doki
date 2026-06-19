use std::{fs, io, time::Instant};

use crate::{DataSource, Matrix, RingBuffer};

#[derive(Debug)]
pub struct NetSource {
    interface: String,
    rssi: RingBuffer,
    noise: RingBuffer,
    rx_mbps: RingBuffer,
    tx_mbps: RingBuffer,
    last_rx: Option<u64>,
    last_tx: Option<u64>,
    last_sample: Instant,
}

impl NetSource {
    pub fn new(interface: Option<String>, history: usize) -> io::Result<Self> {
        let interface = interface.unwrap_or_else(detect_wireless_interface);
        Ok(Self {
            interface,
            rssi: RingBuffer::new(history),
            noise: RingBuffer::new(history),
            rx_mbps: RingBuffer::new(history),
            tx_mbps: RingBuffer::new(history),
            last_rx: None,
            last_tx: None,
            last_sample: Instant::now(),
        })
    }

    fn read_wireless(&self, iface: &str) -> Option<f64> {
        let content = fs::read_to_string("/proc/net/wireless").ok()?;
        for line in content.lines() {
            if !line.trim_start().starts_with(iface) {
                continue;
            }
            let mut parts = line.split_whitespace().skip(1); // skip iface:
            let _status = parts.next()?;
            let _link = parts.next()?;
            let level = parts.next()?.trim_end_matches('.').parse::<f64>().ok()?;
            return Some(level);
        }
        None
    }

    fn read_proc_net_dev(&self, iface: &str) -> Option<(u64, u64)> {
        let content = fs::read_to_string("/proc/net/dev").ok()?;
        for line in content.lines().skip(2) {
            if !line.contains(':') {
                continue;
            }
            let mut split = line.split(':');
            let name = split.next()?.trim();
            if !name.eq(iface) {
                continue;
            }
            let values: Vec<&str> = split.next()?.split_whitespace().collect();
            if values.len() < 9 {
                continue;
            }
            let rx = values[0].parse::<u64>().ok()?;
            let tx = values[8].parse::<u64>().ok()?;
            return Some((rx, tx));
        }
        None
    }
}

impl DataSource for NetSource {
    fn next_frame(&mut self) -> io::Result<Option<Matrix>> {
        let rssi = self.read_wireless(&self.interface).unwrap_or(-90.0);

        let (rx_rate, tx_rate) = if let Some((rx, tx)) = self.read_proc_net_dev(&self.interface) {
            let now = Instant::now();
            let dt = now.duration_since(self.last_sample).as_secs_f64().max(0.05);

            let (mut rx_rate, mut tx_rate) = (0.0, 0.0);
            if let (Some(last_rx), Some(last_tx)) = (self.last_rx, self.last_tx) {
                let d_rx = rx.saturating_sub(last_rx) as f64;
                let d_tx = tx.saturating_sub(last_tx) as f64;
                rx_rate = (d_rx * 8.0) / (dt * 1_000_000.0);
                tx_rate = (d_tx * 8.0) / (dt * 1_000_000.0);
            }

            self.last_rx = Some(rx);
            self.last_tx = Some(tx);
            self.last_sample = now;
            (rx_rate, tx_rate)
        } else {
            (0.0, 0.0)
        };

        self.rssi.push(rssi);
        self.noise.push(rssi - 1.0);
        self.rx_mbps.push(rx_rate);
        self.tx_mbps.push(tx_rate);

        Ok(Some(vec![
            self.rssi.snapshot(),
            self.noise.snapshot(),
            self.rx_mbps.snapshot(),
            self.tx_mbps.snapshot(),
        ]))
    }
}

fn detect_wireless_interface() -> String {
    let fallback = "wlan0".to_string();

    let content = match fs::read_to_string("/proc/net/wireless") {
        Ok(content) => content,
        Err(_) => return fallback,
    };

    for line in content.lines().skip(2) {
        if let Some(name) = line.split(':').next() {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }

    fallback
}
