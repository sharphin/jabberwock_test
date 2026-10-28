//! 小さい上限と実際の上限で、退役・再利用・flag・送信・リストの境界を確認する。
use std::process::Command;
use jabberwock::native;

#[path = "support/retirement.rs"]
mod guard;

#[test]
fn rust_generated_checks_reject_a_retired_marker_even_when_generations_match() {
    if !Command::new(native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        assert!(std::env::var("JW_REQUIRE_RETIREMENT_TESTS").is_err(), "退役のnative検証にclangが必要です");
        eprintln!("retired guard test not executed: clang unavailable");
        return;
    }
    for elide in [false, true] {
        let (ll, _) = jabberwock::llvm_build_with(guard::SOURCE, &jabberwock::no_modules, "retirement.jw", false,
            elide, jabberwock::ir::llvm::Opts::default()).unwrap();
        guard::check_retired_marker(&ll, if elide { "rust-elided" } else { "rust-checked" });
    }
}

#[test]
fn native_runtime_retires_without_reusing_identity_or_wrapping_flags() {
    if !Command::new(native::clang()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        assert!(std::env::var("JW_REQUIRE_RETIREMENT_TESTS").is_err(), "退役のnative検証にclangが必要です");
        eprintln!("retirement native test not executed: clang unavailable");
        return;
    }
    for limit in [None, Some(3), Some(7)] {
        let label = match limit { None => "real", Some(3) => "small3", _ => "small7" };
        let dir = std::env::temp_dir().join(format!("jw-retirement-{}-{}", std::process::id(), label));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("jw_rt.c"), native::RUNTIME_C).unwrap();
        let source = dir.join("retirement.c");
        std::fs::write(&source, include_str!("support/retirement.c")).unwrap();
        let exe = dir.join(if cfg!(windows) { "retirement.exe" } else { "retirement" });
        let mut command = Command::new(native::clang());
        command.arg(&source).args(["-std=c11", "-O1", "-o"]).arg(&exe);
        if let Some(limit) = limit { command.arg(format!("-DJW_TEST_GENERATION_LIMIT={}", limit)); }
        if cfg!(windows) { command.arg("-lshell32"); } else { command.args(["-lm", "-pthread"]); }
        if let Ok(flags) = std::env::var("JW_CFLAGS") { command.args(flags.split_whitespace()); }
        let output = command.output().expect("clang starts");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        if limit.is_none() {
            let invalid = Command::new(native::clang()).arg(&source)
                .args(["-std=c11", "-fsyntax-only", "-DJW_TEST_GENERATION_LIMIT=4"]).output().unwrap();
            assert!(!invalid.status.success() && String::from_utf8_lossy(&invalid.stderr).contains("low state bits"),
                "状態bitを含まない不正な上限を受け付けた");
        }
        let output = Command::new(&exe).arg("checks").output().unwrap();
        assert!(output.status.success(), "{}: {:?}: {}", label, output.status, String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "retirement checks passed\n");
        for (mode, error) in [("send", "世代番号の上限に達しました"), ("list", "世代番号の上限に達しました"), ("view", "dangling reference")] {
            let output = Command::new(&exe).arg(mode).output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{} {}: {}", label, mode, stderr);
            assert!(stderr.contains(error), "{} {}: {}", label, mode, stderr);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
