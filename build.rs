use std::process::Command;

// Bake the git commit into GIT_COMMIT at *build* time so the sim run manifest can
// record it without spawning `git` on every `run()` (reproducibility only). An
// externally provided GIT_COMMIT (e.g. from CI) takes precedence; in non-git or
// sandboxed builds the lookup fails gracefully and the manifest records null.
fn main() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    if std::path::Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }
    let commit = std::env::var("GIT_COMMIT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|s| !s.is_empty())
        });
    if let Some(commit) = commit {
        println!("cargo:rustc-env=GIT_COMMIT={commit}");
    }
}
