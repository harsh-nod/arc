//! AIR binary format (.airb): compact binary serialization of AIR modules.
//!
//! Format layout:
//! ```text
//! [magic: 4 bytes "AIRB"]
//! [version: u16 LE]
//! [flags: u16 LE]
//! [module_json_len: u32 LE]
//! [module_json: N bytes]
//! [checksum: u32 LE (CRC32 of module_json)]
//! ```

use arc_ir::Module;

/// Magic bytes identifying an AIR binary file.
pub const MAGIC: &[u8; 4] = b"AIRB";
/// Current binary format version.
pub const VERSION: u16 = 1;

/// Flags for the binary format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatFlags(u16);

impl FormatFlags {
    pub const NONE: Self = Self(0);
    pub const COMPRESSED: Self = Self(1);
    pub const SIGNED: Self = Self(2);

    pub fn bits(self) -> u16 {
        self.0
    }

    pub fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub fn is_compressed(self) -> bool {
        self.0 & 1 != 0
    }

    pub fn is_signed(self) -> bool {
        self.0 & 2 != 0
    }
}

/// Serialize a Module to the compact AIR binary format (version 2).
pub fn encode_compact(module: &Module) -> Result<Vec<u8>, FormatError> {
    let body = compact::encode_module(module)?;
    let checksum = crc32(&body);
    let body_len = body.len() as u32;

    let mut buf = Vec::with_capacity(12 + body.len() + 4);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&2u16.to_le_bytes()); // version 2
    buf.extend_from_slice(&FormatFlags::NONE.bits().to_le_bytes());
    buf.extend_from_slice(&body_len.to_le_bytes());
    buf.extend_from_slice(&body);
    buf.extend_from_slice(&checksum.to_le_bytes());
    Ok(buf)
}

/// Deserialize a Module from the compact AIR binary format (version 2).
pub fn decode_compact(data: &[u8]) -> Result<Module, FormatError> {
    if data.len() < 12 {
        return Err(FormatError::TooShort);
    }
    if &data[0..4] != MAGIC {
        return Err(FormatError::InvalidMagic);
    }
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != 2 {
        return Err(FormatError::UnsupportedVersion(version));
    }
    let flags = FormatFlags::from_bits(u16::from_le_bytes([data[6], data[7]]));
    if flags.is_compressed() {
        return Err(FormatError::UnsupportedFeature("compression".to_string()));
    }
    let body_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let body_end = 12 + body_len;
    if data.len() < body_end + 4 {
        return Err(FormatError::TooShort);
    }
    let body = &data[12..body_end];
    let expected_checksum = u32::from_le_bytes([
        data[body_end],
        data[body_end + 1],
        data[body_end + 2],
        data[body_end + 3],
    ]);
    let actual_checksum = crc32(body);
    if expected_checksum != actual_checksum {
        return Err(FormatError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }
    compact::decode_module(body)
}

/// Serialize a Module to the AIR binary format (version 1, JSON-based).
pub fn encode(module: &Module) -> Result<Vec<u8>, FormatError> {
    let json = serde_json::to_vec(module).map_err(|e| FormatError::Serialization(e.to_string()))?;

    let checksum = crc32(&json);
    let json_len = json.len() as u32;

    let mut buf = Vec::with_capacity(12 + json.len() + 4);
    // Header
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&FormatFlags::NONE.bits().to_le_bytes());
    buf.extend_from_slice(&json_len.to_le_bytes());
    // Body
    buf.extend_from_slice(&json);
    // Checksum
    buf.extend_from_slice(&checksum.to_le_bytes());

    Ok(buf)
}

/// Deserialize a Module from the AIR binary format.
pub fn decode(data: &[u8]) -> Result<Module, FormatError> {
    if data.len() < 12 {
        return Err(FormatError::TooShort);
    }
    // Check magic.
    if &data[0..4] != MAGIC {
        return Err(FormatError::InvalidMagic);
    }
    // Check version.
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version == 2 {
        return decode_compact(data);
    }
    if version != VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    // Read flags.
    let flags = FormatFlags::from_bits(u16::from_le_bytes([data[6], data[7]]));
    if flags.is_compressed() {
        return Err(FormatError::UnsupportedFeature("compression".to_string()));
    }
    // Read JSON length.
    let json_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let json_end = 12 + json_len;
    if data.len() < json_end + 4 {
        return Err(FormatError::TooShort);
    }
    let json_data = &data[12..json_end];
    // Verify checksum.
    let expected_checksum = u32::from_le_bytes([
        data[json_end],
        data[json_end + 1],
        data[json_end + 2],
        data[json_end + 3],
    ]);
    let actual_checksum = crc32(json_data);
    if expected_checksum != actual_checksum {
        return Err(FormatError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }
    // Deserialize.
    let module: Module =
        serde_json::from_slice(json_data).map_err(|e| FormatError::Serialization(e.to_string()))?;
    Ok(module)
}

/// Compute a simple CRC32 checksum (IEEE polynomial).
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("data too short")]
    TooShort,
    #[error("invalid magic (not an AIRB file)")]
    InvalidMagic,
    #[error("unsupported format version: {0}")]
    UnsupportedVersion(u16),
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("checksum mismatch: expected {expected:#010X}, got {actual:#010X}")]
    ChecksumMismatch { expected: u32, actual: u32 },
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("content hash mismatch: expected {expected}, got {actual}")]
    ContentHashMismatch { expected: String, actual: String },
    #[error("invalid signature")]
    InvalidSignature,
}

/// Compact binary encoding for AIR modules.
///
/// Layout: string table + module body using varint indices.
/// Much smaller than JSON for typical modules.
mod compact {
    use super::FormatError;
    use arc_ir::*;
    use std::collections::HashMap;

    /// Encode a u32 as a varint (LEB128-style).
    fn write_varint(buf: &mut Vec<u8>, mut val: u32) {
        loop {
            let byte = (val & 0x7F) as u8;
            val >>= 7;
            if val != 0 {
                buf.push(byte | 0x80);
            } else {
                buf.push(byte);
                break;
            }
        }
    }

