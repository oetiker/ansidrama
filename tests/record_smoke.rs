//! End-to-end: `record` drives a real embedded terminal (no tmux) and writes a
//! non-empty WebP. `tmux` is shadowed on `PATH` with a stub that always fails,
//! so this test breaks if `record` ever shells out to `tmux` again.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

#[test]
fn record_produces_webp_without_tmux() {
    let dir = std::env::temp_dir().join(format!("ansidrama-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Set up a bin/ dir containing a tmux stub that always fails, and put it
    // at the front of PATH. bash/printf/sleep still resolve via the original
    // (appended) PATH, so only tmux is shadowed.
    let bindir = dir.join("bin");
    std::fs::create_dir_all(&bindir).unwrap();
    let tmux_stub = bindir.join("tmux");
    std::fs::write(
        &tmux_stub,
        "#!/bin/sh\necho 'tmux must not be used' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&tmux_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let orig_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", bindir.display(), orig_path));

    let toml = dir.join("drama.toml");
    let out = dir.join("out.webp");
    std::fs::write(
        &toml,
        "launch = \"printf 'HELLO WORLD'; sleep 2\"\n\
         cols = 40\n\
         rows = 6\n\
         [[scene]]\n\
         keys = []\n\
         hold_cs = 20\n",
    )
    .unwrap();

    let result = ansidrama::record(&toml, Some(&out), None);

    // Restore PATH and clean up before asserting, so failures don't leave a
    // shadowed tmux or a stray temp dir behind.
    std::env::set_var("PATH", orig_path);

    result.unwrap();

    let len = std::fs::metadata(&out).unwrap().len();
    assert!(len > 0, "webp should be non-empty");

    let _ = std::fs::remove_dir_all(&dir); // the test's own scratch dir
}
