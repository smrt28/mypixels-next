fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    let revision = git(&["rev-parse", "--verify", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let modified = std::process::Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .output().is_ok_and(|output| output.status.code() == Some(1));
    let revision = format!("{revision}{}", if modified { " (modified)" } else { "" });
    println!("cargo:rustc-env=MYPIXELS_GIT_REVISION={revision}");

    // A commit or checkout can change the revision without touching source.
    // Resolve Git paths so linked worktrees work too.
    let mut git_paths = vec!["HEAD".to_string(), "index".into(), "packed-refs".into()];
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        git_paths.push(reference);
    }
    for name in git_paths {
        if let Some(path) = git(&["rev-parse", "--git-path", &name]) {
            if std::path::Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    // Stamp every build so a running session can always be asked which
    // binary it actually is. A daemon that never dies makes "did my fix
    // take effect?" unanswerable otherwise - and this session-skew has
    // now caused two rounds of debugging the wrong process.
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%h %cd", "--date=format:%Y-%m-%d %H:%M"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    // Build *time*, not commit time: the question this answers is "is the
    // running session older than the last rebuild", and a commit date says
    // nothing about either.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let stamp = format!("{} built {}", if out.is_empty() { "unknown".to_string() } else { out }, now);
    println!("cargo:rustc-env=MYPIXELS_BUILD={stamp}");
    // Re-run on ANY source change, or the stamp freezes at the last build
    // that touched build.rs and every later binary claims that old time -
    // an odometer that stops is worse than no odometer (observed within
    // the hour of the stamp's introduction, naturally).
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=vendor");
    for entry in std::fs::read_dir("src").expect("src") {
        if let Ok(e) = entry {
            println!("cargo:rerun-if-changed={}", e.path().display());
        }
    }
}
