//! CLI must honor --no-elide when emitting LLVM, not silently use the optimized path.
use std::process::Command;

#[test]
fn llvm_cli_honors_no_elide() {
    let source = "fn main() { let a = own 7; let r = ref a; print(*r); }";
    let path = std::env::temp_dir().join(format!("jw-llvm-cli-{}.jw", std::process::id()));
    std::fs::write(&path, source).unwrap();
    let emit = |checked: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_jabberwock"));
        if checked { cmd.arg("--no-elide"); }
        let result = cmd.arg("--ll").arg(&path).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        String::from_utf8(result.stdout).unwrap()
    };
    let checked = emit(true);
    let elided = emit(false);
    std::fs::remove_file(&path).unwrap();
    assert_ne!(checked, elided, "--no-elide must retain checks the optimizer would remove");
    for text in [&checked, &elided] {
        assert!(text.contains("define void @jw_main("));
    }
}
