//! Compiles `guests/hello.c` static (`-O2 -static`, contract D21) and checks that under
//! calco it prints the same stdout and exits with the same code as natively (k1-04).

mod common;

#[test]
fn static_hello_matches_native() {
    let exe = common::cc("static_hello", "hello.c", "hello", &["-O2", "-static"]);
    let (args, env) = (["one", "two words", ""], [("CALCO_HELLO", "translated")]);
    let native = common::native(&exe, &args, &env);
    assert_eq!(native.status.code(), Some(3), "native: {}", native.status);
    common::assert_same(&native, &common::calco(&exe, &args, &env), "");
}
