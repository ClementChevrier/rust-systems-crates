#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

const LARGE_ALLOCATION: usize = 64 * 1024 * 1024;
const PAGE_SIZE: usize = 4096;

#[test]
fn working_set_grows_after_touching_a_large_allocation() {
    let before = memory().expect("memory must be readable").ram_use;

    let mut buffer = vec![0u8; LARGE_ALLOCATION];
    for page in buffer.chunks_mut(PAGE_SIZE) {
        page[0] = 1;
    }

    let after = memory().expect("memory must be readable").ram_use;
    assert!(
        after > before,
        "working set {before} -> {after} after touching {LARGE_ALLOCATION} bytes"
    );
    drop(buffer);
}