    /// Read a varint from a byte slice, returning (value, bytes_consumed).
    fn read_varint(data: &[u8]) -> Result<(u32, usize), FormatError> {
        let mut result: u32 = 0;
        let mut shift = 0u32;
        for (i, &byte) in data.iter().enumerate() {
            result |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Ok((result, i + 1));
            }
            shift += 7;
            if shift >= 35 {
                return Err(FormatError::Serialization("varint too long".into()));
            }
        }
        Err(FormatError::TooShort)
    }

    fn write_i64(buf: &mut Vec<u8>, val: i64) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    fn read_i64(data: &[u8]) -> Result<(i64, usize), FormatError> {
        if data.len() < 8 {
            return Err(FormatError::TooShort);
        }
        let val = i64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]);
        Ok((val, 8))
    }

    /// String table: collects all strings and assigns varint indices.
    struct StringTable {
        strings: Vec<String>,
        indices: HashMap<String, u32>,
    }

    impl StringTable {
        fn new() -> Self {
            Self {
                strings: Vec::new(),
                indices: HashMap::new(),
            }
        }

        fn intern(&mut self, s: &str) -> u32 {
            if let Some(&idx) = self.indices.get(s) {
                return idx;
            }
            let idx = self.strings.len() as u32;
            self.indices.insert(s.to_string(), idx);
            self.strings.push(s.to_string());
            idx
        }

        fn encode(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            write_varint(&mut buf, self.strings.len() as u32);
            for s in &self.strings {
                write_varint(&mut buf, s.len() as u32);
                buf.extend_from_slice(s.as_bytes());
            }
            buf
        }

        fn decode(data: &[u8]) -> Result<(Self, usize), FormatError> {
            let mut pos = 0;
            let (count, n) = read_varint(&data[pos..])?;
            pos += n;
            let mut strings = Vec::with_capacity(count as usize);
            let mut indices = HashMap::new();
            for i in 0..count {
                let (len, n) = read_varint(&data[pos..])?;
                pos += n;
                let len = len as usize;
                if data.len() < pos + len {
                    return Err(FormatError::TooShort);
                }
                let s = String::from_utf8(data[pos..pos + len].to_vec())
                    .map_err(|e| FormatError::Serialization(e.to_string()))?;
                indices.insert(s.clone(), i);
                strings.push(s);
                pos += len;
            }
            Ok((Self { strings, indices }, pos))
        }

        fn get(&self, idx: u32) -> Result<&str, FormatError> {
            self.strings
                .get(idx as usize)
                .map(|s| s.as_str())
                .ok_or_else(|| FormatError::Serialization("invalid string index".into()))
        }
    }

    // --- Tag constants for OperationKind ---
    const TAG_CONST_I64: u8 = 1;
    const TAG_ADD: u8 = 2;
    const TAG_SUB: u8 = 3;
    const TAG_MUL: u8 = 4;
    const TAG_DIV: u8 = 5;
    const TAG_ICMP: u8 = 6;
    const TAG_ALLOC: u8 = 7;
    const TAG_LOAD: u8 = 8;
    const TAG_STORE: u8 = 9;
    const TAG_LOAD_ELEM: u8 = 10;
    const TAG_ASSUME: u8 = 11;
    const TAG_ASSERT: u8 = 12;
    const TAG_PROVE: u8 = 13;
    const TAG_REFINE: u8 = 14;
    const TAG_BRANCH: u8 = 15;
    const TAG_COND_BRANCH: u8 = 16;
    const TAG_CALL: u8 = 17;
    const TAG_REQUIRE_APPROVAL: u8 = 18;
    const TAG_INVOKE: u8 = 19;
    const TAG_RETURN: u8 = 20;
    const TAG_IF: u8 = 21;
    const TAG_LOOP: u8 = 22;
    const TAG_YIELD: u8 = 23;
    const TAG_SPAWN: u8 = 24;
    const TAG_AWAIT: u8 = 25;
    const TAG_CHECKPOINT: u8 = 26;
    const TAG_UNKNOWN: u8 = 27;

    const PRED_EQ: u8 = 0;
    const PRED_NE: u8 = 1;
    const PRED_SLT: u8 = 2;
    const PRED_SLE: u8 = 3;
    const PRED_SGT: u8 = 4;
    const PRED_SGE: u8 = 5;

    // --- Encoding ---

    pub fn encode_module(module: &Module) -> Result<Vec<u8>, FormatError> {
        let mut st = StringTable::new();
        // Pre-intern all strings
        collect_strings(module, &mut st);

        let mut body = Vec::new();
        // Module name
        let name_idx = st.intern(module.name.as_str());
        write_varint(&mut body, name_idx);

        // Capabilities
        write_varint(&mut body, module.capabilities.len() as u32);
        for (_, cap) in &module.capabilities {
            encode_capability(&mut body, cap, &mut st);
        }

        // Functions
        write_varint(&mut body, module.functions.len() as u32);
        for (_, func) in &module.functions {
            encode_function(&mut body, func, &mut st);
        }

        // Combine: string table + body
        let mut result = st.encode();
        result.extend_from_slice(&body);
        Ok(result)
    }

    fn collect_strings(module: &Module, st: &mut StringTable) {
        st.intern(module.name.as_str());
        for (_, cap) in &module.capabilities {
            st.intern(cap.name.as_str());
            for arg in &cap.inputs {
                st.intern(arg.name.as_str());
                st.intern(arg.ty.as_str());
            }
            for arg in &cap.outputs {
                st.intern(arg.name.as_str());
                st.intern(arg.ty.as_str());
            }
            for eff in &cap.effects {
                st.intern(eff);
            }
            for fail in &cap.failures {
                st.intern(fail);
            }
        }
        for (_, func) in &module.functions {
            st.intern(func.name.as_str());
            if let Some(rt) = &func.result {
                st.intern(rt.as_str());
            }
            for p in &func.params {
                st.intern(p.name.as_str());
                st.intern(p.ty.as_str());
            }
            for ip in &func.index_params {
                st.intern(ip.name.as_str());
            }
            for block in &func.blocks {
                if let Some(l) = &block.label {
                    st.intern(l);
                }
                for arg in &block.args {
                    st.intern(arg.name.as_str());
                    st.intern(arg.ty.as_str());
                }
                for op in &block.ops {
                    collect_op_strings(op, st);
                }
            }
        }
    }

    fn collect_op_strings(op: &Operation, st: &mut StringTable) {
        for r in &op.results {
            st.intern(r.as_str());
        }
        for o in &op.operands {
            st.intern(o.as_str());
        }
        for t in &op.result_types {
            st.intern(t.as_str());
        }
        for e in &op.effects {
            st.intern(e);
        }
        match &op.kind {
            OperationKind::Call { callee } => {
                st.intern(callee.as_str());
            }
            OperationKind::Invoke { capability } => {
                st.intern(capability.as_str());
            }
            OperationKind::Branch { target } => {
                st.intern(&target.label);
                for a in &target.arguments {
                    st.intern(a.as_str());
                }
            }
            OperationKind::CondBranch {
                true_target,
                false_target,
            } => {
                st.intern(&true_target.label);
                for a in &true_target.arguments {
                    st.intern(a.as_str());
                }
                st.intern(&false_target.label);
                for a in &false_target.arguments {
                    st.intern(a.as_str());
                }
            }
            OperationKind::Loop { iter_args } => {
                for a in iter_args {
                    st.intern(a.as_str());
                }
            }
            OperationKind::Unknown(name) => {
                st.intern(name);
            }
            _ => {}
        }
        for region in &op.regions {
            for block in &region.blocks {
                if let Some(l) = &block.label {
                    st.intern(l);
                }
                for arg in &block.args {
                    st.intern(arg.name.as_str());
                    st.intern(arg.ty.as_str());
                }
                for rop in &block.ops {
                    collect_op_strings(rop, st);
                }
            }
        }
    }

    fn encode_capability(buf: &mut Vec<u8>, cap: &Capability, st: &mut StringTable) {
        write_varint(buf, st.intern(cap.name.as_str()));
        write_varint(buf, cap.inputs.len() as u32);
        for arg in &cap.inputs {
            encode_argument(buf, arg, st);
        }
        write_varint(buf, cap.outputs.len() as u32);
        for arg in &cap.outputs {
            encode_argument(buf, arg, st);
        }
        write_varint(buf, cap.effects.len() as u32);
        for eff in &cap.effects {
            write_varint(buf, st.intern(eff));
        }
        write_varint(buf, cap.failures.len() as u32);
        for fail in &cap.failures {
            write_varint(buf, st.intern(fail));
        }
        encode_location(buf, cap.location);
    }

    fn encode_argument(buf: &mut Vec<u8>, arg: &Argument, st: &mut StringTable) {
        write_varint(buf, st.intern(arg.name.as_str()));
        write_varint(buf, st.intern(arg.ty.as_str()));
        encode_location(buf, arg.location);
    }

    fn encode_location(buf: &mut Vec<u8>, loc: Location) {
        write_varint(buf, loc.offset as u32);
        write_varint(buf, loc.length as u32);
    }

    fn encode_function(buf: &mut Vec<u8>, func: &Function, st: &mut StringTable) {
        write_varint(buf, st.intern(func.name.as_str()));
        // index params
        write_varint(buf, func.index_params.len() as u32);
        for ip in &func.index_params {
            write_varint(buf, st.intern(ip.name.as_str()));
            encode_location(buf, ip.location);
        }
        // params
        write_varint(buf, func.params.len() as u32);
        for p in &func.params {
            encode_argument(buf, p, st);
        }
        // result type
        match &func.result {
            Some(t) => {
                buf.push(1);
                write_varint(buf, st.intern(t.as_str()));
            }
            None => buf.push(0),
        }
        encode_location(buf, func.location);
        // blocks
        write_varint(buf, func.blocks.len() as u32);
        for block in &func.blocks {
            encode_block(buf, block, st);
        }
    }

    fn encode_block(buf: &mut Vec<u8>, block: &Block, st: &mut StringTable) {
        match &block.label {
            Some(l) => {
                buf.push(1);
                write_varint(buf, st.intern(l));
            }
            None => buf.push(0),
        }
        encode_location(buf, block.location);
        write_varint(buf, block.args.len() as u32);
        for arg in &block.args {
            encode_argument(buf, arg, st);
        }
        write_varint(buf, block.ops.len() as u32);
        for op in &block.ops {
            encode_op(buf, op, st);
        }
    }

    fn encode_block_target(buf: &mut Vec<u8>, target: &BlockTarget, st: &mut StringTable) {
        write_varint(buf, st.intern(&target.label));
        write_varint(buf, target.arguments.len() as u32);
        for a in &target.arguments {
            write_varint(buf, st.intern(a.as_str()));
        }
    }

    fn encode_op(buf: &mut Vec<u8>, op: &Operation, st: &mut StringTable) {
        // results
        write_varint(buf, op.results.len() as u32);
        for r in &op.results {
            write_varint(buf, st.intern(r.as_str()));
        }
        // kind tag + kind-specific data
        match &op.kind {
            OperationKind::ConstI64(val) => {
                buf.push(TAG_CONST_I64);
                write_i64(buf, *val);
            }
            OperationKind::Add => buf.push(TAG_ADD),
            OperationKind::Sub => buf.push(TAG_SUB),
            OperationKind::Mul => buf.push(TAG_MUL),
            OperationKind::Div => buf.push(TAG_DIV),
            OperationKind::ICmp { predicate } => {
                buf.push(TAG_ICMP);
                buf.push(match predicate {
                    IcmpPredicate::Eq => PRED_EQ,
                    IcmpPredicate::Ne => PRED_NE,
                    IcmpPredicate::Slt => PRED_SLT,
                    IcmpPredicate::Sle => PRED_SLE,
                    IcmpPredicate::Sgt => PRED_SGT,
                    IcmpPredicate::Sge => PRED_SGE,
                });
            }
            OperationKind::Alloc => buf.push(TAG_ALLOC),
            OperationKind::Load => buf.push(TAG_LOAD),
            OperationKind::Store => buf.push(TAG_STORE),
            OperationKind::LoadElem => buf.push(TAG_LOAD_ELEM),
            OperationKind::Assume => buf.push(TAG_ASSUME),
            OperationKind::Assert => buf.push(TAG_ASSERT),
            OperationKind::Prove => buf.push(TAG_PROVE),
            OperationKind::Refine => buf.push(TAG_REFINE),
            OperationKind::Branch { target } => {
                buf.push(TAG_BRANCH);
                encode_block_target(buf, target, st);
            }
            OperationKind::CondBranch {
                true_target,
                false_target,
            } => {
                buf.push(TAG_COND_BRANCH);
                encode_block_target(buf, true_target, st);
                encode_block_target(buf, false_target, st);
            }
            OperationKind::Call { callee } => {
                buf.push(TAG_CALL);
                write_varint(buf, st.intern(callee.as_str()));
            }
            OperationKind::RequireApproval => buf.push(TAG_REQUIRE_APPROVAL),
            OperationKind::Invoke { capability } => {
                buf.push(TAG_INVOKE);
                write_varint(buf, st.intern(capability.as_str()));
            }
            OperationKind::Return => buf.push(TAG_RETURN),
            OperationKind::If => buf.push(TAG_IF),
            OperationKind::Loop { iter_args } => {
                buf.push(TAG_LOOP);
                write_varint(buf, iter_args.len() as u32);
                for a in iter_args {
                    write_varint(buf, st.intern(a.as_str()));
                }
            }
            OperationKind::Yield => buf.push(TAG_YIELD),
            OperationKind::Spawn { callee } => {
                buf.push(TAG_SPAWN);
                write_varint(buf, st.intern(callee.as_str()));
            }
            OperationKind::Await => buf.push(TAG_AWAIT),
            OperationKind::Checkpoint { label } => {
                buf.push(TAG_CHECKPOINT);
                write_varint(buf, st.intern(label.as_str()));
            }
            OperationKind::Unknown(name) => {
                buf.push(TAG_UNKNOWN);
                write_varint(buf, st.intern(name));
            }
        }
        // operands
        write_varint(buf, op.operands.len() as u32);
        for o in &op.operands {
            write_varint(buf, st.intern(o.as_str()));
        }
        // result types
        write_varint(buf, op.result_types.len() as u32);
        for t in &op.result_types {
            write_varint(buf, st.intern(t.as_str()));
        }
        // effects
        write_varint(buf, op.effects.len() as u32);
        for e in &op.effects {
            write_varint(buf, st.intern(e));
        }
        encode_location(buf, op.location);
        // regions
        write_varint(buf, op.regions.len() as u32);
        for region in &op.regions {
            write_varint(buf, region.blocks.len() as u32);
            for block in &region.blocks {
                encode_block(buf, block, st);
            }
        }
    }

    // --- Decoding ---

    struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        fn new(data: &'a [u8]) -> Self {
            Self { data, pos: 0 }
        }

        fn varint(&mut self) -> Result<u32, FormatError> {
            let (val, n) = read_varint(&self.data[self.pos..])?;
            self.pos += n;
            Ok(val)
        }

        fn i64(&mut self) -> Result<i64, FormatError> {
            let (val, n) = read_i64(&self.data[self.pos..])?;
            self.pos += n;
            Ok(val)
        }

        fn byte(&mut self) -> Result<u8, FormatError> {
            if self.pos >= self.data.len() {
                return Err(FormatError::TooShort);
            }
            let b = self.data[self.pos];
            self.pos += 1;
            Ok(b)
        }
    }

    pub fn decode_module(data: &[u8]) -> Result<Module, FormatError> {
        let (st, st_len) = StringTable::decode(data)?;
        let mut r = Reader::new(&data[st_len..]);

        let name_idx = r.varint()?;
        let name = st.get(name_idx)?;
        let mut module = Module::new(Symbol::new(name));

        // Capabilities
        let cap_count = r.varint()?;
        for _ in 0..cap_count {
            let cap = decode_capability(&mut r, &st)?;
            module
                .add_capability(cap)
                .map_err(|e| FormatError::Serialization(e.to_string()))?;
        }

        // Functions
        let func_count = r.varint()?;
        for _ in 0..func_count {
            let func = decode_function(&mut r, &st)?;
            module
                .add_function(func)
                .map_err(|e| FormatError::Serialization(e.to_string()))?;
        }

        Ok(module)
    }

    fn decode_location(r: &mut Reader) -> Result<Location, FormatError> {
        let offset = r.varint()? as usize;
        let length = r.varint()? as usize;
        Ok(Location::new(offset, length))
    }

    fn decode_argument(r: &mut Reader, st: &StringTable) -> Result<Argument, FormatError> {
        let name_idx = r.varint()?;
        let ty_idx = r.varint()?;
        let loc = decode_location(r)?;
        Ok(Argument {
            name: ValueId::new(st.get(name_idx)?),
            ty: Type::new(st.get(ty_idx)?),
            location: loc,
        })
    }

    fn decode_capability(r: &mut Reader, st: &StringTable) -> Result<Capability, FormatError> {
        let name_idx = r.varint()?;
        let input_count = r.varint()?;
        let mut inputs = Vec::with_capacity(input_count as usize);
        for _ in 0..input_count {
            inputs.push(decode_argument(r, st)?);
        }
        let output_count = r.varint()?;
        let mut outputs = Vec::with_capacity(output_count as usize);
        for _ in 0..output_count {
            outputs.push(decode_argument(r, st)?);
        }
        let eff_count = r.varint()?;
        let mut effects = Vec::with_capacity(eff_count as usize);
        for _ in 0..eff_count {
            effects.push(st.get(r.varint()?)?.to_string());
        }
        let fail_count = r.varint()?;
        let mut failures = Vec::with_capacity(fail_count as usize);
        for _ in 0..fail_count {
            failures.push(st.get(r.varint()?)?.to_string());
        }
        let loc = decode_location(r)?;
        Ok(Capability {
            name: Symbol::new(st.get(name_idx)?),
            inputs,
            outputs,
            effects,
            failures,
            location: loc,
        })
    }

    fn decode_function(r: &mut Reader, st: &StringTable) -> Result<Function, FormatError> {
        let name_idx = r.varint()?;
        // index params
        let ip_count = r.varint()?;
        let mut index_params = Vec::with_capacity(ip_count as usize);
        for _ in 0..ip_count {
            let n_idx = r.varint()?;
            let loc = decode_location(r)?;
            index_params.push(IndexParam {
                name: ValueId::new(st.get(n_idx)?),
                location: loc,
            });
        }
        // params
        let p_count = r.varint()?;
        let mut params = Vec::with_capacity(p_count as usize);
        for _ in 0..p_count {
            params.push(decode_argument(r, st)?);
        }
        // result
        let has_result = r.byte()?;
        let result = if has_result != 0 {
            let t_idx = r.varint()?;
            Some(Type::new(st.get(t_idx)?))
        } else {
            None
        };
        let loc = decode_location(r)?;
        let mut func = Function::new(
            Symbol::new(st.get(name_idx)?),
            index_params,
            params,
            result,
            loc,
        );
        // blocks
        let block_count = r.varint()?;
        for _ in 0..block_count {
            func.add_block(decode_block(r, st)?);
        }
        Ok(func)
    }

    fn decode_block(r: &mut Reader, st: &StringTable) -> Result<Block, FormatError> {
        let has_label = r.byte()?;
        let label = if has_label != 0 {
            let l_idx = r.varint()?;
            Some(st.get(l_idx)?.into())
        } else {
            None
        };
        let loc = decode_location(r)?;
        let mut block = Block::new(label, loc);
        let arg_count = r.varint()?;
        for _ in 0..arg_count {
            block.add_arg(decode_argument(r, st)?);
        }
        let op_count = r.varint()?;
        for _ in 0..op_count {
            block.add_op(decode_op(r, st)?);
        }
        Ok(block)
    }

    fn decode_block_target(r: &mut Reader, st: &StringTable) -> Result<BlockTarget, FormatError> {
        let l_idx = r.varint()?;
        let arg_count = r.varint()?;
        let mut arguments = Vec::with_capacity(arg_count as usize);
        for _ in 0..arg_count {
            arguments.push(ValueId::new(st.get(r.varint()?)?));
        }
        Ok(BlockTarget::new(st.get(l_idx)?.into(), arguments))
    }

    fn decode_op(r: &mut Reader, st: &StringTable) -> Result<Operation, FormatError> {
        // results
        let res_count = r.varint()?;
        let mut results = Vec::with_capacity(res_count as usize);
        for _ in 0..res_count {
            results.push(ValueId::new(st.get(r.varint()?)?));
        }
        // kind
        let tag = r.byte()?;
        let kind = match tag {
            TAG_CONST_I64 => OperationKind::ConstI64(r.i64()?),
            TAG_ADD => OperationKind::Add,
            TAG_SUB => OperationKind::Sub,
            TAG_MUL => OperationKind::Mul,
            TAG_DIV => OperationKind::Div,
            TAG_ICMP => {
                let pred = match r.byte()? {
                    PRED_EQ => IcmpPredicate::Eq,
                    PRED_NE => IcmpPredicate::Ne,
                    PRED_SLT => IcmpPredicate::Slt,
                    PRED_SLE => IcmpPredicate::Sle,
                    PRED_SGT => IcmpPredicate::Sgt,
                    PRED_SGE => IcmpPredicate::Sge,
                    other => {
                        return Err(FormatError::Serialization(format!(
                            "unknown icmp predicate: {}",
                            other
                        )))
                    }
                };
                OperationKind::ICmp { predicate: pred }
            }
            TAG_ALLOC => OperationKind::Alloc,
            TAG_LOAD => OperationKind::Load,
            TAG_STORE => OperationKind::Store,
            TAG_LOAD_ELEM => OperationKind::LoadElem,
            TAG_ASSUME => OperationKind::Assume,
            TAG_ASSERT => OperationKind::Assert,
            TAG_PROVE => OperationKind::Prove,
            TAG_REFINE => OperationKind::Refine,
            TAG_BRANCH => {
                let target = decode_block_target(r, st)?;
                OperationKind::Branch { target }
            }
            TAG_COND_BRANCH => {
                let true_target = decode_block_target(r, st)?;
                let false_target = decode_block_target(r, st)?;
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                }
            }
            TAG_CALL => {
                let c_idx = r.varint()?;
                OperationKind::Call {
                    callee: Symbol::new(st.get(c_idx)?),
                }
            }
            TAG_REQUIRE_APPROVAL => OperationKind::RequireApproval,
            TAG_INVOKE => {
                let c_idx = r.varint()?;
                OperationKind::Invoke {
                    capability: Symbol::new(st.get(c_idx)?),
                }
            }
            TAG_RETURN => OperationKind::Return,
            TAG_IF => OperationKind::If,
            TAG_LOOP => {
                let ia_count = r.varint()?;
                let mut iter_args = Vec::with_capacity(ia_count as usize);
                for _ in 0..ia_count {
                    iter_args.push(ValueId::new(st.get(r.varint()?)?));
                }
                OperationKind::Loop { iter_args }
            }
            TAG_YIELD => OperationKind::Yield,
            TAG_SPAWN => {
                let c_idx = r.varint()?;
                OperationKind::Spawn {
                    callee: Symbol::new(st.get(c_idx)?),
                }
            }
            TAG_AWAIT => OperationKind::Await,
            TAG_CHECKPOINT => {
                let l_idx = r.varint()?;
                OperationKind::Checkpoint {
                    label: st.get(l_idx)?.into(),
                }
            }
            TAG_UNKNOWN => {
                let n_idx = r.varint()?;
                OperationKind::Unknown(st.get(n_idx)?.into())
            }
            other => {
                return Err(FormatError::Serialization(format!(
                    "unknown op tag: {}",
                    other
                )))
            }
        };
        // operands
        let op_count = r.varint()?;
        let mut operands = Vec::with_capacity(op_count as usize);
        for _ in 0..op_count {
            operands.push(ValueId::new(st.get(r.varint()?)?));
        }
        // result types
        let rt_count = r.varint()?;
        let mut result_types = Vec::with_capacity(rt_count as usize);
        for _ in 0..rt_count {
            result_types.push(Type::new(st.get(r.varint()?)?));
        }
        // effects
        let eff_count = r.varint()?;
        let mut effects = Vec::with_capacity(eff_count as usize);
        for _ in 0..eff_count {
            effects.push(st.get(r.varint()?)?.to_string());
        }
        let location = decode_location(r)?;
        // regions
        let region_count = r.varint()?;
        let mut regions = Vec::with_capacity(region_count as usize);
        for _ in 0..region_count {
            let block_count = r.varint()?;
            let mut blocks = Vec::with_capacity(block_count as usize);
            for _ in 0..block_count {
                blocks.push(decode_block(r, st)?);
            }
            regions.push(Region { blocks });
        }

        Ok(Operation {
            results,
            kind,
            operands,
            result_types,
            effects,
            location,
            regions,
        })
    }
}

