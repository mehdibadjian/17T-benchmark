use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::storage::LsmStorageEngine;

/// Client connection target: either a remote TCP server address or a local LSM storage engine.
#[derive(Clone)]
pub enum ClientTarget {
    Remote(String),
    Local(Arc<Mutex<LsmStorageEngine>>),
}

/// Statistics for a single benchmark phase.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub name: String,
    pub ops: usize,
    pub duration: Duration,
    pub ops_per_second: f64,
    pub avg_latency_us: f64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub p99_latency_us: u64,
    pub min_latency_us: u64,
    pub max_latency_us: u64,
    pub errors: usize,
}

/// Aggregated report from running the multi-threaded benchmark suite.
#[derive(Debug, Clone)]
pub struct BenchmarkReport {
    pub target: String,
    pub total_ops: usize,
    pub num_threads: usize,
    pub total_duration: Duration,
    pub overall_ops_per_second: f64,
    pub phases: Vec<PhaseResult>,
}

impl fmt::Display for BenchmarkReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "================================================================================")?;
        writeln!(f, "                   TURING HIGH-PERFORMANCE BENCHMARK SUITE")?;
        writeln!(f, "================================================================================")?;
        writeln!(f, " Target Engine    : {}", self.target)?;
        writeln!(f, " Total Operations : {} ops", self.total_ops)?;
        writeln!(f, " Concurrency      : {} worker threads", self.num_threads)?;
        writeln!(f, " Total Wall Time  : {:.4} s", self.total_duration.as_secs_f64())?;
        writeln!(f, " Aggregate Rate   : {:.2} ops/sec", self.overall_ops_per_second)?;
        writeln!(f, "--------------------------------------------------------------------------------")?;
        for (idx, phase) in self.phases.iter().enumerate() {
            writeln!(f, " Phase {}: {}", idx + 1, phase.name)?;
            writeln!(f, "   Operations     : {} ops", phase.ops)?;
            writeln!(f, "   Duration       : {:.4} s", phase.duration.as_secs_f64())?;
            writeln!(f, "   Throughput     : {:.2} ops/sec", phase.ops_per_second)?;
            writeln!(f, "   Avg Latency    : {:.2} µs", phase.avg_latency_us)?;
            writeln!(f, "   p50 Latency    : {} µs", phase.p50_latency_us)?;
            writeln!(f, "   p95 Latency    : {} µs", phase.p95_latency_us)?;
            writeln!(f, "   p99 Latency    : {} µs", phase.p99_latency_us)?;
            writeln!(f, "   Min / Max      : {} µs / {} µs", phase.min_latency_us, phase.max_latency_us)?;
            writeln!(f, "   Errors         : {}", phase.errors)?;
            if idx + 1 < self.phases.len() {
                writeln!(f, "   ----------------------------------------------------------------------------")?;
            }
        }
        writeln!(f, "================================================================================")?;
        Ok(())
    }
}

/// CLI and REPL client for interacting with `turing-server` remotely or locally.
pub struct CliClient {
    target: ClientTarget,
}

impl CliClient {
    /// Creates a new CLI client pointing to a remote server TCP address (e.g. "127.0.0.1:8088").
    pub fn new_remote(addr: impl Into<String>) -> Self {
        Self {
            target: ClientTarget::Remote(addr.into()),
        }
    }

    /// Creates a new CLI client operating directly on a local LSM storage engine.
    pub fn new_local(engine: LsmStorageEngine) -> Self {
        Self {
            target: ClientTarget::Local(Arc::new(Mutex::new(engine))),
        }
    }

    /// Creates a new CLI client with a shared Arc<Mutex<LsmStorageEngine>>.
    pub fn new_local_shared(engine: Arc<Mutex<LsmStorageEngine>>) -> Self {
        Self {
            target: ClientTarget::Local(engine),
        }
    }

