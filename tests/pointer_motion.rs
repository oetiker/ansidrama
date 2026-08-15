//! The pointer reports its motion — but only to an application that asked.
//!
//! Both tests drive a shell that puts its terminal in raw mode and echoes what
//! it receives through `cat -v`, so a mouse report arrives on screen as
//! printable text (`^[[<35;12;19M`). The assertion is then carried by `await`:
//! the recording either finds the report on screen or aborts.
//!
//! The pair is the point. The first shows the gate opening, the second shows it
//! staying shut — and a gate that has quietly stopped gating looks exactly like
//! one that works if you only ever test the open case.
#![cfg(unix)]

/// `stty raw -echo` so `cat` sees bytes as they land rather than waiting for a
/// newline that a mouse report never contains, and so the line discipline does
/// not echo the report itself — which would put the text on screen whether or
/// not the recorder ever sent it, and make both tests vacuous.
fn launch_with_mode(decset: &str) -> String {
    format!("stty raw -echo; printf \"\\033[?{decset}h\"; cat -v")
}

fn write_config(dir: &std::path::Path, decset: &str, await_ms: u64) -> std::path::PathBuf {
    let toml = dir.join("drama.toml");
    std::fs::write(
        &toml,
        format!(
            // A TOML *literal* string: the launch line carries `\033`, which a
            // basic string would reject as an escape sequence.
            "launch = '{}'\n\
             cols = 40\n\
             rows = 6\n\
             startup_ms = 1200\n\
             [[scene]]\n\
             move = {{ x = 12, y = 3 }}\n\
             await = \"35;12;3\"\n\
             await_ms = {await_ms}\n\
             hold_cs = 20\n",
            launch_with_mode(decset)
        ),
    )
    .unwrap();
    toml
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ansidrama-motion-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `?1003h` — any-event tracking. The application asked for motion, so a glide
/// delivers it, and the report lands on screen where `await` can find it.
#[test]
fn any_motion_app_receives_bare_motion() {
    let dir = scratch("on");
    let toml = write_config(&dir, "1003", 5000);
    let out = dir.join("out.webp");

    let result = ansidrama::record(&toml, Some(&out), None);

    let _ = std::fs::remove_dir_all(&dir);
    result.expect("the report should reach an app in any-event tracking");
}

/// `?1000h` — press/release only. The application never asked to hear about
/// motion, so it must receive none, and the `await` it cannot satisfy is what
/// proves the silence.
#[test]
fn press_release_app_receives_no_motion() {
    let dir = scratch("off");
    let toml = write_config(&dir, "1000", 800);
    let out = dir.join("out.webp");

    let result = ansidrama::record(&toml, Some(&out), None);

    let _ = std::fs::remove_dir_all(&dir);
    let err = result.expect_err("an app outside any-event tracking must receive no motion");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("35;12;3"),
        "the abort should name the pattern that went unmatched, got: {msg}"
    );
}
