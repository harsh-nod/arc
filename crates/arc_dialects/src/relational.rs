//! Relational dialect: typed relational algebra (scan, filter, join, project, aggregate).

use std::fmt;

/// A column type in a relational schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnType {
    Int64,
    Float64,
    Text,
    Bool,
    Timestamp,
}

impl fmt::Display for ColumnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int64 => write!(f, "int64"),
            Self::Float64 => write!(f, "float64"),
            Self::Text => write!(f, "text"),
            Self::Bool => write!(f, "bool"),
            Self::Timestamp => write!(f, "timestamp"),
        }
    }
}

/// A named column in a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
    pub nullable: bool,
}

impl Column {
    pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
            nullable: false,
        }
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }
}

/// A relation schema (ordered list of columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub columns: Vec<Column>,
}

impl Schema {
    pub fn new(columns: Vec<Column>) -> Self {
        Self { columns }
    }

    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn has_column(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.name == name)
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// Project: select a subset of columns.
    pub fn project(&self, names: &[&str]) -> Result<Schema, RelationalError> {
        let mut cols = Vec::new();
        for name in names {
            let col = self
                .column_by_name(name)
                .ok_or_else(|| RelationalError::ColumnNotFound(name.to_string()))?;
            cols.push(col.clone());
        }
        Ok(Schema::new(cols))
    }

    /// Join: merge two schemas, checking for the join key.
    pub fn join(&self, other: &Schema, key: &str) -> Result<Schema, RelationalError> {
        if !self.has_column(key) {
            return Err(RelationalError::ColumnNotFound(format!(
                "{} not in left schema",
                key
            )));
        }
        if !other.has_column(key) {
            return Err(RelationalError::ColumnNotFound(format!(
                "{} not in right schema",
                key
            )));
        }
        // Check join key types match.
        let left_col = self.column_by_name(key).unwrap();
        let right_col = other.column_by_name(key).unwrap();
        if left_col.ty != right_col.ty {
            return Err(RelationalError::TypeMismatch {
                column: key.to_string(),
                left: format!("{}", left_col.ty),
                right: format!("{}", right_col.ty),
            });
        }
        // Result: all columns from left + non-key columns from right.
        let mut cols = self.columns.clone();
        for col in &other.columns {
            if col.name != key {
                if self.has_column(&col.name) {
                    // Disambiguate with suffix.
                    let mut renamed = col.clone();
                    renamed.name = format!("{}_right", col.name);
                    cols.push(renamed);
                } else {
                    cols.push(col.clone());
                }
            }
        }
        Ok(Schema::new(cols))
    }
}

/// A predicate for filtering relations.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Eq(String, Value),
    Ne(String, Value),
    Gt(String, Value),
    Lt(String, Value),
    Ge(String, Value),
    Le(String, Value),
    IsNull(String),
    IsNotNull(String),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

/// A literal value for predicates.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Null,
}

/// Aggregate function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl fmt::Display for AggFunc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count => write!(f, "count"),
            Self::Sum => write!(f, "sum"),
            Self::Avg => write!(f, "avg"),
            Self::Min => write!(f, "min"),
            Self::Max => write!(f, "max"),
        }
    }
}

/// A relational algebra operation.
#[derive(Debug, Clone, PartialEq)]
pub enum RelOp {
    /// Scan a base table.
    Scan { table: String, schema: Schema },
    /// Filter rows by a predicate.
    Filter {
        input: Box<RelOp>,
        predicate: Predicate,
    },
    /// Project (select) specific columns.
    Project {
        input: Box<RelOp>,
        columns: Vec<String>,
    },
    /// Join two relations on a key column.
    Join {
        left: Box<RelOp>,
        right: Box<RelOp>,
        key: String,
    },
    /// Aggregate with grouping.
    Aggregate {
        input: Box<RelOp>,
        group_by: Vec<String>,
        aggregates: Vec<(AggFunc, String, String)>, // (func, input_col, output_col)
    },
    /// Sort by columns.
    OrderBy {
        input: Box<RelOp>,
        keys: Vec<(String, bool)>, // (column, ascending)
    },
    /// Limit number of rows.
    Limit { input: Box<RelOp>, count: u64 },
}

