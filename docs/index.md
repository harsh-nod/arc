---
layout: default
title: ARC
---

<div class="hero-intro">
  <h1>The Capability-Aware Compiler IR</h1>
  <p class="hero-tagline">
    Effects. Authority. Proofs. Traces.<br>One IR for programs that act in the real world.
  </p>
  <p class="hero-subtitle"><em>Opaque calls are not enough when software can send, spend, mutate, and persist.</em></p>
  <div class="hero-actions">
    <a class="action primary" href="{{ '/getting-started' | relative_url }}">Get Started</a>
    <a class="action" href="{{ '/authority-effects' | relative_url }}">Authority and Effects</a>
    <a class="action" href="https://github.com/harsh-nod/arc">View on GitHub</a>
  </div>
</div>

## How It Works

<div class="pipeline">
  <div class="pipeline-node">parse</div>
  <div class="pipeline-arrow"><svg viewBox="0 0 32 12"><line x1="0" y1="6" x2="24" y2="6"/><polygon points="24,2 32,6 24,10"/></svg></div>
  <div class="pipeline-node">verify</div>
  <div class="pipeline-arrow"><svg viewBox="0 0 32 12"><line x1="0" y1="6" x2="24" y2="6"/><polygon points="24,2 32,6 24,10"/></svg></div>
  <div class="pipeline-node">optimize</div>
  <div class="pipeline-arrow"><svg viewBox="0 0 32 12"><line x1="0" y1="6" x2="24" y2="6"/><polygon points="24,2 32,6 24,10"/></svg></div>
  <div class="pipeline-node">lower</div>
  <div class="pipeline-arrow"><svg viewBox="0 0 32 12"><line x1="0" y1="6" x2="24" y2="6"/><polygon points="24,2 32,6 24,10"/></svg></div>
  <div class="pipeline-node">codegen</div>
</div>

ARC is a compiler intermediate representation for programs that interact with the outside world through **capabilities**: external actions like sending email, reading files, calling APIs, or writing files. Capability declarations expose effects, failure modes, and authority requirements to the verifier, optimizer, runtime, and audit trail.

<div class="demo-grid">
<div class="demo-panel demo-danger" markdown="1">
<h4>Traditional IR</h4>

```llvm
call i64 @email_send(i64 %to, i64 %body)
```

The compiler sees a call. It does not know whether the call is pure, reversible, networked, financial, authorized, retryable, or safe to reorder.
</div>

<div class="demo-panel demo-safe" markdown="1">
<h4>ARC</h4>

```air
arc.capability @email.send {
  inputs(%to: i64, %body: i64)
  outputs(%status: i64)
  effects [#arc.effect<network>, #arc.effect<irreversible>]
  failures [#arc.fail<delivery_error>]
}

%auth = arc.require_approval %to, %body : !arc.auth<email.send>
%status = arc.invoke @email.send(%to, %body)
```

The compiler can verify authority, preserve effects, detect confidential leaks, and emit an execution trace.
</div>
</div>

## Why ARC?

<div class="landing-grid">
  <article class="card">
    <h3>Capabilities are declared</h3>
    <p>Every external action has a name, typed inputs and outputs, declared effects, and enumerated failure modes.</p>
  </article>
  <article class="card">
    <h3>Authority is explicit</h3>
    <p><code>arc.invoke</code> requires an available matching <code>!arc.auth&lt;capability&gt;</code> token produced by <code>require_approval</code>.</p>
  </article>
  <article class="card">
    <h3>Effects are tracked</h3>
    <p>The effect lattice distinguishes memory, filesystem, network, irreversible, financial, credential, and other observable effects.</p>
  </article>
  <article class="card">
    <h3>Security is built in</h3>
    <p>Information-flow, taint, sandbox, proof, and memory checks run over the IR instead of relying on prompt discipline.</p>
  </article>
</div>

## Quick Start

```bash
git clone https://github.com/harsh-nod/arc.git
cd arc
cargo build --release
cargo install --path crates/arc_cli

arcc verify examples/send_email.air
arcc audit examples/send_email.air
arcc run examples/hello.air
arcc codegen examples/hello.air --target wasm32
```

## Documentation

| Page | Description |
|------|-------------|
| [Getting Started](getting-started) | Install, first program, CLI walkthrough |
| [Language Reference](language-reference) | Complete `.air` syntax and operation reference |
| [Examples](examples) | Annotated programs with capabilities, effects, branching, async |
| [CLI Reference](cli) | Command-by-command reference for `arcc` |
| [Authority and Effects](authority-effects) | Capability effects, approval tokens, and verifier rules |
| [Architecture](architecture) | Compiler pipeline, crate map, stage descriptions |
| [Security](security) | Information-flow analysis, taint tracking, sandbox policies |
| [Conformance](conformance) | Test suites, fuzz smoke tests, and release verification |
| [Package and Binary Format](package-format) | Experimental module/package serialization |
| [Release Guide](release) | Maintainer checklist for public releases |

## Project Status

ARC is an early-stage research project. The core pipeline works end-to-end for the documented subset: parsing, verification, interpretation, traces, optimization, lowering, x86-64 codegen, WebAssembly codegen, security analysis, proof checks, and package-format experiments.

Still evolving: dialect extensibility, SMT-backed proof solving, runtime capability dispatch, dependent types, and stable binary/package formats.
