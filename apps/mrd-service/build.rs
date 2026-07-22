fn main() {
    emit_git_commit_env();
    add_macos_swift_runtime_rpaths();
}

fn emit_git_commit_env() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=VERGEN_GIT_SHA");
    emit_git_rerun_paths();

    if let Ok(commit) = std::env::var("GIT_COMMIT") {
        let commit = commit.trim();
        if !commit.is_empty() {
            println!("cargo:rustc-env=GIT_COMMIT={commit}");
            return;
        }
    }

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

fn emit_git_rerun_paths() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    let workspace_git = manifest_dir.join("../../.git");
    let git_dir = resolve_git_dir(&workspace_git).unwrap_or(workspace_git);
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());

    let Ok(head) = std::fs::read_to_string(&head_path) else {
        return;
    };
    let Some(ref_name) = head.trim().strip_prefix("ref: ") else {
        return;
    };

    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join(ref_name).display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
}

fn resolve_git_dir(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.is_dir() {
        return Some(path.to_path_buf());
    }

    let contents = std::fs::read_to_string(path).ok()?;
    let gitdir = contents.trim().strip_prefix("gitdir:")?.trim();
    let gitdir_path = std::path::PathBuf::from(gitdir);
    if gitdir_path.is_absolute() {
        Some(gitdir_path)
    } else {
        path.parent().map(|parent| parent.join(gitdir_path))
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
