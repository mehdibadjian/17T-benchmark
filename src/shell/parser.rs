#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    None,
    Overwrite(String),
    Append(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub commands: Vec<Command>,
    pub redirect: Redirection,
    pub background: bool,
}

pub struct Parser;

impl Parser {
    pub fn parse(input: &str) -> Option<Pipeline> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let mut background = false;
        let mut work_str = trimmed;
        if work_str.ends_with('&') {
            background = true;
            work_str = work_str[..work_str.len() - 1].trim();
        }

        // Split by redirection
        let (command_part, redirect) = Self::extract_redirection(work_str);

        // Split pipeline commands by '|'
        let pipe_parts = Self::split_pipes(&command_part);
        let mut commands = Vec::new();

        for part in pipe_parts {
            let tokens = Self::tokenize(&part);
            if !tokens.is_empty() {
                commands.push(Command { args: tokens });
            }
        }

        if commands.is_empty() {
            None
        } else {
            Some(Pipeline {
                commands,
                redirect,
                background,
            })
        }
    }

    fn extract_redirection(input: &str) -> (String, Redirection) {
        if let Some(pos) = input.find(">>") {
            let cmd = input[..pos].trim().to_string();
            let file = input[pos + 2..].trim().to_string();
            (cmd, Redirection::Append(file))
        } else if let Some(pos) = input.find('>') {
            let cmd = input[..pos].trim().to_string();
            let file = input[pos + 1..].trim().to_string();
            (cmd, Redirection::Overwrite(file))
        } else {
            (input.to_string(), Redirection::None)
        }
    }

    fn split_pipes(input: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        for ch in input.chars() {
            match ch {
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                    current.push(ch);
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                    current.push(ch);
                }
                '|' if !in_single_quote && !in_double_quote => {
                    parts.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(ch),
            }
        }
        if !current.trim().is_empty() {
            parts.push(current.trim().to_string());
        }
        parts
    }

    pub fn tokenize(input: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut chars = input.chars().peekable();
        let mut in_single = false;
        let mut in_double = false;

        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    if let Some(next_ch) = chars.next() {
                        current.push(next_ch);
                    }
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                }
                '"' if !in_single => {
                    in_double = !in_double;
                }
                c if c.is_whitespace() && !in_single && !in_double => {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                }
                _ => {
                    current.push(ch);
                }
            }
        }

        if !current.is_empty() {
            tokens.push(current);
        }

        tokens
    }
}