// ---------------------------------------------------------------------------
// Content-addressing (SHA-256) and package signing
// ---------------------------------------------------------------------------

/// SHA-256 content hash of binary module data.
pub mod content_hash {
    /// A 32-byte SHA-256 digest.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ContentHash(pub [u8; 32]);

    impl ContentHash {
        /// Format the hash as a hex string.
        pub fn to_hex(&self) -> String {
            self.0.iter().map(|b| format!("{:02x}", b)).collect()
        }

        /// Parse from a 64-character hex string.
        pub fn from_hex(s: &str) -> Option<Self> {
            if s.len() != 64 {
                return None;
            }
            let mut bytes = [0u8; 32];
            for i in 0..32 {
                bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(Self(bytes))
        }
    }

    /// Compute the SHA-256 hash of a byte slice (pure Rust implementation).
    pub fn sha256(data: &[u8]) -> ContentHash {
        ContentHash(sha256_digest(data))
    }

    // --- Minimal SHA-256 implementation (FIPS 180-4) ---

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn sha256_digest(data: &[u8]) -> [u8; 32] {
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        // Pre-processing: pad the message
        let bit_len = (data.len() as u64) * 8;
        let mut padded = data.to_vec();
        padded.push(0x80);
        while (padded.len() % 64) != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        // Process each 512-bit (64-byte) block
        for chunk in padded.chunks_exact(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);

                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut result = [0u8; 32];
        for (i, &val) in h.iter().enumerate() {
            result[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
        }
        result
    }

    /// HMAC-SHA256 for signing.
    pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
        let block_size = 64;
        let mut k = [0u8; 64];
        if key.len() > block_size {
            let h = sha256_digest(key);
            k[..32].copy_from_slice(&h);
        } else {
            k[..key.len()].copy_from_slice(key);
        }

        let mut ipad = [0x36u8; 64];
        let mut opad = [0x5cu8; 64];
        for i in 0..64 {
            ipad[i] ^= k[i];
            opad[i] ^= k[i];
        }

        // inner hash: H(ipad || message)
        let mut inner_data = Vec::with_capacity(64 + message.len());
        inner_data.extend_from_slice(&ipad);
        inner_data.extend_from_slice(message);
        let inner_hash = sha256_digest(&inner_data);

        // outer hash: H(opad || inner_hash)
        let mut outer_data = Vec::with_capacity(64 + 32);
        outer_data.extend_from_slice(&opad);
        outer_data.extend_from_slice(&inner_hash);
        sha256_digest(&outer_data)
    }
}

/// Package signing and verification.
pub mod signing {
    use super::content_hash::{hmac_sha256, sha256, ContentHash};
    use super::FormatError;

