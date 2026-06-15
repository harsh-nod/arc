//! AIR type system: type parsing, subtyping, refinement types, and normalization.

use arc_ir::Type;

/// Parsed AIR type representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// Integer types: i1, i8, i16, i32, i64
    Int { width: u8 },
    /// Index type (size-dependent integer)
    Index,
    /// Memory resource type
    Mem,
    /// Pointer type
    Ptr { pointee: Box<TypeKind> },
    /// Slice type: `!arc.slice<elem_ty, length>`
    Slice {
        elem: Box<TypeKind>,
        length: Option<u64>,
    },
    /// Proof type: `!arc.proof<predicate>`
    Proof { predicate: String },
    /// Authority token type: `!arc.auth<capability>`
    Auth { capability: String },
    /// Refined type: `!arc.refined<base, predicate>`
    Refined {
        base: Box<TypeKind>,
        predicate: String,
    },
    /// Function type: (args) -> result
    Func {
        params: Vec<TypeKind>,
        result: Option<Box<TypeKind>>,
    },
    /// Tuple type: (t1, t2, ...)
    Tuple { elements: Vec<TypeKind> },
    /// Unknown or opaque type
    Unknown(String),
}

/// Parse an AIR type string into a structured TypeKind.
pub fn parse_type(repr: &str) -> TypeKind {
    let s = repr.trim();
    match s {
        "i1" => TypeKind::Int { width: 1 },
        "i8" => TypeKind::Int { width: 8 },
        "i16" => TypeKind::Int { width: 16 },
        "i32" => TypeKind::Int { width: 32 },
        "i64" => TypeKind::Int { width: 64 },
        "index" => TypeKind::Index,
        "!arc.mem" => TypeKind::Mem,
        _ if s.starts_with("!arc.ptr<") && s.ends_with('>') => {
            let inner = &s[9..s.len() - 1];
            TypeKind::Ptr {
                pointee: Box::new(parse_type(inner)),
            }
        }
        _ if s.starts_with("!arc.slice<") && s.ends_with('>') => {
            let inner = &s[11..s.len() - 1];
            // Parse "elem_ty, length" or just "elem_ty"
            if let Some(comma) = inner.rfind(',') {
                let elem_str = inner[..comma].trim();
                let len_str = inner[comma + 1..].trim();
                let length = len_str.parse::<u64>().ok();
                TypeKind::Slice {
                    elem: Box::new(parse_type(elem_str)),
                    length,
                }
            } else {
                TypeKind::Slice {
                    elem: Box::new(parse_type(inner)),
                    length: None,
                }
            }
        }
        _ if s.starts_with("!arc.proof<") && s.ends_with('>') => {
            let inner = &s[11..s.len() - 1];
            TypeKind::Proof {
                predicate: inner.to_string(),
            }
        }
        _ if s.starts_with("!arc.auth<") && s.ends_with('>') => {
            let inner = &s[10..s.len() - 1];
            TypeKind::Auth {
                capability: inner.to_string(),
            }
        }
        _ if s.starts_with("!arc.refined<") && s.ends_with('>') => {
            let inner = &s[13..s.len() - 1];
            if let Some(comma) = inner.find(',') {
                let base_str = inner[..comma].trim();
                let pred_str = inner[comma + 1..].trim();
                TypeKind::Refined {
                    base: Box::new(parse_type(base_str)),
                    predicate: pred_str.to_string(),
                }
            } else {
                TypeKind::Unknown(s.to_string())
            }
        }
        _ => TypeKind::Unknown(s.to_string()),
    }
}

/// Check if type `sub` is a subtype of type `sup`.
pub fn is_subtype(sub: &TypeKind, sup: &TypeKind) -> bool {
    if sub == sup {
        return true;
    }

    match (sub, sup) {
        // Integer widening: smaller integers are subtypes of larger ones
        (TypeKind::Int { width: w1 }, TypeKind::Int { width: w2 }) => w1 <= w2,
        // Index is a subtype of i64
        (TypeKind::Index, TypeKind::Int { width: 64 }) => true,
        // Refined type is subtype of its base
        (TypeKind::Refined { base, .. }, sup) => is_subtype(base, sup),
        // Slices with known length are subtypes of slices with unknown length
        (
            TypeKind::Slice {
                elem: e1,
                length: Some(_),
            },
            TypeKind::Slice {
                elem: e2,
                length: None,
            },
        ) => is_subtype(e1, e2),
        // Slices with same element type and matching length
        (
            TypeKind::Slice {
                elem: e1,
                length: l1,
            },
            TypeKind::Slice {
                elem: e2,
                length: l2,
            },
        ) => e1 == e2 && l1 == l2,
        _ => false,
    }
}

/// Check if two types are equivalent (mutual subtypes).
pub fn types_equivalent(a: &TypeKind, b: &TypeKind) -> bool {
    is_subtype(a, b) && is_subtype(b, a)
}

/// Get the machine size in bytes for a type, if known.
pub fn type_size(ty: &TypeKind) -> Option<u64> {
    match ty {
        TypeKind::Int { width } => Some((*width as u64).div_ceil(8)),
        TypeKind::Index => Some(8),
        TypeKind::Ptr { .. } => Some(8),
        TypeKind::Mem => None,             // resource type, no size
        TypeKind::Proof { .. } => Some(0), // zero-sized proof
        TypeKind::Auth { .. } => Some(0),  // zero-sized token
        TypeKind::Slice { elem, length } => {
            let elem_size = type_size(elem)?;
            let len = (*length)?;
            Some(elem_size * len)
        }
        TypeKind::Tuple { elements } => {
            let mut total = 0u64;
            for e in elements {
                total += type_size(e)?;
            }
            Some(total)
        }
        _ => None,
    }
}

