use std::fs;

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub os_name: String,
    pub kernel_version: String,
    pub arch: String,
    pub cpu_cores: usize,
    pub cpu_model: String,
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub free_ram_mb: u64,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    pub load_1m: f32,
    pub load_5m: f32,
    pub load_15m: f32,
    pub uptime_secs: u64,
}

impl SystemInfo {
    pub fn collect() -> Self {
        let arch = std::env::consts::ARCH.to_string();
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let os_name = Self::read_os_name();
        let kernel_version = Self::read_kernel_version();
        let cpu_model = Self::read_cpu_model();
        let (total_ram_mb, available_ram_mb, free_ram_mb, swap_total_mb, swap_free_mb) =
            Self::read_meminfo();
        let (load_1m, load_5m, load_15m) = Self::read_loadavg();
        let uptime_secs = Self::read_uptime();

        Self {
            os_name,
            kernel_version,
            arch,
            cpu_cores,
            cpu_model,
            total_ram_mb,
            available_ram_mb,
            free_ram_mb,
            swap_total_mb,
            swap_free_mb,
            load_1m,
            load_5m,
            load_15m,
            uptime_secs,
        }
    }

    fn read_os_name() -> String {
        if let Ok(content) = fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                    return val.trim_matches('"').to_string();
                }
            }
        }
        std::env::consts::OS.to_string()
    }

    fn read_kernel_version() -> String {
        if let Ok(content) = fs::read_to_string("/proc/version") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 3 {
                return format!("{} {}", parts[0], parts[2]);
            }
            return content.lines().next().unwrap_or("Unknown").to_string();
        }
        "Linux (unknown)".to_string()
    }

    fn read_cpu_model() -> String {
        if let Ok(content) = fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("model name") || trimmed.starts_with("Hardware") || trimmed.starts_with("CPU part") {
                    if let Some(pos) = trimmed.find(':') {
                        let val = trimmed[pos + 1..].trim();
                        if !val.is_empty() {
                            if trimmed.starts_with("CPU part") {
                                return format!("ARM Cortex part {}", val);
                            }
                            return val.to_string();
                        }
                    }
                }
            }
        }
        format!("Generic {} processor", std::env::consts::ARCH)
    }

    fn read_meminfo() -> (u64, u64, u64, u64, u64) {
        let mut total = 0;
        let mut avail = 0;
        let mut free = 0;
        let mut swap_total = 0;
        let mut swap_free = 0;

        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                let mut parts = line.split_whitespace();
                if let (Some(key), Some(val_str)) = (parts.next(), parts.next()) {
                    if let Ok(kb) = val_str.parse::<u64>() {
                        let mb = kb / 1024;
                        match key {
                            "MemTotal:" => total = mb,
                            "MemAvailable:" => avail = mb,
                            "MemFree:" => free = mb,
                            "SwapTotal:" => swap_total = mb,
                            "SwapFree:" => swap_free = mb,
                            _ => {}
                        }
                    }
                }
            }
        }
        (total, avail, free, swap_total, swap_free)
    }

    fn read_loadavg() -> (f32, f32, f32) {
        if let Ok(content) = fs::read_to_string("/proc/loadavg") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 3 {
                let l1 = parts[0].parse::<f32>().unwrap_or(0.0);
                let l5 = parts[1].parse::<f32>().unwrap_or(0.0);
                let l15 = parts[2].parse::<f32>().unwrap_or(0.0);
                return (l1, l5, l15);
            }
        }
        (0.0, 0.0, 0.0)
    }

    fn read_uptime() -> u64 {
        if let Ok(content) = fs::read_to_string("/proc/uptime") {
            if let Some(sec_str) = content.split_whitespace().next() {
                if let Ok(secs) = sec_str.parse::<f64>() {
                    return secs as u64;
                }
            }
        }
        0
    }

    pub fn display_summary(&self) {
        let uptime_hours = self.uptime_secs / 3600;
        let uptime_mins = (self.uptime_secs % 3600) / 60;
        let uptime_rem_secs = self.uptime_secs % 60;

        let used_ram = self.total_ram_mb.saturating_sub(self.available_ram_mb);
        let ram_pct = if self.total_ram_mb > 0 {
            (used_ram as f64 / self.total_ram_mb as f64) * 100.0
        } else {
            0.0
        };

        println!("\x1b[1;36m===============================================================\x1b[0m");
        println!("\x1b[1;32m                  DEVICE & SYSTEM SPECIFICATIONS               \x1b[0m");
        println!("\x1b[1;36m===============================================================\x1b[0m");
        println!("  \x1b[1mOS\x1b[0m:                 {}", self.os_name);
        println!("  \x1b[1mKernel\x1b[0m:             {}", self.kernel_version);
        println!("  \x1b[1mArchitecture\x1b[0m:       \x1b[33m{}\x1b[0m", self.arch);
        println!("  \x1b[1mCPU Cores\x1b[0m:          \x1b[32m{} logical cores\x1b[0m", self.cpu_cores);
        println!("  \x1b[1mCPU Model\x1b[0m:          {}", self.cpu_model);
        println!(
            "  \x1b[1mSystem Memory\x1b[0m:      {:.2} GB total ({:.2} GB available, {:.1}% used)",
            self.total_ram_mb as f64 / 1024.0,
            self.available_ram_mb as f64 / 1024.0,
            ram_pct
        );
        if self.swap_total_mb > 0 {
            let used_swap = self.swap_total_mb.saturating_sub(self.swap_free_mb);
            println!(
                "  \x1b[1mSwap Memory\x1b[0m:        {:.2} GB total ({:.2} GB used)",
                self.swap_total_mb as f64 / 1024.0,
                used_swap as f64 / 1024.0
            );
        }
        println!(
            "  \x1b[1mLoad Average\x1b[0m:       1m: {:.2}, 5m: {:.2}, 15m: {:.2}",
            self.load_1m, self.load_5m, self.load_15m
        );
        println!(
            "  \x1b[1mSystem Uptime\x1b[0m:      {}h {}m {}s",
            uptime_hours, uptime_mins, uptime_rem_secs
        );
        println!("\x1b[1;36m===============================================================\x1b[0m");
    }
}