    /// A cryptographic signature over module content.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Signature {
        /// The signature bytes (32 bytes for HMAC-SHA256).
        pub bytes: Vec<u8>,
        /// The signer identity (e.g., key ID or name).
        pub signer: String,
    }

    /// A signed package: module binary + content hash + signature.
    #[derive(Debug, Clone)]
    pub struct SignedPackage {
        /// The raw encoded module data (AIRB binary).
        pub data: Vec<u8>,
        /// SHA-256 content hash of `data`.
        pub content_hash: ContentHash,
        /// Optional signature.
        pub signature: Option<Signature>,
    }

    /// Trait for signing backends.
    pub trait Signer {
        /// Sign the given data and return a Signature.
        fn sign(&self, data: &[u8]) -> Signature;

        /// Verify a signature over data.
        fn verify(&self, data: &[u8], sig: &Signature) -> bool;

        /// The signer's identity string.
        fn identity(&self) -> &str;
    }

    /// HMAC-SHA256 signer (symmetric key scheme).
    pub struct HmacSigner {
        key: Vec<u8>,
        identity: String,
    }

    impl HmacSigner {
        pub fn new(key: &[u8], identity: &str) -> Self {
            Self {
                key: key.to_vec(),
                identity: identity.to_string(),
            }
        }
    }