/// Normalize an AIR type string.
pub fn normalize_type(repr: &str) -> Type {
    Type::new(repr.trim())
}

/// Convert a TypeKind back to its string representation.
pub fn type_to_string(ty: &TypeKind) -> String {
    match ty {
        TypeKind::Int { width } => format!("i{}", width),
        TypeKind::Index => "index".to_string(),
        TypeKind::Mem => "!arc.mem".to_string(),
        TypeKind::Ptr { pointee } => format!("!arc.ptr<{}>", type_to_string(pointee)),
        TypeKind::Slice { elem, length } => match length {
            Some(l) => format!("!arc.slice<{}, {}>", type_to_string(elem), l),
            None => format!("!arc.slice<{}>", type_to_string(elem)),
        },
        TypeKind::Proof { predicate } => format!("!arc.proof<{}>", predicate),
        TypeKind::Auth { capability } => format!("!arc.auth<{}>", capability),
        TypeKind::Refined { base, predicate } => {
            format!("!arc.refined<{}, {}>", type_to_string(base), predicate)
        }
        TypeKind::Func { params, result } => {
            let params_str: Vec<String> = params.iter().map(type_to_string).collect();
            match result {
                Some(r) => format!("({}) -> {}", params_str.join(", "), type_to_string(r)),
                None => format!("({})", params_str.join(", ")),
            }
        }
        TypeKind::Tuple { elements } => {
            let elems: Vec<String> = elements.iter().map(type_to_string).collect();
            format!("({})", elems.join(", "))
        }
        TypeKind::Unknown(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_integer_types() {
        assert_eq!(parse_type("i1"), TypeKind::Int { width: 1 });
        assert_eq!(parse_type("i32"), TypeKind::Int { width: 32 });
        assert_eq!(parse_type("i64"), TypeKind::Int { width: 64 });
    }

    #[test]
    fn parse_special_types() {
        assert_eq!(parse_type("index"), TypeKind::Index);
        assert_eq!(parse_type("!arc.mem"), TypeKind::Mem);
    }

    #[test]
    fn parse_proof_type() {
        let ty = parse_type("!arc.proof<x >= 0>");
        assert_eq!(
            ty,
            TypeKind::Proof {
                predicate: "x >= 0".to_string()
            }
        );
    }

    #[test]
    fn parse_auth_type() {
        let ty = parse_type("!arc.auth<email.send>");
        assert_eq!(
            ty,
            TypeKind::Auth {
                capability: "email.send".to_string()
            }
        );
    }

    #[test]
    fn parse_slice_type() {
        let ty = parse_type("!arc.slice<i32, 4>");
        match ty {
            TypeKind::Slice { elem, length } => {
                assert_eq!(*elem, TypeKind::Int { width: 32 });
                assert_eq!(length, Some(4));
            }
            _ => panic!("expected slice type"),
        }
    }

    #[test]
    fn parse_ptr_type() {
        let ty = parse_type("!arc.ptr<i64>");
        match ty {
            TypeKind::Ptr { pointee } => {
                assert_eq!(*pointee, TypeKind::Int { width: 64 });
            }
            _ => panic!("expected ptr type"),
        }
    }

    #[test]
    fn subtype_int_widening() {
        let i32_ty = TypeKind::Int { width: 32 };
        let i64_ty = TypeKind::Int { width: 64 };
        assert!(is_subtype(&i32_ty, &i64_ty));
        assert!(!is_subtype(&i64_ty, &i32_ty));
    }

    #[test]
    fn subtype_index_is_i64() {
        assert!(is_subtype(&TypeKind::Index, &TypeKind::Int { width: 64 }));
    }

    #[test]
    fn subtype_refined_to_base() {
        let refined = TypeKind::Refined {
            base: Box::new(TypeKind::Int { width: 64 }),
            predicate: "x > 0".to_string(),
        };
        assert!(is_subtype(&refined, &TypeKind::Int { width: 64 }));
    }

    #[test]
    fn subtype_reflexive() {
        let ty = TypeKind::Int { width: 32 };
        assert!(is_subtype(&ty, &ty));
    }

    #[test]
    fn type_size_integers() {
        assert_eq!(type_size(&TypeKind::Int { width: 1 }), Some(1));
        assert_eq!(type_size(&TypeKind::Int { width: 32 }), Some(4));
        assert_eq!(type_size(&TypeKind::Int { width: 64 }), Some(8));
    }

    #[test]
    fn type_size_slice() {
        let slice = TypeKind::Slice {
            elem: Box::new(TypeKind::Int { width: 32 }),
            length: Some(10),
        };
        assert_eq!(type_size(&slice), Some(40));
    }

    #[test]
    fn type_size_proof_is_zero() {
        let proof = TypeKind::Proof {
            predicate: "true".to_string(),
        };
        assert_eq!(type_size(&proof), Some(0));
    }

    #[test]
    fn type_roundtrip_string() {
        let cases = vec![
            "i64",
            "i32",
            "index",
            "!arc.mem",
            "!arc.proof<true>",
            "!arc.auth<email.send>",
        ];
        for case in cases {
            let parsed = parse_type(case);
            let back = type_to_string(&parsed);
            assert_eq!(back, case, "roundtrip failed for {}", case);
        }
    }

    #[test]
    fn type_equivalence() {
        let a = TypeKind::Int { width: 32 };
        let b = TypeKind::Int { width: 32 };
        assert!(types_equivalent(&a, &b));

        let c = TypeKind::Int { width: 64 };
        assert!(!types_equivalent(&a, &c));
    }
}
