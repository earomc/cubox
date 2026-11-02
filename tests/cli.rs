//! What the command line refuses, and what it takes.
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

/// Ends right away by what clap says, or at the search that can't be parsed.
fn search_stderr(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cubox"))
        .current_dir(std::env::temp_dir())
        .args(args)
        .output()
        .unwrap();
    (output.status.code(), String::from_utf8(output.stderr).unwrap())
}

#[test]
fn searches_refuse_what_excludes_each_other() {
    let condition = "biome desert within 100 of 0,0";
    for args in [
        vec!["seeds", "--threads", "0", condition],
        vec!["seeds", "--step", "5", condition],
        vec!["seeds", "--from", "1", "--seeds", "file", condition],
        vec!["seeds", "--limit", "1", "--tested", "1", condition],
        vec!["seeds", "--within", "100", condition],
        vec!["locations", "1", "--of", "nowhere", "poi village"],
        vec!["locations", "1", "--tested", "5", "poi village"],
        vec!["find", condition],
    ] {
        assert_eq!(search_stderr(&args).0, Some(2), "{args:?}");
    }
}

#[test]
fn each_search_says_what_it_needs() {
    let (code, stderr) = search_stderr(&["seeds", "biome desert within 100"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("needs `of"), "{stderr}");
    let (code, stderr) = search_stderr(&["locations", "hello", "biome desert within 100"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("first line"), "{stderr}");
    let (code, stderr) = search_stderr(&["locations", "hello", "poi village\nbiome desert within 100 of 0,0"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("leave out `of`"), "{stderr}");
}

#[test]
fn a_location_search_ends_by_itself() {
    let output = Command::new(env!("CARGO_BIN_EXE_cubox"))
        .current_dir(std::env::temp_dir())
        .args(["locations", "12345", "--within", "2000", "poi village"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
}

#[test]
fn img_takes_negative_heights() {
    let dir = std::env::temp_dir().join("cubox-negative-height");
    std::fs::create_dir_all(&dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cubox"))
        .current_dir(&dir)
        .args(["img", "1", "--tile-size", "4", "-y", "-32"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}
