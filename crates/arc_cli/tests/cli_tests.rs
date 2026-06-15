//! Integration tests for the `arcc` CLI binary.
//!
//! Each test spawns the CLI as a subprocess and checks exit code + output.

use std::path::PathBuf;
use std::process::Command;

fn arcc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_arcc"))
}

fn fixture(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    root.join(name)
}

// --- Parse ---

#[test]
fn parse_valid_file() {
    let out = arcc()
        .args(["parse", fixture("examples/hello.air").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "parse should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn parse_print_outputs_air() {
    let out = arcc()
        .args([
            "parse",
            "--print",
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("arc.module"),
        "output should contain arc.module"
    );
    assert!(
        stdout.contains("arc.func"),
        "output should contain arc.func"
    );
}

#[test]
fn parse_nonexistent_file_fails() {
    let out = arcc()
        .args(["parse", "/nonexistent/file.air"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// --- Verify ---

#[test]
fn verify_valid_file() {
    let out = arcc()
        .args(["verify", fixture("examples/hello.air").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("verification succeeded"));
}

#[test]
fn verify_extended_valid_file() {
    let out = arcc()
        .args([
            "verify",
            "--extended",
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "extended verify should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("extended verification succeeded"));
}

#[test]
fn verify_invalid_file_fails() {
    let out = arcc()
        .args([
            "verify",
            fixture("tests/verify/undefined_value.air")
                .to_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// --- Run ---

#[test]
fn run_hello_returns_10() {
    let out = arcc()
        .args(["run", fixture("examples/hello.air").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().contains("10"), "expected 10, got: {}", stdout);
}

#[test]
fn run_with_trace_flag() {
    let trace_file = std::env::temp_dir().join("test_trace.json");
    let out = arcc()
        .args([
            "run",
            "--trace",
            trace_file.to_str().unwrap(),
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "run --trace failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(trace_file.exists(), "trace file should be created");
    let contents = std::fs::read_to_string(&trace_file).unwrap();
    assert!(contents.contains("FunctionEntry"));
    std::fs::remove_file(&trace_file).ok();
}

// --- Trace ---

#[test]
fn trace_prints_events() {
    let out = arcc()
        .args(["trace", fixture("examples/hello.air").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("arc.trace"), "should print trace block");
    assert!(stdout.contains("result:"));
}

// --- Opt ---

#[test]
fn opt_constant_fold() {
    let out = arcc()
        .args([
            "opt",
            "--passes",
            "constant_fold",
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("arc.module"));
}

#[test]
fn opt_with_output_flag() {
    let out_file = std::env::temp_dir().join("opt_output.air");
    let out = arcc()
        .args([
            "opt",
            "--passes",
            "constant_fold",
            "-o",
            out_file.to_str().unwrap(),
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "opt -o failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out_file.exists());
    let contents = std::fs::read_to_string(&out_file).unwrap();
    assert!(contents.contains("arc.module"));
    std::fs::remove_file(&out_file).ok();
}

// --- Lower ---

#[test]
fn lower_default() {
    let out = arcc()
        .args(["lower", fixture("examples/hello.air").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("arc.module"));
}

// --- Codegen ---

#[test]
fn codegen_x86_64() {
    let input = fixture("tests/codegen/simple_return.air");
    let out = arcc()
        .args(["codegen", "--target", "x86_64", input.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "codegen failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("compiled"));
    // Cleanup generated .o file
    std::fs::remove_file(input.with_extension("o")).ok();
}

#[test]
fn codegen_wasm32() {
    let input = fixture("tests/codegen/simple_return.air");
    let out = arcc()
        .args(["codegen", "--target", "wasm32", input.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "wasm codegen failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("compiled"));
    // Cleanup generated .wasm file
    std::fs::remove_file(input.with_extension("wasm")).ok();
}

// --- Replay ---

#[test]
fn replay_trace_file() {
    // First generate a trace
    let trace_file = std::env::temp_dir().join("replay_test.json");
    let out = arcc()
        .args([
            "run",
            "--trace",
            trace_file.to_str().unwrap(),
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Then replay it
    let out = arcc()
        .args(["replay", trace_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("events loaded"));
    assert!(stdout.contains("arc.trace"));
    std::fs::remove_file(&trace_file).ok();
}

// --- TraceCompare ---

#[test]
fn trace_compare_identical() {
    let trace_file = std::env::temp_dir().join("cmp_test.json");
    arcc()
        .args([
            "run",
            "--trace",
            trace_file.to_str().unwrap(),
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let out = arcc()
        .args([
            "trace-compare",
            trace_file.to_str().unwrap(),
            trace_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "identical traces should match");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("match"));
    std::fs::remove_file(&trace_file).ok();
}

// --- FuzzSmoke ---

#[test]
fn fuzz_smoke_runs() {
    let out = arcc()
        .args(["fuzz-smoke", "--seeds", "10"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fuzz-smoke failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("passed"));
}

// --- Explain ---

#[test]
fn explain_filters_events() {
    let trace_file = std::env::temp_dir().join("explain_test.json");
    arcc()
        .args([
            "run",
            "--trace",
            trace_file.to_str().unwrap(),
            fixture("examples/hello.air").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let out = arcc()
        .args(["explain", "--event", "invoke", trace_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("events matching"));
    std::fs::remove_file(&trace_file).ok();
}
