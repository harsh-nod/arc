use arc_ir::{
    Argument, Block, BlockTarget, Function, IcmpPredicate, IndexParam, Location, Module,
    ModuleError, Operation, OperationKind, Region, Symbol, Type, ValueId,
};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug, thiserror::Error)]
#[error("{message} at bytes {location:?}")]
pub struct ParseError {
    pub message: String,
    pub location: Location,
}

impl ParseError {
    fn new(message: impl Into<String>, location: Location) -> Self {
        Self {
            message: message.into(),
            location,
        }
    }
}

#[derive(Clone)]
struct Line {
    text: String,
    range: Range<usize>,
}

pub fn parse_module(source: &str) -> Result<Module, ParseError> {
    let lines = collect_lines(source);
    let mut cursor = 0usize;

    let (module_idx, module_line) = next_significant(&lines, &mut cursor)
        .ok_or_else(|| ParseError::new("expected module declaration", Location::new(0, 0)))?;
    let (module_name, has_open) = parse_module_header(&module_line)?;
    if !has_open {
        // Expect the next significant line to be the opening brace.
        let (_, brace_line) = next_significant(&lines, &mut cursor).ok_or_else(|| {
            ParseError::new("expected '{' after module header", module_line.location())
        })?;
        if brace_line.text.trim() != "{" {
            return Err(ParseError::new(
                "expected '{' after module header",
                brace_line.location(),
            ));
        }
    }

    let mut module = Module::new(Symbol::new(module_name));
    cursor = module_idx + 1;

    loop {
        let (line_idx, line) = match peek_significant(&lines, cursor) {
            Some(pair) => pair,
            None => {
                return Err(ParseError::new(
                    "expected '}' to end module",
                    module_line.location(),
                ))
            }
        };
        let trimmed = line.text.trim();
        if trimmed == "}" {
            break;
        }
        if trimmed.starts_with("arc.func") {
            let (function, next_cursor) = parse_function(&lines, line_idx)?;
            cursor = next_cursor;
            module.add_function(function).map_err(|err| match err {
                ModuleError::DuplicateSymbol(sym) => ParseError::new(
                    format!("duplicate function symbol {}", sym),
                    line.location(),
                ),
            })?;
            continue;
        }
        if trimmed.starts_with("arc.capability") {
            let (capability, next_cursor) = parse_capability(&lines, line_idx)?;
            cursor = next_cursor;
            module.add_capability(capability).map_err(|err| match err {
                ModuleError::DuplicateSymbol(sym) => ParseError::new(
                    format!("duplicate capability symbol {}", sym),
                    line.location(),
                ),
            })?;
            continue;
        }
        return Err(ParseError::new(
            "expected function, capability, or '}' inside module",
            line.location(),
        ));
    }

    annotate_capability_invokes(&mut module);
    Ok(module)
}

fn annotate_capability_invokes(module: &mut Module) {
    let capabilities: HashMap<String, (Vec<String>, Vec<Type>)> = module
        .capabilities
        .iter()
        .map(|(name, cap)| {
            (
                name.as_str().to_string(),
                (
                    cap.effects.clone(),
                    cap.outputs.iter().map(|output| output.ty.clone()).collect(),
                ),
            )
        })
        .collect();

    for function in module.functions.values_mut() {
        for block in &mut function.blocks {
            annotate_ops(&mut block.ops, &capabilities);
        }
    }
}

fn annotate_ops(ops: &mut [Operation], capabilities: &HashMap<String, (Vec<String>, Vec<Type>)>) {
    for op in ops {
        if let OperationKind::Invoke { capability } = &op.kind {
            if let Some((effects, result_types)) = capabilities.get(capability.as_str()) {
                op.effects = effects.clone();
                if op.result_types.is_empty() {
                    op.result_types = result_types.clone();
                }
            }
        }

        for region in &mut op.regions {
            for block in &mut region.blocks {
                annotate_ops(&mut block.ops, capabilities);
            }
        }
    }
}

fn parse_function(lines: &[Line], start_idx: usize) -> Result<(Function, usize), ParseError> {
    let mut idx = start_idx;
    let header_line = lines
        .get(idx)
        .cloned()
        .ok_or_else(|| ParseError::new("missing function header", Location::new(0, 0)))?;
    let header_trimmed = header_line.text.trim();
    if !header_trimmed.starts_with("arc.func") {
        return Err(ParseError::new(
            "function header must start with 'arc.func'",
            header_line.location(),
        ));
    }

    let (sig_body, has_brace) = strip_trailing_brace(header_trimmed);
    let signature = parse_function_signature(sig_body, header_line.location())?;
    let ParsedFunctionSignature {
        name,
        index_params,
        params_part,
        result_ty,
    } = signature;

    idx += 1;

    if !has_brace {
        let (brace_idx, brace_line) = find_next_significant(lines, idx).ok_or_else(|| {
            ParseError::new("expected '{' to open function body", header_line.location())
        })?;
        if brace_line.text.trim() != "{" {
            return Err(ParseError::new(
                "expected '{' to open function body",
                brace_line.location(),
            ));
        }
        idx = brace_idx + 1;
    }

    let function_params = parse_argument_list(params_part, header_line.location())?;
    let function_result = result_ty.map(Type::new);
    let mut function = Function::new(
        Symbol::new(name),
        index_params,
        function_params,
        function_result,
        header_line.location(),
    );

    let mut current_block: Option<Block> = None;

    loop {
        let maybe_line = find_next_significant(lines, idx);
        let (line_idx, line) = match maybe_line {
            Some(pair) => pair,
            None => {
                return Err(ParseError::new(
                    "unterminated function body",
                    header_line.location(),
                ))
            }
        };
        let trimmed = line.text.trim();
        if trimmed == "}" {
            idx = line_idx + 1;
            break;
        }
        if trimmed.starts_with('^') {
            if let Some(block) = current_block.take() {
                function.add_block(block);
            }
            current_block = Some(parse_block_header(line.clone())?);
            idx = line_idx + 1;
            continue;
        }
        if let Some(block) = current_block.as_mut() {
            // Check for multi-line structured ops (arc.if, arc.loop)
            if is_structured_op(trimmed) {
                let (op, next_idx) = parse_structured_op(lines, line_idx)?;
                block.add_op(op);
                idx = next_idx;
                continue;
            }
            let op = parse_operation(&line)?;
            block.add_op(op);
            idx = line_idx + 1;
            continue;
        } else {
            return Err(ParseError::new(
                "operation found outside of block",
                line.location(),
            ));
        }
    }

    if let Some(block) = current_block.take() {
        function.add_block(block);
    }

    Ok((function, idx))
}

