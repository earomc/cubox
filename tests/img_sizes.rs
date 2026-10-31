//! `img` must refuse sizes it can't make before it starts on them.
#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

fn stderr_of_refused(args: &[&str]) -> String {
    let dir = std::env::temp_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_cubox"))
        .current_dir(dir)
        .args(["img", "1"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{args:?}");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn sizes_out_of_range_are_refused() {
    for args in [
        ["--tile-size", "0"],
        ["--tiles", "0"],
        ["--tile-size", "32769"],
        ["--tiles", "32769"],
        ["--tile-size", "2000000000"],
    ] {
        assert!(stderr_of_refused(&args).contains("is not in"), "{args:?}");
    }
}

#[test]
fn images_too_big_together_are_refused() {
    let stderr = stderr_of_refused(&["--tiles", "1000", "--tile-size", "1000"]);
    assert!(stderr.contains("too big"), "{stderr}");
}
