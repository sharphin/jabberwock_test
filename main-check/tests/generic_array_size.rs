//! Concrete generic arguments must be substituted before checking fixed-array size.
#[test]
fn generic_field_size_is_checked_after_substitution() {
    for declaration in ["struct Buffer<T> { T[131072] cells; }", "enum Buffer<T> { Data(T[131072] cells); }"] {
        let bad = format!("{} fn accept(Buffer<i64[2]> b) {{}} fn main() {{}}", declaration);
        let error = match jabberwock::typed(&bad, &jabberwock::no_modules) {
            Err(error) => error,
            Ok(_) => panic!("oversized instantiated array was accepted"),
        };
        assert!(error.title.contains("配列が大きすぎます"), "{}", error.title);
        let boundary = format!("{} fn accept(Buffer<i64> b) {{}} fn main() {{}}", declaration);
        assert!(jabberwock::typed(&boundary, &jabberwock::no_modules).is_ok());
    }
}
