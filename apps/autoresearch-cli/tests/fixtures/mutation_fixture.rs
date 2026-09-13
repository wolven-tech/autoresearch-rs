//! Local-only executable used to test command mutation containment.

use std::io::Read;

fn main() {
    let mut request = String::new();
    std::io::stdin()
        .read_to_string(&mut request)
        .expect("read mutation request");
    if !request.contains("one falsifiable change") {
        std::process::exit(2);
    }
    std::fs::write("src/example.rs", "pub fn command_candidate() {}\n")
        .expect("write declared candidate source");
}
