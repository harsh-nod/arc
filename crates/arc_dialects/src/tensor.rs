//! Tensor dialect: typed multi-dimensional arrays with shape inference.

use std::fmt;

/// A tensor shape: a list of dimension sizes (symbolic or concrete).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dim {
    /// Concrete known dimension.
    Known(u64),
    /// Symbolic dimension (a named index variable).
    Symbolic(String),
    /// Dynamic (unknown at compile time).
    Dynamic,
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Dim::Known(n) => write!(f, "{}", n),
            Dim::Symbolic(s) => write!(f, "{}", s),
            Dim::Dynamic => write!(f, "?"),
        }
    }
}

/// Element type of a tensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElemType {
    F32,
    F64,
    I32,
    I64,
    I8,
    Bool,
}

impl ElemType {
    pub fn size_bytes(&self) -> u64 {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::F64 | Self::I64 => 8,
            Self::I8 | Self::Bool => 1,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I8 => "i8",
            Self::Bool => "bool",
        }
    }
}

impl fmt::Display for ElemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tensor type with element type and shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorType {
    pub elem: ElemType,
    pub shape: Vec<Dim>,
}

impl TensorType {
    pub fn new(elem: ElemType, shape: Vec<Dim>) -> Self {
        Self { elem, shape }
    }

    pub fn rank(&self) -> usize {
        self.shape.len()
    }

    /// Compute the total number of elements (if all dimensions are known).
    pub fn num_elements(&self) -> Option<u64> {
        let mut total = 1u64;
        for dim in &self.shape {
            match dim {
                Dim::Known(n) => total *= n,
                _ => return None,
            }
        }
        Some(total)
    }

    /// Compute total size in bytes (if all dims are known).
    pub fn size_bytes(&self) -> Option<u64> {
        self.num_elements().map(|n| n * self.elem.size_bytes())
    }

    pub fn to_air_type_string(&self) -> String {
        let dims: Vec<String> = self.shape.iter().map(|d| d.to_string()).collect();
        format!("!arc.tensor<{}, [{}]>", self.elem, dims.join(", "))
    }
}

impl fmt::Display for TensorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_air_type_string())
    }
}

