use std::fs;

#[test]
fn makefile_has_release_target() {
    let contents = fs::read_to_string("Makefile").expect("Makefile should exist");
    assert!(
        contents.contains("omni-release:"),
        "Makefile should define omni-release target"
    );
    assert!(
        contents.contains("cargo build --release"),
        "omni-release should build release binary"
    );
}
