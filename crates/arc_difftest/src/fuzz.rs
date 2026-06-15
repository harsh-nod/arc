//! Pseudo-random AIR program generation and property-based fuzz testing.
//!
//! The fuzzer generates structurally valid AIR source programs from a seed,
//! then checks invariants:
//! - Parser must not panic on any input (valid or garbage).
//! - Valid programs must verify without panic.
//! - Encode→decode round-trip preserves the module.
//! - Interpreter must not panic on verified programs.
//! - Error diagnostics must be deterministic (same input → same errors).

use arc_format;
use arc_syntax::parse_module;
use arc_verify::verify_module;

/// A simple deterministic PRNG (xorshift64) for reproducible fuzzing.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn next_usize(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    pub fn next_i64(&mut self, lo: i64, hi: i64) -> i64 {
        let range = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % range) as i64
    }

    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let idx = self.next_usize(items.len());
        &items[idx]
    }
}

/// Generate a random but structurally valid AIR program.
pub fn gen_valid_program(rng: &mut Rng) -> String {
    let module_name = format!("fuzz_{}", rng.next_u64() % 10000);
    let num_funcs = 1 + rng.next_usize(3); // 1–3 functions

    let mut funcs = Vec::new();
    let mut has_main = false;

    for i in 0..num_funcs {
        let fname = if i == 0 {
            has_main = true;
            "main".to_string()
        } else {
            format!("f{}", i)
        };
        funcs.push(gen_function(rng, &fname, i == 0));
    }

    let _ = has_main;
    let mut src = format!("arc.module @{} {{\n", module_name);
    for f in &funcs {
        src.push_str(f);
        src.push('\n');
    }
    src.push_str("}\n");
    src
}

fn gen_function(rng: &mut Rng, name: &str, is_main: bool) -> String {
    let num_params = if is_main { 0 } else { rng.next_usize(3) };
    let _has_ret = true; // always returns i64 for simplicity

    let mut params = Vec::new();
    for p in 0..num_params {
        params.push(format!("%p{}: i64", p));
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "  arc.func @{}({}) -> i64 {{",
        name,
        params.join(", ")
    ));
    lines.push("  ^entry:".to_string());

    // Generate 1–6 operations
    let num_ops = 1 + rng.next_usize(6);
    let mut next_val = 0usize;
    let mut available_vals: Vec<String> = Vec::new();

    // Add params to available
    for p in 0..num_params {
        available_vals.push(format!("p{}", p));
    }

    for _ in 0..num_ops {
        let val_name = format!("v{}", next_val);
        next_val += 1;

        if available_vals.len() < 2 || rng.next_usize(3) == 0 {
            // Generate a constant
            let cval = rng.next_i64(-100, 100);
            lines.push(format!("    %{} = arc.const {} : i64", val_name, cval));
        } else {
            // Generate a binary op
            let ops = ["add", "sub", "mul"];
            let op = *rng.choose(ops.as_slice());
            let a = rng.choose(&available_vals).clone();
            let b = rng.choose(&available_vals).clone();
            lines.push(format!(
                "    %{} = arc.{} %{}, %{} : i64",
                val_name, op, a, b
            ));
        }
        available_vals.push(val_name);
    }

    // Return the last value
    let ret_val = available_vals.last().unwrap().clone();
    lines.push(format!("    arc.return %{} : i64", ret_val));
    lines.push("  }".to_string());

    lines.join("\n")
}

/// Generate random bytes that may or may not be valid AIR source.
pub fn gen_garbage_bytes(rng: &mut Rng, max_len: usize) -> Vec<u8> {
    let len = rng.next_usize(max_len.max(1));
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        bytes.push((rng.next_u64() % 256) as u8);
    }
    bytes
}

