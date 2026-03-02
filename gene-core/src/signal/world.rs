use crate::signal::bus::SignalBus;
use crate::signal::types::SignalId;

/// Well-known World signal IDs — parallel to the constants in main.rs.
pub struct WorldSignalIds {
    pub cpu_load:     SignalId,
    pub net_rx:       SignalId,
    pub net_tx:       SignalId,
    pub disk_io:      SignalId,
    pub uptime_cycle: SignalId,
    pub proc_count:   SignalId,
    pub swap_used:     SignalId,
    pub iowait:        SignalId,
    pub ctx_switches:  SignalId,
    pub mem_available: SignalId,
}

/// Polls OS metrics from /proc every N ticks and writes them into the SignalBus.
///
/// Net and disk metrics are delta-based (bytes/sectors since last poll).
/// On the first call, signals are initialised to baseline and state is saved
/// without emitting a value — this avoids a spurious spike from an unknown
/// cumulative starting point.
pub struct WorldSignalPoller {
    prev_net_rx_bytes:  u64,
    prev_net_tx_bytes:  u64,
    prev_disk_sectors:  u64,
    prev_cpu_total:     u64,
    prev_cpu_iowait:    u64,
    prev_ctx_switches:  u64,
    initialized:        bool,
}

impl WorldSignalPoller {
    pub fn new() -> Self {
        Self {
            prev_net_rx_bytes: 0,
            prev_net_tx_bytes: 0,
            prev_disk_sectors: 0,
            prev_cpu_total: 0,
            prev_cpu_iowait: 0,
            prev_ctx_switches: 0,
            initialized: false,
        }
    }

    /// Update all World signals on the bus. Should be called every 10 ticks.
    pub fn poll(&mut self, bus: &mut SignalBus, ids: &WorldSignalIds) {
        let (net_rx, net_tx) = Self::read_net_bytes();
        let disk_sectors     = Self::read_disk_sectors();

        let (cpu_total, cpu_iowait) = Self::read_cpu_stat();
        let ctx_switches = Self::read_ctx_switches();

        if !self.initialized {
            // First call: save baseline cumulative values, set signals to baseline.
            self.prev_net_rx_bytes = net_rx;
            self.prev_net_tx_bytes = net_tx;
            self.prev_disk_sectors = disk_sectors;
            self.prev_cpu_total    = cpu_total;
            self.prev_cpu_iowait   = cpu_iowait;
            self.prev_ctx_switches = ctx_switches;
            self.initialized = true;
            return;
        }

        // ── CPU load ─────────────────────────────────────────────────────────
        let raw_cpu = Self::read_cpu_load();
        let cur = bus.get_value(ids.cpu_load);
        bus.set_value(ids.cpu_load, cur * 0.9 + raw_cpu * 0.1);

        // ── Network RX ───────────────────────────────────────────────────────
        let rx_delta = net_rx.saturating_sub(self.prev_net_rx_bytes);
        // Ceiling: 10 MB per poll interval — normalises to 0–1
        let raw_rx = (rx_delta as f64 / 10_000_000.0).clamp(0.0, 1.0);
        let cur = bus.get_value(ids.net_rx);
        bus.set_value(ids.net_rx, cur * 0.9 + raw_rx * 0.1);

        // ── Network TX ───────────────────────────────────────────────────────
        let tx_delta = net_tx.saturating_sub(self.prev_net_tx_bytes);
        let raw_tx = (tx_delta as f64 / 10_000_000.0).clamp(0.0, 1.0);
        let cur = bus.get_value(ids.net_tx);
        bus.set_value(ids.net_tx, cur * 0.9 + raw_tx * 0.1);

        // ── Disk I/O ─────────────────────────────────────────────────────────
        let disk_delta = disk_sectors.saturating_sub(self.prev_disk_sectors);
        // Ceiling: 10 000 sectors per poll
        let raw_disk = (disk_delta as f64 / 10_000.0).clamp(0.0, 1.0);
        let cur = bus.get_value(ids.disk_io);
        bus.set_value(ids.disk_io, cur * 0.9 + raw_disk * 0.1);

        // ── Uptime circadian cycle ────────────────────────────────────────────
        // Sine wave over 24h period → [0, 1], baseline 0.5.
        let uptime = Self::read_uptime_secs();
        let raw_cycle = ((uptime / 86400.0) * std::f64::consts::TAU).sin() * 0.5 + 0.5;
        let cur = bus.get_value(ids.uptime_cycle);
        bus.set_value(ids.uptime_cycle, cur * 0.99 + raw_cycle * 0.01);

        // ── Process count ─────────────────────────────────────────────────────
        let raw_proc = Self::read_process_count();
        let cur = bus.get_value(ids.proc_count);
        bus.set_value(ids.proc_count, cur * 0.9 + raw_proc * 0.1);

        // ── Swap used ─────────────────────────────────────────────────────────
        let raw_swap = Self::read_swap_pressure();
        let cur = bus.get_value(ids.swap_used);
        bus.set_value(ids.swap_used, cur * 0.9 + raw_swap * 0.1);

        // ── I/O wait ──────────────────────────────────────────────────────────
        let delta_total  = cpu_total.saturating_sub(self.prev_cpu_total);
        let delta_iowait = cpu_iowait.saturating_sub(self.prev_cpu_iowait);
        let raw_iowait = if delta_total > 0 {
            (delta_iowait as f64 / delta_total as f64).clamp(0.0, 1.0)
        } else { 0.0 };
        let cur = bus.get_value(ids.iowait);
        bus.set_value(ids.iowait, cur * 0.9 + raw_iowait * 0.1);

        // ── Context switches (gene's own) ─────────────────────────────────────
        let delta_ctx = ctx_switches.saturating_sub(self.prev_ctx_switches);
        // Ceiling: 500 context switches per poll interval
        let raw_ctx = (delta_ctx as f64 / 500.0).clamp(0.0, 1.0);
        let cur = bus.get_value(ids.ctx_switches);
        bus.set_value(ids.ctx_switches, cur * 0.8 + raw_ctx * 0.2);

        // ── System memory available ───────────────────────────────────────────
        let raw_mem = Self::read_mem_available_pressure();
        let cur = bus.get_value(ids.mem_available);
        bus.set_value(ids.mem_available, cur * 0.9 + raw_mem * 0.1);

        // Save cumulative state for next poll
        self.prev_net_rx_bytes = net_rx;
        self.prev_net_tx_bytes = net_tx;
        self.prev_disk_sectors = disk_sectors;
        self.prev_cpu_total    = cpu_total;
        self.prev_cpu_iowait   = cpu_iowait;
        self.prev_ctx_switches = ctx_switches;
    }

