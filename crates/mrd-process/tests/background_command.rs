#![cfg(windows)]

use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::OnceLock,
};

fn child_command(mode: &str) -> Command {
    static LAUNCHER: OnceLock<std::path::PathBuf> = OnceLock::new();
    let launcher = LAUNCHER.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("mrd-background-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("background-launcher.exe");
        let rustc = std::env::var_os("RUSTC")
            .or_else(|| {
                let cargo_home = std::env::var_os("CARGO_HOME")
                    .map(std::path::PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("USERPROFILE")
                            .map(|home| std::path::PathBuf::from(home).join(".cargo"))
                    })?;
                let compiler = cargo_home.join("bin/rustc.exe");
                compiler.is_file().then(|| compiler.into_os_string())
            })
            .unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .args(["--edition=2021", "--crate-name=mrd_background_launcher"])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/background_launcher.rs"
            ))
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile GUI supervisor fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        executable
    });
    let mut command = Command::new(launcher);
    command
        .arg(std::env::current_exe().expect("test executable"))
        .arg(mode);
    command
}

// Run the real console subsystem executable, rather than infer flags from source.
#[test]
fn child_process_fixture() {
    let Ok(mode) = std::env::var("MRD_BACKGROUND_PROCESS_TEST_CHILD") else {
        return;
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
    }
    println!("console={}", unsafe { GetConsoleWindow() } as usize);
    std::io::stdout().flush().unwrap();
    match mode.as_str() {
        "output" => {
            println!("stdout-marker");
            eprintln!("stderr-marker");
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            std::process::exit(7);
        }
        "echo" => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            println!("echo={input}");
        }
        "hold" => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
        }
        _ => panic!("unknown child mode"),
    }
}

#[test]
fn output_has_no_console_and_preserves_stdout_stderr_and_exit_status() {
    let output = child_command("output").output().expect("launch child");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stdout.contains("console=0\n"),
        "child allocated a console: {stdout}"
    );
    assert!(stdout.contains("stdout-marker"));
    assert!(stderr.contains("stderr-marker"));
    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn spawn_has_no_console_and_preserves_piped_stdin() {
    let mut child = child_command("echo")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch child");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"frame-data")
        .unwrap();
    let output = child.wait_with_output().expect("wait child");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("console=0\n"),
        "child allocated a console: {stdout}"
    );
    assert!(stdout.contains("echo=frame-data"));
    assert!(output.status.success());
}

#[test]
fn long_running_spawn_has_no_console_and_can_be_killed_and_reaped() {
    let output = child_command("hold").output().expect("launch supervisor");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("console=0\n"),
        "child allocated a console: {stdout}"
    );
    assert!(stdout.contains("was-running=true"));
    assert!(stdout.contains("reaped=true"));
    assert!(output.status.success());
}

#[test]
fn missing_executable_preserves_spawn_error() {
    let output = child_command("missing")
        .output()
        .expect("launch supervisor");
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("error=NotFound"));
}
