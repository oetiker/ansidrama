//! End-to-end: `record` drives a real embedded terminal (no tmux) and writes a
//! non-empty WebP. Uses only bash/coreutils, so it runs anywhere `record` does.
#![cfg(unix)]

#[test]
fn record_produces_webp() {
    let dir = std::env::temp_dir().join(format!("ansidrama-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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

    ansidrama::record(&toml, Some(&out), None).unwrap();

    let len = std::fs::metadata(&out).unwrap().len();
    assert!(len > 0, "webp should be non-empty");

    let _ = std::fs::remove_dir_all(&dir); // the test's own scratch dir
}
