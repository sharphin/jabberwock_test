//! 検査器の負例。検査を有効にした CI で、意図的な欠陥を本当に検出することを確かめる。
use std::process::Command;
use jabberwock::{llvm_build, native, no_modules};

fn selected_llvm(src: &str) -> String {
    let subject = std::env::var("JW_SAFETY_COMPILER").unwrap_or_else(|_| "rust".to_string());
    if subject == "rust" { return llvm_build(src, &no_modules, "main.jw", false).unwrap(); }
    assert_eq!(subject, "jwc", "unknown compiler subject");
    let compiler = std::env::var("JW_SAFETY_JWC").expect("jwc negative probe requires an explicit compiler path");
    let dir = std::env::temp_dir().join(format!("jw-jwc-negative-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.jw");
    std::fs::write(&source, src).unwrap();
    let output = Command::new(&compiler).arg("--ll").arg(&source).output().expect("runs selected jwc");
    assert!(output.status.success(), "selected jwc failed: {}", String::from_utf8_lossy(&output.stderr));
    let ll = String::from_utf8(output.stdout).expect("jwc LLVM UTF-8");
    assert!(ll.contains("define void @jw_main("), "selected jwc did not generate LLVM");
    std::fs::remove_dir_all(dir).unwrap();
    ll
}

#[test]
fn generated_llvm_is_instrumented() {
    let flags = std::env::var("JW_CFLAGS").unwrap_or_default();
    let (body, expected) = if flags.contains("address") {
        ("  %probe = call ptr @malloc(i64 8)\n  store volatile i64 7, ptr %probe\n  call void @free(ptr %probe)\n  %bad = load volatile i64, ptr %probe\n  call void @jw_report_checks(i64 %bad, i64 0, i64 0)\n", "AddressSanitizer: heap-use-after-free")
    } else if flags.contains("memory") {
        ("  %probe = call ptr @malloc(i64 8)\n  %bad = load volatile i64, ptr %probe\n  call void @jw_report_checks(i64 %bad, i64 0, i64 0)\n  call void @free(ptr %probe)\n", "MemorySanitizer: use-of-uninitialized-value")
    } else if flags.contains("thread") {
        ("  br label %probe.loop\nprobe.loop:\n  %probe.i = phi i64 [0, %entry], [%probe.next, %probe.loop]\n  %probe.v = load volatile i64, ptr @probe.shared\n  %probe.w = add i64 %probe.v, 1\n  store volatile i64 %probe.w, ptr @probe.shared\n  %probe.next = add i64 %probe.i, 1\n  %probe.more = icmp ult i64 %probe.next, 100000\n  br i1 %probe.more, label %probe.loop, label %probe.cont\nprobe.cont:\n", "ThreadSanitizer: data race")
    } else { return; };
    let src = if flags.contains("thread") {
        "fn work() {} fn main() { let a = spawn(work); let b = spawn(work); a.join(); b.join(); }"
    } else { "fn main() {}" };
    let mut ll = selected_llvm(src);
    let function = if flags.contains("thread") { "; work\n" } else { "; main\n" };
    let start = ll.find(function).expect("probe function");
    let entry = ll[start..].find("entry:\n").unwrap() + start + "entry:\n".len();
    ll.insert_str(entry, body);
    ll.push_str("\ndeclare void @jw_report_checks(i64, i64, i64)\n@probe.shared = global i64 0\n");
    let dir = std::env::temp_dir().join(format!("jw-sanitizer-probe-{}", std::process::id()));
    let exe = native::compile(&ll, &dir, "bad-llvm", "-O1").unwrap();
    let output = Command::new(exe).output().unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success() && err.contains(expected), "検査器が生成LLVMの欠陥を検出しない: {:?}: {}", output.status, err);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn c_runtime_instrumentation_detects_races_and_undefined_behavior() {
    let flags = std::env::var("JW_CFLAGS").unwrap_or_default();
    let (src, expected) = if flags.contains("thread") {
        ("#include <pthread.h>\nvolatile int shared;\nvoid *race(void *p) { for (int i=0;i<100000;i++) shared++; return 0; }\nint main() { pthread_t a,b; pthread_create(&a,0,race,0); pthread_create(&b,0,race,0); pthread_join(a,0); pthread_join(b,0); return 0; }", "ThreadSanitizer: data race")
    } else if flags.contains("undefined") {
        ("volatile int zero; int main() { return 7 / zero; }", "runtime error: division by zero")
    } else { return; };
    let dir = std::env::temp_dir().join(format!("jw-c-sanitizer-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("bad.c");
    let exe = dir.join("bad-c");
    std::fs::write(&input, src).unwrap();
    let b = Command::new(native::clang()).arg(&input).args(["-O1", "-pthread", "-o"]).arg(&exe).args(flags.split_whitespace()).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let o = Command::new(&exe).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success() && err.contains(expected), "検査器がCの欠陥を検出しない: {:?}: {}", o.status, err);
    let _ = std::fs::remove_dir_all(dir);
}
