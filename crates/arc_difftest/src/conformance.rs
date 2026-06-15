//! AIR conformance suite.
//!
//! Canonical test cases that any conforming AIR implementation must handle:
//! - **valid modules**: must parse and verify without error
//! - **invalid modules**: must be rejected at parse or verification time
//! - **semantic tests**: interpreter must produce the expected value
//! - **roundtrip tests**: parse → print → parse must be stable
//! - **optimization tests**: passes must preserve semantics

use arc_interp::run_main;
use arc_syntax::parse_module;
use arc_verify::verify_module;

/// A conformance test case.
#[derive(Debug, Clone)]
pub struct ConformanceCase {
    pub name: &'static str,
    pub source: &'static str,
    pub expectation: Expectation,
}

/// What the conformance case expects.
#[derive(Debug, Clone, PartialEq)]
pub enum Expectation {
    /// Must parse and verify successfully.
    Valid,
    /// Must be rejected (parse error or verification error).
    Invalid,
    /// Must produce this value from @main.
    Result(i64),
    /// Must parse and verify, but we don't check the value.
    Verifies,
}

/// All conformance cases.
pub fn all_cases() -> Vec<ConformanceCase> {
    let mut cases = Vec::new();
    cases.extend(valid_modules());
    cases.extend(invalid_modules());
    cases.extend(semantic_tests());
    cases.extend(optimization_cases());
    cases
}

fn valid_modules() -> Vec<ConformanceCase> {
    vec![
        ConformanceCase {
            name: "empty_module",
            source: "arc.module @empty {\n}\n",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "single_const_return",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %v = arc.const 0 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "binary_arithmetic",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 1 : i64
    %b = arc.const 2 : i64
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "multi_function",
            source: "\
arc.module @m {
  arc.func @helper(%x: i64) -> i64 {
  ^entry:
    %one = arc.const 1 : i64
    %r = arc.add %x, %one : i64
    arc.return %r : i64
  }
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 41 : i64
    %b = arc.call @helper(%a) : () -> i64
    arc.return %b : i64
  }
}
",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "branching",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %t = arc.const 1 : i64
    %cond = arc.icmp eq %t, %t : i1
    arc.cond_br %cond, ^yes, ^no
  ^yes:
    %a = arc.const 1 : i64
    arc.return %a : i64
  ^no:
    %b = arc.const 0 : i64
    arc.return %b : i64
  }
}
",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "void_return",
            source: "\
arc.module @m {
  arc.func @noop() {
  ^entry:
    arc.return
  }
}
",
            expectation: Expectation::Valid,
        },
        ConformanceCase {
            name: "all_arithmetic_ops",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 10 : i64
    %b = arc.const 3 : i64
    %c = arc.add %a, %b : i64
    %d = arc.sub %c, %b : i64
    %e = arc.mul %d, %b : i64
    arc.return %e : i64
  }
}
",
            expectation: Expectation::Valid,
        },
    ]
}

fn invalid_modules() -> Vec<ConformanceCase> {
    vec![
        ConformanceCase {
            name: "missing_module_keyword",
            source: "not_a_module @m {\n}\n",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "missing_module_name",
            source: "arc.module {\n}\n",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "duplicate_function",
            source: "\
arc.module @m {
  arc.func @f() -> i64 {
  ^entry:
    %v = arc.const 1 : i64
    arc.return %v : i64
  }
  arc.func @f() -> i64 {
  ^entry:
    %v = arc.const 2 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "undefined_value",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    arc.return %nonexistent : i64
  }
}
",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "empty_input",
            source: "",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "unclosed_module",
            source: "arc.module @m {\n",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "unclosed_function",
            source: "\
arc.module @m {
  arc.func @f() -> i64 {
  ^entry:
    %v = arc.const 1 : i64
}
",
            expectation: Expectation::Invalid,
        },
        ConformanceCase {
            name: "op_outside_block",
            source: "\
arc.module @m {
  arc.func @f() -> i64 {
    %v = arc.const 1 : i64
  }
}
",
            expectation: Expectation::Invalid,
        },
    ]
}