    /// Returns a human-readable description of the target.
    pub fn target_description(&self) -> String {
        match &self.target {
            ClientTarget::Remote(addr) => format!("Remote Server ({})", addr),
            ClientTarget::Local(_) => "Local LSM Engine".to_string(),
        }
    }

    /// Inserts or updates a key-value pair.
    pub fn put(&mut self, key: &str, value: &str) -> Result<String, String> {
        match &self.target {
            ClientTarget::Remote(addr) => {
                let cmd = format!("PUT {} {}", key, value);
                Self::send_line_cmd(addr, &cmd)
            }
            ClientTarget::Local(engine) => {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                eng.put(key, value.as_bytes())
                    .map_err(|e| format!("Storage error: {}", e))?;
                Ok("OK".to_string())
            }
        }
    }

    /// Retrieves the value associated with a key.
    pub fn get(&mut self, key: &str) -> Result<Option<String>, String> {
        match &self.target {
            ClientTarget::Remote(addr) => {
                let cmd = format!("GET {}", key);
                let resp = Self::send_line_cmd(addr, &cmd)?;
                if resp.starts_with("VALUE ") {
                    Ok(Some(resp["VALUE ".len()..].to_string()))
                } else if resp == "NOT_FOUND" {
                    Ok(None)
                } else {
                    Err(format!("Unexpected server response: {}", resp))
                }
            }
            ClientTarget::Local(engine) => {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                match eng.get(key).map_err(|e| format!("Storage error: {}", e))? {
                    Some(val) => Ok(Some(String::from_utf8_lossy(&val).to_string())),
                    None => Ok(None),
                }
            }
        }
    }

    /// Deletes a key (writing a tombstone).
    pub fn del(&mut self, key: &str) -> Result<bool, String> {
        match &self.target {
            ClientTarget::Remote(addr) => {
                let cmd = format!("DEL {}", key);
                let resp = Self::send_line_cmd(addr, &cmd)?;
                if resp == "OK" {
                    Ok(true)
                } else {
                    Err(resp)
                }
            }
            ClientTarget::Local(engine) => {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                eng.delete(key).map_err(|e| format!("Storage error: {}", e))?;
                Ok(true)
            }
        }
    }

