fn main() {
    // rust-embed needs the folder to exist even before the web UI is built.
    std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/../nidoo-web/dist"))
        .expect("create the web dist folder");
    println!("cargo:rerun-if-changed=../nidoo-web/dist");
}