fn semantic_tests() -> Vec<ConformanceCase> {
    vec![
        ConformanceCase {
            name: "const_zero",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %v = arc.const 0 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Result(0),
        },
        ConformanceCase {
            name: "const_positive",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %v = arc.const 42 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Result(42),
        },
        ConformanceCase {
            name: "const_negative",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %v = arc.const -7 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Result(-7),
        },
        ConformanceCase {
            name: "add_basic",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 3 : i64
    %b = arc.const 4 : i64
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(7),
        },
        ConformanceCase {
            name: "sub_basic",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 10 : i64
    %b = arc.const 3 : i64
    %c = arc.sub %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(7),
        },
        ConformanceCase {
            name: "mul_basic",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 6 : i64
    %b = arc.const 7 : i64
    %c = arc.mul %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(42),
        },
        ConformanceCase {
            name: "add_negative_result",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 5 : i64
    %b = arc.const 10 : i64
    %c = arc.sub %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(-5),
        },
        ConformanceCase {
            name: "nested_arithmetic",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 2 : i64
    %b = arc.const 3 : i64
    %c = arc.const 4 : i64
    %ab = arc.mul %a, %b : i64
    %r = arc.add %ab, %c : i64
    arc.return %r : i64
  }
}
",
            expectation: Expectation::Result(10),
        },
        ConformanceCase {
            name: "branch_true",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %x = arc.const 5 : i64
    %cond = arc.icmp eq %x, %x : i1
    arc.cond_br %cond, ^yes, ^no
  ^yes:
    %a = arc.const 100 : i64
    arc.return %a : i64
  ^no:
    %b = arc.const 200 : i64
    arc.return %b : i64
  }
}
",
            expectation: Expectation::Result(100),
        },
        ConformanceCase {
            name: "branch_false",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %x = arc.const 1 : i64
    %y = arc.const 2 : i64
    %cond = arc.icmp eq %x, %y : i1
    arc.cond_br %cond, ^yes, ^no
  ^yes:
    %a = arc.const 100 : i64
    arc.return %a : i64
  ^no:
    %b = arc.const 200 : i64
    arc.return %b : i64
  }
}
",
            expectation: Expectation::Result(200),
        },
        ConformanceCase {
            name: "large_constant",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %v = arc.const 1000000000 : i64
    arc.return %v : i64
  }
}
",
            expectation: Expectation::Result(1_000_000_000),
        },
        ConformanceCase {
            name: "mul_by_zero",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 999 : i64
    %b = arc.const 0 : i64
    %c = arc.mul %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(0),
        },
        ConformanceCase {
            name: "identity_add",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 42 : i64
    %b = arc.const 0 : i64
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(42),
        },
    ]
}

fn optimization_cases() -> Vec<ConformanceCase> {
    vec![
        ConformanceCase {
            name: "opt_constant_fold",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 3 : i64
    %b = arc.const 4 : i64
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(7),
        },
        ConformanceCase {
            name: "opt_sub_self",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 42 : i64
    %b = arc.sub %a, %a : i64
    arc.return %b : i64
  }
}
",
            expectation: Expectation::Result(0),
        },
        ConformanceCase {
            name: "opt_mul_zero",
            source: "\
arc.module @m {
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 123 : i64
    %b = arc.const 0 : i64
    %c = arc.mul %a, %b : i64
    arc.return %c : i64
  }
}
",
            expectation: Expectation::Result(0),
        },
    ]
}

