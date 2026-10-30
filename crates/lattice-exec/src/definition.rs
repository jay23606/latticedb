//! Conversion between the executor's [`TableDef`] and the durable record of RFC 0012
//! (`lattice_engines::sqldef`).
//!
//! `CREATE TABLE` turns its definition into a record that is committed in the same change set as the
//! table; `Database::open` turns every live record back into a [`TableDef`]. The record is the only
//! durable source of a table's columns, constraints and the order of its `UNIQUE` constraints (which
//! fixes their marker tables), so both directions are strict: a record the executor cannot use
//! refuses the open instead of being repaired or guessed.

use lattice_engines::sqldef::{
    self, CANONICAL_NAN, Column, DefaultValue, KeyConstraint, SqlTableDef,
};
use lattice_sql::plan::SqlType;

use crate::decimal::Decimal;
use crate::error::{ErrorKind, ExecError};
use crate::schema::{ColumnDef, MAX_TABLE_ID, TableDef};
use crate::value::{Datum, parse_text};

fn not_storable(why: impl std::fmt::Display) -> ExecError {
    ExecError::unsupported(format!("this table definition cannot be stored: {why}"))
}

fn unusable(table: &str, why: impl std::fmt::Display) -> ExecError {
    ExecError::new(
        ErrorKind::Storage,
        "XX001",
        format!("the stored definition of table \"{table}\" cannot be used: {why}"),
    )
}

fn positions(cols: &[usize]) -> Result<Vec<u16>, ExecError> {
    cols.iter()
        .map(|c| u16::try_from(*c).map_err(|_| not_storable("column position out of range")))
        .collect()
}

fn sql_type(c: &ColumnDef) -> Result<(sqldef::SqlType, u32), ExecError> {
    use sqldef::SqlType as T;
    Ok(match (c.ty, c.varchar_len) {
        (SqlType::Text, Some(n)) => (T::Varchar, n),
        (SqlType::Text, None) => (T::Text, 0),
        (_, Some(_)) => return Err(ExecError::internal("a varchar limit on a non-text column")),
        (SqlType::Boolean, None) => (T::Boolean, 0),
        (SqlType::SmallInt, None) => (T::SmallInt, 0),
        (SqlType::Integer, None) => (T::Integer, 0),
        (SqlType::BigInt, None) => (T::BigInt, 0),
        (SqlType::DoublePrecision, None) => (T::Double, 0),
        (SqlType::Numeric, None) => (T::Numeric, 0),
        (SqlType::Bytea, None) => (T::Bytea, 0),
        (SqlType::Uuid, None) => (T::Uuid, 0),
        (SqlType::Date, None) => (T::Date, 0),
        (SqlType::Timestamptz, None) => (T::Timestamptz, 0),
        (SqlType::Jsonb, None) => (T::Jsonb, 0),
        (SqlType::Unknown, None) => {
            return Err(ExecError::internal("a column of unknown type"));
        }
    })
}

fn default_value(d: &Datum) -> Option<DefaultValue> {
    Some(match d {
        Datum::Null => return None,
        Datum::Bool(b) => DefaultValue::Bool(*b),
        Datum::Int(i) => DefaultValue::Int(*i),
        Datum::Float(f) => DefaultValue::Double(if f.is_nan() {
            CANONICAL_NAN
        } else {
            f.to_bits()
        }),
        Datum::Numeric(n) => DefaultValue::Numeric(n.normalized().to_string()),
        Datum::Text(s) => DefaultValue::Text(s.clone()),
        Datum::Bytes(b) => DefaultValue::Bytes(b.clone()),
        Datum::Uuid(u) => DefaultValue::Uuid(u.to_be_bytes()),
        Datum::Date(x) => DefaultValue::Date(*x),
        Datum::Timestamp(t) => DefaultValue::Timestamptz(*t),
        Datum::Json(s) => DefaultValue::Jsonb(s.clone()),
    })
}

