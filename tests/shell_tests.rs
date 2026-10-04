use nimble_shell::shell::builtins::Builtins;
use nimble_shell::shell::parser::{Parser, Redirection};
use nimble_shell::shell::Shell;
use nimble_shell::subagents::compute::ComputeSubagent;
use nimble_shell::subagents::io::IoSubagent;
use nimble_shell::subagents::ipc::IpcSubagent;
use nimble_shell::subagents::memory::MemorySubagent;
use nimble_shell::subagents::{BenchmarkConfig, SubagentPool, WorkloadKind};
use nimble_shell::sysinfo::SystemInfo;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn test_sysinfo_collection() {
    let info = SystemInfo::collect();
    assert!(info.cpu_cores >= 1);
    assert!(!info.arch.is_empty());
    assert!(!info.os_name.is_empty());
    println!("Detected system: {} on {} ({} cores)", info.os_name, info.arch, info.cpu_cores);
}

#[test]
fn test_parser_tokenization() {
    let tokens = Parser::tokenize("echo 'hello world' \"rust shell\" arg3\\ with\\ space");
    assert_eq!(
        tokens,
        vec!["echo", "hello world", "rust shell", "arg3 with space"]
    );
}

#[test]
fn test_parser_pipelines_and_redirection() {
    let pipeline = Parser::parse("sysinfo | grep CPU > /workspace/nimble-turing/output.txt").unwrap();
    assert_eq!(pipeline.commands.len(), 2);
    assert_eq!(pipeline.commands[0].args, vec!["sysinfo"]);
    assert_eq!(pipeline.commands[1].args, vec!["grep", "CPU"]);
    assert_eq!(
        pipeline.redirect,
        Redirection::Overwrite("/workspace/nimble-turing/output.txt".to_string())
    );
    assert!(!pipeline.background);

    let bg_pipeline = Parser::parse("bench --compute &").unwrap();
    assert!(bg_pipeline.background);
}

#[test]
fn test_builtins_is_builtin() {
    assert!(Builtins::is_builtin("cd"));
    assert!(Builtins::is_builtin("pwd"));
    assert!(Builtins::is_builtin("echo"));
    assert!(Builtins::is_builtin("bench"));
    assert!(Builtins::is_builtin("sysinfo"));
    assert!(Builtins::is_builtin("agents"));
    assert!(!Builtins::is_builtin("nonexistent_command"));
}

#[test]
fn test_compute_subagent_standalone() {
    let stop = Arc::new(AtomicBool::new(false));
    let metrics = ComputeSubagent::run(0, Duration::from_millis(300), stop);
    assert!(metrics.ops_completed > 0);
    assert!(metrics.primary_metric_val > 0.0);
    println!(
        "Compute Subagent test: {} ops, {:.2} MHashes/s",
        metrics.ops_completed, metrics.primary_metric_val
    );
}

#[test]
fn test_memory_subagent_standalone() {
    let stop = Arc::new(AtomicBool::new(false));
    let metrics = MemorySubagent::run(0, Duration::from_millis(300), stop);
    assert!(metrics.ops_completed > 0);
    assert!(metrics.primary_metric_val > 0.0);
    println!(
        "Memory Subagent test: {} ops, {:.2} GB/s",
        metrics.ops_completed, metrics.primary_metric_val
    );
}

#[test]
fn test_ipc_subagent_standalone() {
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::new(AtomicU64::new(0));
    let mutex = Arc::new(Mutex::new(Vec::new()));
    let metrics = IpcSubagent::run(0, Duration::from_millis(300), stop, counter, mutex);
    assert!(metrics.ops_completed > 0);
    assert!(metrics.primary_metric_val > 0.0);
    println!(
        "IPC Subagent test: {} ops, {:.0} msg/s",
        metrics.ops_completed, metrics.primary_metric_val
    );
}

#[test]
fn test_io_subagent_standalone() {
    let stop = Arc::new(AtomicBool::new(false));
    let metrics = IoSubagent::run(0, Duration::from_millis(300), stop);
    assert!(metrics.ops_completed > 0);
    println!(
        "IO Subagent test: {} IOPS, {:.2} MB/s",
        metrics.ops_completed, metrics.primary_metric_val
    );
}

#[test]
fn test_subagent_pool_orchestration() {
    let config = BenchmarkConfig {
        num_subagents: 2,
        duration_secs: 1,
        workload: WorkloadKind::Compute,
        verbose: false,
    };
    let report = SubagentPool::run_benchmark(&config);
    assert_eq!(report.num_subagents, 2);
    assert_eq!(report.per_agent_metrics.len(), 2);
    assert!(report.total_ops > 0);
    assert!(report.aggregate_ops_per_sec > 0.0);
}

#[test]
fn test_shell_execute_string() {
    let mut shell = Shell::new();
    let code = shell.execute_string("echo 'Testing nimble shell'; pwd; export FOO=BAR");
    assert_eq!(code, 0);
    assert_eq!(std::env::var("FOO").unwrap(), "BAR");
}
