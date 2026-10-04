use super::builtins::Builtins;
use super::parser::{Pipeline, Redirection};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command as SysCommand, Stdio};

pub struct Executor;

impl Executor {
    pub fn execute(
        pipeline: Pipeline,
        history: &[String],
        last_dir: &mut Option<PathBuf>,
    ) -> i32 {
        if pipeline.commands.is_empty() {
            return 0;
        }

        // Single builtin command with no pipe
        if pipeline.commands.len() == 1 {
            let cmd = &pipeline.commands[0];
            let name = &cmd.args[0];
            if Builtins::is_builtin(name) {
                if pipeline.redirect == Redirection::None {
                    return match Builtins::execute(name, &cmd.args[1..], history, last_dir) {
                        Ok(code) => code,
                        Err(e) => {
                            eprintln!("nimble-shell: {}", e);
                            1
                        }
                    };
                }
            }
        }

        // Execute pipeline (one or more commands)
        Self::execute_pipeline(pipeline, history, last_dir)
    }

    fn execute_pipeline(
        pipeline: Pipeline,
        _history: &[String],
        _last_dir: &mut Option<PathBuf>,
    ) -> i32 {
        let mut previous_child: Option<Child> = None;
        let mut children = Vec::new();
        let total_commands = pipeline.commands.len();

        for (idx, cmd) in pipeline.commands.into_iter().enumerate() {
            if cmd.args.is_empty() {
                continue;
            }
            let is_last = idx == total_commands - 1;
            let prog_name = &cmd.args[0];
            let prog_args = &cmd.args[1..];

            // If it's a builtin within a pipeline, we execute it in a subshell or handle output
            if Builtins::is_builtin(prog_name) {
                let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nimble-shell"));
                let mut sys_cmd = SysCommand::new(current_exe);
                sys_cmd.arg("-c");
                let full_cmd_line = format!("{} {}", prog_name, prog_args.join(" "));
                sys_cmd.arg(full_cmd_line);

                Self::configure_stdio(&mut sys_cmd, &mut previous_child, is_last, &pipeline.redirect);

                match sys_cmd.spawn() {
                    Ok(child) => {
                        if !is_last {
                            previous_child = Some(child);
                        } else {
                            children.push(child);
                        }
                    }
                    Err(e) => {
                        eprintln!("nimble-shell: failed to spawn {}: {}", prog_name, e);
                        return 1;
                    }
                }
            } else {
                let mut sys_cmd = SysCommand::new(prog_name);
                sys_cmd.args(prog_args);

                Self::configure_stdio(&mut sys_cmd, &mut previous_child, is_last, &pipeline.redirect);

                match sys_cmd.spawn() {
                    Ok(child) => {
                        if !is_last {
                            previous_child = Some(child);
                        } else {
                            children.push(child);
                        }
                    }
                    Err(e) => {
                        eprintln!("nimble-shell: command not found: {} ({})", prog_name, e);
                        return 127;
                    }
                }
            }
        }

        if pipeline.background {
            if let Some(child) = children.last() {
                println!("[Background PID: {}]", child.id());
            }
            return 0;
        }

        // Wait for pipeline children to complete
        let mut final_status = 0;
        for mut child in children {
            if let Ok(status) = child.wait() {
                final_status = status.code().unwrap_or(0);
            }
        }

        final_status
    }

    fn configure_stdio(
        cmd: &mut SysCommand,
        previous_child: &mut Option<Child>,
        is_last: bool,
        redirect: &Redirection,
    ) {
        // Set stdin from previous pipeline stage
        if let Some(mut prev) = previous_child.take() {
            if let Some(stdout) = prev.stdout.take() {
                cmd.stdin(Stdio::from(stdout));
            }
        } else {
            cmd.stdin(Stdio::inherit());
        }

        // Set stdout for next pipeline stage or redirection
        if !is_last {
            cmd.stdout(Stdio::piped());
        } else {
            match redirect {
                Redirection::Overwrite(file) => {
                    if let Ok(f) = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(file)
                    {
                        cmd.stdout(Stdio::from(f));
                    } else {
                        eprintln!("nimble-shell: cannot open {} for writing", file);
                        cmd.stdout(Stdio::inherit());
                    }
                }
                Redirection::Append(file) => {
                    if let Ok(f) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(file)
                    {
                        cmd.stdout(Stdio::from(f));
                    } else {
                        eprintln!("nimble-shell: cannot open {} for appending", file);
                        cmd.stdout(Stdio::inherit());
                    }
                }
                Redirection::None => {
                    cmd.stdout(Stdio::inherit());
                }
            }
        }
        cmd.stderr(Stdio::inherit());
    }
}
