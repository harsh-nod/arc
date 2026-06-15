You are an expert compiler/runtime/agent-systems engineering agent.

Your task is to design and implement AIR: a next-generation intermediate representation intended to replace MLIR and LLVM IR while also serving as the native IR for agents, applications, runtimes, languages, and machine-code generation.

Status note:

The active implementation has been renamed from AIR to ARC. This file remains the original AIR plan and roadmap; references to `air.*`, AIR-Core, and `airc` map to the current `arc.*`, ARC-Core, and `arc_cli` implementation unless a section is explicitly historical. Keep this plan as the long-range design target, and track current implementation maturity in `README.md`.

AIR should be treated as a sovereign replacement IR, not as a layer above MLIR/LLVM.

Core thesis:

AIR is a multi-level, SSA/resource-based, dependently typed, effect-aware, policy-aware intermediate representation for verified transformation of values, memory, capabilities, and world state, lowering from human or program intent to machine code, distributed execution, application actions, and agent runtime effects.

AIR must support ordinary compiler workloads, agentic workflows, low-level backend/codegen workloads, runtime orchestration, policy enforcement, traceability, and native object-code emission.

Do not depend on LLVM or MLIR as a backend. Optional comparison tests are allowed, but AIR must stand alone.

============================================================
SECTION 1 — FOUNDATIONAL REQUIREMENTS
============================================================

AIR must support:

1. SSA values.
2. Explicit resource values.
3. Dependent/indexed/refinement types.
4. Proof-carrying operations.
5. Explicit effect system.
6. Authority tokens.
7. Policy proofs.
8. Provenance.
9. Trust labels.
10. Runtime capability invocation.
11. Static modules and dynamic traces.
12. Multi-level lowering.
13. Verified transformations.
14. Low-level machine IR.
15. Target descriptions.
16. Instruction selection.
17. Register allocation.
18. Object emission.
19. Runtime ABI.
20. Agent-native workflows.
21. Memory model.
22. Concurrency.
23. Continuations.
24. Deterministic replay.
25. Information-flow security.
26. Privacy and purpose binding.
27. Cost/budget/objective-aware optimization.
28. Package format.
29. Fuzz/property/conformance tests.
30. Self-hosting path.

Core invariant:

Every AIR operation must expose its semantics.

An operation should declare:

- name
- operands
- results
- regions
- successors
- attributes
- types
- effects
- resource reads/writes
- authority requirements
- failure modes
- rollback/compensation behavior if applicable
- provenance
- trust classification
- lowering contract
- optimization contract

No dialect operation may be a semantic black box.

============================================================
SECTION 2 — REPOSITORY STRUCTURE
============================================================

Create a monorepo with this approximate layout:

air/
  spec/
    AIR-Core.md
    AIR-TypeSystem.md
    AIR-Effects.md
    AIR-Memory.md
    AIR-Authority.md
    AIR-Policy.md
    AIR-Provenance.md
    AIR-Lowering.md
    AIR-Trace.md
    AIR-RuntimeABI.md
    AIR-Target.md
    AIR-Object.md

  crates/
    air_syntax/
      Lexer, parser, source locations, diagnostics.

    air_ir/
      In-memory IR: modules, ops, regions, blocks, values, resources, attributes.

    air_types/
      Type system, dependent indices, refinements, proofs.

    air_verify/
      SSA verifier, type verifier, resource verifier, effect verifier, authority verifier.

    air_effects/
      Effect lattice, effect analysis, commutativity, scheduling legality.

    air_proof/
      Proof kernel, proof obligations, solver interface, proof erasure.

    air_pass/
      Pass manager, rewrite engine, canonicalization, refinement checking.

    air_interp/
      Reference interpreter, memory model, capability stubs, trace emitter.

    air_runtime/
      Capability ABI, sandbox runtime, fake providers, event log, replay.

    air_codegen/
      Low-level lowering, machine IR, instruction selection, regalloc.

    air_targets/
      x86_64_air/
      wasm_air/
      riscv64_air/
      aarch64_air/

    air_object/
      ELF/Mach-O/COFF object model, relocations, symbol tables.

    air_pkg/
      Package manifest, capability manifest, signing, versioning.

    air_cli/
      airc command-line interface.

  dialects/
    core/
    index/
    mem/
    control/
    tensor/
    data/
    rel/
    doc/
    effect/
    policy/
    agent/
    model/
    runtime/
    ui/
    machine/
    x86_64/
    wasm/
    riscv64/

  tests/
    parse/
    print/
    verify/
    typecheck/
    effects/
    authority/
    policy/
    proof/
    interp/
    opt/
    lower/
    codegen/
    runtime/
    agent/
    trace/
    security/
    package/
    fuzz/
    e2e/

  examples/
    hello.air
    safe_index.air
    matmul.air
    approval_flow.air
    send_email.air
    file_copy.air
    spreadsheet_summary.air
    tiny_language/

Use Rust for the reference implementation unless there is a strong reason not to. Rust is a good fit for compiler infrastructure, memory safety, enums, pattern matching, and robust tooling.

============================================================
SECTION 3 — AIR CLI
============================================================

Implement a CLI named airc.

Initial commands:

