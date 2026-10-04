use crate::subagents::{BenchmarkConfig, SubagentPool, WorkloadKind};
use crate::sysinfo::SystemInfo;
use std::env;
use std::path::PathBuf;

pub struct Builtins;

impl Builtins {
    pub fn is_builtin(cmd: &str) -> bool {
        matches!(
            cmd,
            "cd" | "pwd"
                | "echo"
                | "export"
                | "env"
                | "sysinfo"
                | "device"
                | "bench"
                | "agents"
                | "history"
                | "clear"
                | "help"
                | "exit"
                | "quit"
        )
    }

    pub fn execute(
        cmd: &str,
        args: &[String],
        history: &[String],
        last_dir: &mut Option<PathBuf>,
    ) -> Result<i32, String> {
        match cmd {
            "cd" => Self::cd(args, last_dir),
            "pwd" => Self::pwd(),
            "echo" => Self::echo(args),
            "export" => Self::export(args),
            "env" => Self::env_cmd(),
            "sysinfo" | "device" => Self::sysinfo(),
            "bench" => Self::bench(args),
            "agents" => Self::agents(args),
            "history" => Self::history(history),
            "clear" => Self::clear(),
            "help" => Self::help(),
            "exit" | "quit" => Ok(0),
            _ => Err(format!("Unknown builtin: {}", cmd)),
        }
    }

    fn cd(args: &[String], last_dir: &mut Option<PathBuf>) -> Result<i32, String> {
        let current = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let target = if args.is_empty() {
            env::var("HOME").unwrap_or_else(|_| "/root".to_string())
        } else if args[0] == "-" {
            if let Some(prev) = last_dir.as_ref() {
                println!("{}", prev.display());
                prev.to_string_lossy().to_string()
            } else {
                return Err("cd: OLDPWD not set".to_string());
            }
        } else if args[0].starts_with('~') {
            let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            args[0].replacen('~', &home, 1)
        } else {
            args[0].clone()
        };

        let target_path = PathBuf::from(&target);
        if let Err(e) = env::set_current_dir(&target_path) {
            eprintln!("cd: {}: {}", target, e);
            Ok(1)
        } else {
            *last_dir = Some(current);
            Ok(0)
        }
    }

    fn pwd() -> Result<i32, String> {
        match env::current_dir() {
            Ok(path) => {
                println!("{}", path.display());
                Ok(0)
            }
            Err(e) => {
                eprintln!("pwd: {}", e);
                Ok(1)
            }
        }
    }

    fn echo(args: &[String]) -> Result<i32, String> {
        let output: Vec<String> = args
            .iter()
            .map(|arg| {
                if arg.starts_with('$') && arg.len() > 1 {
                    let var_name = &arg[1..];
                    env::var(var_name).unwrap_or_default()
                } else {
                    arg.clone()
                }
            })
            .collect();
        println!("{}", output.join(" "));
        Ok(0)
    }

    fn export(args: &[String]) -> Result<i32, String> {
        if args.is_empty() {
            return Self::env_cmd();
        }
        for arg in args {
            if let Some((k, v)) = arg.split_once('=') {
                env::set_var(k, v);
            } else {
                env::set_var(arg, "");
            }
        }
        Ok(0)
    }

    fn env_cmd() -> Result<i32, String> {
        for (k, v) in env::vars() {
            println!("{}={}", k, v);
        }
        Ok(0)
    }

    fn sysinfo() -> Result<i32, String> {
        let info = SystemInfo::collect();
        info.display_summary();
        Ok(0)
    }