    /// Scans keys starting with prefix/start up to `limit`.
    pub fn scan(&mut self, start: Option<&str>, limit: usize) -> Result<Vec<(String, String)>, String> {
        match &self.target {
            ClientTarget::Remote(addr) => {
                let start_token = start.unwrap_or("");
                let cmd = format!("SCAN {} {}", start_token, limit);
                let resp = Self::send_line_cmd(addr, &cmd)?;
                let mut entries = Vec::new();
                for line in resp.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("ENTRIES") || trimmed == "END" || trimmed.is_empty() {
                        continue;
                    }
                    if let Some(space_idx) = trimmed.find(' ') {
                        let k = trimmed[..space_idx].to_string();
                        let v = trimmed[space_idx + 1..].to_string();
                        entries.push((k, v));
                    }
                }
                Ok(entries)
            }
            ClientTarget::Local(engine) => {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                let start_key = start.unwrap_or("");
                let raw_entries = eng
                    .scan(start_key, limit)
                    .map_err(|e| format!("Storage error: {}", e))?;
                let converted = raw_entries
                    .into_iter()
                    .map(|(k, v)| (k, String::from_utf8_lossy(&v).to_string()))
                    .collect();
                Ok(converted)
            }
        }
    }

    /// Retrieves statistics from the storage engine or server metrics.
    pub fn stats(&mut self) -> Result<String, String> {
        match &self.target {
            ClientTarget::Remote(addr) => Self::send_line_cmd(addr, "STATS"),
            ClientTarget::Local(engine) => {
                let _eng = engine.lock().map_err(|e| e.to_string())?;
                Ok("STAT engine local_lsm\r\nSTAT status active\r\nEND".to_string())
            }
        }
    }

    /// Sends a PING command to verify connectivity.
    pub fn ping(&mut self) -> Result<String, String> {
        match &self.target {
            ClientTarget::Remote(addr) => Self::send_line_cmd(addr, "PING"),
            ClientTarget::Local(_) => Ok("PONG (local engine)".to_string()),
        }
    }

    /// Parses and executes a single command line string, returning a user-presentable string.
    pub fn execute_command(&mut self, line: &str) -> Result<String, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }

        let mut parts = trimmed.splitn(3, |c: char| c.is_ascii_whitespace());
        let cmd = parts.next().unwrap_or("").to_lowercase();

        match cmd.as_str() {
            "help" => Ok(Self::help_text()),
            "ping" => self.ping(),
            "put" => {
                let key = parts.next().ok_or_else(|| "Usage: put <key> <value>".to_string())?;
                let val = parts.next().ok_or_else(|| "Usage: put <key> <value>".to_string())?;
                self.put(key, val)
            }
            "get" => {
                let key = parts.next().ok_or_else(|| "Usage: get <key>".to_string())?;
                match self.get(key)? {
                    Some(val) => Ok(format!("VALUE {}", val)),
                    None => Ok("(not found)".to_string()),
                }
            }
            "del" => {
                let key = parts.next().ok_or_else(|| "Usage: del <key>".to_string())?;
                self.del(key)?;
                Ok("OK".to_string())
            }
            "scan" => {
                let start_arg = parts.next();
                let limit_arg = parts.next();
                let start = match start_arg {
                    Some(s) if !s.is_empty() && s != "\"\"" && s != "*" => Some(s),
                    _ => None,
                };
                let limit = limit_arg.and_then(|l| l.parse::<usize>().ok()).unwrap_or(20);
                let entries = self.scan(start, limit)?;
                if entries.is_empty() {
                    Ok("(empty range)".to_string())
                } else {
                    let mut out = format!("Found {} entries:\n", entries.len());
                    for (k, v) in entries {
                        out.push_str(&format!("  {} => {}\n", k, v));
                    }
                    Ok(out.trim_end().to_string())
                }
            }
            "stats" => self.stats(),
            "bench" => {
                let ops_arg = parts.next();
                let threads_arg = parts.next();
                let ops = ops_arg.and_then(|o| o.parse::<usize>().ok()).unwrap_or(10_000);
                let threads = threads_arg.and_then(|t| t.parse::<usize>().ok()).unwrap_or(4);
                let report = self.run_benchmark(ops, threads);
                Ok(report.to_string())
            }
            "exit" | "quit" => Ok("BYE".to_string()),
            _ => Err(format!(
                "Unknown command: '{}'. Type 'help' for available commands.",
                cmd
            )),
        }
    }

    /// Interactive REPL loop reading from `reader` and writing to `writer`.
    pub fn run_repl<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> Result<(), String> {
        let desc = self.target_description();
        writeln!(writer, "Connected to {}.", desc).map_err(|e| e.to_string())?;
        writeln!(writer, "Type 'help' for commands or 'exit' to quit.\n").map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;

        let prompt = match &self.target {
            ClientTarget::Remote(_) => "turing [remote]> ",
            ClientTarget::Local(_) => "turing [local]> ",
        };

        let mut line = String::new();
        loop {
            write!(writer, "{}", prompt).map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;

            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
                        writeln!(writer, "Goodbye!").map_err(|e| e.to_string())?;
                        writer.flush().map_err(|e| e.to_string())?;
                        break;
                    }

                    match self.execute_command(trimmed) {
                        Ok(output) => {
                            if !output.is_empty() {
                                writeln!(writer, "{}", output).map_err(|e| e.to_string())?;
                            }
                        }
                        Err(err) => {
                            writeln!(writer, "Error: {}", err).map_err(|e| e.to_string())?;
                        }
                    }
                    writer.flush().map_err(|e| e.to_string())?;
                }
                Err(e) => return Err(format!("Input error: {}", e)),
            }
        }

        Ok(())
    }

    /// Launches interactive REPL on standard input and output.
    pub fn run_interactive(&mut self) -> Result<(), String> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        self.run_repl(stdin.lock(), stdout.lock())
    }

    /// Runs multi-threaded benchmark across the configured target.
    pub fn run_benchmark(&mut self, total_ops: usize, num_threads: usize) -> BenchmarkReport {
        let threads = num_threads.max(1);
        let ops = total_ops.max(threads);
        let target_desc = self.target_description();

        let bench_start = Instant::now();
        let mut phases = Vec::new();

        // Phase 1: Concurrent PUT
        let put_ops = ops / 2;
        let p1 = self.bench_phase("Concurrent PUT (Writes)", put_ops, threads, PhaseKind::Put);
        phases.push(p1);

        // Phase 2: Concurrent GET
        let get_ops = ops / 2;
        let p2 = self.bench_phase("Concurrent GET (Point Reads)", get_ops, threads, PhaseKind::Get);
        phases.push(p2);

        // Phase 3: Mixed Workload (70% GET, 20% PUT, 10% DEL)
        let mixed_ops = ops / 2;
        let p3 = self.bench_phase(
            "Concurrent Mixed (70% Read / 20% Write / 10% Delete)",
            mixed_ops,
            threads,
            PhaseKind::Mixed,
        );
        phases.push(p3);

        let total_duration = bench_start.elapsed();
        let total_executed_ops: usize = phases.iter().map(|p| p.ops).sum();
        let overall_ops_per_second = if total_duration.as_secs_f64() > 0.0 {
            total_executed_ops as f64 / total_duration.as_secs_f64()
        } else {
            0.0
        };

        BenchmarkReport {
            target: target_desc,
            total_ops: total_executed_ops,
            num_threads: threads,
            total_duration,
            overall_ops_per_second,
            phases,
        }
    }

    fn bench_phase(
        &self,
        name: &str,
        phase_ops: usize,
        num_threads: usize,
        kind: PhaseKind,
    ) -> PhaseResult {
        let ops_per_thread = phase_ops / num_threads;
        let remainder = phase_ops % num_threads;
        let mut handles = Vec::with_capacity(num_threads);

        let phase_start = Instant::now();

        for tid in 0..num_threads {
            let thread_ops = ops_per_thread + if tid == 0 { remainder } else { 0 };
            let target = self.target.clone();
            let k = kind;

            handles.push(thread::spawn(move || {
                let mut latencies_us = Vec::with_capacity(thread_ops);
                let mut errors = 0usize;

                match target {
                    ClientTarget::Remote(addr) => {
                        let stream_res = TcpStream::connect(&addr);
                        if let Ok(mut stream) = stream_res {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                            let mut reader = BufReader::new(stream.try_clone().unwrap());

                            for i in 0..thread_ops {
                                let key_idx = tid * 100_000 + i;
                                let cmd = match k {
                                    PhaseKind::Put => {
                                        format!("PUT key_{} value_{}\r\n", key_idx, key_idx)
                                    }
                                    PhaseKind::Get => {
                                        format!("GET key_{}\r\n", key_idx)
                                    }
                                    PhaseKind::Mixed => {
                                        let mod10 = i % 10;
                                        if mod10 < 7 {
                                            format!("GET key_{}\r\n", key_idx)
                                        } else if mod10 < 9 {
                                            format!("PUT key_{} val_up_{}\r\n", key_idx, key_idx)
                                        } else {
                                            format!("DEL key_{}\r\n", key_idx)
                                        }
                                    }
                                };

                                let t0 = Instant::now();
                                if stream.write_all(cmd.as_bytes()).is_ok()
                                    && stream.flush().is_ok()
                                {
                                    let mut resp_line = String::new();
                                    if reader.read_line(&mut resp_line).is_ok() {
                                        let elapsed_us = t0.elapsed().as_micros() as u64;
                                        latencies_us.push(elapsed_us);
                                    } else {
                                        errors += 1;
                                    }
                                } else {
                                    errors += 1;
                                }
                            }
                        } else {
                            errors += thread_ops;
                        }
                    }
                    ClientTarget::Local(engine) => {
                        for i in 0..thread_ops {
                            let key = format!("key_{}_{}", tid, i);
                            let t0 = Instant::now();
                            let mut eng = match engine.lock() {
                                Ok(guard) => guard,
                                Err(poisoned) => poisoned.into_inner(),
                            };

                            let success = match k {
                                PhaseKind::Put => {
                                    let val = format!("val_{}_{}", tid, i);
                                    eng.put(&key, val.as_bytes()).is_ok()
                                }
                                PhaseKind::Get => eng.get(&key).is_ok(),
                                PhaseKind::Mixed => {
                                    let mod10 = i % 10;
                                    if mod10 < 7 {
                                        eng.get(&key).is_ok()
                                    } else if mod10 < 9 {
                                        let val = format!("up_{}", i);
                                        eng.put(&key, val.as_bytes()).is_ok()
                                    } else {
                                        eng.delete(&key).is_ok()
                                    }
                                }
                            };

                            let elapsed_us = t0.elapsed().as_micros() as u64;
                            if success {
                                latencies_us.push(elapsed_us);
                            } else {
                                errors += 1;
                            }
                        }
                    }
                }

                (latencies_us, errors)
            }));
        }

        let mut all_latencies = Vec::with_capacity(phase_ops);
        let mut total_errors = 0usize;

        for h in handles {
            if let Ok((lats, errs)) = h.join() {
                all_latencies.extend(lats);
                total_errors += errs;
            }
        }

        let duration = phase_start.elapsed();
        let ops_completed = all_latencies.len();

        let ops_per_second = if duration.as_secs_f64() > 0.0 {
            ops_completed as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        all_latencies.sort_unstable();

        let count = all_latencies.len();
        let (p50, p95, p99, min_lat, max_lat, avg_lat) = if count > 0 {
            let p50 = all_latencies[count * 50 / 100];
            let p95 = all_latencies[(count * 95 / 100).min(count - 1)];
            let p99 = all_latencies[(count * 99 / 100).min(count - 1)];
            let min_lat = all_latencies[0];
            let max_lat = all_latencies[count - 1];
            let sum: u64 = all_latencies.iter().sum();
            let avg_lat = sum as f64 / count as f64;
            (p50, p95, p99, min_lat, max_lat, avg_lat)
        } else {
            (0, 0, 0, 0, 0, 0.0)
        };

        PhaseResult {
            name: name.to_string(),
            ops: phase_ops,
            duration,
            ops_per_second,
            avg_latency_us: avg_lat,
            p50_latency_us: p50,
            p95_latency_us: p95,
            p99_latency_us: p99,
            min_latency_us: min_lat,
            max_latency_us: max_lat,
            errors: total_errors,
        }
    }

    fn send_line_cmd(addr: &str, cmd: &str) -> Result<String, String> {
        let mut stream = TcpStream::connect(addr)
            .map_err(|e| format!("Failed to connect to server at {}: {}", addr, e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;

        let wire_cmd = format!("{}\r\n", cmd.trim());
        stream
            .write_all(wire_cmd.as_bytes())
            .map_err(|e| format!("Failed to send command: {}", e))?;
        stream
            .flush()
            .map_err(|e| format!("Failed to flush: {}", e))?;

        let mut reader = BufReader::new(stream);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let trimmed = first_line.trim_end_matches(|c| c == '\r' || c == '\n');

        if trimmed.starts_with("ENTRIES") || trimmed.starts_with("STAT") {
            let mut full = format!("{}\n", trimmed);
            loop {
                let mut next_line = String::new();
                match reader.read_line(&mut next_line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let t = next_line.trim_end_matches(|c| c == '\r' || c == '\n');
                        full.push_str(t);
                        full.push('\n');
                        if t == "END" {
                            break;
                        }
                    }
                    Err(e) => return Err(format!("Error reading multiline response: {}", e)),
                }
            }
            Ok(full.trim_end().to_string())
        } else {
            Ok(trimmed.to_string())
        }
    }

    fn help_text() -> String {
        r#"Available Commands:
  put <key> <value>            Insert or update a key-value pair
  get <key>                    Retrieve the value of a key
  del <key>                    Delete a key (write tombstone)
  scan <start> [limit]         Scan keys starting from <start> up to [limit] (default: 20)
  bench [ops] [threads]        Run multi-threaded benchmark suite (default: 10000 ops, 4 threads)
  stats                        Display server / storage engine performance stats
  ping                         Check server connectivity
  help                         Display this help message
  exit / quit                  Exit the interactive REPL"#
            .to_string()
    }
}

#[derive(Copy, Clone)]
enum PhaseKind {
    Put,
    Get,
    Mixed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CLI_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            let count = CLI_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "turing_cli_test_{}_{}_{}",
                prefix,
                std::process::id(),
                count
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_cli_local_crud_and_scan() {
        let dir = TempDir::new("local_crud");
        let engine = LsmStorageEngine::open(dir.0.as_path()).unwrap();
        let mut client = CliClient::new_local(engine);

        // Put
        assert_eq!(client.put("k1", "v1").unwrap(), "OK");
        assert_eq!(client.put("k2", "v2").unwrap(), "OK");

        // Get
        assert_eq!(client.get("k1").unwrap(), Some("v1".to_string()));
        assert_eq!(client.get("k2").unwrap(), Some("v2".to_string()));
        assert_eq!(client.get("k3").unwrap(), None);

        // Scan
        let entries = client.scan(Some("k"), 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ("k1".to_string(), "v1".to_string()));
        assert_eq!(entries[1], ("k2".to_string(), "v2".to_string()));

        // Delete
        assert!(client.del("k1").unwrap());
        assert_eq!(client.get("k1").unwrap(), None);
    }

    #[test]
    fn test_cli_command_parser_local() {
        let dir = TempDir::new("parser");
        let engine = LsmStorageEngine::open(dir.0.as_path()).unwrap();
        let mut client = CliClient::new_local(engine);

        assert_eq!(client.execute_command("put foo bar").unwrap(), "OK");
        assert_eq!(client.execute_command("get foo").unwrap(), "VALUE bar");
        assert_eq!(client.execute_command("get missing").unwrap(), "(not found)");
        assert_eq!(client.execute_command("del foo").unwrap(), "OK");
        assert_eq!(client.execute_command("get foo").unwrap(), "(not found)");
        assert!(client.execute_command("help").unwrap().contains("Available Commands"));
        assert!(client.execute_command("stats").unwrap().contains("local_lsm"));
        assert!(client.execute_command("ping").unwrap().contains("PONG"));
    }

    #[test]
    fn test_cli_local_benchmark() {
        let dir = TempDir::new("bench");
        let engine = LsmStorageEngine::open(dir.0.as_path()).unwrap();
        let mut client = CliClient::new_local(engine);

        let report = client.run_benchmark(600, 3);
        assert_eq!(report.num_threads, 3);
        assert!(report.total_ops >= 600);
        assert!(report.overall_ops_per_second > 0.0);
        assert_eq!(report.phases.len(), 3);
        for p in &report.phases {
            assert!(p.ops_per_second > 0.0);
            assert_eq!(p.errors, 0);
        }
        let report_str = report.to_string();
        assert!(report_str.contains("TURING HIGH-PERFORMANCE BENCHMARK SUITE"));
        assert!(report_str.contains("Concurrent PUT"));
        assert!(report_str.contains("Concurrent GET"));
    }
}
