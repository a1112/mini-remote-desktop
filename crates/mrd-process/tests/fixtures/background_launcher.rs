#![windows_subsystem = "windows"]

use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Output, Stdio},
};

#[path = "../../src/lib.rs"]
mod mrd_process;

fn forward(output: Output) -> ! {
    std::io::stdout().write_all(&output.stdout).unwrap();
    std::io::stderr().write_all(&output.stderr).unwrap();
    std::io::stdout().flush().unwrap();
    std::io::stderr().flush().unwrap();
    std::process::exit(output.status.code().unwrap_or(1));
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let child_executable = args.next().unwrap();
    let mode = args.next().unwrap().into_string().unwrap();
    let mut command = mrd_process::background_command(child_executable);
    command
        .args(["--exact", "child_process_fixture", "--nocapture"])
        .env("MRD_BACKGROUND_PROCESS_TEST_CHILD", &mode);
    match mode.as_str() {
        "missing" => {
            let program = std::env::temp_dir().join(format!(
                "mrd-background-absent-executable-{}.exe",
                std::process::id()
            ));
            let error = mrd_process::background_command(program)
                .output()
                .unwrap_err();
            println!("error={:?}", error.kind());
        }
        "output" => forward(command.output().unwrap()),
        "echo" => {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            child.stdin.take().unwrap().write_all(&input).unwrap();
            forward(child.wait_with_output().unwrap());
        }
        "hold" => {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                assert_ne!(stdout.read_line(&mut line).unwrap(), 0);
                if line.starts_with("console=") {
                    break;
                }
            }
            let was_running = child.try_wait().unwrap().is_none();
            child.kill().unwrap();
            child.wait().unwrap();
            println!("{}was-running={was_running}", line);
            println!("reaped={}", child.try_wait().unwrap().is_some());
        }
        _ => panic!("unknown fixture mode"),
    }
}