    fn bench(args: &[String]) -> Result<i32, String> {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let mut num_agents = cores;
        let mut duration_secs = 3;
        let mut scaling = false;
        let mut workload = WorkloadKind::Compute;
        let mut run_all = false;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--all" | "-a" => run_all = true,
                "--compute" | "-c" => workload = WorkloadKind::Compute,
                "--memory" | "-m" => workload = WorkloadKind::Memory,
                "--ipc" => workload = WorkloadKind::Ipc,
                "--io" => workload = WorkloadKind::Io,
                "--scaling" | "-s" => scaling = true,
                "--agents" | "-n" => {
                    if i + 1 < args.len() {
                        if let Ok(val) = args[i + 1].parse::<usize>() {
                            num_agents = val.max(1);
                        }
                        i += 1;
                    }
                }
                "--duration" | "-d" => {
                    if i + 1 < args.len() {
                        if let Ok(val) = args[i + 1].parse::<u64>() {
                            duration_secs = val.max(1);
                        }
                        i += 1;
                    }
                }
                "--help" | "-h" => {
                    println!("\x1b[1mUsage: bench [OPTIONS]\x1b[0m");
                    println!("Options:");
                    println!("  --all, -a            Run full performance check suite across all subagents");
                    println!("  --compute, -c        Run compute ALU / SHA-256 stress benchmark");
                    println!("  --memory, -m         Run memory sequential & random bandwidth test");
                    println!("  --ipc                Run IPC channel messaging and lock contention test");
                    println!("  --io                 Run storage and workspace I/O test");
                    println!("  --agents, -n <N>     Number of concurrent subagents (default: {})", cores);
                    println!("  --duration, -d <S>   Duration in seconds per test phase (default: 3)");
                    println!("  --scaling, -s        Run multi-core scaling efficiency analysis");
                    return Ok(0);
                }
                _ => {}
            }
            i += 1;
        }

        if scaling {
            SubagentPool::run_scaling_analysis(workload, num_agents, duration_secs);
            return Ok(0);
        }

        if run_all {
            println!("\x1b[1;36m========================================================================\x1b[0m");
            println!("\x1b[1;32m      RUNNING COMPREHENSIVE MULTI-SUBAGENT PERFORMANCE BENCHMARK        \x1b[0m");
            println!("\x1b[1;36m========================================================================\x1b[0m");
            let workloads = [
                WorkloadKind::Compute,
                WorkloadKind::Memory,
                WorkloadKind::Ipc,
                WorkloadKind::Io,
            ];

            let mut all_reports = Vec::new();

            for w in &workloads {
                let config = BenchmarkConfig {
                    num_subagents: num_agents,
                    duration_secs,
                    workload: *w,
                    verbose: true,
                };
                let report = SubagentPool::run_benchmark(&config);
                all_reports.push(report);
            }

            println!("\x1b[1;32m========================================================================\x1b[0m");
            println!("\x1b[1;32m                 BENCHMARK PERFORMANCE SUMMARY                          \x1b[0m");
            println!("\x1b[1;32m========================================================================\x1b[0m");
            for r in &all_reports {
                println!(
                    "  • \x1b[1m{:<25}\x1b[0m: {:>12.0} aggregate ops/s | {}",
                    r.workload_name.split('(').next().unwrap_or(&r.workload_name).trim(),
                    r.aggregate_ops_per_sec,
                    r.primary_summary
                );
            }
            println!("\x1b[1;32m========================================================================\x1b[0m");
            return Ok(0);
        }

        let config = BenchmarkConfig {
            num_subagents: num_agents,
            duration_secs,
            workload,
            verbose: true,
        };
        SubagentPool::run_benchmark(&config);

        Ok(0)
    }

    fn agents(args: &[String]) -> Result<i32, String> {
        let subcmd = args.get(0).map(|s| s.as_str()).unwrap_or("list");
        match subcmd {
            "list" => {
                println!("\x1b[1;36mAvailable Subagent Workloads in Rust:\x1b[0m");
                println!("  1. \x1b[1;32mcompute\x1b[0m: 64-bit prime search, SHA-256 compression rounds, float matrices");
                println!("  2. \x1b[1;32mmemory\x1b[0m:  Sequential read/write RAM bandwidth, pointer-chasing latency");
                println!("  3. \x1b[1;32mipc\x1b[0m:     Cross-thread MPSC actor channels, contended atomics & locks");
                println!("  4. \x1b[1;32mio\x1b[0m:      Workspace file chunk writes/reads, flush & IOPS");
                println!();
                println!("Use `bench --<workload> --agents <N>` to run benchmarks with subagents.");
                Ok(0)
            }
            "run" => {
                let workload_name = args.get(1).map(|s| s.as_str()).unwrap_or("compute");
                let remaining = if args.len() > 2 { &args[2..] } else { &[] };
                let mut bench_args = vec![format!("--{}", workload_name)];
                bench_args.extend_from_slice(remaining);
                Self::bench(&bench_args)
            }
            _ => {
                println!("Usage: agents [list | run <workload> [options]]");
                Ok(0)
            }
        }
    }

    fn history(history: &[String]) -> Result<i32, String> {
        for (i, cmd) in history.iter().enumerate() {
            println!("{:>5}  {}", i + 1, cmd);
        }
        Ok(0)
    }

    fn clear() -> Result<i32, String> {
        print!("\x1b[2J\x1b[H");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        Ok(0)
    }

    fn help() -> Result<i32, String> {
        println!("\x1b[1;36m========================================================================\x1b[0m");
        println!("\x1b[1;32m                       NIMBLE-SHELL HELP & COMMANDS                     \x1b[0m");
        println!("\x1b[1;36m========================================================================\x1b[0m");
        println!("  \x1b[1mBuilt-in Commands:\x1b[0m");
        println!("    \x1b[33mcd [dir]\x1b[0m             Change directory (supports ~, -, relative, absolute)");
        println!("    \x1b[33mpwd\x1b[0m                  Print current working directory");
        println!("    \x1b[33mecho [args...]\x1b[0m       Print text and expand environment variables ($VAR)");
        println!("    \x1b[33mexport KEY=VAL\x1b[0m       Set environment variables");
        println!("    \x1b[33menv\x1b[0m                  List current environment variables");
        println!("    \x1b[33msysinfo\x1b[0m              Display device hardware specs, CPU cores, RAM, OS");
        println!("    \x1b[33mbench [options]\x1b[0m      Run multi-subagent performance benchmarks in Rust");
        println!("    \x1b[33magents [list|run]\x1b[0m    Manage and inspect subagent workloads");
        println!("    \x1b[33mhistory\x1b[0m              Show command history");
        println!("    \x1b[33mclear\x1b[0m                Clear terminal screen");
        println!("    \x1b[33mexit / quit\x1b[0m          Exit nimble-shell");
        println!();
        println!("  \x1b[1mShell Features:\x1b[0m");
        println!("    • Pipelines:         \x1b[36mcmd1 | cmd2 | cmd3\x1b[0m");
        println!("    • Output Redirect:   \x1b[36mcmd > file.txt\x1b[0m or \x1b[36mcmd >> file.txt\x1b[0m");
        println!("    • External Programs: Any system binary (\x1b[36mls, cat, grep, uname\x1b[0m...)");
        println!("    • Script Execution:  \x1b[36mnimble-shell -c \"<command>\"\x1b[0m or \x1b[36mnimble-shell script.nsh\x1b[0m");
        println!("\x1b[1;36m========================================================================\x1b[0m");
        Ok(0)
    }
}