fn parse_capability(
    lines: &[Line],
    start_idx: usize,
) -> Result<(arc_ir::Capability, usize), ParseError> {
    let header_line = lines
        .get(start_idx)
        .cloned()
        .ok_or_else(|| ParseError::new("missing capability header", Location::new(0, 0)))?;
    let header_trimmed = header_line.text.trim();
    let rest = header_trimmed
        .strip_prefix("arc.capability")
        .ok_or_else(|| {
            ParseError::new(
                "capability header must start with 'arc.capability'",
                header_line.location(),
            )
        })?
        .trim();

    // Parse name
    if !rest.starts_with('@') {
        return Err(ParseError::new(
            "capability name must start with '@'",
            header_line.location(),
        ));
    }
    let name_end = rest[1..]
        .find(|c: char| c.is_whitespace() || c == '{')
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    let name = &rest[1..name_end];

    // Find the opening brace
    let mut idx = start_idx;
    let has_brace = rest[name_end..].trim().starts_with('{');
    if !has_brace {
        idx += 1;
        while idx < lines.len() {
            let trimmed = lines[idx].text.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("//") {
                if trimmed == "{" {
                    break;
                }
                return Err(ParseError::new(
                    "expected '{' after capability header",
                    lines[idx].location(),
                ));
            }
            idx += 1;
        }
    }
    idx += 1;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut effects = Vec::new();
    let mut failures = Vec::new();

    // Parse body lines until '}'
    while idx < lines.len() {
        let line = &lines[idx];
        let trimmed = line.text.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            idx += 1;
            continue;
        }
        if trimmed == "}" {
            idx += 1;
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("inputs") {
            inputs = parse_capability_args(rest, line.location())?;
        } else if let Some(rest) = trimmed.strip_prefix("outputs") {
            outputs = parse_capability_args(rest, line.location())?;
        } else if let Some(rest) = trimmed.strip_prefix("effects") {
            effects = parse_string_list(rest, line.location())?;
        } else if let Some(rest) = trimmed.strip_prefix("failures") {
            failures = parse_string_list(rest, line.location())?;
        }
        idx += 1;
    }

    let cap = arc_ir::Capability {
        name: arc_ir::Symbol::new(name),
        inputs,
        outputs,
        effects,
        failures,
        location: header_line.location(),
    };
    Ok((cap, idx))
}

fn parse_capability_args(rest: &str, loc: Location) -> Result<Vec<arc_ir::Argument>, ParseError> {
    let trimmed = rest.trim();
    if let Some((inner, _)) = split_parenthesized(trimmed, loc) {
        parse_argument_list(inner, loc)
    } else {
        Ok(Vec::new())
    }
}

