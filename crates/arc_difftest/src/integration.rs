//! Integration tests that run .air files from the tests/ directory.
//!
//! Tests cover parse round-trips, verification rejection, interpreter results,
//! optimization correctness, and codegen compilation.

use arc_interp::{run_main, run_main_traced};
use arc_pass::{resolve_pass, PassManager};
use arc_syntax::{parse_module, print_module};
use arc_verify::verify_module;
use std::path::Path;

/// Load an .air file relative to the workspace root.
fn load_air(relative_path: &str) -> String {
    // Walk up from the crate directory to find the workspace root
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    dir.push(relative_path);
    std::fs::read_to_string(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", dir.display(), e))
}

/// Parse, print, re-parse a program and check stability.
fn assert_parse_roundtrip(path: &str) {
    let source = load_air(path);
    let module1 = parse_module(&source).unwrap_or_else(|e| panic!("{}: parse failed: {}", path, e));
    let printed = print_module(&module1);
    let module2 = parse_module(&printed).unwrap_or_else(|e| {
        panic!(
            "{}: re-parse failed after printing:\n{}\nerror: {}",
            path, printed, e
        )
    });
    let printed2 = print_module(&module2);
    assert_eq!(
        printed, printed2,
        "{}: print not stable across round-trips",
        path
    );
}

/// Parse and verify should succeed.
fn assert_valid(path: &str) {
    let source = load_air(path);
    let module = parse_module(&source).unwrap_or_else(|e| panic!("{}: parse failed: {}", path, e));
    verify_module(&module).unwrap_or_else(|e| panic!("{}: verify failed: {}", path, e));
}

/// Parse should succeed but verify should fail.
fn assert_verify_rejects(path: &str) {
    let source = load_air(path);
    let module = parse_module(&source).unwrap_or_else(|e| panic!("{}: parse failed: {}", path, e));
    assert!(
        verify_module(&module).is_err(),
        "{}: expected verification to fail, but it succeeded",
        path
    );
}

/// Run through interpreter and check the result value.
fn assert_interp_result(path: &str, expected: i64) {
    let source = load_air(path);
    let module = parse_module(&source).unwrap();
    verify_module(&module).unwrap();
    let result = run_main(&module).unwrap();
    assert_eq!(
        result,
        Some(expected),
        "{}: expected {} but got {:?}",
        path,
        expected,
        result
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Parse round-trip tests ---

    #[test]
    fn roundtrip_const_and_arithmetic() {
        assert_parse_roundtrip("tests/parse/const_and_arithmetic.air");
    }

    #[test]
    fn roundtrip_branch() {
        assert_parse_roundtrip("tests/parse/branch.air");
    }

    #[test]
    fn roundtrip_multi_func() {
        assert_parse_roundtrip("tests/parse/multi_func.air");
    }

    #[test]
    fn roundtrip_spec_valid_minimal() {
        assert_parse_roundtrip("tests/spec/valid-minimal-module.air");
    }

    #[test]
    fn roundtrip_spec_valid_branch() {
        assert_parse_roundtrip("tests/spec/valid-branch.air");
    }

    #[test]
    fn roundtrip_example_hello() {
        assert_parse_roundtrip("examples/hello.air");
    }

    // --- Verification tests ---

    #[test]
    fn valid_const_and_arithmetic() {
        assert_valid("tests/parse/const_and_arithmetic.air");
    }

    #[test]
    fn valid_branch() {
        assert_valid("tests/parse/branch.air");
    }

    #[test]
    fn valid_multi_func() {
        assert_valid("tests/parse/multi_func.air");
    }

    #[test]
    fn verify_rejects_undefined_value() {
        assert_verify_rejects("tests/verify/undefined_value.air");
    }

    #[test]
    fn verify_rejects_missing_terminator() {
        assert_verify_rejects("tests/verify/missing_terminator.air");
    }

    #[test]
    fn verify_rejects_duplicate_def() {
        assert_verify_rejects("tests/verify/duplicate_def.air");
    }

    #[test]
    fn verify_rejects_type_mismatch() {
        assert_verify_rejects("tests/verify/type_mismatch.air");
    }

    #[test]
    fn verify_rejects_missing_authority() {
        assert_verify_rejects("tests/verify/missing_authority.air");
    }

    #[test]
    fn verify_rejects_wrong_authority() {
        assert_verify_rejects("tests/verify/wrong_authority.air");
    }

    // --- Interpreter tests ---

    #[test]
    fn interp_simple_add() {
        assert_interp_result("tests/interp/simple_add.air", 10);
    }

    #[test]
    fn interp_branch_true() {
        assert_interp_result("tests/interp/branch_true.air", 1);
    }

    #[test]
    fn interp_branch_false() {
        assert_interp_result("tests/interp/branch_false.air", 2);
    }

    #[test]
    fn interp_call() {
        assert_interp_result("tests/interp/call.air", 10);
    }

    #[test]
    fn interp_example_hello() {
        assert_interp_result("examples/hello.air", 10);
    }

    // --- Optimizer correctness tests ---

    #[test]
    fn opt_const_fold_preserves_result() {
        let source = load_air("tests/opt/const_fold.air");
        let mut module = parse_module(&source).unwrap();
        verify_module(&module).unwrap();

        // Run before optimization
        let before = run_main(&module).unwrap();

        // Apply constant folding
        let mut pm = PassManager::new();
        pm.add_pass_boxed(resolve_pass("constant_fold").unwrap());
        pm.run(&mut module);

        // Run after optimization — must produce the same result
        let after = run_main(&module).unwrap();
        assert_eq!(before, after, "constant folding changed the result");
        assert_eq!(after, Some(10));
    }

    /// Helper: run a pass pipeline on an .air file and assert the interpreter
    /// result is preserved.
    fn assert_opt_preserves(path: &str, passes: &[&str]) {
        let source = load_air(path);
        let mut module = parse_module(&source).unwrap();
        verify_module(&module).unwrap();

        let before = run_main(&module).unwrap();

        let mut pm = PassManager::new();
        for pass_name in passes {
            pm.add_pass_boxed(resolve_pass(pass_name).unwrap());
        }
        pm.run(&mut module);

        let after = run_main(&module).unwrap();
        assert_eq!(
            before, after,
            "{}: passes {:?} changed result from {:?} to {:?}",
            path, passes, before, after
        );
    }

    #[test]
    fn opt_dce_preserves_result() {
        assert_opt_preserves("tests/opt/dce_target.air", &["dce"]);
    }

    #[test]
    fn opt_strength_reduce_preserves_result() {
        assert_opt_preserves("tests/opt/strength_reduce_target.air", &["strength_reduce"]);
    }

    #[test]
    fn opt_canonicalize_preserves_result() {
        assert_opt_preserves("tests/opt/const_fold.air", &["canonicalize"]);
    }

    #[test]
    fn opt_cse_preserves_result() {
        assert_opt_preserves("tests/opt/const_fold.air", &["cse"]);
    }

    #[test]
    fn opt_multi_pass_pipeline_preserves_result() {
        assert_opt_preserves(
            "tests/opt/multi_pass.air",
            &["constant_fold", "dce", "canonicalize"],
        );
    }

    #[test]
    fn opt_all_passes_on_hello() {
        assert_opt_preserves(
            "examples/hello.air",
            &[
                "constant_fold",
                "dce",
                "cse",
                "strength_reduce",
                "canonicalize",
            ],
        );
    }

    // --- Structured control flow tests ---

    #[test]
    fn interp_if_else_true() {
        assert_interp_result("tests/interp/if_else.air", 10);
    }

    #[test]
    fn interp_if_else_false() {
        assert_interp_result("tests/interp/if_else_false.air", 20);
    }

    #[test]
    fn roundtrip_if_else() {
        assert_parse_roundtrip("tests/interp/if_else.air");
    }

    #[test]
    fn verify_rejects_region_undefined_value() {
        assert_verify_rejects("tests/verify/region_undefined_value.air");
    }

    // --- Codegen tests ---

    #[test]
    fn codegen_simple_return_compiles() {
        let source = load_air("tests/codegen/simple_return.air");
        let module = parse_module(&source).unwrap();
        verify_module(&module).unwrap();
        let target = arc_targets::x86_64::target();
        let obj = arc_codegen::compile(&module, &target).unwrap();
        assert!(!obj.text.is_empty(), "compiled code should not be empty");
    }

    #[test]
    fn codegen_wasm_simple_return() {
        let source = load_air("tests/codegen/simple_return.air");
        let module = parse_module(&source).unwrap();
        verify_module(&module).unwrap();
        let low = arc_codegen::low_ir::lower_module(&module).unwrap();
        let wasm = arc_codegen::wasm_emit::emit_wasm_module(&low.functions).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm", "wasm magic missing");
    }

    // --- Codegen: x86_64 pipeline tests ---

    /// Helper: parse, verify, compile to x86_64 and check .text is non-empty.
    fn assert_codegen_x86(path: &str) {
        let source = load_air(path);
        let module =
            parse_module(&source).unwrap_or_else(|e| panic!("{}: parse failed: {}", path, e));
        verify_module(&module).unwrap_or_else(|e| panic!("{}: verify failed: {}", path, e));
        let target = arc_targets::x86_64::target();
        let obj = arc_codegen::compile(&module, &target)
            .unwrap_or_else(|e| panic!("{}: x86_64 codegen failed: {}", path, e));
        assert!(!obj.text.is_empty(), "{}: compiled .text is empty", path);
        assert!(!obj.symbols.is_empty(), "{}: no symbols emitted", path);
    }

    /// Helper: parse, verify, lower to low IR, emit wasm and check magic.
    fn assert_codegen_wasm(path: &str) {
        let source = load_air(path);
        let module =
            parse_module(&source).unwrap_or_else(|e| panic!("{}: parse failed: {}", path, e));
        verify_module(&module).unwrap_or_else(|e| panic!("{}: verify failed: {}", path, e));
        let low = arc_codegen::low_ir::lower_module(&module)
            .unwrap_or_else(|e| panic!("{}: low IR lowering failed: {}", path, e));
        let wasm = arc_codegen::wasm_emit::emit_wasm_module(&low.functions)
            .unwrap_or_else(|e| panic!("{}: wasm emit failed: {}", path, e));
        assert!(
            wasm.len() >= 8,
            "{}: wasm too short ({} bytes)",
            path,
            wasm.len()
        );
        assert_eq!(&wasm[0..4], b"\0asm", "{}: wasm magic missing", path);
    }

    #[test]
    fn codegen_x86_arithmetic() {
        assert_codegen_x86("tests/codegen/arithmetic.air");
    }

    #[test]
    fn codegen_x86_branch() {
        assert_codegen_x86("tests/codegen/branch.air");
    }

    #[test]
    fn codegen_x86_call() {
        assert_codegen_x86("tests/codegen/call.air");
    }

    #[test]
    fn codegen_x86_compare() {
        assert_codegen_x86("tests/codegen/compare.air");
    }

    #[test]
    fn codegen_x86_multi_block() {
        assert_codegen_x86("tests/codegen/multi_block.air");
    }

    #[test]
    fn codegen_x86_chain_call() {
        assert_codegen_x86("tests/codegen/chain_call.air");
    }

    #[test]
    fn codegen_wasm_arithmetic() {
        assert_codegen_wasm("tests/codegen/arithmetic.air");
    }

    #[test]
    fn codegen_wasm_branch() {
        assert_codegen_wasm("tests/codegen/branch.air");
    }

    #[test]
    fn codegen_wasm_call() {
        assert_codegen_wasm("tests/codegen/call.air");
    }

    #[test]
    fn codegen_wasm_compare() {
        assert_codegen_wasm("tests/codegen/compare.air");
    }

    #[test]
    fn codegen_wasm_multi_block() {
        assert_codegen_wasm("tests/codegen/multi_block.air");
    }

    #[test]
    fn codegen_wasm_chain_call() {
        assert_codegen_wasm("tests/codegen/chain_call.air");
    }

    // --- Trace tests ---

    #[test]
    fn trace_roundtrip_on_file() {
        let source = load_air("tests/interp/simple_add.air");
        let module = parse_module(&source).unwrap();
        verify_module(&module).unwrap();
        let (result, trace) = run_main_traced(&module).unwrap();
        assert_eq!(result, Some(10));
        assert!(!trace.is_empty());

        // JSON round-trip
        let json = trace.to_json();
        let restored = arc_interp::trace::Trace::from_json(&json).unwrap();
        assert_eq!(restored.len(), trace.len());
    }
}
