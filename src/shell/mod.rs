pub mod builtins;
pub mod executor;
pub mod parser;

use executor::Executor;
use parser::Parser;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub struct Shell {
    history: Vec<String>,
    last_dir: Option<PathBuf>,
    last_exit_code: i32,
    is_running: bool,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            last_dir: None,
            last_exit_code: 0,
            is_running: true,
        }
    }

    pub fn run_repl(&mut self) {
        Self::print_banner();

        let stdin = io::stdin();
        let mut reader = stdin.lock();

        while self.is_running {
            self.print_prompt();

            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // EOF / Ctrl+D
                    println!("\nExiting nimble-shell.");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    self.history.push(trimmed.to_string());

                    if trimmed == "exit" || trimmed == "quit" {
                        self.is_running = false;
                        break;
                    }

                    self.execute_line(trimmed);
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    break;
                }
            }
        }
    }

    pub fn execute_string(&mut self, input: &str) -> i32 {
        for line in input.lines() {
            for subcmd in line.split(';') {
                let trimmed = subcmd.trim();
                if !trimmed.is_empty() {
                    self.history.push(trimmed.to_string());
                    if trimmed == "exit" || trimmed == "quit" {
                        self.is_running = false;
                        return 0;
                    }
                    self.execute_line(trimmed);
                }
            }
        }
        self.last_exit_code
    }

    pub fn execute_file(&mut self, path: &Path) -> i32 {
        match fs::read_to_string(path) {
            Ok(content) => self.execute_string(&content),
            Err(e) => {
                eprintln!("nimble-shell: cannot read {}: {}", path.display(), e);
                1
            }
        }
    }

    fn execute_line(&mut self, line: &str) {
        if let Some(pipeline) = Parser::parse(line) {
            // Check for exit builtin
            if pipeline.commands.len() == 1 {
                if let Some(first_arg) = pipeline.commands[0].args.first() {
                    if first_arg == "exit" || first_arg == "quit" {
                        self.is_running = false;
                        self.last_exit_code = 0;
                        return;
                    }
                }
            }

            self.last_exit_code = Executor::execute(pipeline, &self.history, &mut self.last_dir);
        }
    }

    fn print_prompt(&self) {
        let cwd = env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let status_color = if self.last_exit_code == 0 {
            "\x1b[32m"
        } else {
            "\x1b[31m"
        };

        print!(
            "\x1b[1;36mnimble-shell\x1b[0m:[\x1b[1;33m{}\x1b[0m] {}${}\x1b[0m ",
            cwd, status_color, "\x1b[0m"
        );
        let _ = io::stdout().flush();
    }

    fn print_banner() {
        println!("\x1b[1;36m┌────────────────────────────────────────────────────────────────────────┐\x1b[0m");
        println!("\x1b[1;36m│\x1b[0m \x1b[1;32m  NIMBLE-SHELL v0.1.0 (Rust) - Multi-Subagent Performance Environment  \x1b[0m\x1b[1;36m│\x1b[0m");
        println!("\x1b[1;36m├────────────────────────────────────────────────────────────────────────┤\x1b[0m");
        println!("│ Built-ins: \x1b[33mcd, pwd, echo, export, env, sysinfo, bench, agents, help, exit\x1b[0m │");
        println!("│ Try:       \x1b[1;32mbench --all\x1b[0m  to run the complete multi-subagent benchmark    │");
        println!("│            \x1b[1;32mbench --scaling\x1b[0m to test multi-core scaling efficiency        │");
        println!("│            \x1b[1;32msysinfo\x1b[0m      to inspect hardware & CPU architecture          │");
        println!("\x1b[1;36m└────────────────────────────────────────────────────────────────────────┘\x1b[0m");
        println!();
    }
}