/// The durable record of `def`, validated with the engine's structural rules so an unsupported
/// shape (for example an uppercase column name) is refused as a statement error before anything is
/// written.
pub(crate) fn to_record(def: &TableDef) -> Result<SqlTableDef, ExecError> {
    if def.id > MAX_TABLE_ID {
        return Err(ExecError::unsupported("too many tables"));
    }
    let mut columns = Vec::with_capacity(def.columns.len());
    for c in &def.columns {
        let (ty, varchar_limit) = sql_type(c)?;
        columns.push(Column {
            name: c.name.clone(),
            ty,
            varchar_limit,
            not_null: c.not_null,
            default: c.default.as_ref().and_then(default_value),
        });
    }
    let primary_key = if def.primary_key.is_empty() {
        None
    } else {
        Some(KeyConstraint {
            columns: positions(&def.primary_key)?,
            name: def.primary_key_name.clone(),
        })
    };
    let mut uniques = Vec::with_capacity(def.uniques.len());
    for (k, cols) in def.uniques.iter().enumerate() {
        uniques.push(KeyConstraint {
            columns: positions(cols)?,
            name: def.unique_name(k),
        });
    }
    let record = SqlTableDef {
        columns,
        primary_key,
        uniques,
    };
    record.validate().map_err(not_storable)?;
    Ok(record)
}

fn exec_type(ty: sqldef::SqlType) -> (SqlType, bool) {
    use sqldef::SqlType as T;
    match ty {
        T::Boolean => (SqlType::Boolean, false),
        T::SmallInt => (SqlType::SmallInt, false),
        T::Integer => (SqlType::Integer, false),
        T::BigInt => (SqlType::BigInt, false),
        T::Double => (SqlType::DoublePrecision, false),
        T::Numeric => (SqlType::Numeric, false),
        T::Text => (SqlType::Text, false),
        T::Varchar => (SqlType::Text, true),
        T::Bytea => (SqlType::Bytea, false),
        T::Uuid => (SqlType::Uuid, false),
        T::Date => (SqlType::Date, false),
        T::Timestamptz => (SqlType::Timestamptz, false),
        T::Jsonb => (SqlType::Jsonb, false),
    }
}

fn datum(table: &str, value: &DefaultValue) -> Result<Datum, ExecError> {
    Ok(match value {
        DefaultValue::Bool(b) => Datum::Bool(*b),
        DefaultValue::Int(i) => Datum::Int(*i),
        DefaultValue::Double(bits) => Datum::Float(f64::from_bits(*bits)),
        DefaultValue::Numeric(text) => {
            Datum::Numeric(Decimal::parse(text).map_err(|_| unusable(table, "a numeric default"))?)
        }
        DefaultValue::Text(s) => Datum::Text(s.clone()),
        DefaultValue::Bytes(b) => Datum::Bytes(b.clone()),
        DefaultValue::Uuid(u) => Datum::Uuid(u128::from_be_bytes(*u)),
        DefaultValue::Date(d) => Datum::Date(*d),
        DefaultValue::Timestamptz(t) => Datum::Timestamp(*t),
        // The executor re-checks what the engine cannot: the text must be JSON.
        DefaultValue::Jsonb(s) => match parse_text(s, SqlType::Jsonb) {
            Ok(Datum::Json(_)) => Datum::Json(s.clone()),
            _ => return Err(unusable(table, "a jsonb default is not JSON")),
        },
    })
}

/// The table a record describes under the name and id the catalog gives it. Refuses a record the
/// executor cannot serve.
pub(crate) fn from_record(
    id: u32,
    name: &str,
    record: &SqlTableDef,
) -> Result<TableDef, ExecError> {
    if id > MAX_TABLE_ID {
        return Err(unusable(
            name,
            "the table id is above the largest id served",
        ));
    }
    let mut columns = Vec::with_capacity(record.columns.len());
    for c in &record.columns {
        let (ty, varchar) = exec_type(c.ty);
        columns.push(ColumnDef {
            name: c.name.clone(),
            ty,
            varchar_len: (varchar && c.varchar_limit > 0).then_some(c.varchar_limit),
            not_null: c.not_null,
            default: match &c.default {
                Some(d) => Some(datum(name, d)?),
                None => None,
            },
        });
    }
    let index = |cols: &[u16]| -> Vec<usize> { cols.iter().map(|c| usize::from(*c)).collect() };
    let (primary_key, primary_key_name) = match &record.primary_key {
        Some(k) => (index(&k.columns), k.name.clone()),
        None => (Vec::new(), String::new()),
    };
    Ok(TableDef {
        id,
        name: name.to_string(),
        columns,
        primary_key,
        uniques: record.uniques.iter().map(|u| index(&u.columns)).collect(),
        primary_key_name,
        unique_names: record.uniques.iter().map(|u| u.name.clone()).collect(),
    })
}
