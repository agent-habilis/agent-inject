// Cargo-style status output: this module `include!`s the canonical source at
// `../crates/agent-inject/src/util/output.rs`, so the CLI and `cargo task`
// print identically with no crate dependency. The dead-code expect for
// the subset this crate uses lives on the `mod output` declaration in `util`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/agent-inject/src/util/output.rs"
));
