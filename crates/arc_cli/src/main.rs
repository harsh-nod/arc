use anyhow::{Context, Result};
use arc_interp::trace::Trace;
use arc_interp::{run_main, run_main_traced};
use arc_lower::{
    AsyncLowering, InvokeToCallLowering, LoweringPipeline, ProofErasureLowering,
    StructuredControlFlowLowering,
};
use arc_pass::{resolve_pass, PassManager};
use arc_security::{
    audit_module, check_information_flow, SandboxPolicy, SecurityContext, SecurityLevel, TaintLabel,
};
use arc_syntax::{parse_module, print_module};
use arc_verify::{verify_module, verify_module_extended};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "arcc")]
#[command(about = "ARC command line interface")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse an ARC file.
    Parse {
        /// ARC source file
        file: PathBuf,
        /// Print the parsed module
        #[arg(long)]
        print: bool,
    },
    /// Verify an ARC module.
    Verify {
        /// ARC source file
        file: PathBuf,
        /// Run security, memory, and proof integration checks after base verification.
        #[arg(long)]
        extended: bool,
        /// Mark values as confidential for extended verification.
        #[arg(long)]
        confidential: Option<String>,
        /// Mark values as tainted by user input for extended verification.
        #[arg(long)]
        tainted: Option<String>,
        /// Restrict allowed capabilities for extended verification.
        #[arg(long)]
        sandbox: Option<String>,
        /// Maximum security level for sandbox (public, internal, confidential, secret).
        #[arg(long, default_value = "confidential")]
        max_level: String,
    },
    /// Run an ARC module using the reference interpreter.
    Run {
        /// ARC source file
        file: PathBuf,
        /// Save execution trace to a JSON file
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Optimize an ARC module.
    Opt {
        file: PathBuf,
        #[arg(long)]
        passes: Option<String>,
        /// Skip verifier checks after each optimization pass.
        #[arg(long)]
        no_verify_each: bool,
        /// Write output to file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Lower an ARC module to a target dialect.
    Lower {
        file: PathBuf,
        #[arg(long)]
        to: Option<String>,
        /// Write output to file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate native code from an ARC module.
    Codegen {
        file: PathBuf,
        #[arg(long)]
        target: Option<String>,
    },
    /// Run and trace an ARC module, printing the full trace.
    Trace { file: PathBuf },
    /// Replay a saved trace file.
    Replay {
        /// JSON trace file
        file: PathBuf,
    },
    /// Compare two saved traces.
    TraceCompare {
        /// First trace file
        a: PathBuf,
        /// Second trace file
        b: PathBuf,
    },
    /// Run a quick fuzz smoke test.
    FuzzSmoke {
        /// Number of seeds to test (default: 50)
        #[arg(long, default_value = "50")]
        seeds: u64,
    },
    /// Filter trace events by keyword.
    Explain {
        /// JSON trace file
        file: PathBuf,
        /// Event pattern to filter by
        #[arg(long)]
        event: String,
    },
    /// Run security analysis on an ARC module.
    Security {
        /// ARC source file
        file: PathBuf,
        /// Mark values as confidential (comma-separated value names)
        #[arg(long)]
        confidential: Option<String>,
        /// Mark values as tainted by user input (comma-separated value names)
        #[arg(long)]
        tainted: Option<String>,
        /// Restrict allowed capabilities (comma-separated, prefix with ! to deny)
        #[arg(long)]
        sandbox: Option<String>,
        /// Maximum security level for sandbox (public, internal, confidential, secret)
        #[arg(long, default_value = "confidential")]
        max_level: String,
    },
    /// Audit an ARC module's security posture.
    Audit {
        /// ARC source file
        file: PathBuf,
    },
    /// Run tests: parse, verify, and optionally interpret .air files.
    Test {
        /// .air files or directories to test
        files: Vec<PathBuf>,
        /// Check that @main returns this value
        #[arg(long)]
        expect: Option<i64>,
        /// Only parse (skip verify and run)
        #[arg(long)]
        parse_only: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Parse { file, print } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            if print {
                print!("{}", print_module(&module));
            }
        }
        Commands::Verify {
            file,
            extended,
            confidential,
            tainted,
            sandbox,
            max_level,
        } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            if extended {
                let ctx = build_security_context(confidential.as_deref(), tainted.as_deref());
                let policy = build_sandbox_policy(sandbox.as_deref(), &max_level);
                let result = verify_module_extended(&module, &ctx, policy.as_ref())?;
                if result.is_clean() {
                    println!("extended verification succeeded for {}", file.display());
                    println!(
                        "  capability invocations: {}",
                        result.audit.capability_invocations
                    );
                    println!(
                        "  approval requests:      {}",
                        result.audit.approval_requests
                    );
                    println!(
                        "  external effects:       {}",
                        result.audit.has_external_effects
                    );
                } else {
                    print_extended_issues(&result);
                    anyhow::bail!(
                        "extended verification failed for {} with {} issue(s)",
                        file.display(),
                        result.issue_count()
                    );
                }
            } else {
                verify_module(&module)?;
                println!("verification succeeded for {}", file.display());
            }
        }
        Commands::Run { file, trace } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            if let Some(trace_path) = trace {
                let (result, trace_data) = run_main_traced(&module)?;
                match result {
                    Some(value) => println!("{}", value),
                    None => println!("program completed without value"),
                }
                fs::write(&trace_path, trace_data.to_json()).with_context(|| {
                    format!("failed to write trace to {}", trace_path.display())
                })?;
                eprintln!("trace saved to {}", trace_path.display());
            } else {
                match run_main(&module)? {
                    Some(value) => println!("{}", value),
                    None => println!("program completed without value"),
                }
            }
        }
        Commands::Opt {
            file,
            passes,
            no_verify_each,
            output,
        } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let mut module = parse_module(&source)?;
            verify_module(&module)?;
            let mut pm = PassManager::new();
            if let Some(pass_list) = passes {
                for pass_name in pass_list.split(',') {
                    let pass_name = pass_name.trim();
                    let pass = resolve_pass(pass_name)
                        .ok_or_else(|| anyhow::anyhow!("unknown pass: {}", pass_name))?;
                    pm.add_pass_boxed(pass);
                }
            }
            if no_verify_each {
                pm.run(&mut module);
            } else {
                pm.run_verified(&mut module, verify_module)?;
            }
            let text = print_module(&module);
            if let Some(out_path) = output {
                fs::write(&out_path, &text)
                    .with_context(|| format!("failed to write {}", out_path.display()))?;
            } else {
                print!("{}", text);
            }
        }
        Commands::Trace { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            let (result, trace) = run_main_traced(&module)?;
            match result {
                Some(value) => println!("result: {}", value),
                None => println!("program completed without value"),
            }
            print!("{}", trace.format("run"));
        }
        Commands::Lower { file, to, output } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            let target = to.as_deref().unwrap_or("arc-cfg");
            let mut pipeline = LoweringPipeline::new();
            match target {
                "arc-cfg" => {
                    pipeline.add_pass(Box::new(StructuredControlFlowLowering::new()));
                    pipeline.add_pass(Box::new(AsyncLowering));
                    pipeline.add_pass(Box::new(InvokeToCallLowering));
                    pipeline.add_pass(Box::new(ProofErasureLowering));
                }
                other => {
                    anyhow::bail!("unknown lowering target: {}", other);
                }
            }
            let (lowered, refinements) = pipeline.run(&module)?;
            let text = print_module(&lowered);
            if let Some(out_path) = output {
                fs::write(&out_path, &text)
                    .with_context(|| format!("failed to write {}", out_path.display()))?;
            } else {
                print!("{}", text);
                eprintln!("--- refinements ---");
                for r in &refinements {
                    eprint!("{}", r.format());
                }
            }
        }
        Commands::Codegen { file, target } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            let target_name = target.as_deref().unwrap_or("x86_64-arc-linux-elf");
            // Lower structured control flow before codegen
            let mut pre_codegen = LoweringPipeline::new();
            pre_codegen.add_pass(Box::new(StructuredControlFlowLowering::new()));
            pre_codegen.add_pass(Box::new(AsyncLowering));
            pre_codegen.add_pass(Box::new(InvokeToCallLowering));
            pre_codegen.add_pass(Box::new(ProofErasureLowering));
            let (module, _) = pre_codegen.run(&module)?;
            match target_name {
                "wasm32" | "wasm32-arc" => {
                    // Wasm codegen: lower to low IR, then emit wasm binary directly
                    let low_module = arc_codegen::low_ir::lower_module(&module)?;
                    let wasm = arc_codegen::wasm_emit::emit_wasm_module(&low_module.functions)?;
                    let out_path = file.with_extension("wasm");
                    fs::write(&out_path, &wasm)
                        .with_context(|| format!("failed to write {}", out_path.display()))?;
                    println!(
                        "compiled {} -> {} ({} bytes)",
                        file.display(),
                        out_path.display(),
                        wasm.len(),
                    );
                }
                "x86_64-arc-linux-elf" | "x86_64" => {
                    let target_desc = arc_targets::x86_64::target();
                    let obj = arc_codegen::compile(&module, &target_desc)?;
                    let out_path = file.with_extension("o");
                    let elf = obj.to_elf();
                    fs::write(&out_path, &elf)
                        .with_context(|| format!("failed to write {}", out_path.display()))?;
                    println!(
                        "compiled {} -> {} ({} bytes, {} symbols)",
                        file.display(),
                        out_path.display(),
                        elf.len(),
                        obj.symbols.len()
                    );
                }
                other => anyhow::bail!("unknown target: {}", other),
            }
        }
        Commands::Replay { file } => {
            let json = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let trace = Trace::from_json(&json).map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("{} events loaded from {}", trace.len(), file.display());
            print!("{}", trace.format("replay"));
        }
        Commands::TraceCompare { a, b } => {
            let json_a = fs::read_to_string(&a)
                .with_context(|| format!("failed to read {}", a.display()))?;
            let json_b = fs::read_to_string(&b)
                .with_context(|| format!("failed to read {}", b.display()))?;
            let trace_a =
                Trace::from_json(&json_a).map_err(|e| anyhow::anyhow!("trace A: {}", e))?;
            let trace_b =
                Trace::from_json(&json_b).map_err(|e| anyhow::anyhow!("trace B: {}", e))?;
            let comparison = trace_a.compare(&trace_b);
            println!("{}", comparison.summary);
            if comparison.matches {
                std::process::exit(0);
            } else {
                std::process::exit(1);
            }
        }
        Commands::FuzzSmoke { seeds } => {
            println!("running fuzz smoke test ({} seeds)...", seeds);
            let mut failures = Vec::new();
            for seed in 1..=seeds {
                let result = arc_difftest::fuzz::fuzz_valid(seed);
                if result.parse_panicked {
                    failures.push(format!("seed {}: parser panicked", seed));
                }
                if result.verify_panicked {
                    failures.push(format!("seed {}: verifier panicked", seed));
                }
            }
            for seed in 1..=seeds {
                let result = arc_difftest::fuzz::fuzz_garbage(seed);
                if result.parse_panicked {
                    failures.push(format!("garbage seed {}: parser panicked", seed));
                }
                if result.verify_panicked {
                    failures.push(format!("garbage seed {}: verifier panicked", seed));
                }
            }
            if failures.is_empty() {
                println!("all {} seeds passed (valid + garbage)", seeds * 2);
            } else {
                for f in &failures {
                    eprintln!("FAIL: {}", f);
                }
                anyhow::bail!("{} failures", failures.len());
            }
        }
        Commands::Test {
            files,
            expect,
            parse_only,
        } => {
            let mut test_files: Vec<PathBuf> = Vec::new();
            for path in &files {
                if path.is_dir() {
                    // Collect all .air files in the directory
                    for entry in fs::read_dir(path)
                        .with_context(|| format!("failed to read directory {}", path.display()))?
                    {
                        let entry = entry?;
                        let p = entry.path();
                        if p.extension().is_some_and(|e| e == "air" || e == "arc") {
                            test_files.push(p);
                        }
                    }
                } else {
                    test_files.push(path.clone());
                }
            }
            test_files.sort();

            let mut passed = 0u32;
            let mut failed = 0u32;
            let mut errors: Vec<String> = Vec::new();

            for test_file in &test_files {
                let source = match fs::read_to_string(test_file) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.push(format!("{}: read error: {}", test_file.display(), e));
                        failed += 1;
                        continue;
                    }
                };

                // Parse
                let module = match parse_module(&source) {
                    Ok(m) => m,
                    Err(e) => {
                        errors.push(format!("{}: parse error: {}", test_file.display(), e));
                        failed += 1;
                        continue;
                    }
                };

                if parse_only {
                    passed += 1;
                    continue;
                }

                // Verify
                if let Err(e) = verify_module(&module) {
                    errors.push(format!("{}: verify error: {}", test_file.display(), e));
                    failed += 1;
                    continue;
                }

                // Run if --expect is given
                if let Some(expected) = expect {
                    match run_main(&module) {
                        Ok(Some(value)) if value == expected => {}
                        Ok(Some(value)) => {
                            errors.push(format!(
                                "{}: expected {} but got {}",
                                test_file.display(),
                                expected,
                                value,
                            ));
                            failed += 1;
                            continue;
                        }
                        Ok(None) => {
                            errors.push(format!(
                                "{}: expected {} but got no value",
                                test_file.display(),
                                expected,
                            ));
                            failed += 1;
                            continue;
                        }
                        Err(e) => {
                            errors.push(format!("{}: runtime error: {}", test_file.display(), e,));
                            failed += 1;
                            continue;
                        }
                    }
                }

                passed += 1;
            }

            for err in &errors {
                eprintln!("FAIL: {}", err);
            }
            println!("{} passed, {} failed", passed, failed);
            if failed > 0 {
                std::process::exit(1);
            }
        }
        Commands::Security {
            file,
            confidential,
            tainted,
            sandbox,
            max_level,
        } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;

            let ctx = build_security_context(confidential.as_deref(), tainted.as_deref());
            let policy = build_sandbox_policy(sandbox.as_deref(), &max_level);

            let violations = check_information_flow(&module, &ctx, policy.as_ref());

            if violations.is_empty() {
                println!("no security violations found in {}", file.display());
            } else {
                println!(
                    "{} security violation(s) in {}:",
                    violations.len(),
                    file.display()
                );
                for (i, v) in violations.iter().enumerate() {
                    println!("  {}. {}", i + 1, v);
                }
                std::process::exit(1);
            }
        }
        Commands::Audit { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;

            let ctx = SecurityContext::new();
            let audit = audit_module(&module, &ctx, None);

            println!("Security audit for {}:", file.display());
            println!("  capability invocations: {}", audit.capability_invocations);
            println!("  approval requests:      {}", audit.approval_requests);
            println!("  capabilities used:      {:?}", audit.capabilities_used);
            println!("  external effects:       {}", audit.has_external_effects);
            println!("  handles credentials:    {}", audit.handles_credentials);
            if audit.violations.is_empty() {
                println!("  violations:             none");
            } else {
                println!("  violations ({}):", audit.violations.len());
                for (i, v) in audit.violations.iter().enumerate() {
                    println!("    {}. {}", i + 1, v);
                }
            }
        }
        Commands::Explain { file, event } => {
            let json = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let trace = Trace::from_json(&json).map_err(|e| anyhow::anyhow!("{}", e))?;
            let matches = trace.filter_events(&event);
            println!("{} events matching '{}':", matches.len(), event);
            for ev in &matches {
                println!("  {}", ev);
            }
        }
    }
    Ok(())
}