fn parse_string_list(rest: &str, _loc: Location) -> Result<Vec<String>, ParseError> {
    let trimmed = rest.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(inner
        .split(',')
        .map(|s| {
            s.trim()
                .trim_matches(|c| c == '#' || c == '"')
                .trim_start_matches("arc.effect<")
                .trim_start_matches("arc.fail<")
                .trim_end_matches('>')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect())
}

fn parse_operation(line: &Line) -> Result<Operation, ParseError> {
    let trimmed = line.text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::new("expected operation", line.location()));
    }
    let loc = line.location();
    if let Some((lhs, rhs)) = trimmed.split_once('=') {
        let results = parse_result_list(lhs, loc)?;
        let rhs_trimmed = rhs.trim();
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.const") {
            return parse_const(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.add") {
            return parse_binary_numeric(results, rest, line, OperationKind::Add);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.sub") {
            return parse_binary_numeric(results, rest, line, OperationKind::Sub);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.mul") {
            return parse_binary_numeric(results, rest, line, OperationKind::Mul);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.div") {
            return parse_binary_numeric(results, rest, line, OperationKind::Div);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.icmp") {
            return parse_icmp(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.alloc") {
            return parse_alloc(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.load_elem") {
            return parse_load_elem(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.load") {
            return parse_load(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.store") {
            return parse_store(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.prove") {
            return parse_prove(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.refine") {
            return parse_refine(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.assume") {
            return parse_assume(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.call") {
            return parse_call(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.require_approval") {
            return parse_require_approval(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.invoke") {
            return parse_invoke(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.spawn") {
            return parse_spawn(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.await") {
            return parse_await(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("arc.checkpoint") {
            return parse_checkpoint(results, rest, line);
        }
        return Err(ParseError::new(
            "unsupported operation on assignment RHS",
            line.location(),
        ));
    }
    if trimmed.starts_with("arc.return") {
        let rest = trimmed.trim_start_matches("arc.return").trim();
        if rest.is_empty() {
            return Ok(Operation {
                results: Vec::new(),
                kind: OperationKind::Return,
                operands: Vec::new(),
                result_types: Vec::new(),
                effects: Vec::new(),
                location: line.location(),
                regions: vec![],
            });
        }
        let (value_part, ty_part_opt) = rest
            .split_once(':')
            .map(|(lhs, rhs)| (lhs.trim(), Some(rhs.trim().to_string())))
            .unwrap_or_else(|| (rest, None));
        if !value_part.starts_with('%') {
            return Err(ParseError::new(
                "return operand must be an SSA value",
                line.location(),
            ));
        }
        let operand = ValueId::new(value_part.trim_start_matches('%'));
        let mut result_types = Vec::new();
        if let Some(ty_str) = ty_part_opt {
            result_types.push(Type::new(ty_str));
        }
        return Ok(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![operand],
            result_types,
            effects: Vec::new(),
            location: line.location(),
            regions: vec![],
        });
    }
    if trimmed.starts_with("arc.br") {
        let rest = trimmed.trim_start_matches("arc.br").trim();
        let target = parse_block_target(rest, line.location())?;
        return Ok(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch { target },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: line.location(),
            regions: vec![],
        });
    }
    if trimmed.starts_with("arc.cond_br") {
        let rest = trimmed.trim_start_matches("arc.cond_br").trim();
        let parts = split_top_level(rest, ',');
        if parts.len() != 3 {
            return Err(ParseError::new(
                "arc.cond_br requires condition and two targets",
                line.location(),
            ));
        }
        let cond = parse_value_operand(parts[0].trim(), line.location())?;
        let true_target = parse_block_target(parts[1].trim(), line.location())?;
        let false_target = parse_block_target(parts[2].trim(), line.location())?;
        return Ok(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target,
                false_target,
            },
            operands: vec![cond],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: line.location(),
            regions: vec![],
        });
    }
    if trimmed.starts_with("arc.assert") {
        let rest = trimmed.trim_start_matches("arc.assert").trim();
        return parse_assert(rest, line);
    }
    if trimmed.starts_with("arc.yield") {
        let rest = trimmed.trim_start_matches("arc.yield").trim();
        return parse_yield(rest, line);
    }
    Err(ParseError::new("unsupported operation", line.location()))
}

/// Check if a trimmed line starts a multi-line structured operation.
fn is_structured_op(trimmed: &str) -> bool {
    // Could be `%result = arc.if ...` or just `arc.if ...` (unlikely but handle)
    // or `%result = arc.loop ...`
    if let Some((_lhs, rhs)) = trimmed.split_once('=') {
        let rhs = rhs.trim();
        rhs.starts_with("arc.if ") || rhs.starts_with("arc.loop ") || rhs.starts_with("arc.loop{")
    } else {
        trimmed.starts_with("arc.if ")
            || trimmed.starts_with("arc.loop ")
            || trimmed.starts_with("arc.loop{")
    }
}

/// Parse a multi-line structured op (arc.if or arc.loop) starting at `start_idx`.
/// Returns the Operation and the next line index to continue parsing from.
fn parse_structured_op(lines: &[Line], start_idx: usize) -> Result<(Operation, usize), ParseError> {
    let line = &lines[start_idx];
    let trimmed = line.text.trim();
    let loc = line.location();

    // Split results from RHS
    let (results, rhs) = if let Some((lhs, rhs)) = trimmed.split_once('=') {
        (parse_result_list(lhs, loc)?, rhs.trim())
    } else {
        (Vec::new(), trimmed)
    };

    if rhs.starts_with("arc.if") {
        parse_if_op(lines, start_idx, results, rhs, loc)
    } else if rhs.starts_with("arc.loop") {
        parse_loop_op(lines, start_idx, results, rhs, loc)
    } else {
        Err(ParseError::new("expected arc.if or arc.loop", loc))
    }
}

/// Parse `arc.if %cond { ... } else { ... }`
fn parse_if_op(
    lines: &[Line],
    start_idx: usize,
    results: Vec<ValueId>,
    rhs: &str,
    loc: Location,
) -> Result<(Operation, usize), ParseError> {
    let after_if = rhs.strip_prefix("arc.if").unwrap().trim();

    // Extract condition operand (everything before '{')
    let brace_pos = after_if
        .find('{')
        .ok_or_else(|| ParseError::new("arc.if requires '{' to open then-region", loc))?;
    let cond_str = after_if[..brace_pos].trim();
    let condition = parse_value_operand(cond_str, loc)?;

    // Parse the then-region body
    let mut idx = start_idx + 1;
    let (then_region, next_idx) = parse_region_body(lines, idx, loc)?;
    idx = next_idx;

    // Check for `else {` or `} else {` on the closing line
    // The closing `}` line might have `else {` appended, or the next significant line might be `} else {`
    let mut else_region = Region::new();
    // Look at the line that ended the then-region — it should be `}` possibly followed by `else {`
    if idx > 0 {
        let closing_line = &lines[idx - 1];
        let closing_trimmed = closing_line.text.trim();
        if closing_trimmed.contains("else") && closing_trimmed.contains('{') {
            // `} else {` on same line
            let (else_body, next_else_idx) = parse_region_body(lines, idx, loc)?;
            else_region = else_body;
            idx = next_else_idx;
        } else {
            // Check next significant line for `else {`
            if let Some((_next_line_idx, next_line)) = find_next_significant(lines, idx) {
                let next_trimmed = next_line.text.trim();
                if next_trimmed.starts_with("else") || next_trimmed.starts_with("} else") {
                    // skip the `else {` line
                    idx = _next_line_idx + 1;
                    let (else_body, next_else_idx) = parse_region_body(lines, idx, loc)?;
                    else_region = else_body;
                    idx = next_else_idx;
                }
            }
        }
    }

    let mut regions = vec![then_region];
    if !else_region.blocks.is_empty() {
        regions.push(else_region);
    }

    Ok((
        Operation {
            results,
            kind: OperationKind::If,
            operands: vec![condition],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc,
            regions,
        },
        idx,
    ))
}

/// Parse `arc.loop iter_args(%a, %b) { ... }`
fn parse_loop_op(
    lines: &[Line],
    start_idx: usize,
    results: Vec<ValueId>,
    rhs: &str,
    loc: Location,
) -> Result<(Operation, usize), ParseError> {
    let after_loop = rhs.strip_prefix("arc.loop").unwrap().trim();

    // Parse optional iter_args(...)
    let mut iter_args = Vec::new();
    let rest = if let Some(after_iter) = after_loop.strip_prefix("iter_args") {
        let after_iter = after_iter.trim();
        if let Some((args_str, remainder)) = split_parenthesized(after_iter, loc) {
            iter_args = parse_value_list(args_str, loc)?;
            remainder.trim()
        } else {
            after_iter
        }
    } else {
        after_loop
    };

    // Expect `{`
    if !rest.starts_with('{') {
        return Err(ParseError::new("arc.loop requires '{' to open body", loc));
    }

    let mut idx = start_idx + 1;
    let (body_region, next_idx) = parse_region_body(lines, idx, loc)?;
    idx = next_idx;

    Ok((
        Operation {
            results,
            kind: OperationKind::Loop { iter_args },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc,
            regions: vec![body_region],
        },
        idx,
    ))
}

/// Parse a region body: collects blocks until a matching `}` is found.
/// Returns the Region and the line index after the closing `}`.
fn parse_region_body(
    lines: &[Line],
    start_idx: usize,
    loc: Location,
) -> Result<(Region, usize), ParseError> {
    let mut region = Region::new();
    let mut current_block: Option<Block> = None;
    let mut idx = start_idx;
    let mut depth = 0usize;

    loop {
        let (line_idx, line) = match find_next_significant(lines, idx) {
            Some(pair) => pair,
            None => return Err(ParseError::new("unterminated region body", loc)),
        };
        let trimmed = line.text.trim();

        // Track brace depth for nested structured ops
        if trimmed == "}" || (trimmed.starts_with('}') && !trimmed.contains("else")) {
            if depth == 0 {
                // This is the closing `}` for our region
                if let Some(block) = current_block.take() {
                    region.add_block(block);
                }
                idx = line_idx + 1;
                return Ok((region, idx));
            }
            depth -= 1;
        }

        // Check for `} else {` — this closes our then-region
        if depth == 0 && trimmed.contains('}') && trimmed.contains("else") {
            if let Some(block) = current_block.take() {
                region.add_block(block);
            }
            idx = line_idx + 1;
            return Ok((region, idx));
        }

        // Count opening braces for nested structured ops
        if trimmed.ends_with('{') && (trimmed.contains("arc.if") || trimmed.contains("arc.loop")) {
            depth += 1;
        }

        // Block header
        if trimmed.starts_with('^') {
            if let Some(block) = current_block.take() {
                region.add_block(block);
            }
            current_block = Some(parse_block_header(line.clone())?);
            idx = line_idx + 1;
            continue;
        }

        // Operation inside region block
        if let Some(block) = current_block.as_mut() {
            // Handle nested structured ops within regions
            if is_structured_op(trimmed) {
                let (op, next_idx) = parse_structured_op(lines, line_idx)?;
                block.add_op(op);
                idx = next_idx;
                continue;
            }
            let op = parse_operation(&line)?;
            block.add_op(op);
            idx = line_idx + 1;
            continue;
        }

        // If no block has started yet, auto-create an anonymous one
        let mut auto_block = Block::new(Some("body".to_string().into()), line.location());
        if is_structured_op(trimmed) {
            let (op, next_idx) = parse_structured_op(lines, line_idx)?;
            auto_block.add_op(op);
            current_block = Some(auto_block);
            idx = next_idx;
        } else {
            let op = parse_operation(&line)?;
            auto_block.add_op(op);
            current_block = Some(auto_block);
            idx = line_idx + 1;
        }
    }
}

fn parse_yield(rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let operands = if rest.is_empty() {
        Vec::new()
    } else {
        parse_value_list(rest, line.location())?
    };
    Ok(Operation {
        results: Vec::new(),
        kind: OperationKind::Yield,
        operands,
        result_types: Vec::new(),
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_spawn(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    if !trimmed.starts_with('@') {
        return Err(ParseError::new(
            "arc.spawn requires callee starting with '@'",
            line.location(),
        ));
    }
    let name_end = trimmed
        .char_indices()
        .find_map(|(idx, ch)| {
            if ch == '(' || ch.is_whitespace() {
                Some(idx)
            } else {
                None
            }
        })
        .unwrap_or(trimmed.len());
    let callee = trimmed[1..name_end].to_string();
    if callee.is_empty() {
        return Err(ParseError::new(
            "arc.spawn callee name cannot be empty",
            line.location(),
        ));
    }
    let remainder = trimmed[name_end..].trim();
    let (args_str, after_args) =
        split_parenthesized(remainder, line.location()).ok_or_else(|| {
            ParseError::new("arc.spawn requires argument list '(...)'", line.location())
        })?;
    let operands = parse_value_list(args_str, line.location())?;
    let _ = after_args;
    Ok(Operation {
        results,
        kind: OperationKind::Spawn {
            callee: Symbol::new(callee),
        },
        operands,
        result_types: Vec::new(),
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_await(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    let operand = parse_value_operand(trimmed, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Await,
        operands: vec![operand],
        result_types: Vec::new(),
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_checkpoint(
    results: Vec<ValueId>,
    rest: &str,
    line: &Line,
) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    // Expect "label" (quoted string)
    let label = trimmed.trim_matches('"');
    if label.is_empty() {
        return Err(ParseError::new(
            "arc.checkpoint requires a label",
            line.location(),
        ));
    }
    Ok(Operation {
        results,
        kind: OperationKind::Checkpoint {
            label: label.into(),
        },
        operands: Vec::new(),
        result_types: Vec::new(),
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_block_header(line: Line) -> Result<Block, ParseError> {
    let trimmed = line.text.trim();
    if !trimmed.starts_with('^') {
        return Err(ParseError::new(
            "block header must start with '^'",
            line.location(),
        ));
    }
    let after_caret = &trimmed[1..];
    let (label_with_args, _) = after_caret
        .rsplit_once(':')
        .ok_or_else(|| ParseError::new("block header missing ':'", line.location()))?;
    let label_with_args = label_with_args.trim();
    let (label_str, args) = if let Some(start) = label_with_args.find('(') {
        let after = &label_with_args[start + 1..];
        let end_rel = after.find(')').ok_or_else(|| {
            ParseError::new(
                format!("block arguments must end with ')' in '{}'", label_with_args),
                line.location(),
            )
        })?;
        let end = start + 1 + end_rel;
        let label = label_with_args[..start].trim();
        let args_str = &label_with_args[start + 1..end];
        let trailing_tokens = &label_with_args[end + 1..];
        if !trailing_tokens.trim().is_empty() {
            return Err(ParseError::new(
                "unexpected tokens after block argument list",
                line.location(),
            ));
        }
        let args = parse_argument_list(args_str, line.location())?;
        (label, args)
    } else {
        (label_with_args.trim(), Vec::new())
    };
    if label_str.is_empty() {
        return Err(ParseError::new(
            "block label may not be empty",
            line.location(),
        ));
    }
    let mut block = Block::new(Some(label_str.to_string().into()), line.location());
    block.args = args;
    Ok(block)
}

fn parse_argument_list(list_str: &str, loc: Location) -> Result<Vec<Argument>, ParseError> {
    let trimmed = list_str.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    for segment in split_top_level(trimmed, ',') {
        let seg = segment.trim();
        if seg.is_empty() {
            return Err(ParseError::new("empty argument segment", loc));
        }
        let (name, ty) = seg
            .split_once(':')
            .ok_or_else(|| ParseError::new("argument must be '%name: type'", loc))?;
        let name = name.trim();
        if !name.starts_with('%') {
            return Err(ParseError::new("argument must start with '%'", loc));
        }
        let ty = Type::new(ty.trim().to_string());
        args.push(Argument {
            name: ValueId::new(name.trim_start_matches('%')),
            ty,
            location: loc,
        });
    }
    Ok(args)
}

fn parse_index_param_list(list_str: &str, loc: Location) -> Result<Vec<IndexParam>, ParseError> {
    let trimmed = list_str.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut params = Vec::new();
    for segment in split_top_level(trimmed, ',') {
        let seg = segment.trim();
        if seg.is_empty() {
            return Err(ParseError::new("empty index parameter segment", loc));
        }
        let (name_part, ty_part) = seg
            .split_once(':')
            .ok_or_else(|| ParseError::new("index parameter must be '%name: index'", loc))?;
        let name_part = name_part.trim();
        if !name_part.starts_with('%') {
            return Err(ParseError::new(
                "index parameter name must start with '%'",
                loc,
            ));
        }
        let name = name_part.trim_start_matches('%').trim();
        if name.is_empty() {
            return Err(ParseError::new("index parameter name cannot be empty", loc));
        }
        let ty_part = ty_part.trim();
        if ty_part != "index" {
            return Err(ParseError::new("index parameter type must be 'index'", loc));
        }
        params.push(IndexParam {
            name: ValueId::new(name),
            location: loc,
        });
    }
    Ok(params)
}

fn parse_const(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.const must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (value_str, ty_str) = trimmed
        .split_once(':')
        .ok_or_else(|| ParseError::new("expected ':' in const operation", line.location()))?;
    let value = value_str
        .trim()
        .parse::<i64>()
        .map_err(|_| ParseError::new("const literal must be integer", line.location()))?;
    let ty = parse_type_token(ty_str, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::ConstI64(value),
        operands: Vec::new(),
        result_types: vec![ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_binary_numeric(
    results: Vec<ValueId>,
    rest: &str,
    line: &Line,
    kind: OperationKind,
) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "binary operation must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, ty_part) = trimmed
        .split_once(':')
        .ok_or_else(|| ParseError::new("expected ':' in binary operation", line.location()))?;
    let operands = parse_value_list(operands_part, line.location())?;
    if operands.len() != 2 {
        return Err(ParseError::new(
            "binary operation expects exactly two operands",
            line.location(),
        ));
    }
    let ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind,
        operands,
        result_types: vec![ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_icmp(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.icmp must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (predicate_token, remainder) = split_first_token(trimmed)
        .ok_or_else(|| ParseError::new("icmp requires predicate and operands", line.location()))?;
    let predicate = parse_icmp_predicate(predicate_token, line.location())?;
    if remainder.is_empty() {
        return Err(ParseError::new(
            "icmp requires operands after predicate",
            line.location(),
        ));
    }
    let (operands_part, ty_part) = remainder
        .split_once(':')
        .ok_or_else(|| ParseError::new("expected ':' in icmp operation", line.location()))?;
    let operands = parse_value_list(operands_part, line.location())?;
    if operands.len() != 2 {
        return Err(ParseError::new(
            "icmp expects exactly two operands",
            line.location(),
        ));
    }
    let ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::ICmp { predicate },
        operands,
        result_types: vec![ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_alloc(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 2 {
        return Err(ParseError::new(
            "arc.alloc must produce updated memory and pointer results",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.alloc requires type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    let result_types = parse_result_types(type_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Alloc,
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_load(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 2 {
        return Err(ParseError::new(
            "arc.load must produce updated memory and loaded value results",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.load requires type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    let result_types = parse_result_types(type_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Load,
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_load_elem(
    results: Vec<ValueId>,
    rest: &str,
    line: &Line,
) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.load_elem must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (access_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.load_elem requires type annotation after ':'",
            line.location(),
        )
    })?;
    let (access_expr, requires_part) = access_part
        .split_once("requires")
        .map(|(lhs, rhs)| (lhs.trim(), Some(rhs.trim())))
        .unwrap_or((access_part.trim(), None));
    let operands = parse_load_elem_operands(access_expr, requires_part, line)?;
    let result_types = parse_result_types(type_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::LoadElem,
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_store(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.store must produce exactly one memory result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.store requires type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    let result_types = parse_result_types(type_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Store,
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_prove(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.prove must produce exactly one proof result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, ty_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.prove requires result type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    if operands.len() != 1 {
        return Err(ParseError::new(
            "arc.prove expects exactly one operand",
            line.location(),
        ));
    }
    let proof_ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Prove,
        operands,
        result_types: vec![proof_ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_refine(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.refine must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, ty_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.refine requires result type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    if operands.len() != 2 {
        return Err(ParseError::new(
            "arc.refine expects value and proof operands",
            line.location(),
        ));
    }
    let ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Refine,
        operands,
        result_types: vec![ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_load_elem_operands(
    access_expr: &str,
    requires_part: Option<&str>,
    line: &Line,
) -> Result<Vec<ValueId>, ParseError> {
    let trimmed = access_expr.trim();
    let open = trimmed.find('[').ok_or_else(|| {
        ParseError::new("arc.load_elem expects '%slice[%index]'", line.location())
    })?;
    let close = trimmed
        .rfind(']')
        .ok_or_else(|| ParseError::new("arc.load_elem expects closing ']'", line.location()))?;
    if close < open {
        return Err(ParseError::new(
            "arc.load_elem has malformed index expression",
            line.location(),
        ));
    }
    let slice_token = trimmed[..open].trim();
    let index_token = trimmed[open + 1..close].trim();
    let trailing = trimmed[close + 1..].trim();
    if !trailing.is_empty() {
        return Err(ParseError::new(
            "unexpected tokens after index expression",
            line.location(),
        ));
    }
    if !slice_token.starts_with('%') {
        return Err(ParseError::new(
            "arc.load_elem slice must be an SSA value",
            line.location(),
        ));
    }
    if !index_token.starts_with('%') {
        return Err(ParseError::new(
            "arc.load_elem index must be an SSA value",
            line.location(),
        ));
    }
    let mut operands = Vec::new();
    operands.push(ValueId::new(slice_token.trim_start_matches('%')));
    operands.push(ValueId::new(index_token.trim_start_matches('%')));
    if let Some(reqs) = requires_part {
        let proofs = parse_value_list(reqs, line.location())?;
        operands.extend(proofs);
    }
    Ok(operands)
}

fn parse_assume(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.assume must produce exactly one result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operand_part, ty_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.assume requires result type annotation",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operand_part, line.location())?;
    if operands.len() != 1 {
        return Err(ParseError::new(
            "arc.assume expects exactly one operand",
            line.location(),
        ));
    }
    let proof_ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::Assume,
        operands,
        result_types: vec![proof_ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_assert(rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return Err(ParseError::new(
            "arc.assert requires an operand",
            line.location(),
        ));
    }
    let (operand_part, ty_part_opt) = trimmed
        .split_once(':')
        .map(|(lhs, rhs)| (lhs.trim(), Some(rhs.trim())))
        .unwrap_or_else(|| (trimmed, None));
    let operand = parse_value_operand(operand_part, line.location())?;
    let mut result_types = Vec::new();
    if let Some(ty_str) = ty_part_opt {
        result_types.push(parse_type_token(ty_str, line.location())?);
    }
    Ok(Operation {
        results: Vec::new(),
        kind: OperationKind::Assert,
        operands: vec![operand],
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_call(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    // Syntax: arc.call @callee(%arg1, %arg2) : (types) -> result_type
    if !trimmed.starts_with('@') {
        return Err(ParseError::new(
            "arc.call requires callee starting with '@'",
            line.location(),
        ));
    }
    // Find the callee name (up to '(' or whitespace)
    let name_end = trimmed
        .char_indices()
        .find_map(|(idx, ch)| {
            if ch == '(' || ch.is_whitespace() {
                Some(idx)
            } else {
                None
            }
        })
        .unwrap_or(trimmed.len());
    let callee = trimmed[1..name_end].to_string(); // strip '@'
    if callee.is_empty() {
        return Err(ParseError::new(
            "arc.call callee name cannot be empty",
            line.location(),
        ));
    }
    let remainder = trimmed[name_end..].trim();

    // Parse arguments in parentheses
    let (args_str, after_args) =
        split_parenthesized(remainder, line.location()).ok_or_else(|| {
            ParseError::new("arc.call requires argument list '(...)'", line.location())
        })?;
    let operands = parse_value_list(args_str, line.location())?;

    // Parse optional type annotation
    let after_args = after_args.trim();
    let result_types = if let Some(ty_part) = after_args.strip_prefix(':') {
        parse_result_types(ty_part, line.location())?
    } else {
        Vec::new()
    };

    Ok(Operation {
        results,
        kind: OperationKind::Call {
            callee: arc_ir::Symbol::new(callee),
        },
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

/// Parse `arc.require_approval %principal, %resource : type`
fn parse_require_approval(
    results: Vec<ValueId>,
    rest: &str,
    line: &Line,
) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "arc.require_approval must produce exactly one auth result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, ty_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "arc.require_approval requires type annotation after ':'",
            line.location(),
        )
    })?;
    let operands = parse_value_list(operands_part, line.location())?;
    if operands.len() != 2 {
        return Err(ParseError::new(
            "arc.require_approval expects principal and resource operands",
            line.location(),
        ));
    }
    let ty = parse_type_token(ty_part, line.location())?;
    Ok(Operation {
        results,
        kind: OperationKind::RequireApproval,
        operands,
        result_types: vec![ty],
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

/// Parse `arc.invoke @capability(%args) : type` or multi-result variant
fn parse_invoke(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    let trimmed = rest.trim();
    if !trimmed.starts_with('@') {
        return Err(ParseError::new(
            "arc.invoke requires capability name starting with '@'",
            line.location(),
        ));
    }
    let name_end = trimmed
        .char_indices()
        .find_map(|(idx, ch)| {
            if ch == '(' || ch.is_whitespace() {
                Some(idx)
            } else {
                None
            }
        })
        .unwrap_or(trimmed.len());
    let capability = trimmed[1..name_end].to_string();
    if capability.is_empty() {
        return Err(ParseError::new(
            "arc.invoke capability name cannot be empty",
            line.location(),
        ));
    }
    let remainder = trimmed[name_end..].trim();

    let (args_str, after_args) =
        split_parenthesized(remainder, line.location()).ok_or_else(|| {
            ParseError::new("arc.invoke requires argument list '(...)'", line.location())
        })?;
    let operands = parse_value_list(args_str, line.location())?;

    let after_args = after_args.trim();
    let result_types = if let Some(ty_part) = after_args.strip_prefix(':') {
        parse_result_types(ty_part, line.location())?
    } else {
        Vec::new()
    };

    Ok(Operation {
        results,
        kind: OperationKind::Invoke {
            capability: arc_ir::Symbol::new(capability),
        },
        operands,
        result_types,
        effects: Vec::new(),
        location: line.location(),
        regions: vec![],
    })
}

fn parse_result_list(lhs: &str, loc: Location) -> Result<Vec<ValueId>, ParseError> {
    let mut results = Vec::new();
    for segment in lhs.split(',') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return Err(ParseError::new(
                "result list cannot contain empty names",
                loc,
            ));
        }
        if !trimmed.starts_with('%') {
            return Err(ParseError::new("result must begin with '%'", loc));
        }
        let name = trimmed.trim_start_matches('%').trim();
        if name.is_empty() {
            return Err(ParseError::new("result name cannot be empty", loc));
        }
        results.push(ValueId::new(name));
    }
    if results.is_empty() {
        return Err(ParseError::new(
            "assignment must include at least one result name",
            loc,
        ));
    }
    Ok(results)
}

fn split_parenthesized(input: &str, _loc: Location) -> Option<(&str, &str)> {
    let mut start_idx = None;
    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => {
                if start_idx.is_none() {
                    start_idx = Some(idx);
                }
                depth += 1;
            }
            ')' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    let start = start_idx?;
                    let inner = &input[start + 1..idx];
                    let rest = &input[idx + 1..];
                    return Some((inner, rest));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_result_types(text: &str, loc: Location) -> Result<Vec<Type>, ParseError> {
    let trimmed = text.trim();
    let (_, outputs_str) = trimmed
        .split_once("->")
        .ok_or_else(|| ParseError::new("type annotation must include '->' for results", loc))?;
    parse_type_group(outputs_str, loc)
}

fn parse_type_group(text: &str, loc: Location) -> Result<Vec<Type>, ParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('(') {
        if !trimmed.ends_with(')') {
            return Err(ParseError::new("type list must end with ')'", loc));
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        let inner_trimmed = inner.trim();
        if inner_trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let mut types = Vec::new();
        for segment in split_top_level(inner_trimmed, ',') {
            if segment.trim().is_empty() {
                return Err(ParseError::new("empty type in list", loc));
            }
            types.push(parse_type_token(segment.as_str(), loc)?);
        }
        Ok(types)
    } else {
        Ok(vec![parse_type_token(trimmed, loc)?])
    }
}

fn parse_block_target(segment: &str, loc: Location) -> Result<BlockTarget, ParseError> {
    let trimmed = segment.trim();
    if !trimmed.starts_with('^') {
        return Err(ParseError::new("branch target must start with '^'", loc));
    }
    let rest = &trimmed[1..];
    let (label, args) = if let Some(start) = rest.find('(') {
        let end = rest
            .rfind(')')
            .ok_or_else(|| ParseError::new("branch arguments must end with ')'", loc))?;
        let label = rest[..start].trim();
        let args_str = &rest[start + 1..end];
        let remaining = rest[end + 1..].trim();
        if !remaining.is_empty() {
            return Err(ParseError::new(
                "unexpected trailing tokens after branch args",
                loc,
            ));
        }
        let args = parse_value_list(args_str, loc)?;
        (label, args)
    } else {
        (rest.trim(), Vec::new())
    };
    if label.is_empty() {
        return Err(ParseError::new("branch target label cannot be empty", loc));
    }
    Ok(BlockTarget::new(label.to_string().into(), args))
}

fn parse_value_list(list_str: &str, loc: Location) -> Result<Vec<ValueId>, ParseError> {
    let trimmed = list_str.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut values = Vec::new();
    for segment in split_top_level(trimmed, ',') {
        if segment.trim().is_empty() {
            return Err(ParseError::new("empty value in list", loc));
        }
        values.push(parse_value_operand(segment.trim(), loc)?);
    }
    Ok(values)
}

fn parse_value_operand(text: &str, loc: Location) -> Result<ValueId, ParseError> {
    let trimmed = text.trim();
    if !trimmed.starts_with('%') {
        return Err(ParseError::new("expected SSA value starting with '%'", loc));
    }
    let name = trimmed.trim_start_matches('%').trim();
    if name.is_empty() {
        return Err(ParseError::new("value name cannot be empty", loc));
    }
    Ok(ValueId::new(name))
}

fn parse_type_token(text: &str, loc: Location) -> Result<Type, ParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::new("expected type", loc));
    }
    Ok(Type::new(trimmed.to_string()))
}

fn parse_icmp_predicate(token: &str, loc: Location) -> Result<IcmpPredicate, ParseError> {
    match token {
        "eq" => Ok(IcmpPredicate::Eq),
        "ne" => Ok(IcmpPredicate::Ne),
        "slt" => Ok(IcmpPredicate::Slt),
        "sle" => Ok(IcmpPredicate::Sle),
        "sgt" => Ok(IcmpPredicate::Sgt),
        "sge" => Ok(IcmpPredicate::Sge),
        _ => Err(ParseError::new(
            format!("unknown icmp predicate '{}'", token),
            loc,
        )),
    }
}

fn split_first_token(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let mut end = trimmed.len();
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_whitespace() {
            end = idx;
            break;
        }
    }
    let token = &trimmed[..end];
    let remainder = trimmed[end..].trim_start();
    Some((token, remainder))
}

fn split_top_level(input: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        if depth == 0 && ch == delimiter {
            parts.push(input[start..idx].to_string());
            start = idx + ch.len_utf8();
        }
    }
    parts.push(input[start..].to_string());
    parts
}

struct ParsedFunctionSignature<'a> {
    name: &'a str,
    index_params: Vec<IndexParam>,
    params_part: &'a str,
    result_ty: Option<&'a str>,
}

fn parse_function_signature<'a>(
    sig: &'a str,
    loc: Location,
) -> Result<ParsedFunctionSignature<'a>, ParseError> {
    let mut remainder = sig.trim_start_matches("arc.func").trim();
    if !remainder.starts_with('@') {
        return Err(ParseError::new("function name must start with '@'", loc));
    }
    let name_end = remainder
        .char_indices()
        .find_map(|(idx, ch)| {
            if ch == '(' || ch.is_whitespace() {
                Some(idx)
            } else {
                None
            }
        })
        .unwrap_or(remainder.len());
    let name = remainder[..name_end].trim().trim_start_matches('@');
    if name.is_empty() {
        return Err(ParseError::new("function name cannot be empty", loc));
    }
    remainder = remainder[name_end..].trim_start();

    let mut index_params = Vec::new();
    if remainder.starts_with("forall") {
        remainder = remainder["forall".len()..].trim_start();
        let (list, rest) = split_parenthesized(remainder, loc)
            .ok_or_else(|| ParseError::new("expected '(...)' after forall", loc))?;
        index_params = parse_index_param_list(list, loc)?;
        remainder = rest.trim_start();
    }

    let (params_part, after_params) = split_parenthesized(remainder, loc).ok_or_else(|| {
        ParseError::new("expected parameter list '(...)' in function signature", loc)
    })?;
    let after_params = after_params.trim();
    let result_ty = after_params.strip_prefix("->").map(|rest| rest.trim());

    Ok(ParsedFunctionSignature {
        name,
        index_params,
        params_part,
        result_ty,
    })
}

fn parse_module_header(line: &Line) -> Result<(&str, bool), ParseError> {
    let trimmed = line.text.trim();
    if !trimmed.starts_with("arc.module") {
        return Err(ParseError::new(
            "module header must start with 'arc.module'",
            line.location(),
        ));
    }
    let rest = trimmed.trim_start_matches("arc.module").trim();
    let (name_part, remainder) = rest.split_once(' ').unwrap_or((rest, ""));
    let name = name_part.trim().trim_start_matches('@');
    if name.is_empty() {
        return Err(ParseError::new("module requires a name", line.location()));
    }
    let has_brace = remainder.trim().starts_with('{');
    Ok((name, has_brace))
}

fn strip_trailing_brace(line: &str) -> (&str, bool) {
    if line.ends_with('{') {
        let without = line.trim_end_matches('{').trim_end();
        (without, true)
    } else {
        (line, false)
    }
}

fn collect_lines(source: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    if source.is_empty() {
        return lines;
    }
    for part in source.split_inclusive('\n') {
        let text = part.strip_suffix('\n').unwrap_or(part).to_string();
        let end = offset + part.len();
        lines.push(Line {
            text,
            range: offset..end,
        });
        offset = end;
    }
    if !source.ends_with('\n') {
        // ensure final line is represented (split_inclusive already covered it, so nothing to do)
    }
    lines
}

fn next_significant(lines: &[Line], cursor: &mut usize) -> Option<(usize, Line)> {
    while *cursor < lines.len() {
        let idx = *cursor;
        *cursor += 1;
        if is_significant(&lines[idx].text) {
            return Some((idx, lines[idx].clone()));
        }
    }
    None
}

fn peek_significant(lines: &[Line], cursor: usize) -> Option<(usize, Line)> {
    find_next_significant(lines, cursor)
}

fn find_next_significant(lines: &[Line], mut cursor: usize) -> Option<(usize, Line)> {
    while cursor < lines.len() {
        if is_significant(&lines[cursor].text) {
            return Some((cursor, lines[cursor].clone()));
        }
        cursor += 1;
    }
    None
}

fn is_significant(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && !trimmed.starts_with("//")
}

trait LineExt {
    fn location(&self) -> Location;
}

impl LineExt for Line {
    fn location(&self) -> Location {
        Location::new(self.range.start, self.range.end - self.range.start)
    }
}
