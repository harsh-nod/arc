use air_ir::{
    Argument, Block, BlockTarget, Function, IcmpPredicate, Location, Module, ModuleError,
    Operation, OperationKind, Symbol, Type, ValueId,
};
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
        if trimmed.starts_with("air.func") {
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
        return Err(ParseError::new(
            "expected function or '}' inside module",
            line.location(),
        ));
    }

    Ok(module)
}

fn parse_function(lines: &[Line], start_idx: usize) -> Result<(Function, usize), ParseError> {
    let mut idx = start_idx;
    let header_line = lines
        .get(idx)
        .cloned()
        .ok_or_else(|| ParseError::new("missing function header", Location::new(0, 0)))?;
    let header_trimmed = header_line.text.trim();
    if !header_trimmed.starts_with("air.func") {
        return Err(ParseError::new(
            "function header must start with 'air.func'",
            header_line.location(),
        ));
    }

    let (sig_body, has_brace) = strip_trailing_brace(header_trimmed);
    let (name, params_str, result_ty) = parse_function_signature(sig_body, header_line.location())?;

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

    let mut function = Function::new(
        Symbol::new(name),
        parse_argument_list(params_str, header_line.location())?,
        result_ty.map(Type::new),
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

fn parse_operation(line: &Line) -> Result<Operation, ParseError> {
    let trimmed = line.text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::new("expected operation", line.location()));
    }
    let loc = line.location();
    if let Some((lhs, rhs)) = trimmed.split_once('=') {
        let results = parse_result_list(lhs, loc)?;
        let rhs_trimmed = rhs.trim();
        if let Some(rest) = rhs_trimmed.strip_prefix("air.const") {
            return parse_const(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.add") {
            return parse_binary_numeric(results, rest, line, OperationKind::Add);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.sub") {
            return parse_binary_numeric(results, rest, line, OperationKind::Sub);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.mul") {
            return parse_binary_numeric(results, rest, line, OperationKind::Mul);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.div") {
            return parse_binary_numeric(results, rest, line, OperationKind::Div);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.icmp") {
            return parse_icmp(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.alloc") {
            return parse_alloc(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.load") {
            return parse_load(results, rest, line);
        }
        if let Some(rest) = rhs_trimmed.strip_prefix("air.store") {
            return parse_store(results, rest, line);
        }
        return Err(ParseError::new(
            "unsupported operation on assignment RHS",
            line.location(),
        ));
    }
    if trimmed.starts_with("air.return") {
        let rest = trimmed.trim_start_matches("air.return").trim();
        if rest.is_empty() {
            return Ok(Operation {
                results: Vec::new(),
                kind: OperationKind::Return,
                operands: Vec::new(),
                result_types: Vec::new(),
                location: line.location(),
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
            location: line.location(),
        });
    }
    if trimmed.starts_with("air.br") {
        let rest = trimmed.trim_start_matches("air.br").trim();
        let target = parse_block_target(rest, line.location())?;
        return Ok(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch { target },
            operands: Vec::new(),
            result_types: Vec::new(),
            location: line.location(),
        });
    }
    if trimmed.starts_with("air.cond_br") {
        let rest = trimmed.trim_start_matches("air.cond_br").trim();
        let parts = split_top_level(rest, ',');
        if parts.len() != 3 {
            return Err(ParseError::new(
                "air.cond_br requires condition and two targets",
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
            location: line.location(),
        });
    }
    Err(ParseError::new("unsupported operation", line.location()))
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

fn parse_const(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "air.const must produce exactly one result",
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
        location: line.location(),
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
        location: line.location(),
    })
}

fn parse_icmp(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "air.icmp must produce exactly one result",
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
        location: line.location(),
    })
}

fn parse_alloc(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 2 {
        return Err(ParseError::new(
            "air.alloc must produce updated memory and pointer results",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "air.alloc requires type annotation after ':'",
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
        location: line.location(),
    })
}

fn parse_load(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 2 {
        return Err(ParseError::new(
            "air.load must produce updated memory and loaded value results",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "air.load requires type annotation after ':'",
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
        location: line.location(),
    })
}

fn parse_store(results: Vec<ValueId>, rest: &str, line: &Line) -> Result<Operation, ParseError> {
    if results.len() != 1 {
        return Err(ParseError::new(
            "air.store must produce exactly one memory result",
            line.location(),
        ));
    }
    let trimmed = rest.trim();
    let (operands_part, type_part) = trimmed.split_once(':').ok_or_else(|| {
        ParseError::new(
            "air.store requires type annotation after ':'",
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
        location: line.location(),
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
                if depth > 0 {
                    depth -= 1;
                }
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

fn parse_function_signature<'a>(
    sig: &'a str,
    loc: Location,
) -> Result<(&'a str, &'a str, Option<&'a str>), ParseError> {
    let remainder = sig.trim_start_matches("air.func").trim();
    let (name_part, rest) = remainder
        .split_once('(')
        .ok_or_else(|| ParseError::new("expected '(' in function signature", loc))?;
    let name = name_part.trim().trim_start_matches('@');
    let (params_part, after_params) = rest
        .rsplit_once(')')
        .ok_or_else(|| ParseError::new("expected ')' in function signature", loc))?;
    let after_params = after_params.trim();
    let (result_ty, _) = if let Some(rest) = after_params.strip_prefix("->") {
        let ty = rest.trim();
        (Some(ty), "")
    } else {
        (None, after_params)
    };
    Ok((name, params_part, result_ty))
}

fn parse_module_header(line: &Line) -> Result<(&str, bool), ParseError> {
    let trimmed = line.text.trim();
    if !trimmed.starts_with("air.module") {
        return Err(ParseError::new(
            "module header must start with 'air.module'",
            line.location(),
        ));
    }
    let rest = trimmed.trim_start_matches("air.module").trim();
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

fn next_significant<'a>(lines: &'a [Line], cursor: &mut usize) -> Option<(usize, Line)> {
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