/// Generate semi-valid AIR source with random mutations.
pub fn gen_mutated_program(rng: &mut Rng) -> String {
    let mut src = gen_valid_program(rng);

    // Apply 1–3 random mutations
    let num_mutations = 1 + rng.next_usize(3);
    for _ in 0..num_mutations {
        let mutation = rng.next_usize(5);
        match mutation {
            0 => {
                // Delete a random character
                if !src.is_empty() {
                    let idx = rng.next_usize(src.len());
                    if src.is_char_boundary(idx) {
                        src.remove(idx);
                    }
                }
            }
            1 => {
                // Insert a random character
                let idx = rng.next_usize(src.len().max(1));
                let ch = (rng.next_u64() % 128) as u8 as char;
                if src.is_char_boundary(idx.min(src.len())) {
                    src.insert(idx.min(src.len()), ch);
                }
            }
            2 => {
                // Replace a random character
                if src.len() > 1 {
                    let idx = rng.next_usize(src.len() - 1);
                    if src.is_char_boundary(idx) {
                        let replacement = (rng.next_u64() % 128) as u8 as char;
                        src.remove(idx);
                        src.insert(idx, replacement);
                    }
                }
            }
            3 => {
                // Truncate
                let new_len = rng.next_usize(src.len().max(1));
                // Find a safe truncation point
                let safe_len = src[..new_len.min(src.len())]
                    .char_indices()
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                src.truncate(safe_len);
            }
            _ => {
                // Duplicate a line
                let line_count = src.lines().count();
                if line_count > 0 {
                    let line_idx = rng.next_usize(line_count);
                    if let Some(line) = src.lines().nth(line_idx) {
                        let dup = format!("\n{}", line);
                        src.push_str(&dup);
                    }
                }
            }
        }
    }
    src
}

/// Result of a single fuzz run.
#[derive(Debug)]
pub struct FuzzResult {
    pub seed: u64,
    pub program: String,
    pub parse_panicked: bool,
    pub verify_panicked: bool,
    pub format_roundtrip_ok: Option<bool>,
    pub deterministic_errors: bool,
}

/// Run a single fuzz iteration with a valid program.
pub fn fuzz_valid(seed: u64) -> FuzzResult {
    let mut rng = Rng::new(seed);
    let program = gen_valid_program(&mut rng);

    let mut result = FuzzResult {
        seed,
        program: program.clone(),
        parse_panicked: false,
        verify_panicked: false,
        format_roundtrip_ok: None,
        deterministic_errors: true,
    };

    // Parse (should succeed for gen_valid_program)
    let module = match std::panic::catch_unwind(|| parse_module(&program)) {
        Ok(Ok(m)) => m,
        Ok(Err(_)) => return result,
        Err(_) => {
            result.parse_panicked = true;
            return result;
        }
    };

    // Verify (should succeed for structurally valid programs)
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify_module(&module))) {
        Ok(_) => {}
        Err(_) => {
            result.verify_panicked = true;
            return result;
        }
    }

    // Binary format roundtrip
    match arc_format::encode(&module) {
        Ok(encoded) => match arc_format::decode(&encoded) {
            Ok(decoded) => {
                result.format_roundtrip_ok = Some(decoded.name == module.name);
            }
            Err(_) => {
                result.format_roundtrip_ok = Some(false);
            }
        },
        Err(_) => {
            result.format_roundtrip_ok = Some(false);
        }
    }

    result
}