airc parse file.air
airc print file.air
airc verify file.air
airc run file.air
airc opt file.air --passes canonicalize,dce,cse
airc lower file.air --to air-cfg
airc codegen file.air --target x86_64-air-linux
airc trace run file.air
airc replay file.airtrace
airc explain file.airtrace --event EVENT
airc test package.airpkg
airc fuzz-smoke

Every command must have tests.

============================================================
SECTION 4 — MILESTONE 0: EXECUTABLE SPECIFICATION
============================================================

Goal:

Write a precise but implementable AIR-Core specification before building too much code.

Define:

- module structure
- symbols
- operation structure
- regions
- blocks
- SSA values
- resource values
- attributes
- types
- effects
- verification phases
- parser grammar
- printer format
- diagnostic format
- trace format

Minimal textual form:

air.module @m {
  air.func @add(%a: i64, %b: i64) -> i64 {
  ^entry:
    %c = air.add %a, %b : i64
    air.return %c : i64
  }
}

Resource example:

air.func @store_one(%mem0: !air.mem, %ptr: !air.ptr<i64>) -> !air.mem {
^entry:
  %one = air.const 1 : i64
  %mem1 = air.store %mem0, %ptr, %one
    effects [#air.effect<memory.write>]
    : (!air.mem, !air.ptr<i64>, i64) -> !air.mem

  air.return %mem1 : !air.mem
}

Tests:

tests/spec/valid-minimal-module.air
tests/spec/invalid-missing-type.air
tests/spec/invalid-use-before-def.air
tests/spec/invalid-region-scope.air
tests/spec/invalid-resource-duplication.air

Acceptance criteria:

- The grammar is written.
- The IR invariants are written.
- The verifier rules are stated as checkable conditions.
- The spec distinguishes values, resources, proofs, authority, policies, provenance, and effects.

============================================================
SECTION 5 — MILESTONE 1: PARSER, PRINTER, IR OBJECT MODEL
============================================================

Goal:

Implement the textual AIR format and in-memory IR.

Core objects:

- Context
- Module
- Operation
- Region
- Block
- Value
- Resource
- Type
- Attribute
- Effect
- Symbol
- Location
- Diagnostic

Parser requirements:

- Preserve source locations.
- Preserve symbol names.
- Preserve SSA value names.
- Preserve region nesting.
- Preserve attributes.
- Preserve types.
- Produce precise diagnostics.
- Never panic on malformed input.

Printer requirements:

parse -> print -> parse -> print must stabilize.

Tests:

1. Golden round-trip tests.

airc parse tests/parse/simple.air --print > out.air
airc parse out.air --print > out2.air
diff out.air out2.air

2. Parser error tests:

- missing closing brace
- unknown type
- unknown sigil
- unterminated string
- malformed attribute
- duplicate block argument
- duplicate symbol

3. Property tests:

Generate random small valid modules, print them, parse them, and assert structural equality.

4. Fuzz tests:

Feed arbitrary bytes to the parser. Assert:

- no panic
- no unbounded memory growth
- deterministic diagnostics
- bounded error recovery

Acceptance criteria:

- Parser is robust.
- Printer output is parseable.
- Round-trip stabilizes.
- Every IR node has source location info.

============================================================
SECTION 6 — MILESTONE 2: AIR-CORE VERIFIER
============================================================

Goal:

Implement the core verifier.

Verifier checks:

- symbol table validity
- region and block structure
- SSA dominance
- operand/result arity
- type checking
- terminator checking
- branch target checking
- function call validity
- resource linearity
- effect declaration well-formedness

Initial core ops:

- air.module
- air.func
- air.return
- air.const
- air.add
- air.sub
- air.mul
- air.div
- air.icmp
- air.br
- air.cond_br
- air.if
- air.loop
- air.call
- air.alloc
- air.load
- air.store

Positive tests:

- valid function call
- valid nested region
- valid if/else
- valid loop with iter_args
- valid memory store returning new memory resource
- valid branch arguments

Negative tests:

- use before definition
- value escapes region
- duplicate symbol
- wrong operand type
- wrong result type
- wrong branch argument count
- missing terminator
- resource used twice
- resource dropped without permission
- effectful op missing effect declaration
- call to missing function

Example negative test:

air.func @bad_resource(%mem0: !air.mem, %p: !air.ptr<i64>) -> !air.mem {
^entry:
  %x = air.const 1 : i64

  %mem1 = air.store %mem0, %p, %x
    effects [#air.effect<memory.write>]
    : (!air.mem, !air.ptr<i64>, i64) -> !air.mem

  // expected-error {{resource %mem0 already consumed}}
  %mem2 = air.store %mem0, %p, %x
    effects [#air.effect<memory.write>]
    : (!air.mem, !air.ptr<i64>, i64) -> !air.mem

  air.return %mem2 : !air.mem
}

Acceptance criteria:

- Invalid SSA is rejected.
- Invalid resource use is rejected.
- Diagnostics are precise.
- Verifier does not depend on optimizer output.

============================================================
SECTION 7 — MILESTONE 3: DEPENDENT TYPES, REFINEMENTS, PROOFS
============================================================

Goal:

Add a practical dependent type core.

Support:

- ordinary scalar types
- tuple types
- function types
- resource types
- indexed types
- refinement types
- proof types
- authority types
- existential packages

Example types:

!air.slice<i32, %n>
!air.tensor<f32, [%m, %k]>
!air.ptr<i64, region = %r, align = 8>
!air.proof<%i < %n>
!air.auth<action = @email.send, resource = %draft>
exists (%n: index). !air.slice<i32, %n>

Index language:

- integer constants
- symbolic index values
- addition
- subtraction
- multiplication by constants
- division by constants
- min/max
- comparisons
- boolean combinations
- shape expressions

Keep the initial fragment decidable and solver-friendly.

Proof operations:

- air.assume
- air.assert
- air.prove
- air.branch_proof
- air.refine
- air.rewrite_type
- air.pack
- air.unpack

Tests:

1. Shape tests.

air.func @matmul
  forall (%m: index, %k: index, %n: index)
  (%a: !air.tensor<f32, [%m, %k]>,
   %b: !air.tensor<f32, [%k, %n]>)
  -> !air.tensor<f32, [%m, %n]> {
  ...
}

Negative test:

// expected-error {{tensor inner dimensions do not match}}
air.call @matmul(%a_mk, %b_qn)

2. Bounds proof tests.

air.func @get
  forall (%n: index)
  (%xs: !air.slice<i32, %n>,
   %i: index,
   %pf: !air.proof<%i < %n>)
  -> i32 {
^entry:
  %x = air.load_elem %xs[%i]
    requires %pf
    : (!air.slice<i32, %n>, index) -> i32
  air.return %x : i32
}

Negative test:

// expected-error {{operation requires proof of %i < %n}}
%x = air.load_elem %xs[%i]

3. Existential package tests.

%pkg = air.read_dynamic_array %source
  : !air.source -> exists (%n: index). !air.slice<i32, %n>

air.unpack %pkg as (%n, %xs) {
  %len = air.length %xs : !air.slice<i32, %n> -> !air.index<value = %n>
}

Acceptance criteria:

- Indexed types work.
- Refinement facts are scoped by dominance.
- Proofs are checked.
- Proofs can be erased when safe.
- Runtime checks can be inserted when static proofs are unavailable.

============================================================
SECTION 8 — MILESTONE 4: EFFECTS, RESOURCES, AUTHORITY
============================================================

Goal:

Make side effects first-class and non-optional.

Initial effect lattice:

- pure
- memory.read
- memory.write
- allocate
- deallocate
- filesystem.read
- filesystem.write
- network
- database.read
- database.write
- ui
- llm
- human.approval
- external_communication
- external_mutation
- financial
- credential
- physical
- irreversible

Effect qualifiers:

- deterministic
- nondeterministic
- idempotent
- reversible
- transactional
- blocking
- async
- may_fail
- may_timeout
- speculatable
- commutative

Resource types:

!air.mem
!air.fs
!air.net
!air.db
!air.world
!air.clock
!air.rng
!air.ui
!air.gpu
!air.vault

Authority type:

!air.auth<principal = %user,
          action = @email.send,
          resource = %draft,
          valid_until = %deadline>

Tests:

1. Effect declaration tests.

Reject:

// expected-error {{operation performs external effect but declares pure}}
%msg, %world1 = air.invoke @email.send(%world0, %auth, %draft)
  effects []

2. Reordering tests.

The optimizer may reorder pure ops:

%x = air.add %a, %b : i64
%y = air.mul %c, %d : i64

The optimizer must not reorder non-commuting effects:

%world1 = air.invoke @email.send(%world0, ...)
%world2 = air.invoke @payment.submit(%world1, ...)

unless an explicit proof shows the effects commute.

3. Authority exactness tests.

%auth = air.require_approval %user, %draft_a
  : (!air.principal, !air.email_draft)
      -> !air.auth<action = @email.send, resource = %draft_a>

// expected-error {{authority token authorizes %draft_a, not %draft_b}}
%msg, %world1 = air.invoke @email.send(%world0, %auth, %draft_b)

4. Expired authority tests.

// expected-error {{authority token may be expired at use site}}
air.invoke @email.send(%world0, %expired_auth, %draft)

Acceptance criteria:

- Effects constrain optimization.
- Authority is a typed value.
- Dangerous operations require matching authority.
- Resource sequencing is explicit and checkable.

============================================================
SECTION 9 — MILESTONE 5: REFERENCE INTERPRETER AND TRACE ENGINE
============================================================

Goal:

Build a reference interpreter before the optimizer/backend becomes complex.

Interpreter supports:

- pure computation
- control flow
- memory allocation/load/store
- checked bounds
- dependent assertions
- fake filesystem
- fake network
- fake world
- fake capabilities
- authority requests
- failure injection
- trace emission

Trace example:

air.trace @run_001 {
  air.event @e1 invoke @main
  air.event @e2 memory.alloc %buf
  air.event @e3 proof.checked %pf
  air.event @e4 capability.proposed @email.send
  air.event @e5 human.approved %auth
  air.event @e6 capability.invoked @email.send
  air.event @e7 returned @main
}

Tests:

1. Interpreter correctness:

- integer arithmetic
- branching
- loops
- function calls
- recursion
- memory load/store
- bounds checks
- shape computations
- proof erasure
- trap behavior

2. Trace tests:

- trace includes all effectful operations
- trace records authority creation and use
- trace records runtime checks
- trace can replay deterministic runs

3. Replay tests:

airc run examples/send_email.air --trace out.airtrace
airc replay out.airtrace

Expected:

- same observable events
- same authority checks
- same capability outputs

4. Fault injection tests:

- network timeout
- permission denied
- policy revoked
- user rejected approval
- malformed tool output
- clock advanced past deadline

Acceptance criteria:

- Interpreter is the semantic reference.
- Every effectful behavior can be traced.
- Replay works for deterministic inputs.

============================================================
SECTION 10 — MILESTONE 6: PASS MANAGER AND REWRITE SYSTEM
============================================================

Goal:

Implement transformations with verification after every pass.

Each pass declares:

- input dialects
- output dialects
- preserved analyses
- required analyses
- effects it may introduce
- proof obligations it creates
- refinement relation

Initial passes:

- canonicalize
- constant_fold
- dce
- cse
- inline
- simplify_cfg
- lower_if_to_cfg
- lower_loop_to_cfg
- erase_proofs
- insert_runtime_checks
- effect_schedule
- authority_sinking

Declarative rewrite example:

air.rewrite @fold_add_zero {
  match {
    %y = air.add %x, 0 : i64
  }
  replace {
    %y = %x
  }
  preserves [#air.effects, #air.authority, #air.provenance]
}

Authority sinking example:

air.rewrite @sink_approval_to_exact_draft {
  match {
    %auth = air.require_approval %user, %goal
    %draft = air.invoke @email.compose(%recipient, %body)
    %msg, %world1 = air.invoke @email.send(%world0, %auth, %draft)
  }
  replace {
    %draft = air.invoke @email.compose(%recipient, %body)
    %auth = air.require_approval %user, %draft
    %msg, %world1 = air.invoke @email.send(%world0, %auth, %draft)
  }
  requires [
    #air.proof<email.compose_has_no_external_effect>,
    #air.proof<approval_scope_can_narrow>
  ]
}

Tests:

1. Classic optimization tests:

- constant folding
- dead pure code elimination
- CSE
- inline small function
- simplify branch on constant
- remove unreachable block

2. Effect safety tests:

DCE must not remove:

%world1 = air.invoke @email.send(%world0, %auth, %draft)
%world2 = air.invoke @file.delete(%world1, %path)

3. Proof preservation tests:

Before:

%pf = air.prove %i < %n
%x = air.load_elem %xs[%i] requires %pf

After optimization, either:

- proof still exists
- runtime check is inserted
- program is rejected

4. Metamorphic tests:

For every optimization:

- run original in interpreter
- run optimized in interpreter
- compare return value
- compare observable trace
- assert no new unauthorized effects
- assert no weakened policies

Acceptance criteria:

- Every pass verifies its output.
- Every pass has positive and negative tests.
- Effectful transformations require explicit proof.
- Optimized programs preserve observable semantics.

============================================================
SECTION 11 — MILESTONE 7: RUNTIME ABI AND CAPABILITY SYSTEM
============================================================

Goal:

Define how AIR invokes external capabilities.

Capability declaration example:

air.capability @email.send
  inputs(
    %world: !air.world,
    %auth: !air.auth<action = @email.send, resource = %draft>,
    %draft: !air.email_draft
  )
  outputs(
    %msg: !air.sent_message,
    %world1: !air.world
  )
  effects [
    #air.effect<external_communication>,
    #air.effect<network>,
    #air.effect<irreversible>
  ]
  failures [
    #air.fail<network_timeout>,
    #air.fail<permission_denied>,
    #air.fail<recipient_invalid>
  ]

Provider API:

- describe()
- validate(action)
- invoke(action)
- dry_run(action)
- rollback(action)
- subscribe(query)
- explain(ref)

Fake providers:

- fake_email
- fake_calendar
- fake_filesystem
- fake_model
- fake_human
- fake_policy
- fake_clock

Tests:

- capability discovery returns schemas
- invoke validates input types
- invoke rejects missing authority
- dry_run produces no external effect
- rollback works only when declared
- event log records invoke and result
- policy revocation invalidates prior proof
- malformed tool output is rejected or marked untrusted

Prompt-injection boundary test:

%body = air.content.from_email %email
  trust = #air.trust<third_party_content>

// expected-error {{third-party content cannot be used as instruction authority}}
%goal = air.goal.from_instruction %body

Acceptance criteria:

- External actions are never untyped calls.
- Capabilities expose effects, failures, rollback, and authority requirements.
- Fake runtime can test workflows deterministically.

============================================================
SECTION 12 — MILESTONE 8: MEMORY MODEL
============================================================

Goal:

Replace implicit undefined behavior with explicit memory facts, provenance, and proofs.

Memory concepts:

- allocation
- region
- pointer
- reference
- slice
- borrow
- lifetime
- alignment
- initialization
- alias set
- address space
- provenance
- volatile
- atomic
- external-state pointer

Example:

%alloc, %mem1 = air.alloc %mem0, size = %n, align = 8
  : (!air.mem, index) -> (!air.ptr<i64, region = %r>, !air.mem)

%pf_bounds = air.prove in_bounds(%alloc, %i)
%pf_init = air.prove initialized(%alloc, %i)

%v = air.load %mem1, %alloc[%i]
  requires [%pf_bounds, %pf_init]
  : (!air.mem, !air.ptr<i64>) -> i64

Tests:

- load uninitialized memory rejected or traps
- store initializes memory
- out-of-bounds pointer rejected or checked
- use-after-free rejected
- double-free rejected
- alias proof permits optimization
- noalias proof blocks invalid aliasing
- volatile prevents reordering
- atomic ordering respected
- unique borrow cannot alias mutable reference

Acceptance criteria:

- Memory safety facts are explicit.
- Optimizer does not rely on invisible UB.
- Memory checks can be static, dynamic, or rejected.

============================================================
SECTION 13 — MILESTONE 9: HIGH-LEVEL DIALECTS
============================================================

Goal:

Add domain dialects only after core semantics are stable.

Initial dialects:

- air.tensor
- air.data
- air.rel
- air.doc
- air.stream
- air.policy
- air.agent
- air.model
- air.ui

Tensor example:

%out = air.tensor.matmul %a, %b
  : (!air.tensor<f32, [%m, %k]>,
     !air.tensor<f32, [%k, %n]>)
      -> !air.tensor<f32, [%m, %n]>

Relational example:

%orders = air.rel.scan @orders
%big = air.rel.filter %orders where (%amount > 10000)
%joined = air.rel.join %big, %customers on customer_id

Agent example:

air.task @send_report(%world0: !air.world, %user: !air.principal) -> !air.world {
  %doc = air.invoke @files.latest("Q2 forecast")
  %summary = air.invoke @model.summarize(%doc)
  %draft = air.invoke @email.compose(%summary)
  %auth = air.require_approval %user, %draft
  %msg, %world1 = air.invoke @email.send(%world0, %auth, %draft)
  air.return %world1
}

Tests:

- tensor shape inference
- invalid tensor dimensions
- relational query lowering
- document span provenance
- model output marked model_generated
- model output cannot be used as proof
- UI lowering preserves semantic action

Acceptance criteria:

- Every dialect op declares effects.
- Every dialect op has verifier tests.
- Every dialect op has lowering or interpreter semantics.
- No dialect op is a black box.

============================================================
SECTION 14 — MILESTONE 10: LOWERING AND REFINEMENT
============================================================

Goal:

Implement progressive lowering inside AIR.

Pipeline:

AIR-HL
  -> AIR-Core
  -> AIR-CFG
  -> AIR-Mem
  -> AIR-MachineIndependent
  -> AIR-Target

Every lowering emits or checks a refinement record.

Example:

air.refinement @lower_if_to_cfg {
  source = @foo.air.hl
  target = @foo.air.cfg
  preserves [
    #air.prop<same_return_value>,
    #air.prop<same_effect_trace>,
    #air.prop<same_authority_requirements>
  ]
}

Tests:

- if -> cond_br
- loop -> blocks and branches
- tensor matmul -> nested loops
- safe load -> checked or unchecked load
- agent invoke -> runtime ABI call
- capability call -> provider dispatch
- proofs -> erased or runtime checks

Semantic preservation test:

airc run input.air --trace before.trace
airc lower input.air --to air-cfg -o lowered.air
airc run lowered.air --trace after.trace
airc trace-compare before.trace after.trace

Negative lowering tests reject transformations that:

- drop authority
- drop irreversible effects
- weaken policy requirements
- use unchecked load without proof
- use target instruction without target feature proof

Acceptance criteria:

- Lowering is semantic, not textual.
- Every major lowering has trace-equivalence tests.
- Policy/effect/authority obligations are preserved.

============================================================
SECTION 15 — MILESTONE 11: LOW-LEVEL AIR AND NATIVE CODEGEN
============================================================

Goal:

Replace LLVM’s backend role.

Start with target:

x86_64-air-linux-elf

Then add:

- wasm32-air
- riscv64-air-elf
- aarch64-air

Low-level stages:

AIR-M:
  machine-independent low-level ops

AIR-Legal:
  target-legal ops and types only

AIR-Target:
  target-specific virtual-register instructions

AIR-Phys:
  physical registers, stack slots, frame layout

AIR-Obj:
  sections, symbols, relocations, object emission

Backend components:

- target description format
- type legalization
- operation legalization
- calling convention lowering
- instruction selection
- virtual registers
- liveness analysis
- interference graph
- register allocation
- spill insertion
- stack frame layout
- prologue/epilogue insertion
- instruction scheduling
- branch relaxation
- object emission
- debug mapping

Target description example:

air.target @x86_64_air_linux {
  pointer_size = 64
  endian = little
  object_format = elf
  calling_convention = @sysv_amd64

  registers {
    class @gpr64 = [rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp, r8, r9, r10, r11, r12, r13, r14, r15]
  }

  legal_types [i1, i8, i16, i32, i64, f32, f64]

  instruction @ADD64rr {
    operands(%dst: @gpr64, %a: @gpr64, %b: @gpr64)
    pattern = air.add %a, %b : i64
    latency = 1
    encoding = ...
  }
}

Tests:

1. Backend unit tests:

- instruction selection
- legalization
- liveness
- interference graph
- register allocation
- spilling
- stack slot assignment
- calling convention argument placement
- return value placement
- relocation encoding
- object section layout

2. Golden machine tests.

Input:

air.func @add(%a: i64, %b: i64) -> i64 {
  %c = air.add %a, %b : i64
  air.return %c : i64
}

Expected target AIR:

x86_64.func @add {
  %v0 = x86_64.copy %rdi
  %v1 = x86_64.copy %rsi
  %v2 = x86_64.add64rr %v0, %v1
  x86_64.ret %v2
}

3. Executable tests:

airc codegen tests/codegen/add.air --target x86_64-air-linux -o add.o
airc link add.o -o add
./add

4. Differential tests:

- run in AIR interpreter
- compile to native
- run native
- compare return value and observable trace

5. ABI tests:

- integer args
- float args
- mixed args
- many args requiring stack
- struct return
- foreign C call later
- varargs later

Acceptance criteria:

- AIR compiles simple programs to native object code without LLVM.
- Backend output agrees with interpreter.
- Calling convention tests pass.
- Object files link and execute.

============================================================
SECTION 16 — MILESTONE 12: OBJECT, LINKER, PACKAGE FORMATS
============================================================

Goal:

AIR needs its own serialization and packaging model.

Formats:

.air:
  textual IR

.airb:
  binary IR

.airtrace:
  execution trace

.airpkg:
  package containing modules, policies, capabilities, proofs, target variants

.airo:
  AIR object file or native object wrapper

Package example:

air.package @report_agent {
  modules [@main]

  requires [
    @capability.email.compose,
    @capability.email.send,
    @capability.files.read,
    @capability.model.summarize
  ]

  policies [
    @policy.require_approval_for_external_send
  ]

  targets [
    @x86_64_air_linux,
    @wasm32_air,
    @air_runtime
  ]
}

Tests:

- textual/binary round-trip
- package hash stability
- content-addressed artifact identity
- missing capability rejected
- wrong dialect version rejected
- migration test v1 -> v2
- signature verification
- tampered package rejected
- trace package replay

Acceptance criteria:

- AIR modules can be stored, transported, signed, versioned, and replayed.
- Package validation checks capabilities and policies before execution.

============================================================
SECTION 17 — MILESTONE 13: AGENT-NATIVE EXECUTION
============================================================

Goal:

Prove AIR’s unique value with safe agent workflows.

Initial examples:

- compose email but do not send
- send email with approval
- schedule calendar event with approval
- summarize private document without retention
- copy file with rollback
- query spreadsheet and run numeric computation
- detect prompt injection in untrusted document

Workflow example:

air.task @send_summary(
  %world0: !air.world,
  %user: !air.principal,
  %doc: !air.document<classification = confidential>,
  %recipient: !air.person
) -> !air.world {
  %summary = air.invoke @model.summarize(%doc)
    effects [#air.effect<llm>, #air.effect<read_private>]
    output_trust = #air.trust<model_generated>

  %policy_pf = air.policy.check @company_policy, %user, %recipient, %summary
    : !air.proof<allowed_to_send(%user, %recipient, %summary)>

  %draft = air.invoke @email.compose(%recipient, %summary)
    effects [#air.effect<local_draft>]

  %auth = air.require_approval %user, %draft
    : (!air.principal, !air.email_draft)
      -> !air.auth<action = @email.send, resource = %draft>

  %msg, %world1 = air.invoke @email.send(%world0, %auth, %policy_pf, %draft)
    effects [
      #air.effect<external_communication>,
      #air.effect<irreversible>
    ]

  air.return %world1
}

Tests:

- send rejected without approval
- send rejected with approval for wrong draft
- send rejected if draft changed after approval
- send rejected if policy revoked
- send rejected if recipient uncertain
- model output marked untrusted/model-generated
- untrusted content cannot become instruction
- confidential document cannot flow to public tool
- dry-run creates no external event
- trace explains every external action

Acceptance criteria:

- AIR expresses agent workflows better than traditional compiler IR.
- Policy and authority are enforced by verifier/runtime, not convention.

============================================================
SECTION 18 — MILESTONE 14: ADVANCED OPTIMIZER
============================================================

Goal:

Optimize both traditional compute code and agent/runtime workflows.

Classic passes:

- constant folding
- DCE
- CSE
- LICM
- inlining
- escape analysis
- bounds-check elimination
- loop unrolling
- vectorization
- allocation sinking
- devirtualization

AIR-specific passes:

- effect-aware scheduling
- authority sinking
- policy hoisting
- privacy minimization
- capability batching
- UI-to-API replacement
- proof erasure
- runtime-check insertion
- rollback synthesis
- trace minimization
- placement optimization
- data-movement minimization
- model-call caching

Tests:

1. Bounds-check elimination.

Before:

air.assert %i < %n
%x = air.checked_load_elem %xs[%i]

After:

%x = air.load_elem %xs[%i] requires %pf

Only valid when proof dominates the load.

2. Authority sinking.

Before:

%auth = air.require_approval %user, %goal
%summary = air.invoke @model.summarize(%doc)
%draft = air.invoke @email.compose(%summary)
air.invoke @email.send(%world, %auth, %draft)

After:

%summary = air.invoke @model.summarize(%doc)
%draft = air.invoke @email.compose(%summary)
%auth = air.require_approval %user, %draft
air.invoke @email.send(%world, %auth, %draft)

Only valid if preceding operations have no external irreversible effect.

3. Privacy minimization.

Before:

%summary = air.invoke @external_model.summarize(%full_doc)

After:

%relevant = air.doc.extract_relevant_spans %full_doc
%summary = air.invoke @external_model.summarize(%relevant)

Only valid with proof that extracted spans are sufficient.

Acceptance criteria:

- Optimizer improves both numeric code and agent workflows.
- All optimizations are effect-, proof-, policy-, and authority-aware.

============================================================
SECTION 19 — MILESTONE 15: CONCURRENCY, CONTINUATIONS, DISTRIBUTION
============================================================

Goal:

Support async runtimes, distributed systems, agents, and long-running tasks.

Features:

- async
- await
- structured concurrency
- actors
- cancellation
- timeouts
- continuations
- checkpoint/resume
- distributed placement
- deterministic replay boundaries

Example:

air.parallel {
  %contacts_f = air.async @contacts.resolve(%name)
  %calendar_f = air.async @calendar.availability(%week)
  %files_f = air.async @files.search("Q2 forecast")
} join {
  %contact = air.await %contacts_f
  %calendar = air.await %calendar_f
  %file = air.await %files_f
}

Continuation example:

%k = air.suspend
  reason = #air.reason<waiting_for_user_approval>
  captures [%draft, %world0]
  : !air.continuation<resume_with = !air.auth<action = @email.send, resource = %draft>>

Tests:

- async tasks join deterministically when effects commute
- non-commuting effects cannot run in parallel without proof
- cancellation releases resources
- timeout path produces typed failure
- continuation captures required values
- resumed continuation validates stale authority
- checkpoint/replay reproduces event order
- distributed placement respects data policy

Acceptance criteria:

- AIR represents async, human-in-the-loop, and distributed workflows natively.
- Continuation state is typed and serializable.

============================================================
SECTION 20 — MILESTONE 16: INFORMATION FLOW, PRIVACY, SANDBOXING
============================================================

Goal:

Make AIR safe for real agents and applications.

Security labels:

!air.document<label = confidential>
!air.text<label = public>
!air.channel<label = external>

Declassification:

%pf = air.policy.declassify @policy, %doc
  : !air.proof<may_declassify(%doc, public)>

%public_doc = air.declassify %doc using %pf
  : !air.document<label = confidential>
    -> !air.document<label = public>

Sandboxing:

%cap = air.sandbox {
  allow @spreadsheet.read_range(%sheet, %range)
  deny @network.*
  deny @filesystem.write
}

Tests:

- confidential -> public flow rejected
- declassification requires proof
- model provider cannot receive private data if policy forbids retention
- sandboxed subagent cannot access undelegated capability
- capability attenuation cannot be widened by callee
- credential values cannot be logged
- prompt-injected content cannot call tools

Acceptance criteria:

- Information flow is statically checkable where possible.
- Runtime enforces sandbox boundaries.
- Traces do not leak secret values unless policy allows.

============================================================
SECTION 21 — MILESTONE 17: SELF-HOSTING PATH
============================================================

Goal:

AIR should eventually compile its own compiler.

Steps:

1. Build initial airc in Rust.
2. Define a small source language: MiniAIR.
3. Compile MiniAIR to AIR.
4. Rewrite small verifier components in MiniAIR.
5. Compile those components with AIR backend.
6. Replace Rust components one at a time.
7. Eventually compile airc_core with AIR.

Self-hosting tests:

- Rust verifier and AIR-compiled verifier agree.
- Rust parser and AIR parser agree on test corpus.
- AIR-compiled optimizer produces same output as host optimizer.
- stage1 compiler builds stage2 compiler.
- stage2 compiler builds identical or semantically equivalent stage3 compiler.

Acceptance criteria:

- AIR can express serious compiler infrastructure.
- AIR has a path to self-hosting.

============================================================
SECTION 22 — TEST TAXONOMY
============================================================

Every feature should have:

1. Positive test:
   valid AIR is accepted and runs.

2. Negative test:
   invalid AIR is rejected with precise diagnostics.

3. Transformation test:
   optimization/lowering preserves semantics, effects, authority, policies, and trace obligations.

Test categories:

1. Golden tests

Used for parser, printer, canonicalizer, lowering, codegen.

airc opt input.air --passes canonicalize | diff - expected.air

2. Negative verifier tests

Every verifier rule gets invalid examples with expected diagnostics.

// expected-error {{authority token does not dominate use}}

3. Property tests

Generate random valid programs and check:

- parse-print stability
- verifier consistency
- renaming invariance
- optimization preserves interpreter result
- resource linearity remains valid after rewrite

4. Fuzz tests

Targets:

- lexer
- parser
- binary format reader
- verifier
- rewriter
- package reader
- trace replay
- object reader

Assertions:

- no crash
- no unbounded memory growth
- no verifier panic
- no unsafe behavior

5. Metamorphic tests

For program P and transform T:

- interp(P) == interp(T(P))
- trace(P) equivalent to trace(T(P))
- policy obligations of T(P) are no weaker than P
- no new unauthorized effects

6. Differential tests

Compare:

- AIR interpreter vs AIR native backend
- AIR interpreter vs AIR VM
- optimized vs unoptimized
- checked vs proof-erased
- fake runtime provider vs simulated provider

7. Proof tests

- valid proof accepted
- invalid proof rejected
- proof erasure preserves executable behavior
- runtime check inserted when proof absent
- solver certificate checked by kernel

8. Security tests

- tainted content cannot become instruction
- private data cannot flow to public sink
- capability cannot be widened
- stale approval cannot be reused
- approval for wrong draft rejected
- policy revocation invalidates proof

9. Backend tests

- instruction selection
- legalization
- register allocation
- spilling
- calling convention
- object emission
- relocations
- debug info
- native execution
- interpreter/native equivalence

10. Runtime and agent tests

- tool timeout
- malformed tool output
- user rejects approval
- approval expires
- capability unavailable
- rollback succeeds
- rollback unavailable
- trace replay
- dry-run no-op guarantee

11. Performance regression tests

Track:

- parser throughput
- verifier throughput
- optimizer time
- codegen time
- native runtime performance
- memory use
- trace size
- package load time

12. Conformance suite

Create a public AIR conformance suite:

- valid modules every implementation must accept
- invalid modules every implementation must reject
- semantic tests every interpreter/backend must match
- trace tests every runtime must emit
- policy tests every agent runtime must enforce

============================================================
SECTION 23 — CI PIPELINE
============================================================

Every commit should run:

cargo test
airc verify tests/verify/**/*.air
airc test tests/conformance
airc fuzz-smoke
airc opt tests/e2e/**/*.air --passes all --verify-each
airc run tests/e2e/**/*.air
airc codegen tests/codegen/**/*.air --target x86_64-air-linux
airc trace-check tests/trace/**/*.air

CI stages:

1. format
2. lint
3. unit tests
4. golden tests
5. negative verifier tests
6. property tests
7. fuzz smoke
8. interpreter tests
9. optimizer tests
10. runtime tests
11. backend tests
12. package tests
13. security tests
14. performance smoke

Nightly CI:

- long fuzzing
- random program generation
- native backend differential tests
- multi-target tests
- stress tests
- large package tests
- trace replay corpus
- self-hosting build

============================================================
SECTION 24 — FIRST VERTICAL SLICE
==========================================================

The first meaningful demo should prove the AIR thesis.

Build an AIR program that:

1. Computes a summary statistic over a typed slice.
2. Proves bounds safety.
3. Drafts an email.
4. Requires user approval.
5. Sends through fake email capability.
6. Emits a trace.
7. Optimizes pure computation.
8. Refuses to optimize away external effects.

Run:

airc verify demo.air
airc run demo.air --trace demo.trace
airc opt demo.air --passes canonicalize,dce,cse,effect-schedule --verify-each -o demo.opt.air
airc run demo.opt.air --trace demo.opt.trace
airc trace-compare demo.trace demo.opt.trace
airc explain demo.opt.trace --event email.send

Expected result:

- same observable trace
- same computed result
- email send occurs only after approval
- DCE removes dead pure computation
- DCE does not remove capability invocation
- explanation shows goal -> draft -> approval -> send

============================================================
SECTION 25 — REPLACEMENT READINESS CHECKLIST
==========================================================

AIR is not a credible MLIR/LLVM replacement until all of these are true:

- Core IR is formally specified.
- Parser, printer, verifier, and interpreter are robust.
- Dependent index/refinement system works for real shapes and bounds.
- Effect/resource/authority model prevents invalid transformations.
- Pass manager supports verified transformations.
- Runtime ABI executes real capabilities safely.
- Trace system supports replay and explanation.
- Memory model can compile low-level languages.
- At least one native backend emits working object code without LLVM.
- Backend passes have interpreter/native differential tests.
- Package format supports versioning, policies, capabilities, and signatures.
- AIR can compile a small source language.
- AIR can express and execute agent workflows.
- AIR can optimize both numeric code and agent workflows.
- AIR has a public conformance suite.
- AIR has a self-hosting path.

============================================================
SECTION 26 — IMPLEMENTATION ORDER
============================================================

Implement in this order:

1. Spec AIR-Core.
2. Parser/printer.
3. In-memory IR.
4. Core verifier.
5. Simple interpreter.
6. Core ops and memory resource.
7. Dependent index/refinement subset.
8. Effect/resource/authority verifier.
9. Trace engine.
10. Pass manager and canonicalization.
11. Runtime capability ABI with fake providers.
12. Agent workflow examples.
13. Lowering to AIR-CFG/AIR-Mem.
14. Machine-independentow-level AIR.
15. x86_64 target description.
16. Instruction selection.
17. Register allocation.
18. ELF object emission.
19. Interpreter/native differential test suite.
20. Package format.
21. Security/privacy/policy suite.
22. Self-hosting source language.

============================================================
SECTION 27 — CORE TESTING PRINCIPLE
============================================================

For AIR, correctness does not only mean same return value.

Correctness means:

- same allod observable behavior
- no new unauthorized effects
- no dropped authority requirements
- no weakened policies
- no erased required provenance
- no unsafe memory behavior
- no invalid target code
- no stale approval reuse
- no confidential data leak
- no target feature violation
- no unverified proof use
- no resource duplication unless explicitly allowed

Every optimization, lowering, runtime execution, and backend transformation must preserve these properties.