    impl Signer for HmacSigner {
        fn sign(&self, data: &[u8]) -> Signature {
            let mac = hmac_sha256(&self.key, data);
            Signature {
                bytes: mac.to_vec(),
                signer: self.identity.clone(),
            }
        }

        fn verify(&self, data: &[u8], sig: &Signature) -> bool {
            let expected = hmac_sha256(&self.key, data);
            constant_time_eq(&expected, &sig.bytes)
        }

        fn identity(&self) -> &str {
            &self.identity
        }
    }

    /// Constant-time comparison to avoid timing attacks.
    fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut diff = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }

    /// Create a signed package from encoded module data.
    pub fn sign_package(data: Vec<u8>, signer: &dyn Signer) -> SignedPackage {
        let content_hash = sha256(&data);
        let signature = signer.sign(&data);
        SignedPackage {
            data,
            content_hash,
            signature: Some(signature),
        }
    }

    /// Create an unsigned package (content-addressed only).
    pub fn package(data: Vec<u8>) -> SignedPackage {
        let content_hash = sha256(&data);
        SignedPackage {
            data,
            content_hash,
            signature: None,
        }
    }

    /// Verify a signed package: check content hash and signature.
    pub fn verify_package(pkg: &SignedPackage, verifier: &dyn Signer) -> Result<(), FormatError> {
        // Verify content hash
        let actual_hash = sha256(&pkg.data);
        if actual_hash != pkg.content_hash {
            return Err(FormatError::ContentHashMismatch {
                expected: pkg.content_hash.to_hex(),
                actual: actual_hash.to_hex(),
            });
        }

        // Verify signature if present
        if let Some(sig) = &pkg.signature {
            if !verifier.verify(&pkg.data, sig) {
                return Err(FormatError::InvalidSignature);
            }
        }

        Ok(())
    }

    /// Serialize a SignedPackage to bytes.
    ///
    /// Layout:
    /// ```text
    /// [magic: 4 bytes "AIRP"]
    /// [content_hash: 32 bytes]
    /// [has_signature: 1 byte]
    /// [signature_len: u32 LE] (if signed)
    /// [signer_len: u32 LE] (if signed)
    /// [signature_bytes] (if signed)
    /// [signer_bytes] (if signed)
    /// [data_len: u32 LE]
    /// [data]
    /// ```
    pub fn serialize_package(pkg: &SignedPackage) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"AIRP");
        buf.extend_from_slice(&pkg.content_hash.0);

        match &pkg.signature {
            Some(sig) => {
                buf.push(1);
                buf.extend_from_slice(&(sig.bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(sig.signer.len() as u32).to_le_bytes());
                buf.extend_from_slice(&sig.bytes);
                buf.extend_from_slice(sig.signer.as_bytes());
            }
            None => {
                buf.push(0);
            }
        }

        buf.extend_from_slice(&(pkg.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&pkg.data);
        buf
    }

    /// Deserialize a SignedPackage from bytes.
    pub fn deserialize_package(data: &[u8]) -> Result<SignedPackage, FormatError> {
        if data.len() < 37 {
            // 4 magic + 32 hash + 1 has_sig
            return Err(FormatError::TooShort);
        }
        if &data[0..4] != b"AIRP" {
            return Err(FormatError::InvalidMagic);
        }

        let mut content_hash = [0u8; 32];
        content_hash.copy_from_slice(&data[4..36]);
        let has_sig = data[36];

        let mut pos = 37;
        let signature = if has_sig != 0 {
            if data.len() < pos + 8 {
                return Err(FormatError::TooShort);
            }
            let sig_len =
                u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            pos += 4;
            let signer_len =
                u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            pos += 4;
            if data.len() < pos + sig_len + signer_len {
                return Err(FormatError::TooShort);
            }
            let sig_bytes = data[pos..pos + sig_len].to_vec();
            pos += sig_len;
            let signer = String::from_utf8(data[pos..pos + signer_len].to_vec())
                .map_err(|e| FormatError::Serialization(e.to_string()))?;
            pos += signer_len;
            Some(Signature {
                bytes: sig_bytes,
                signer,
            })
        } else {
            None
        };

        if data.len() < pos + 4 {
            return Err(FormatError::TooShort);
        }
        let data_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if data.len() < pos + data_len {
            return Err(FormatError::TooShort);
        }
        let module_data = data[pos..pos + data_len].to_vec();

        Ok(SignedPackage {
            data: module_data,
            content_hash: ContentHash(content_hash),
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::*;

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn sample_module() -> Module {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("add"),
            vec![],
            vec![
                Argument {
                    name: ValueId::new("a"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("b"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
            ],
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation {
            results: vec![ValueId::new("c")],
            kind: OperationKind::Add,
            operands: vec![ValueId::new("a"), ValueId::new("b")],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: loc(),
            regions: vec![],
        });
        block.add_op(Operation {
            results: vec![],
            kind: OperationKind::Return,
            operands: vec![ValueId::new("c")],
            result_types: vec![],
            effects: vec![],
            location: loc(),
            regions: vec![],
        });
        func.add_block(block);
        module.add_function(func).unwrap();
        module
    }

    #[test]
    fn encode_decode_roundtrip() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "test");
        assert!(decoded.functions.contains_key(&Symbol::new("add")));
    }

    #[test]
    fn magic_header() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        assert_eq!(&encoded[0..4], b"AIRB");
    }

    #[test]
    fn version_header() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let version = u16::from_le_bytes([encoded[4], encoded[5]]);
        assert_eq!(version, 1);
    }

    #[test]
    fn invalid_magic_rejected() {
        let bad_data = b"BAAD\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert!(matches!(decode(bad_data), Err(FormatError::InvalidMagic)));
    }

    #[test]
    fn too_short_rejected() {
        assert!(matches!(decode(b"AIR"), Err(FormatError::TooShort)));
    }

    #[test]
    fn tampered_data_detected() {
        let module = sample_module();
        let mut encoded = encode(&module).unwrap();
        // Tamper with the JSON data.
        if encoded.len() > 15 {
            encoded[14] ^= 0xFF;
        }
        assert!(matches!(
            decode(&encoded),
            Err(FormatError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn unsupported_version_rejected() {
        let module = sample_module();
        let mut encoded = encode(&module).unwrap();
        // Change version to 99.
        encoded[4] = 99;
        encoded[5] = 0;
        assert!(matches!(
            decode(&encoded),
            Err(FormatError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn crc32_deterministic() {
        let a = crc32(b"hello world");
        let b = crc32(b"hello world");
        assert_eq!(a, b);
        let c = crc32(b"hello worlD");
        assert_ne!(a, c);
    }

    #[test]
    fn format_flags() {
        assert!(!FormatFlags::NONE.is_compressed());
        assert!(!FormatFlags::NONE.is_signed());
        assert!(FormatFlags::COMPRESSED.is_compressed());
        assert!(FormatFlags::SIGNED.is_signed());
    }

    #[test]
    fn empty_module_roundtrip() {
        let module = Module::new(Symbol::new("empty"));
        let encoded = encode(&module).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "empty");
        assert!(decoded.functions.is_empty());
    }

    // --- Compact format tests ---

    #[test]
    fn compact_roundtrip() {
        let module = sample_module();
        let encoded = encode_compact(&module).unwrap();
        let decoded = decode_compact(&encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "test");
        assert!(decoded.functions.contains_key(&Symbol::new("add")));
        let func = &decoded.functions[&Symbol::new("add")];
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.blocks.len(), 1);
        assert_eq!(func.blocks[0].ops.len(), 2);
    }

    #[test]
    fn compact_smaller_than_json() {
        let module = sample_module();
        let json_encoded = encode(&module).unwrap();
        let compact_encoded = encode_compact(&module).unwrap();
        assert!(
            compact_encoded.len() < json_encoded.len(),
            "compact ({}) should be smaller than JSON ({})",
            compact_encoded.len(),
            json_encoded.len()
        );
    }

    #[test]
    fn compact_magic_and_version() {
        let module = sample_module();
        let encoded = encode_compact(&module).unwrap();
        assert_eq!(&encoded[0..4], b"AIRB");
        let version = u16::from_le_bytes([encoded[4], encoded[5]]);
        assert_eq!(version, 2);
    }

    #[test]
    fn compact_empty_module() {
        let module = Module::new(Symbol::new("empty"));
        let encoded = encode_compact(&module).unwrap();
        let decoded = decode_compact(&encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "empty");
        assert!(decoded.functions.is_empty());
    }

    #[test]
    fn compact_tampered_data_detected() {
        let module = sample_module();
        let mut encoded = encode_compact(&module).unwrap();
        if encoded.len() > 15 {
            encoded[14] ^= 0xFF;
        }
        assert!(matches!(
            decode_compact(&encoded),
            Err(FormatError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn decode_auto_detects_compact() {
        // The generic decode() should accept version 2 (compact)
        let module = sample_module();
        let encoded = encode_compact(&module).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded.name.as_str(), "test");
    }

    #[test]
    fn compact_preserves_operations() {
        let module = sample_module();
        let encoded = encode_compact(&module).unwrap();
        let decoded = decode_compact(&encoded).unwrap();
        let func = &decoded.functions[&Symbol::new("add")];
        let ops = &func.blocks[0].ops;
        assert!(matches!(ops[0].kind, OperationKind::Add));
        assert_eq!(ops[0].results[0].as_str(), "c");
        assert_eq!(ops[0].operands[0].as_str(), "a");
        assert_eq!(ops[0].operands[1].as_str(), "b");
        assert!(matches!(ops[1].kind, OperationKind::Return));
    }

    // --- Content hash tests ---

    #[test]
    fn sha256_known_vector() {
        // SHA-256 of empty string
        let hash = content_hash::sha256(b"");
        assert_eq!(
            hash.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        let hash = content_hash::sha256(b"abc");
        assert_eq!(
            hash.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_deterministic() {
        let a = content_hash::sha256(b"hello world");
        let b = content_hash::sha256(b"hello world");
        assert_eq!(a, b);
        let c = content_hash::sha256(b"hello worlD");
        assert_ne!(a, c);
    }

    #[test]
    fn content_hash_hex_roundtrip() {
        let hash = content_hash::sha256(b"test data");
        let hex = hash.to_hex();
        let restored = content_hash::ContentHash::from_hex(&hex).unwrap();
        assert_eq!(hash, restored);
    }

    #[test]
    fn content_hash_from_hex_rejects_bad_input() {
        assert!(content_hash::ContentHash::from_hex("too_short").is_none());
        assert!(content_hash::ContentHash::from_hex(
            "gg00000000000000000000000000000000000000000000000000000000000000"
        )
        .is_none());
    }

    #[test]
    fn content_hash_of_module() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let hash = content_hash::sha256(&encoded);
        assert_eq!(hash.to_hex().len(), 64);
        // Same encoding should give same hash
        let hash2 = content_hash::sha256(&encoded);
        assert_eq!(hash, hash2);
    }

    // --- Signing tests ---

    #[test]
    fn sign_and_verify_package() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let signer = signing::HmacSigner::new(b"secret-key", "test-signer");
        let pkg = signing::sign_package(encoded, &signer);

        assert!(pkg.signature.is_some());
        assert_eq!(pkg.signature.as_ref().unwrap().signer, "test-signer");
        assert!(signing::verify_package(&pkg, &signer).is_ok());
    }

    #[test]
    fn unsigned_package_content_hash() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let pkg = signing::package(encoded.clone());

        assert!(pkg.signature.is_none());
        assert_eq!(pkg.content_hash, content_hash::sha256(&encoded));
    }

    #[test]
    fn tampered_package_detected() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let signer = signing::HmacSigner::new(b"secret-key", "test-signer");
        let mut pkg = signing::sign_package(encoded, &signer);

        // Tamper with data
        if !pkg.data.is_empty() {
            pkg.data[0] ^= 0xFF;
        }

        let result = signing::verify_package(&pkg, &signer);
        assert!(result.is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let signer = signing::HmacSigner::new(b"correct-key", "signer");
        let pkg = signing::sign_package(encoded, &signer);

        let wrong_signer = signing::HmacSigner::new(b"wrong-key", "signer");
        let result = signing::verify_package(&pkg, &wrong_signer);
        assert!(result.is_err());
    }

    #[test]
    fn signed_package_serialize_roundtrip() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let signer = signing::HmacSigner::new(b"test-key", "alice");
        let pkg = signing::sign_package(encoded, &signer);

        let serialized = signing::serialize_package(&pkg);
        assert_eq!(&serialized[0..4], b"AIRP");

        let restored = signing::deserialize_package(&serialized).unwrap();
        assert_eq!(restored.content_hash, pkg.content_hash);
        assert_eq!(restored.signature, pkg.signature);
        assert_eq!(restored.data, pkg.data);
    }

    #[test]
    fn unsigned_package_serialize_roundtrip() {
        let module = sample_module();
        let encoded = encode(&module).unwrap();
        let pkg = signing::package(encoded);

        let serialized = signing::serialize_package(&pkg);
        let restored = signing::deserialize_package(&serialized).unwrap();
        assert_eq!(restored.content_hash, pkg.content_hash);
        assert!(restored.signature.is_none());
        assert_eq!(restored.data, pkg.data);
    }

    #[test]
    fn hmac_sha256_deterministic() {
        let mac1 = content_hash::hmac_sha256(b"key", b"message");
        let mac2 = content_hash::hmac_sha256(b"key", b"message");
        assert_eq!(mac1, mac2);
        let mac3 = content_hash::hmac_sha256(b"key", b"other");
        assert_ne!(mac1, mac3);
    }
}
