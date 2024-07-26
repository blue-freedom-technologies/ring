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

mod abi_assumptions {
    // TODO: Support ARM64_32; see
    // https://github.com/briansmith/ring/issues/1832#issuecomment-1892928147. This also requires
    // replacing all `cfg(target_pointer_width)` logic for non-pointer/reference things
    // (`N0`, `Limb`, `LimbMask`, `crypto_word_t` etc.).
    #[cfg(target_arch = "aarch64")]
    const _ASSUMED_POINTER_SIZE: usize = 8;
    #[cfg(target_arch = "arm")]
    const _ASSUMED_POINTER_SIZE: usize = 4;
    const _ASSUMED_USIZE_SIZE: () = assert!(core::mem::size_of::<usize>() == _ASSUMED_POINTER_SIZE);
    const _ASSUMED_REF_SIZE: () =
        assert!(core::mem::size_of::<&'static u8>() == _ASSUMED_POINTER_SIZE);

    // To support big-endian, we'd need to make several changes as described in
    // https://github.com/briansmith/ring/issues/1832.
    const _ASSUMED_ENDIANNESS: () = assert!(cfg!(target_endian = "little"));
}

// uclibc: When linked statically, uclibc doesn't provide getauxval.
// When linked dynamically, recent versions do provide it, but we
// want to support older versions too. Assume that if uclibc is being
// used, this is an embedded target where the user cares a lot about
// minimizing code size and also that they know in advance exactly
// what target features are supported, so rely only on static feature
// detection.

cfg_if::cfg_if! {
    if #[cfg(all(target_arch = "aarch64",
                 any(target_os = "ios", target_os = "macos", target_os = "tvos", target_os = "visionos", target_os = "watchos")))] {
        mod darwin;
        use darwin as detect;
    } else if #[cfg(all(target_arch = "aarch64", target_os = "fuchsia"))] {
        mod fuchsia;
        use fuchsia as detect;
    } else if #[cfg(any(target_os = "android", target_os = "linux"))] {
        mod linux;
        use linux as detect;
    } else if #[cfg(all(target_arch = "aarch64", target_os = "windows"))] {
        mod windows;
        use windows as detect;
    } else {
        mod detect {
            pub const FORCE_DYNAMIC_DETECTION: u32 = 0;
            pub fn detect_features() -> u32 { 0 }
        }
    }
}

macro_rules! features {
    {
        $(
            $target_feature_name:expr => $TyName:ident($name:ident) {
                mask: $mask:expr,
            }
        ),+
        , // trailing comma is required.
    } => {
        $(
            #[allow(dead_code)]
            pub(crate) const $name: Feature = Feature {
                mask: $mask,
            };
            impl_get_feature!{ $name => $TyName }
        )+

        // See const assertions below.
        const ARMCAP_STATIC: u32 = ARMCAP_STATIC_DETECTED & !detect::FORCE_DYNAMIC_DETECTION;
        const ARMCAP_STATIC_DETECTED: u32 = 0
            $(
                | (
                    if cfg!(all(any(target_arch = "aarch64", target_arch = "arm"),
                                target_feature = $target_feature_name)) {
                        $name.mask
                    } else {
                        0
                    }
                )
            )+;

        const ALL_FEATURES: &[Feature] = &[
            $(
                $name
            ),+
        ];
    }
}

pub(crate) struct Feature {
    mask: u32,
}

impl Feature {
    #[inline(always)]
    pub fn available(&self, cpu_features: super::Features) -> bool {
        if self.mask == self.mask & ARMCAP_STATIC {
            return true;
        }
        self.mask == self.mask & featureflags::get(cpu_features)
    }
}

#[cfg(target_arch = "aarch64")]
features! {
    // Keep in sync with `ARMV7_NEON`.
    "neon" => Neon(NEON) {
        mask: 1 << 0,
    },

    // Keep in sync with `ARMV8_AES`.
    "aes" => Aes(AES) {
        mask: 1 << 2,
    },

    // Keep in sync with `ARMV8_SHA256`.
    "sha2" => Sha256(SHA256) {
        mask: 1 << 4,
    },

    // Keep in sync with `ARMV8_PMULL`.
    //
    // TODO(MSRV): There is no "pmull" feature listed from
    // `rustc --print cfg --target=aarch64-apple-darwin`. Originally ARMv8 tied
    // PMULL detection into AES detection, but later versions split it; see
    // https://developer.arm.com/downloads/-/exploration-tools/feature-names-for-a-profile
    // "Features introduced prior to 2020." Change this to use "pmull" when
    // that is supported.
    "aes" => PMull(PMULL) {
        mask: 1 << 5,
    },

    // Keep in sync with `ARMV8_SHA512`.
    // "sha3" is overloaded for both SHA-3 and SHA512.
    "sha3" => Sha512(SHA512) {
        mask: 1 << 6,
    },
}

