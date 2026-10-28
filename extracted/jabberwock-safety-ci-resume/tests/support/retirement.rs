use std::process::Command;
use jabberwock::native;

pub const SOURCE: &str = "fn main() { let x = own 9; let r = ref x; drop(x); print(*r); }\n";

/// 有効な区画に対し、退役markerと同じ世代を持つ壊れたrefをLLVM内で注入する。
/// 単なる世代の不一致ではなく、生成した検査がRetired自体を拒否することを実行して確認。
pub fn check_retired_marker(ll: &str, name: &str) {
    let mut lines: Vec<String> = ll.lines().map(str::to_string).collect();
    let mut injected = false;
    for i in 0..lines.len() {
        if !lines[i].contains(" = extractvalue %ref ") || !lines[i].ends_with(", 2") { continue; }
        let generation = lines[i].split(" = ").next().unwrap().trim().to_string();
        let next = (i + 1..(i + 12).min(lines.len())).find(|j| lines[*j].contains("load atomic i64, ptr "));
        if let Some(j) = next {
            let hp = lines[j].split("load atomic i64, ptr ").nth(1).unwrap().split_whitespace().next().unwrap();
            let store = format!("  store atomic i64 -1, ptr {} monotonic, align 8", hp);
            lines[i] = format!("  {} = add i64 0, -1", generation);
            lines.insert(j, store);
            injected = true;
            break;
        }
    }
    assert!(injected, "世代検査の負例を注入できない: {}", name);
    let dir = std::env::temp_dir().join(format!("jw-retired-guard-{}-{}", std::process::id(), name));
    let opt = if std::env::var("JW_NATIVE").map(|v| v == "o2").unwrap_or(false) { "-O2" } else { "-O1" };
    let executable = native::compile(&(lines.join("\n") + "\n"), &dir, name, opt).expect("retirement probe compiles");
    let output = Command::new(executable).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{}: {}", name, stderr);
    assert!(stderr.contains("dangling reference"), "{}: {}", name, stderr);
    assert!(output.stdout.is_empty(), "退役したpayloadを出力した: {}", name);
    std::fs::remove_dir_all(dir).unwrap();
}
