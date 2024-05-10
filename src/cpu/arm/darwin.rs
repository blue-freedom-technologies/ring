// Copyright 2016-2024 Brian Smith.
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHORS DISCLAIM ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY
// SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

use super::{AES, ARMCAP_STATIC, NEON, PMULL, SHA256, SHA512};

// ```
// $ rustc +1.61.0 --print cfg --target=aarch64-apple-ios | grep -E "neon|aes|sha|pmull"
// target_feature="aes"
// target_feature="neon"
// target_feature="sha2"
// $ rustc +1.61.0 --print cfg --target=aarch64-apple-darwin | grep -E "neon|aes|sha|pmull"
// target_feature="aes"
// target_feature="neon"
// target_feature="sha2"
// target_feature="sha3"
// ```
//
// XXX/TODO(coverage)/TODO(size): aarch64-apple-darwin is statically guaranteed to have "sha3" but
// other aarch64-apple-* targets require dynamic detection. Since we don't have test coverage for
// the other targets yet, we wouldn't have a way of testing the dynamic detection if we statically
// enabled `SHA512` for -darwin. So instead, temporarily, we statically ignore the static
// availability of the feature on -darwin so that it runs the dynamic detection.
pub const MIN_STATIC_FEATURES: u32 = NEON.mask | AES.mask | SHA256.mask | PMULL.mask;
pub const FORCE_DYNAMIC_DETECTION: u32 = !MIN_STATIC_FEATURES;

// MSRV: Enforce 1.61.0 onaarch64-apple-*, in particular) prior to. Earlier
// versions of Rust before did not report the AAarch64 CPU features correctly
// for these targets. Cargo.toml specifies `rust-version` but versions before
// Rust 1.56 don't know about it.
#[allow(clippy::assertions_on_constants)]
const _AARCH64_APPLE_TARGETS_EXPECTED_FEATURES: () =
    assert!((ARMCAP_STATIC & MIN_STATIC_FEATURES) == MIN_STATIC_FEATURES);

// Ensure we don't accidentally allow features statically beyond
// `MIN_STATIC_FEATURES` so that dynamic detection is done uniformly for
// all of these targets.
#[allow(clippy::assertions_on_constants)]
const _AARCH64_APPLE_DARWIN_TARGETS_EXPECTED_FEATURES: () =
    assert!(ARMCAP_STATIC == MIN_STATIC_FEATURES);

pub fn detect_features() -> u32 {
    // TODO(MSRV 1.64): Use `name: &core::ffi::CStr`.
    fn detect_feature(name: &[u8]) -> bool {
        use crate::polyfill;
        use core::mem;
        use libc::{c_char, c_int, c_void};

        let nul_terminated = name
            .iter()
            .position(|&b| b == 0)
            .map(|index| (index + 1) == name.len())
            .unwrap_or(false);
        if !nul_terminated {
            return false;
        }
        let name = polyfill::ptr::from_ref(name).cast::<c_char>();

        let mut value: c_int = 0;
        let mut len = mem::size_of_val(&value);
        let value_ptr = polyfill::ptr::from_mut(&mut value).cast::<c_void>();
        // SAFETY: `name` is nul-terminated and it doesn't contain interior nul bytes. `value_ptr`
        // is a valid pointer to `value` and `len` is the size of `value`.
        let rc = unsafe { libc::sysctlbyname(name, value_ptr, &mut len, core::ptr::null_mut(), 0) };
        // All the conditions are separated so we can observe them in code coverage.
        if rc != 0 {
            return false;
        }
        debug_assert_eq!(len, mem::size_of_val(&value));
        if len != mem::size_of_val(&value) {
            return false;
        }
        value != 0
    }

    // We do not need to check for the presence of NEON, as Armv8-A always has it
    const _ASSERT_NEON_DETECTED: () = assert!((ARMCAP_STATIC & NEON.mask) == NEON.mask);

    let mut features = 0;

    // TODO(MSRV 1.64): Use `: &CStr = CStr::from_bytes_with_nul_unchecked`.
    // TODO(MSRV 1.77): Use c"..." literal.
    const SHA512_NAME: &[u8] = b"hw.optional.armv8_2_sha512\0";
    if detect_feature(SHA512_NAME) {
        features |= SHA512.mask;
    }

    features
}
