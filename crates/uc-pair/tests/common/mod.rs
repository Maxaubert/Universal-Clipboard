//! Shared test helpers.
#![allow(dead_code)]

use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

/// A test RNG that hands out the given bytes in order and fails the test if more are asked for.
pub struct FixedRng {
    bytes: Vec<u8>,
    pos: usize,
}

impl FixedRng {
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            pos: 0,
        }
    }
    pub fn used_all(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

impl RngCore for FixedRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        assert!(
            self.pos + dest.len() <= self.bytes.len(),
            "the crate asked for more random bytes than the vector gives"
        );
        dest.copy_from_slice(&self.bytes[self.pos..self.pos + dest.len()]);
        self.pos += dest.len();
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for FixedRng {}

/// A deterministic test RNG (SHA-256 in counter mode). Not for production.
pub struct TestRng {
    seed: [u8; 32],
    ctr: u64,
    buf: Vec<u8>,
}

impl TestRng {
    pub fn new(label: &str) -> Self {
        Self {
            seed: Sha256::digest(label.as_bytes()).into(),
            ctr: 0,
            buf: Vec::new(),
        }
    }
    pub fn from_u64(x: u64) -> Self {
        Self::new(&format!("seed-{x}"))
    }
}

impl RngCore for TestRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        while self.buf.len() < dest.len() {
            let mut h = Sha256::new();
            h.update(self.seed);
            h.update(self.ctr.to_be_bytes());
            self.ctr += 1;
            self.buf.extend_from_slice(&h.finalize());
        }
        dest.copy_from_slice(&self.buf[..dest.len()]);
        self.buf.drain(..dest.len());
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for TestRng {}

/// `n` bytes derived from a label.
pub fn derive(label: &str, n: usize) -> Vec<u8> {
    let mut r = TestRng::new(label);
    let mut v = vec![0u8; n];
    r.fill_bytes(&mut v);
    v
}

pub fn arr32(label: &str) -> [u8; 32] {
    derive(label, 32).try_into().unwrap()
}

pub fn hx(b: &[u8]) -> String {
    hex::encode(b)
}

pub fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s).expect("hex")
}

pub fn vectors_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors")
}
