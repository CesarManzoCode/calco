//! Compiles `guests/hello.c` dynamic with gcc's default PIE (contract D21) and checks that
//! under calco, entering through the system `ld.so`, it prints the same stdout and exits
//! with the same code as natively (k2-05).

mod common;

#[test]
fn dynamic_hello_matches_native() {
    let exe = common::cc("dynamic", "hello.c", "hello-dynamic", &["-O2"]);
    let (args, env) = (["one", "two words"], [("CALCO_HELLO", "dynamic")]);
    let native = common::native(&exe, &args, &env);
    assert_eq!(native.status.code(), Some(3), "native: {}", native.status);
    common::assert_same(&native, &common::calco(&exe, &args, &env), "");
}