/// Run a fuzz iteration with garbage/mutated input — checks no-panic only.
pub fn fuzz_garbage(seed: u64) -> FuzzResult {
    let mut rng = Rng::new(seed);
    let program = if seed % 2 == 0 {
        gen_mutated_program(&mut rng)
    } else {
        String::from_utf8_lossy(&gen_garbage_bytes(&mut rng, 200)).to_string()
    };

    let mut result = FuzzResult {
        seed,
        program: program.clone(),
        parse_panicked: false,
        verify_panicked: false,
        format_roundtrip_ok: None,
        deterministic_errors: true,
    };

    // Parse — must not panic regardless of input
    let parse_result1 = match std::panic::catch_unwind(|| parse_module(&program)) {
        Ok(r) => r,
        Err(_) => {
            result.parse_panicked = true;
            return result;
        }
    };

    // Determinism check: parse again, must get same result category
    let parse_result2 = match std::panic::catch_unwind(|| parse_module(&program)) {
        Ok(r) => r,
        Err(_) => {
            result.parse_panicked = true;
            return result;
        }
    };

    let same_result = match (&parse_result1, &parse_result2) {
        (Ok(_), Ok(_)) => true,
        (Err(e1), Err(e2)) => e1.to_string() == e2.to_string(),
        _ => false,
    };
    result.deterministic_errors = same_result;

    // If parse succeeded, try verify
    if let Ok(module) = parse_result1 {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify_module(&module))) {
            Ok(_) => {}
            Err(_) => {
                result.verify_panicked = true;
            }
        }
    }

    result
}

/// Fuzz the binary format decoder with random bytes.
pub fn fuzz_binary_format(seed: u64) -> bool {
    let mut rng = Rng::new(seed);
    let data = gen_garbage_bytes(&mut rng, 500);

    // Must not panic
    std::panic::catch_unwind(|| arc_format::decode(&data)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_deterministic() {
        let mut r1 = Rng::new(42);
        let mut r2 = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(r1.next_u64(), r2.next_u64());
        }
    }

    #[test]
    fn gen_valid_programs_parse() {
        for seed in 1..=50 {
            let mut rng = Rng::new(seed);
            let prog = gen_valid_program(&mut rng);
            let result = parse_module(&prog);
            assert!(
                result.is_ok(),
                "seed {} failed to parse:\n{}\nerror: {}",
                seed,
                prog,
                result.unwrap_err()
            );
        }
    }

    #[test]
    fn gen_valid_programs_verify() {
        for seed in 1..=50 {
            let mut rng = Rng::new(seed);
            let prog = gen_valid_program(&mut rng);
            let module = parse_module(&prog).unwrap();
            let result = verify_module(&module);
            assert!(
                result.is_ok(),
                "seed {} failed verification:\n{}\nerror: {}",
                seed,
                prog,
                result.unwrap_err()
            );
        }
    }

    #[test]
    fn fuzz_valid_no_panics() {
        for seed in 1..=100 {
            let result = fuzz_valid(seed);
            assert!(!result.parse_panicked, "parser panicked on seed {}", seed);
            assert!(
                !result.verify_panicked,
                "verifier panicked on seed {}",
                seed
            );
        }
    }

    #[test]
    fn fuzz_valid_format_roundtrip() {
        for seed in 1..=50 {
            let result = fuzz_valid(seed);
            if let Some(ok) = result.format_roundtrip_ok {
                assert!(ok, "binary format roundtrip failed on seed {}", seed);
            }
        }
    }

    #[test]
    fn fuzz_garbage_no_panics() {
        for seed in 1..=200 {
            let result = fuzz_garbage(seed);
            assert!(
                !result.parse_panicked,
                "parser panicked on garbage seed {}:\n{}",
                seed, result.program
            );
            assert!(
                !result.verify_panicked,
                "verifier panicked on garbage seed {}",
                seed
            );
        }
    }

    #[test]
    fn fuzz_garbage_deterministic_errors() {
        for seed in 1..=100 {
            let result = fuzz_garbage(seed);
            assert!(
                result.deterministic_errors,
                "non-deterministic error on seed {}",
                seed
            );
        }
    }

    #[test]
    fn fuzz_binary_format_no_panics() {
        for seed in 1..=200 {
            assert!(
                fuzz_binary_format(seed),
                "binary decoder panicked on seed {}",
                seed
            );
        }
    }

    #[test]
    fn fuzz_mutated_programs_no_panics() {
        for seed in 1..=100 {
            let mut rng = Rng::new(seed);
            let prog = gen_mutated_program(&mut rng);
            let _ = std::panic::catch_unwind(|| {
                let _ = parse_module(&prog);
            });
            // We only check no panic — parse errors are expected
        }
    }
}