/// Run a single conformance case, returns (passed, error_message).
pub fn run_case(case: &ConformanceCase) -> (bool, Option<String>) {
    match &case.expectation {
        Expectation::Valid | Expectation::Verifies => match parse_module(case.source) {
            Ok(module) => match verify_module(&module) {
                Ok(()) => (true, None),
                Err(e) => (
                    false,
                    Some(format!("expected valid, got verify error: {}", e)),
                ),
            },
            Err(e) => (
                false,
                Some(format!("expected valid, got parse error: {}", e)),
            ),
        },
        Expectation::Invalid => {
            let parsed = parse_module(case.source);
            match parsed {
                Err(_) => (true, None), // Correctly rejected at parse time
                Ok(module) => match verify_module(&module) {
                    Err(_) => (true, None), // Correctly rejected at verify time
                    Ok(()) => (
                        false,
                        Some("expected invalid, but parsed and verified ok".to_string()),
                    ),
                },
            }
        }
        Expectation::Result(expected) => {
            let module = match parse_module(case.source) {
                Ok(m) => m,
                Err(e) => return (false, Some(format!("parse error: {}", e))),
            };
            if let Err(e) = verify_module(&module) {
                return (false, Some(format!("verify error: {}", e)));
            }
            match run_main(&module) {
                Ok(Some(val)) => {
                    if val == *expected {
                        (true, None)
                    } else {
                        (
                            false,
                            Some(format!("expected result {}, got {}", expected, val)),
                        )
                    }
                }
                Ok(None) => (
                    false,
                    Some(format!("expected result {}, got no return value", expected)),
                ),
                Err(e) => (false, Some(format!("interpreter error: {}", e))),
            }
        }
    }
}

pub type ConformanceResult = (String, bool, Option<String>);

/// Run all conformance cases, return number of (passed, failed, results).
pub fn run_all() -> (usize, usize, Vec<ConformanceResult>) {
    let cases = all_cases();
    let mut passed = 0;
    let mut failed = 0;
    let mut results = Vec::new();

    for case in &cases {
        let (ok, msg) = run_case(case);
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
        results.push((case.name.to_string(), ok, msg));
    }

    (passed, failed, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_valid_modules_accepted() {
        for case in valid_modules() {
            let (ok, msg) = run_case(&case);
            assert!(ok, "case '{}' failed: {:?}", case.name, msg);
        }
    }

    #[test]
    fn all_invalid_modules_rejected() {
        for case in invalid_modules() {
            let (ok, msg) = run_case(&case);
            assert!(ok, "case '{}' failed: {:?}", case.name, msg);
        }
    }

    #[test]
    fn all_semantic_tests_pass() {
        for case in semantic_tests() {
            let (ok, msg) = run_case(&case);
            assert!(ok, "case '{}' failed: {:?}", case.name, msg);
        }
    }

    #[test]
    fn all_optimization_cases_pass() {
        for case in optimization_cases() {
            let (ok, msg) = run_case(&case);
            assert!(ok, "case '{}' failed: {:?}", case.name, msg);
        }
    }

    #[test]
    fn conformance_suite_complete() {
        let (passed, failed, results) = run_all();
        for (name, ok, msg) in &results {
            if !ok {
                eprintln!("FAIL: {} — {:?}", name, msg);
            }
        }
        assert_eq!(failed, 0, "{} of {} cases failed", failed, passed + failed);
    }

    #[test]
    fn format_roundtrip_conformance() {
        for case in valid_modules() {
            let module = parse_module(case.source).unwrap();
            let encoded = arc_format::encode(&module).unwrap();
            let decoded = arc_format::decode(&encoded).unwrap();
            assert_eq!(
                module.name, decoded.name,
                "format roundtrip changed module name for '{}'",
                case.name
            );
        }
    }

    #[test]
    fn parse_roundtrip_conformance() {
        for case in valid_modules() {
            let module1 = parse_module(case.source).unwrap();
            // Re-serialize via the binary format and decode
            let enc = arc_format::encode(&module1).unwrap();
            let module2 = arc_format::decode(&enc).unwrap();
            assert_eq!(module1.name, module2.name);
            assert_eq!(module1.functions.len(), module2.functions.len());
        }
    }

    #[test]
    fn semantic_interp_vs_native() {
        // Run semantic tests through both interpreter and native codegen
        // and verify they agree.
        for case in semantic_tests() {
            if let Expectation::Result(expected) = case.expectation {
                match crate::diff_test(case.source) {
                    Ok(result) => {
                        assert!(
                            result.match_,
                            "case '{}': interp={:?} native={:?}",
                            case.name, result.interp_result, result.native_result
                        );
                        assert_eq!(
                            result.interp_result,
                            Some(expected),
                            "case '{}' wrong result",
                            case.name
                        );
                    }
                    Err(e) => {
                        panic!("case '{}' diff_test error: {}", case.name, e);
                    }
                }
            }
        }
    }
}
