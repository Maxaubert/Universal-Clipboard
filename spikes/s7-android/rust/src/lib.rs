//! Throwaway S7 spike: a tiny Rust library that the Android app loads two ways.
//! 1. Plain JNI: `Java_dev_uc_s7_Native_add` (no helper crates).
//! 2. UniFFI: `greet` and `core_info`, called through generated Kotlin and JNA.

use std::ffi::c_void;

uniffi::setup_scaffolding!();

/// Plain JNI export, matches `external fun add(a: Int, b: Int): Int` in `dev.uc.s7.Native`.
#[no_mangle]
pub extern "system" fn Java_dev_uc_s7_Native_add(
    _env: *mut c_void,
    _class: *mut c_void,
    a: i32,
    b: i32,
) -> i32 {
    a.wrapping_add(b)
}

#[uniffi::export]
pub fn greet(name: String) -> String {
    format!("hello {name} from Rust")
}

#[uniffi::export]
pub fn core_info() -> String {
    format!(
        "uc-s7 {} arch={} os={}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}
