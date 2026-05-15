fn main() {
    emit_git_commit_env();
    add_macos_swift_runtime_rpaths();
}

fn emit_git_commit_env() {
    println!("cargo:rerun-if-changed=.git/HEAD");

    let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !commit.is_empty() {
        println!("cargo:rustc-env=GIT_COMMIT={commit}");
    }
}

fn add_macos_swift_runtime_rpaths() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    emit_rpath("@executable_path/../Frameworks");
    emit_rpath("/usr/lib/swift");

    let Ok(output) = std::process::Command::new("xcode-select")
        .arg("-p")
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let developer_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    for suffix in [
        "Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx",
        "Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
    ] {
        let path = std::path::Path::new(&developer_dir).join(suffix);
        if path.exists() {
            emit_rpath(&path.display().to_string());
        }
    }
}

fn emit_rpath(path: &str) {
    println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
}