    // ── /proc readers ────────────────────────────────────────────────────────

    /// /proc/loadavg — field 0 is 1-min load average.
    /// Normalised against 8-core ceiling.
    fn read_cpu_load() -> f64 {
        if let Ok(s) = std::fs::read_to_string("/proc/loadavg") {
            if let Some(field) = s.split_whitespace().next() {
                if let Ok(load) = field.parse::<f64>() {
                    return (load / 8.0).clamp(0.0, 1.0);
                }
            }
        }
        0.2 // baseline fallback
    }

    /// /proc/net/dev — sum bytes received and transmitted across all non-loopback interfaces.
    /// Returns (rx_bytes_cumulative, tx_bytes_cumulative).
    fn read_net_bytes() -> (u64, u64) {
        let mut rx_total: u64 = 0;
        let mut tx_total: u64 = 0;
        if let Ok(content) = std::fs::read_to_string("/proc/net/dev") {
            for line in content.lines().skip(2) {
                let line = line.trim();
                // Skip loopback
                if line.starts_with("lo:") {
                    continue;
                }
                // Format: "iface: rx_bytes packets ... tx_bytes ..."
                if let Some(colon) = line.find(':') {
                    let fields: Vec<&str> = line[colon + 1..].split_whitespace().collect();
                    if fields.len() >= 9 {
                        rx_total += fields[0].parse::<u64>().unwrap_or(0);
                        tx_total += fields[8].parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }
        (rx_total, tx_total)
    }

    /// /proc/diskstats — sum of sectors read (field index 5) and written (field index 9)
    /// across all physical block devices (sda, vda, nvme0n1 — skip partitions and loops).
    fn read_disk_sectors() -> u64 {
        let mut total: u64 = 0;
        if let Ok(content) = std::fs::read_to_string("/proc/diskstats") {
            for line in content.lines() {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 14 {
                    continue;
                }
                let dev = fields[2];
                // Only count whole-disk devices, not partitions (sda not sda1)
                let is_whole = dev.starts_with("sd") || dev.starts_with("vd")
                    || dev.starts_with("nvme") || dev.starts_with("xvd")
                    || dev.starts_with("hd");
                let has_digit_suffix = dev.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);
                // nvme devices end in p1, p2... so skip those; sd/vd/hd partitions end in digit
                let is_partition = if dev.contains("nvme") {
                    dev.contains('p') && has_digit_suffix
                } else {
                    has_digit_suffix
                };
                if is_whole && !is_partition {
                    // fields[5] = sectors_read, fields[9] = sectors_written
                    total += fields[5].parse::<u64>().unwrap_or(0);
                    total += fields[9].parse::<u64>().unwrap_or(0);
                }
            }
        }
        total
    }

    /// /proc/uptime — first field is seconds since boot.
    fn read_uptime_secs() -> f64 {
        if let Ok(s) = std::fs::read_to_string("/proc/uptime") {
            if let Some(field) = s.split_whitespace().next() {
                return field.parse::<f64>().unwrap_or(0.0);
            }
        }
        0.0
    }

    /// /proc/meminfo — SwapUsed / SwapTotal. Returns 0.0 if no swap configured.
    fn read_swap_pressure() -> f64 {
        let mut swap_total: u64 = 0;
        let mut swap_used: u64 = 0;
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("SwapTotal:") {
                    if let Some(v) = line.split_whitespace().nth(1) {
                        swap_total = v.parse().unwrap_or(0);
                    }
                } else if line.starts_with("SwapFree:") {
                    if let Some(v) = line.split_whitespace().nth(1) {
                        let free: u64 = v.parse().unwrap_or(0);
                        swap_used = swap_total.saturating_sub(free);
                    }
                }
            }
        }
        if swap_total == 0 { 0.0 } else { (swap_used as f64 / swap_total as f64).clamp(0.0, 1.0) }
    }

