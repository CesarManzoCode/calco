use super::*;

const ONES: [u32; 4] = [u32::MAX; 4];
// Leaf 7 subleaf 0.
const AVX2: u32 = 1 << 5;
const AVX512F: u32 = 1 << 16;
const SHSTK: u32 = 1 << 7;

#[test]
fn leaf1_drops_avx_xsave_and_keeps_v2() {
    let host = [
        0x00a5_0f00,
        0x0102_0800,
        AVX | XSAVE | OSXSAVE | FMA | LEAF1_ECX,
        ONES[3],
    ];
    let [a, b, c, d] = filter(1, host);
    assert_eq!((a, b), (host[0], host[1]), "signature and brand/APIC pass");
    assert_eq!(c, LEAF1_ECX);
    assert_eq!(c & (AVX | XSAVE | OSXSAVE | FMA | MOVBE | AES | F16C), 0);
    assert_eq!(d, LEAF1_EDX);
}

#[test]
fn leaf1_only_keeps_what_the_host_has() {
    let [_, _, c, d] = filter(1, [0, 0, 0, 0]);
    assert_eq!((c, d), (0, 0));
}

#[test]
fn leaf7_is_zero_on_every_subleaf() {
    let host = [2, AVX2 | AVX512F, SHSTK, u32::MAX];
    assert_eq!(filter(7, host), [0; 4]);
    assert_eq!(filter(7, ONES), [0; 4]);
}

#[test]
fn extended_leaf1_keeps_lahf_syscall_nx_rdtscp_lm() {
    let [a, _, c, d] = filter(0x8000_0001, [0x00a5_0f00, 0, u32::MAX, u32::MAX]);
    assert_eq!(a, 0x00a5_0f00);
    assert_eq!(c, EXT1_ECX);
    assert_eq!(c & (ABM | SSE4A), 0);
    assert_eq!(d, EXT1_EDX);
}

#[test]
fn informative_leaves_pass() {
    let host = [0x10, 0x6874_7541, 0x444d_4163, 0x6974_6e65];
    for leaf in [
        0,
        4,
        0xb,
        0x8000_0000,
        0x8000_0002,
        0x8000_0003,
        0x8000_0004,
        0x8000_0005,
        0x8000_0006,
        0x8000_0008,
        0x8000_001d,
        0x8000_001e,
    ] {
        assert_eq!(filter(leaf, host), host, "leaf {leaf:#x}");
    }
}

#[test]
fn unknown_leaves_are_zero() {
    for leaf in [2, 3, 5, 6, 0xd, 0x14, 0x4000_0000, 0x8000_0007, 0x8000_001f] {
        assert_eq!(filter(leaf, ONES), [0; 4], "leaf {leaf:#x}");
    }
}

#[test]
fn hwcap_is_filtered_leaf1_edx() {
    let hwcap = hwcap();
    assert_eq!(hwcap & !(LEAF1_EDX as u64), 0);
    assert_eq!(hwcap, (host(1, 0)[3] & LEAF1_EDX) as u64);
}