#[cfg(target_arch = "arm")]
features! {
    // Keep in sync with `ARMV7_NEON`.
    "neon" => Neon(NEON) {
        mask: 1 << 0,
    },
}

pub(super) mod featureflags {
    use super::{detect, ALL_FEATURES, ARMCAP_STATIC, NEON};
    use crate::cpu;
    use core::ptr;

    pub(in super::super) fn get_or_init() -> cpu::Features {
        // SAFETY: `init` must be called only in `INIT.call_once(init)` below.
        unsafe fn init() {
            let detected = detect::detect_features();
            let filtered = (if cfg!(feature = "unstable-testing-arm-no-hw") {
                ALL_FEATURES
                    .iter()
                    .fold(0, |acc, feature| acc | feature.mask)
                    & !NEON.mask
            } else {
                0
            }) | (if cfg!(feature = "unstable-testing-arm-no-neon") {
                NEON.mask
            } else {
                0
            });
            let detected = detected & !filtered;
            let merged = ARMCAP_STATIC | detected;
            // SAFETY: https://github.com/rust-lang/rust/issues/125833
            let p = unsafe { ptr::addr_of_mut!(OPENSSL_armcap_P) };
            // SAFETY: This is the only writer. Any concurrent reading doesn't
            // affect the safety of this write.
            unsafe {
                p.write(merged);
            }
        }
        static INIT: spin::Once<()> = spin::Once::new();
        // SAFETY: This is the only caller. Any concurrent reading doesn't
        // affect the safety of the writing.
        let () = INIT.call_once(|| unsafe { init() });
        // SAFETY: We initialized the CPU features as required.
        // `INIT.call_once` has `happens-before` semantics.
        unsafe { cpu::Features::new_after_feature_flags_written_and_synced_unchecked() }
    }

    pub(super) fn get(_cpu_features: cpu::Features) -> u32 {
        // SAFETY: https://github.com/rust-lang/rust/issues/125833
        let p = unsafe { ptr::addr_of!(OPENSSL_armcap_P) };

        // SAFETY: Since only `get_or_init()` could have created
        // `_cpu_features`, and it only does so after the `INIT.call_once()`,
        // which guarantees `happens-before` semantics, we can read from
        // `OPENSSL_armcap_P` without further synchronization.
        unsafe { ptr::read(p) }
    }

    // Some non-Rust code still checks this even when it is statically known
    // the given feature is available, so we have to ensure that this is
    // initialized properly. Keep this in sync with the initialization in
    // BoringSSL's crypto.c.
    //
    // SAFETY:
    // - Some assembly language functions access `OPENSSL_armcap_P` directly.
    //   Callers of those functions must obtain a `cpu::Features` before calling
    //   them.
    // - `OPENSSL_armcap_P` must always be a superset of `ARMCAP_STATIC`.
    // TODO: Remove all the direct accesses of this from assembly language code, and then replace this
    // with a `OnceCell<u32>` that will provide all the necessary safety guarantees.
    prefixed_extern! {
        static mut OPENSSL_armcap_P: u32;
    }
}

#[allow(clippy::assertions_on_constants)]
const _AARCH64_HAS_NEON: () =
    assert!(((ARMCAP_STATIC & NEON.mask) == NEON.mask) || !cfg!(target_arch = "aarch64"));

#[allow(clippy::assertions_on_constants)]
const _FORCE_DYNAMIC_DETECTION_HONORED: () =
    assert!((ARMCAP_STATIC & detect::FORCE_DYNAMIC_DETECTION) == 0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu;

    #[test]
    fn test_mask_abi() {
        assert_eq!(NEON.mask, 1);
    }

    #[cfg(not(target_arch = "arm"))]
    #[test]
    fn test_mask_abi_hw() {
        assert_eq!(AES.mask, 4);
        assert_eq!(SHA256.mask, 16);
        assert_eq!(PMULL.mask, 32);
        assert_eq!(SHA512.mask, 64);
    }

    #[test]
    fn test_armcap_static_is_subset_of_armcap_dynamic() {
        let cpu = cpu::features();
        let armcap_dynamic = featureflags::get(cpu);
        assert_eq!(armcap_dynamic & ARMCAP_STATIC, ARMCAP_STATIC);

        ALL_FEATURES.iter().for_each(|feature| {
            if (ARMCAP_STATIC & feature.mask) != 0 {
                assert!(feature.available(cpu));
            }
        })
    }
}