/// A tensor operation in the AIR tensor dialect.
#[derive(Debug, Clone, PartialEq)]
pub enum TensorOp {
    /// Matrix multiply: (m,k) x (k,n) -> (m,n)
    Matmul { lhs: TensorType, rhs: TensorType },
    /// Element-wise addition.
    Add { lhs: TensorType, rhs: TensorType },
    /// Transpose: swap last two dimensions.
    Transpose { input: TensorType },
    /// Reshape: change shape without changing element count.
    Reshape {
        input: TensorType,
        target_shape: Vec<Dim>,
    },
    /// Broadcast: expand dimensions.
    Broadcast {
        input: TensorType,
        target_shape: Vec<Dim>,
    },
    /// Reduce along an axis.
    Reduce {
        input: TensorType,
        axis: usize,
        op: ReduceOp,
    },
    /// Slice: extract a sub-tensor.
    Slice {
        input: TensorType,
        offsets: Vec<u64>,
        sizes: Vec<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReduceOp {
    Sum,
    Max,
    Min,
    Mean,
    Product,
}

/// Infer the result type of a tensor operation.
pub fn infer_result_type(op: &TensorOp) -> Result<TensorType, TensorError> {
    match op {
        TensorOp::Matmul { lhs, rhs } => {
            if lhs.rank() != 2 || rhs.rank() != 2 {
                return Err(TensorError::RankMismatch {
                    expected: 2,
                    got: if lhs.rank() != 2 {
                        lhs.rank()
                    } else {
                        rhs.rank()
                    },
                });
            }
            if lhs.elem != rhs.elem {
                return Err(TensorError::TypeMismatch {
                    expected: lhs.elem.as_str().to_string(),
                    got: rhs.elem.as_str().to_string(),
                });
            }
            // Check inner dimensions match.
            if !dims_compatible(&lhs.shape[1], &rhs.shape[0]) {
                return Err(TensorError::ShapeMismatch(format!(
                    "matmul inner dimensions mismatch: {} vs {}",
                    lhs.shape[1], rhs.shape[0]
                )));
            }
            Ok(TensorType::new(
                lhs.elem.clone(),
                vec![lhs.shape[0].clone(), rhs.shape[1].clone()],
            ))
        }
        TensorOp::Add { lhs, rhs } => {
            if lhs.elem != rhs.elem {
                return Err(TensorError::TypeMismatch {
                    expected: lhs.elem.as_str().to_string(),
                    got: rhs.elem.as_str().to_string(),
                });
            }
            if lhs.shape.len() != rhs.shape.len() {
                return Err(TensorError::ShapeMismatch(format!(
                    "add rank mismatch: {} vs {}",
                    lhs.rank(),
                    rhs.rank()
                )));
            }
            for (a, b) in lhs.shape.iter().zip(rhs.shape.iter()) {
                if !dims_compatible(a, b) {
                    return Err(TensorError::ShapeMismatch(format!(
                        "add dimension mismatch: {} vs {}",
                        a, b
                    )));
                }
            }
            Ok(lhs.clone())
        }
        TensorOp::Transpose { input } => {
            if input.rank() < 2 {
                return Err(TensorError::RankMismatch {
                    expected: 2,
                    got: input.rank(),
                });
            }
            let mut shape = input.shape.clone();
            let n = shape.len();
            shape.swap(n - 2, n - 1);
            Ok(TensorType::new(input.elem.clone(), shape))
        }
        TensorOp::Reshape {
            input,
            target_shape,
        } => {
            // If both are fully known, element counts must match.
            let src = input.num_elements();
            let dst_count = {
                let mut total = 1u64;
                let mut all_known = true;
                for d in target_shape {
                    match d {
                        Dim::Known(n) => total *= n,
                        _ => {
                            all_known = false;
                            break;
                        }
                    }
                }
                if all_known {
                    Some(total)
                } else {
                    None
                }
            };
            if let (Some(s), Some(d)) = (src, dst_count) {
                if s != d {
                    return Err(TensorError::ShapeMismatch(format!(
                        "reshape element count mismatch: {} vs {}",
                        s, d
                    )));
                }
            }
            Ok(TensorType::new(input.elem.clone(), target_shape.clone()))
        }
        TensorOp::Reduce { input, axis, op: _ } => {
            if *axis >= input.rank() {
                return Err(TensorError::InvalidAxis {
                    axis: *axis,
                    rank: input.rank(),
                });
            }
            let mut shape: Vec<Dim> = input
                .shape
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != *axis)
                .map(|(_, d)| d.clone())
                .collect();
            if shape.is_empty() {
                shape.push(Dim::Known(1)); // scalar result
            }
            Ok(TensorType::new(input.elem.clone(), shape))
        }
        TensorOp::Slice {
            input,
            offsets,
            sizes,
        } => {
            if offsets.len() != input.rank() || sizes.len() != input.rank() {
                return Err(TensorError::ShapeMismatch(
                    "slice offsets/sizes must match rank".to_string(),
                ));
            }
            let shape: Vec<Dim> = sizes.iter().map(|&s| Dim::Known(s)).collect();
            Ok(TensorType::new(input.elem.clone(), shape))
        }
        TensorOp::Broadcast {
            input,
            target_shape,
        } => {
            if target_shape.len() < input.rank() {
                return Err(TensorError::ShapeMismatch(
                    "broadcast target rank must be >= input rank".to_string(),
                ));
            }
            Ok(TensorType::new(input.elem.clone(), target_shape.clone()))
        }
    }
}

/// Check if two dimensions are compatible (for type checking).
fn dims_compatible(a: &Dim, b: &Dim) -> bool {
    match (a, b) {
        (Dim::Known(x), Dim::Known(y)) => x == y,
        (Dim::Symbolic(x), Dim::Symbolic(y)) => x == y,
        (Dim::Dynamic, _) | (_, Dim::Dynamic) => true,
        _ => false,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TensorError {
    #[error("rank mismatch: expected {expected}, got {got}")]
    RankMismatch { expected: usize, got: usize },
    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: String, got: String },
    #[error("shape mismatch: {0}")]
    ShapeMismatch(String),
    #[error("invalid axis {axis} for rank {rank}")]
    InvalidAxis { axis: usize, rank: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matmul_shape_inference() {
        let lhs = TensorType::new(ElemType::F32, vec![Dim::Known(3), Dim::Known(4)]);
        let rhs = TensorType::new(ElemType::F32, vec![Dim::Known(4), Dim::Known(5)]);
        let op = TensorOp::Matmul { lhs, rhs };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(result.shape, vec![Dim::Known(3), Dim::Known(5)]);
        assert_eq!(result.elem, ElemType::F32);
    }

    #[test]
    fn matmul_inner_dim_mismatch() {
        let lhs = TensorType::new(ElemType::F32, vec![Dim::Known(3), Dim::Known(4)]);
        let rhs = TensorType::new(ElemType::F32, vec![Dim::Known(5), Dim::Known(6)]);
        let op = TensorOp::Matmul { lhs, rhs };
        assert!(infer_result_type(&op).is_err());
    }

    #[test]
    fn matmul_symbolic_dims() {
        let lhs = TensorType::new(
            ElemType::F32,
            vec![Dim::Symbolic("m".into()), Dim::Symbolic("k".into())],
        );
        let rhs = TensorType::new(
            ElemType::F32,
            vec![Dim::Symbolic("k".into()), Dim::Symbolic("n".into())],
        );
        let op = TensorOp::Matmul { lhs, rhs };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(
            result.shape,
            vec![Dim::Symbolic("m".into()), Dim::Symbolic("n".into())]
        );
    }

    #[test]
    fn add_shape_check() {
        let a = TensorType::new(ElemType::I32, vec![Dim::Known(2), Dim::Known(3)]);
        let b = TensorType::new(ElemType::I32, vec![Dim::Known(2), Dim::Known(3)]);
        let op = TensorOp::Add { lhs: a, rhs: b };
        assert!(infer_result_type(&op).is_ok());
    }

    #[test]
    fn add_shape_mismatch() {
        let a = TensorType::new(ElemType::I32, vec![Dim::Known(2), Dim::Known(3)]);
        let b = TensorType::new(ElemType::I32, vec![Dim::Known(2), Dim::Known(4)]);
        let op = TensorOp::Add { lhs: a, rhs: b };
        assert!(infer_result_type(&op).is_err());
    }

    #[test]
    fn transpose_shape() {
        let t = TensorType::new(ElemType::F64, vec![Dim::Known(3), Dim::Known(5)]);
        let op = TensorOp::Transpose { input: t };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(result.shape, vec![Dim::Known(5), Dim::Known(3)]);
    }

    #[test]
    fn reshape_preserves_count() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(2), Dim::Known(6)]);
        let op = TensorOp::Reshape {
            input: t,
            target_shape: vec![Dim::Known(3), Dim::Known(4)],
        };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(result.shape, vec![Dim::Known(3), Dim::Known(4)]);
    }

