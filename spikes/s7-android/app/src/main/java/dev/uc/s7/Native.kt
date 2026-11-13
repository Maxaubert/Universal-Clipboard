package dev.uc.s7

/** Plain JNI binding to the Rust library (no helper crates on the Rust side). */
object Native {
    init { System.loadLibrary("uc_s7") }
    external fun add(a: Int, b: Int): Int
}
