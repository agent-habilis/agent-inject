// Names never embedded or written by `plug`: editor and OS cruft. Both
// `build.rs` (the embed fingerprint) and `src/plug.rs` (the write-out filter)
// `include!` this, so the two sets cannot drift. It is an `include!` fragment
// that expands to a `&[&str]`, not a module.
&[".DS_Store", ".git"]