fn build_security_context(confidential: Option<&str>, tainted: Option<&str>) -> SecurityContext {
    let mut ctx = SecurityContext::new();

    if let Some(confidential_values) = confidential {
        for value in confidential_values.split(',') {
            let value = value.trim();
            if !value.is_empty() {
                ctx.set_level(value, SecurityLevel::Confidential);
            }
        }
    }

    if let Some(tainted_values) = tainted {
        for value in tainted_values.split(',') {
            let value = value.trim();
            if !value.is_empty() {
                ctx.add_taint(value, TaintLabel::UserInput);
            }
        }
    }

    ctx
}

fn build_sandbox_policy(sandbox: Option<&str>, max_level: &str) -> Option<SandboxPolicy> {
    let sandbox = sandbox?;
    let level = SecurityLevel::parse(max_level).unwrap_or(SecurityLevel::Confidential);
    let mut policy = SandboxPolicy::new(level);

    for capability in sandbox.split(',') {
        let capability = capability.trim();
        if let Some(denied) = capability.strip_prefix('!') {
            policy.deny(denied);
        } else if !capability.is_empty() {
            policy.allow(capability);
        }
    }

    Some(policy)
}

fn print_extended_issues(result: &arc_verify::ExtendedVerifyResult) {
    if !result.security_violations.is_empty() {
        eprintln!(
            "{} security violation(s):",
            result.security_violations.len()
        );
        for (idx, violation) in result.security_violations.iter().enumerate() {
            eprintln!("  {}. {}", idx + 1, violation);
        }
    }

    if !result.memory_violations.is_empty() {
        eprintln!("{} memory violation(s):", result.memory_violations.len());
        for (idx, violation) in result.memory_violations.iter().enumerate() {
            eprintln!("  {}. {}", idx + 1, violation);
        }
    }

    if !result.unproved_obligations.is_empty() {
        eprintln!(
            "{} unproved obligation(s):",
            result.unproved_obligations.len()
        );
        for (idx, obligation) in result.unproved_obligations.iter().enumerate() {
            eprintln!("  {}. {}", idx + 1, obligation);
        }
    }
}
