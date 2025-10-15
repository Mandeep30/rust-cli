use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{env, fs, process, process::Command};
enum PrimitiveCommand {
    Echo(String),
    Exit(i32),
    Unknown(String),
    Empty,
}

fn parse_command(line: &str) -> PrimitiveCommand {
    let line = line.trim();
    if line.is_empty() {
        return PrimitiveCommand::Empty;
    }

    if let Some(rest) = line.strip_prefix("exit") {
        let mut parts = rest.split_whitespace();
        if let Some(num_str) = parts.next() {
            if let Ok(code) = num_str.parse::<i32>() {
                return PrimitiveCommand::Exit(code);
            }
        }
        return PrimitiveCommand::Exit(0);
    }
    if let Some(rest) = line.strip_prefix("echo") {
        return PrimitiveCommand::Echo(split_quoted_line(rest.trim_start()).join(" "));
    }
    if let Some(rest) = line.strip_prefix("type") {
        let arg = rest.trim_start();
        let builtins = ["exit", "echo", "type", "pwd", "cd"];

        return if builtins.contains(&arg) {
            PrimitiveCommand::Echo(format!("{} is a shell builtin", arg))
        } else {
            match find_in_path(&arg) {
                Some(p) => PrimitiveCommand::Echo(format!("{} is {}", arg, p.display())),
                None => PrimitiveCommand::Echo(format!("{} not found", arg)),
            }
        };
    }
    if line == "pwd" {
        return PrimitiveCommand::Echo(env::current_dir().unwrap().display().to_string());
    }
    if let Some(rest) = line.strip_prefix("cd") {
        let target = expand_tilde(rest.trim_start());
        if let Err(_e) = env::set_current_dir(&target) {
            return PrimitiveCommand::Echo(format!(
                "cd: {}: No such file or directory",
                target.display()
            ));
        }
        return PrimitiveCommand::Empty;
    }
    //for executing command
    let quoted_split_lines = split_quoted_line(line);
    if quoted_split_lines.is_empty() {
        return PrimitiveCommand::Empty;
    }
    let cmd = quoted_split_lines.get(0).unwrap();
    match find_in_path(cmd) {
        Some(_p) => match Command::new(cmd).args(&quoted_split_lines[1..]).output() {
            Ok(out) if out.status.success() => {
                PrimitiveCommand::Echo(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            Ok(out) => {
                PrimitiveCommand::Echo(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
            Err(_) => PrimitiveCommand::Unknown(cmd.to_string()),
        },
        None => PrimitiveCommand::Unknown(cmd.to_string()),
    }
}

fn run_command(cmd: PrimitiveCommand) {
    match cmd {
        PrimitiveCommand::Exit(code) => process::exit(code),
        PrimitiveCommand::Echo(s) => println!("{}", s),
        PrimitiveCommand::Unknown(name) => println!("{}: command not found", name),
        PrimitiveCommand::Empty => {} // do nothing
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap();
    let directories = env::split_paths(&path);
    for dir in directories {
        let command_path = dir.join(name);
        #[cfg(unix)]
        if is_executable_unix(&command_path) {
            return Some(command_path);
        }

        #[cfg(windows)]
        if is_executable_windows(&command_path) {
            return Some(command_path);
        }
    }
    None
}
#[cfg(unix)]
fn is_executable_unix(p: &Path) -> bool {
    match fs::metadata(p) {
        Ok(md) => md.is_file() && (md.permissions().mode() & 0o111) != 0,
        Err(_) => false,
    }
}
#[cfg(windows)]
const ALLOWED_EXTENSIONS: [&str; 4] = ["exe", "com", "bat", "cmd"];

#[cfg(windows)]
fn is_regular_file(p: &Path) -> bool {
    match fs::metadata(p) {
        Ok(md) => md.is_file(),
        Err(_) => false,
    }
}

#[cfg(windows)]
fn lower_ext(p: &Path) -> Option<String> {
    match p.extension() {
        Some(os) => match os.to_str() {
            Some(s) => Some(s.to_ascii_lowercase()), // extensions are ASCII
            None => None,
        },
        None => None,
    }
}

#[cfg(windows)]
fn is_executable_windows(p: &Path) -> bool {
    //path already has an extension
    if let Some(ext) = lower_ext(p) {
        return ALLOWED_EXTENSIONS.contains(&ext.as_str()) && is_regular_file(p);
    }

    //no extension, try each allowed extension
    for ext in ALLOWED_EXTENSIONS {
        let path_buf: PathBuf = p.with_extension(ext);
        if is_regular_file(&path_buf) {
            return true;
        }
    }
    false
}
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~") {
        if let Ok(home) = env::var("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    PathBuf::from(p)
}
pub fn split_quoted_line(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();

    let mut in_single = false;
    let mut in_double = false;
    let mut esc = false; // backslash escape (context-sensitive)

    for ch in line.chars() {
        if in_double {
            // --- inside "double quotes" ---

            if esc {
                // ex: echo "he\(here)llo"        -> \"  => push '"'
                //     echo "path\\(here)tmp"     -> \\  => push '\'
                //     echo "x\y"                 -> \y  => push '\' and 'y'
                match ch {
                    '"' | '\\' => cur.push(ch), //e.g. echo "\\n" it will come here for initial \ and for rest,
                    // it will go through next match ch
                    other => {
                        cur.push('\\'); //it will come here for echo "\n" will push both
                        cur.push(other);
                    }
                }
                esc = false;
                continue;
            }

            match ch {
                '\\' => {
                    // ex: echo "a\(here)b"       -> start escape inside "
                    esc = true
                }
                '"' => {
                    // ex: echo "hello"(here)     -> end "
                    in_double = false
                }
                c => {
                    // ex: echo "he(re)llo world" -> take literally (spaces included)
                    cur.push(c)
                }
            }
            continue;
        }

        if in_single {
            // --- inside 'single quotes' ---

            match ch {
                '\'' => {
                    // ex: echo 'hello'(here)     -> end '
                    in_single = false
                }
                c => {
                    // ex: echo 'he(re)llo world' -> take literally (no escapes)
                    cur.push(c)
                }
            }
            continue;
        }

        // --- outside quotes (normal) ---

        if esc {
            // ex: echo a\(here) b               -> escape makes next char literal (incl. space)
            cur.push(ch);
            esc = false;
            continue;
        }

        match ch {
            '\'' => {
                // ex: echo '(here)hello'         -> start '
                in_single = true
            }
            '"' => {
                // ex: echo "(here)hello"         -> start "
                in_double = true
            }
            '\\' => {
                // ex: echo a\(here) b            -> begin escape (space/quote/etc. next)
                esc = true
            }
            c if c.is_ascii_whitespace() => {
                //split on whitespace (collapse runs)
                if !cur.is_empty() {
                    parts.push(std::mem::take(&mut cur));
                }
            }
            c => {
                // ex: echo he(re)llo             -> normal char outside quotes
                cur.push(c)
            }
        }
    }

    // trailing backslash outside quotes → keep it literally
    // ex: echo foo\                        -> becomes "foo\"
    if esc {
        cur.push('\\');
    }

    if !cur.is_empty() {
        parts.push(cur);
    }

    parts
}

fn main() {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        let cmd = parse_command(&input);
        run_command(cmd);
    }
}