/// Infer the output schema of a relational operation.
pub fn infer_schema(op: &RelOp) -> Result<Schema, RelationalError> {
    match op {
        RelOp::Scan { schema, .. } => Ok(schema.clone()),
        RelOp::Filter { input, .. } => infer_schema(input),
        RelOp::Project { input, columns } => {
            let input_schema = infer_schema(input)?;
            let col_refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
            input_schema.project(&col_refs)
        }
        RelOp::Join { left, right, key } => {
            let left_schema = infer_schema(left)?;
            let right_schema = infer_schema(right)?;
            left_schema.join(&right_schema, key)
        }
        RelOp::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            let input_schema = infer_schema(input)?;
            let mut cols: Vec<Column> = Vec::new();
            for g in group_by {
                let col = input_schema
                    .column_by_name(g)
                    .ok_or_else(|| RelationalError::ColumnNotFound(g.clone()))?;
                cols.push(col.clone());
            }
            for (func, input_col, output_col) in aggregates {
                // Verify input column exists.
                let _col = input_schema
                    .column_by_name(input_col)
                    .ok_or_else(|| RelationalError::ColumnNotFound(input_col.clone()))?;
                let out_ty = match func {
                    AggFunc::Count => ColumnType::Int64,
                    AggFunc::Avg => ColumnType::Float64,
                    _ => _col.ty.clone(),
                };
                cols.push(Column::new(output_col, out_ty));
            }
            Ok(Schema::new(cols))
        }
        RelOp::OrderBy { input, keys } => {
            let schema = infer_schema(input)?;
            for (col, _) in keys {
                if !schema.has_column(col) {
                    return Err(RelationalError::ColumnNotFound(col.clone()));
                }
            }
            Ok(schema)
        }
        RelOp::Limit { input, .. } => infer_schema(input),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RelationalError {
    #[error("column not found: {0}")]
    ColumnNotFound(String),
    #[error("type mismatch on column {column}: {left} vs {right}")]
    TypeMismatch {
        column: String,
        left: String,
        right: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orders_schema() -> Schema {
        Schema::new(vec![
            Column::new("order_id", ColumnType::Int64),
            Column::new("customer_id", ColumnType::Int64),
            Column::new("amount", ColumnType::Float64),
        ])
    }

    fn customers_schema() -> Schema {
        Schema::new(vec![
            Column::new("customer_id", ColumnType::Int64),
            Column::new("name", ColumnType::Text),
        ])
    }

    #[test]
    fn scan_returns_schema() {
        let op = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let schema = infer_schema(&op).unwrap();
        assert_eq!(schema.columns.len(), 3);
    }

    #[test]
    fn filter_preserves_schema() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let filter = RelOp::Filter {
            input: Box::new(scan),
            predicate: Predicate::Gt("amount".into(), Value::Float(10000.0)),
        };
        let schema = infer_schema(&filter).unwrap();
        assert_eq!(schema.columns.len(), 3);
    }

    #[test]
    fn project_selects_columns() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let project = RelOp::Project {
            input: Box::new(scan),
            columns: vec!["order_id".into(), "amount".into()],
        };
        let schema = infer_schema(&project).unwrap();
        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "order_id");
        assert_eq!(schema.columns[1].name, "amount");
    }

    #[test]
    fn project_missing_column() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let project = RelOp::Project {
            input: Box::new(scan),
            columns: vec!["nonexistent".into()],
        };
        assert!(infer_schema(&project).is_err());
    }

    #[test]
    fn join_merges_schemas() {
        let orders = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let customers = RelOp::Scan {
            table: "customers".into(),
            schema: customers_schema(),
        };
        let join = RelOp::Join {
            left: Box::new(orders),
            right: Box::new(customers),
            key: "customer_id".into(),
        };
        let schema = infer_schema(&join).unwrap();
        // orders: 3 cols + customers: 1 non-key col = 4
        assert_eq!(schema.columns.len(), 4);
        assert!(schema.has_column("name"));
    }

    #[test]
    fn join_key_type_mismatch() {
        let left = Schema::new(vec![Column::new("id", ColumnType::Int64)]);
        let right = Schema::new(vec![Column::new("id", ColumnType::Text)]);
        assert!(left.join(&right, "id").is_err());
    }

    #[test]
    fn aggregate_schema() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let agg = RelOp::Aggregate {
            input: Box::new(scan),
            group_by: vec!["customer_id".into()],
            aggregates: vec![
                (AggFunc::Sum, "amount".into(), "total".into()),
                (AggFunc::Count, "order_id".into(), "num_orders".into()),
            ],
        };
        let schema = infer_schema(&agg).unwrap();
        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.columns[0].name, "customer_id");
        assert_eq!(schema.columns[1].name, "total");
        assert_eq!(schema.columns[2].name, "num_orders");
        assert_eq!(schema.columns[2].ty, ColumnType::Int64); // count returns int
    }

    #[test]
    fn order_by_validates_columns() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let good = RelOp::OrderBy {
            input: Box::new(scan.clone()),
            keys: vec![("amount".into(), false)],
        };
        assert!(infer_schema(&good).is_ok());

        let bad = RelOp::OrderBy {
            input: Box::new(scan),
            keys: vec![("nonexistent".into(), true)],
        };
        assert!(infer_schema(&bad).is_err());
    }

    #[test]
    fn limit_preserves_schema() {
        let scan = RelOp::Scan {
            table: "orders".into(),
            schema: orders_schema(),
        };
        let limited = RelOp::Limit {
            input: Box::new(scan),
            count: 10,
        };
        let schema = infer_schema(&limited).unwrap();
        assert_eq!(schema.columns.len(), 3);
    }
}