    #[test]
    fn reshape_count_mismatch() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(2), Dim::Known(3)]);
        let op = TensorOp::Reshape {
            input: t,
            target_shape: vec![Dim::Known(2), Dim::Known(4)],
        };
        assert!(infer_result_type(&op).is_err());
    }

    #[test]
    fn reduce_removes_axis() {
        let t = TensorType::new(
            ElemType::F32,
            vec![Dim::Known(3), Dim::Known(4), Dim::Known(5)],
        );
        let op = TensorOp::Reduce {
            input: t,
            axis: 1,
            op: ReduceOp::Sum,
        };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(result.shape, vec![Dim::Known(3), Dim::Known(5)]);
    }

    #[test]
    fn reduce_invalid_axis() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(3)]);
        let op = TensorOp::Reduce {
            input: t,
            axis: 1,
            op: ReduceOp::Max,
        };
        assert!(infer_result_type(&op).is_err());
    }

    #[test]
    fn tensor_type_display() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(3), Dim::Known(4)]);
        assert_eq!(t.to_string(), "!arc.tensor<f32, [3, 4]>");
    }

    #[test]
    fn tensor_size_bytes() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(3), Dim::Known(4)]);
        assert_eq!(t.size_bytes(), Some(48));
    }

    #[test]
    fn slice_op() {
        let t = TensorType::new(ElemType::F32, vec![Dim::Known(10), Dim::Known(20)]);
        let op = TensorOp::Slice {
            input: t,
            offsets: vec![2, 5],
            sizes: vec![3, 10],
        };
        let result = infer_result_type(&op).unwrap();
        assert_eq!(result.shape, vec![Dim::Known(3), Dim::Known(10)]);
    }
}
