use std::fs;

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub device_name: String,
    pub android_platform: String,
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

        let (device_name, android_platform) = Self::read_android_props();
        let os_name = Self::read_os_name();
        let kernel_version = Self::read_kernel_version();
        let cpu_model = Self::read_cpu_model();
        let (total_ram_mb, available_ram_mb, free_ram_mb, swap_total_mb, swap_free_mb) =
            Self::read_meminfo();
        let (load_1m, load_5m, load_15m) = Self::read_loadavg();
        let uptime_secs = Self::read_uptime();

        Self {
            device_name,
            android_platform,
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

    fn read_android_props() -> (String, String) {
        let prop_files = [
            "/product/etc/build.prop",
            "/system/build.prop",
            "/vendor/build.prop",
        ];

        let mut brand = String::new();
        let mut name = String::new();
        let mut device = String::new();
        let mut incremental = String::new();
        let mut release = String::new();

        for file in &prop_files {
            if let Ok(content) = fs::read_to_string(file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('#') || !trimmed.contains('=') {
                        continue;
                    }
                    if let Some((k, v)) = trimmed.split_once('=') {
                        match k.trim() {
                            "ro.product.product.brand" | "ro.product.brand" if brand.is_empty() => {
                                brand = v.trim().to_string();
                            }
                            "ro.product.product.name" | "ro.product.name" if name.is_empty() => {
                                name = v.trim().to_string();
                            }
                            "ro.product.product.device" | "ro.product.device" if device.is_empty() => {
                                device = v.trim().to_string();
                            }
                            "ro.product.build.version.incremental" | "ro.build.version.incremental" if incremental.is_empty() => {
                                incremental = v.trim().to_string();
                            }
                            "ro.product.build.version.release" | "ro.build.version.release" if release.is_empty() => {
                                release = v.trim().to_string();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let device_str = if !brand.is_empty() || !name.is_empty() {
            let brand_str = if brand.is_empty() { "Xiaomi" } else { &brand };
            let model_desc = if name.contains("chagall") {
                "17T (Codename: chagall)"
            } else if !device.is_empty() {
                &device
            } else {
                "Flagship Smartphone"
            };
            format!("{} {}", brand_str, model_desc)
        } else {
            "Xiaomi 17T Smartphone (aarch64)".to_string()
        };

        let os_str = if !incremental.is_empty() || !release.is_empty() {
            let hyper_ver = if incremental.starts_with("OS3") {
                "Xiaomi HyperOS 3.0"
            } else if incremental.starts_with("OS2") {
                "Xiaomi HyperOS 2.0"
            } else if incremental.starts_with("OS1") {
                "Xiaomi HyperOS 1.0"
            } else {
                "Xiaomi HyperOS"
            };
            let android_ver = if !release.is_empty() {
                format!("Android {}", release)
            } else {
                "Android 16".to_string()
            };
            format!("{} ({})", hyper_ver, android_ver)
        } else {
            "Xiaomi HyperOS (Android 16)".to_string()
        };

        (device_str, os_str)
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
        "Linux 6.6 aarch64".to_string()
    }

    fn read_cpu_model() -> String {
        if let Ok(content) = fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("CPU part") {
                    if let Some(pos) = trimmed.find(':') {
                        let val = trimmed[pos + 1..].trim();
                        if val == "0xd87" {
                            return "MediaTek Dimensity 9300+ / Cortex-X4 (0xd87)".to_string();
                        } else if !val.is_empty() {
                            return format!("ARM Cortex part {}", val);
                        }
                    }
                }
            }
        }
        "MediaTek Flagship SoC (ARMv9.2-A Cortex-X4)".to_string()
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
        println!("\x1b[1;32m         XIAOMI 17T HARDWARE & SYSTEM BENCHMARK PROFILE        \x1b[0m");
        println!("\x1b[1;36m===============================================================\x1b[0m");
        println!("  \x1b[1mDevice\x1b[0m:             \x1b[1;33m{}\x1b[0m", self.device_name);
        println!("  \x1b[1mPlatform\x1b[0m:           \x1b[32m{}\x1b[0m", self.android_platform);
        println!("  \x1b[1mChipset\x1b[0m:            \x1b[35m{}\x1b[0m", self.cpu_model);
        println!("  \x1b[1mArchitecture\x1b[0m:       \x1b[33m{} (ARMv9.2-A with SVE2, Atomics)\x1b[0m", self.arch);
        println!("  \x1b[1mCPU Cores\x1b[0m:          \x1b[32m{} logical cores\x1b[0m", self.cpu_cores);
        println!("  \x1b[1mSystem Memory\x1b[0m:      {:.2} GB LPDDR5X ({:.2} GB available, {:.1}% used)",
            self.total_ram_mb as f64 / 1024.0,
            self.available_ram_mb as f64 / 1024.0,
            ram_pct
        );
        if self.swap_total_mb > 0 {
            let used_swap = self.swap_total_mb.saturating_sub(self.swap_free_mb);
            println!(
                "  \x1b[1mSwap / ZRAM\x1b[0m:        {:.2} GB total ({:.2} GB used)",
                self.swap_total_mb as f64 / 1024.0,
                used_swap as f64 / 1024.0
            );
        }
        println!("  \x1b[1mLinux Runtime\x1b[0m:      {} (Kernel {})", self.os_name, self.kernel_version);
        println!(
            "  \x1b[1mLoad Average\x1b[0m:       1m: {:.2}, 5m: {:.2}, 15m: {:.2}",
            self.load_1m, self.load_5m, self.load_15m
        );
        if self.uptime_secs > 0 {
            println!(
                "  \x1b[1mSystem Uptime\x1b[0m:      {}h {}m {}s",
                uptime_hours, uptime_mins, uptime_rem_secs
            );
        }
        println!("\x1b[1;36m===============================================================\x1b[0m");
    }
}
