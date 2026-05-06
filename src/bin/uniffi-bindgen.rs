#[cfg(feature = "uniffi-cli")]
fn main() {
    uniffi::uniffi_bindgen_main()
}

#[cfg(not(feature = "uniffi-cli"))]
fn main() {
    eprintln!("enable the `uniffi-cli` feature to run this binary");
    std::process::exit(1);
}