    /// /proc/stat — reads cumulative cpu line: user nice system idle iowait ...
    /// Returns (total_jiffies, iowait_jiffies).
    fn read_cpu_stat() -> (u64, u64) {
        if let Ok(content) = std::fs::read_to_string("/proc/stat") {
            if let Some(line) = content.lines().next() {
                // "cpu  user nice system idle iowait irq softirq steal guest guest_nice"
                let fields: Vec<u64> = line.split_whitespace()
                    .skip(1) // skip "cpu"
                    .filter_map(|f| f.parse().ok())
                    .collect();
                if fields.len() >= 5 {
                    let total: u64 = fields.iter().sum();
                    let iowait = fields[4];
                    return (total, iowait);
                }
            }
        }
        (0, 0)
    }

    /// /proc/self/status — voluntary + nonvoluntary context switches.
    fn read_ctx_switches() -> u64 {
        let mut total: u64 = 0;
        if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
            for line in content.lines() {
                if line.starts_with("voluntary_ctxt_switches:") || line.starts_with("nonvoluntary_ctxt_switches:") {
                    if let Some(v) = line.split_whitespace().nth(1) {
                        total += v.parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }
        total
    }

    /// /proc/meminfo — system-wide memory pressure: 1.0 - (MemAvailable / MemTotal).
    fn read_mem_available_pressure() -> f64 {
        let mut mem_total: u64 = 0;
        let mut mem_available: u64 = 0;
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(v) = line.split_whitespace().nth(1) {
                        mem_total = v.parse().unwrap_or(0);
                    }
                } else if line.starts_with("MemAvailable:") {
                    if let Some(v) = line.split_whitespace().nth(1) {
                        mem_available = v.parse().unwrap_or(0);
                    }
                }
            }
        }
        if mem_total == 0 { 0.3 } else {
            (1.0 - mem_available as f64 / mem_total as f64).clamp(0.0, 1.0)
        }
    }

    /// /proc/loadavg — field 3 is "running/total" processes.
    /// Returns total / 500.0, clamped 0–1.
    fn read_process_count() -> f64 {
        if let Ok(s) = std::fs::read_to_string("/proc/loadavg") {
            if let Some(field) = s.split_whitespace().nth(3) {
                if let Some(total_str) = field.split('/').nth(1) {
                    if let Ok(total) = total_str.parse::<f64>() {
                        return (total / 500.0).clamp(0.0, 1.0);
                    }
                }
            }
        }
        0.3 // baseline fallback
    }
}

impl Default for WorldSignalPoller {
    fn default() -> Self { Self::new() }
}
