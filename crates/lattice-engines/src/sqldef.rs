//! SQL table definition block, version 1 (RFC 0012, Option A; `formats/sql-definition-v1.md`).
//!
//! A definition says how the typed SQL layer reads the rows of one table: column names, types,
//! NOT NULL flags, constant defaults, the primary key and unique constraints. This module is only
//! the byte codec and the structural validator. It has no catalog, no log and no SQL knowledge, and
//! nothing in the row store calls it yet; the envelope that carries it (LCIX version 3) and the
//! capability bit that guards it are separate work packages.
//!
//! The decoder is strict and bounded. It rejects unknown versions and tags, reserved flag bits,
//! trailing bytes and every non-canonical spelling, so a successful decode re-encodes to exactly the
//! input. It never panics and never reserves memory from a length it has not checked against the
//! bytes that remain.
use lattice_core::bugs::{self, Bug};
use std::collections::BTreeSet;
use std::fmt;

/// Definition version byte.
pub const VERSION: u8 = 1;
/// Layout rule 1: the typed-row codec v1 with the type mapping below, columns in declaration order.
pub const LAYOUT_RULE: u8 = 1;
/// Most columns in one definition.
pub const MAX_COLUMNS: usize = 1024;
/// Longest column or constraint name, in bytes.
pub const MAX_NAME: usize = 255;
/// Largest encoded definition body.
pub const MAX_BODY: usize = 65_536;
/// Most unique constraints in one definition (the marker table index is four bits).
pub const MAX_UNIQUES: usize = 15;
/// Largest table id a definition may be attached to: marker table ids use bit 31 and the next
/// four bits for the constraint index.
pub const MAX_TABLE_ID: u32 = (1 << 26) - 1;
/// Most definition operations in one envelope.
pub const MAX_DEFINITIONS: usize = 1024;
/// Tag of the only definition operation kind.
const OP_DEFINE_TABLE: u8 = 1;
/// Canonical quiet NaN bit pattern of a double default.
pub const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;
/// Smallest encoded column: name length, one name byte, type tag, varchar limit and flags.
const MIN_COLUMN_BYTES: usize = 4 + 1 + 1 + 4 + 1;

/// Why a definition or definition list was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlDefError {
    /// The bytes end inside a field.
    Truncated,
    /// Bytes remain after the last field.
    Trailing,
    /// A definition version this code cannot read.
    UnsupportedVersion(u8),
    /// A layout rule this code cannot read.
    UnsupportedLayout(u8),
    /// The body or a nested length exceeds a bound.
    TooLarge,
    /// A column count outside 1..=1024.
    BadColumnCount,
    /// A column name that is empty, longer than 255 bytes or contains an uppercase ASCII letter.
    BadName,
    /// A name or text default that is not valid UTF-8.
    BadUtf8,
    /// Two columns share a name.
    DuplicateColumn,
    /// A type tag outside 1..=13.
    BadTypeTag(u8),
    /// A varchar limit on a type other than varchar.
    BadVarcharLimit,
    /// A flag byte with a reserved bit set.
    BadFlags(u8),
    /// A default tag outside 1..=10.
    BadDefaultTag(u8),
    /// A default whose tag does not belong to the column type.
    DefaultMismatch,
    /// A default outside the range of its column type (integer range, varchar length, NaN spelling).
    DefaultOutOfRange,
    /// A numeric default that is not in canonical decimal text.
    BadNumeric,
    /// A boolean default byte other than 0 or 1.
    BadBool,
    /// A key position outside the columns, a repeated position, or a key without columns.
    BadKeyPosition,
    /// A primary key column that is nullable.
    KeyNullable,
    /// A key column whose type cannot be a key (numeric or jsonb).
    KeyType,
    /// A constraint name that is empty or longer than 255 bytes.
    BadConstraintName,
    /// Two constraints, the primary key included, share a name.
    DuplicateConstraintName,
    /// More than fifteen unique constraints.
    TooManyUniques,
    /// A definition operation tag other than 1.
    BadOpTag(u8),
    /// A table id above [`MAX_TABLE_ID`].
    BadTableId(u32),
    /// Definition operations are not in ascending table id order.
    NotAscending,
    /// Two definition operations carry the same table id.
    DuplicateTableId(u32),
    /// There are no definition operations.
    EmptyDefinitions,
    /// More than [`MAX_DEFINITIONS`] definition operations.
    TooManyDefinitions,
}

impl fmt::Display for SqlDefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for SqlDefError {}

/// Column types, with the tags the format freezes. A type that needs a modifier or a new meaning
/// gets a new definition version, not a new meaning for a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    /// Tag 1.
    Boolean,
    /// Tag 2.
    SmallInt,
    /// Tag 3.
    Integer,
    /// Tag 4.
    BigInt,
    /// Tag 5.
    Double,
    /// Tag 6, without modifiers.
    Numeric,
    /// Tag 7.
    Text,
    /// Tag 8; the limit is in characters.
    Varchar,
    /// Tag 9.
    Bytea,
    /// Tag 10.
    Uuid,
    /// Tag 11.
    Date,
    /// Tag 12.
    Timestamptz,
    /// Tag 13.
    Jsonb,
}

impl SqlType {
    /// The frozen type tag.
    pub const fn tag(self) -> u8 {
        match self {
            SqlType::Boolean => 1,
            SqlType::SmallInt => 2,
            SqlType::Integer => 3,
            SqlType::BigInt => 4,
            SqlType::Double => 5,
            SqlType::Numeric => 6,
            SqlType::Text => 7,
            SqlType::Varchar => 8,
            SqlType::Bytea => 9,
            SqlType::Uuid => 10,
            SqlType::Date => 11,
            SqlType::Timestamptz => 12,
            SqlType::Jsonb => 13,
        }
    }

    /// The type for a tag, or `None` for an unknown tag.
    pub const fn from_tag(tag: u8) -> Option<SqlType> {
        Some(match tag {
            1 => SqlType::Boolean,
            2 => SqlType::SmallInt,
            3 => SqlType::Integer,
            4 => SqlType::BigInt,
            5 => SqlType::Double,
            6 => SqlType::Numeric,
            7 => SqlType::Text,
            8 => SqlType::Varchar,
            9 => SqlType::Bytea,
            10 => SqlType::Uuid,
            11 => SqlType::Date,
            12 => SqlType::Timestamptz,
            13 => SqlType::Jsonb,
            _ => return None,
        })
    }

    /// Whether a column of this type may be part of a primary key or unique constraint (the
    /// executor's `key_type_ok`).
    pub const fn can_be_key(self) -> bool {
        !matches!(self, SqlType::Numeric | SqlType::Jsonb)
    }
}

/// A constant default, in the units of the executor's `Datum`. `now()` and expressions are not
/// representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultValue {
    /// Tag 1.
    Bool(bool),
    /// Tag 2; for smallint, integer or bigint, within the column's range.
    Int(i64),
    /// Tag 3: the IEEE 754 bit pattern. A NaN must be [`CANONICAL_NAN`].
    Double(u64),
    /// Tag 4: canonical decimal text.
    Numeric(String),
    /// Tag 5, for text and varchar.
    Text(String),
    /// Tag 6.
    Bytes(Vec<u8>),
    /// Tag 7, big endian as written.
    Uuid([u8; 16]),
    /// Tag 8: days since 1970-01-01.
    Date(i32),
    /// Tag 9: microseconds since 1970-01-01 00:00:00 UTC.
    Timestamptz(i64),
    /// Tag 10: the JSON text as written (not canonical: two spellings are two definitions).
    Jsonb(String),
}

impl DefaultValue {
    const fn tag(&self) -> u8 {
        match self {
            DefaultValue::Bool(_) => 1,
            DefaultValue::Int(_) => 2,
            DefaultValue::Double(_) => 3,
            DefaultValue::Numeric(_) => 4,
            DefaultValue::Text(_) => 5,
            DefaultValue::Bytes(_) => 6,
            DefaultValue::Uuid(_) => 7,
            DefaultValue::Date(_) => 8,
            DefaultValue::Timestamptz(_) => 9,
            DefaultValue::Jsonb(_) => 10,
        }
    }
}

/// One column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// 1..=255 bytes of UTF-8 without an uppercase ASCII letter.
    pub name: String,
    /// The column type.
    pub ty: SqlType,
    /// Varchar limit in characters (Unicode scalar values); 0 means none. Nonzero only for varchar.
    pub varchar_limit: u32,
    /// Whether the column is NOT NULL.
    pub not_null: bool,
    /// The constant default, if any.
    pub default: Option<DefaultValue>,
}

/// A primary key or unique constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConstraint {
    /// Column positions in key order, without repeats.
    pub columns: Vec<u16>,
    /// 1..=255 bytes of UTF-8.
    pub name: String,
}

/// The structure of one SQL table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlTableDef {
    /// Columns in declaration order; the order is the row layout (layout rule 1).
    pub columns: Vec<Column>,
    /// The primary key, or `None` for a table without one.
    pub primary_key: Option<KeyConstraint>,
    /// Unique constraints; the position is the marker table index `k`.
    pub uniques: Vec<KeyConstraint>,
}

/// A definition attached to a table id: the unit of the envelope's definition block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefineTable {
    /// The table the definition belongs to, at most [`MAX_TABLE_ID`].
    pub table_id: u32,
    /// The definition.
    pub definition: SqlTableDef,
}

/// Whether `text` is canonical decimal numeric text: an optional `-`, digits without a leading zero
/// (a lone `0` is fine), optionally `.` and digits without a trailing zero, and never `-0`.
fn canonical_numeric(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let negative = body.len() != text.len();
    let (int, frac) = match body.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (body, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || (int.len() > 1 && int.starts_with('0')) {
        return false;
    }
    match frac {
        Some(frac) => digits(frac) && !frac.ends_with('0'),
        None => !(negative && int == "0"),
    }
}

fn check_default(col: &Column, value: &DefaultValue) -> Result<(), SqlDefError> {
    use DefaultValue as D;
    use SqlType as T;
    match (col.ty, value) {
        (T::Boolean, D::Bool(_))
        | (T::Bytea, D::Bytes(_))
        | (T::Uuid, D::Uuid(_))
        | (T::Date, D::Date(_))
        | (T::Timestamptz, D::Timestamptz(_))
        | (T::Jsonb, D::Jsonb(_))
        | (T::Text, D::Text(_))
        | (T::BigInt, D::Int(_)) => Ok(()),
        (T::SmallInt, D::Int(v)) => i16::try_from(*v)
            .map(|_| ())
            .map_err(|_| SqlDefError::DefaultOutOfRange),
        (T::Integer, D::Int(v)) => i32::try_from(*v)
            .map(|_| ())
            .map_err(|_| SqlDefError::DefaultOutOfRange),
        (T::Double, D::Double(bits)) => {
            if f64::from_bits(*bits).is_nan() && *bits != CANONICAL_NAN {
                Err(SqlDefError::DefaultOutOfRange)
            } else {
                Ok(())
            }
        }
        (T::Numeric, D::Numeric(text)) => {
            if canonical_numeric(text) {
                Ok(())
            } else {
                Err(SqlDefError::BadNumeric)
            }
        }
        (T::Varchar, D::Text(text)) => {
            let fits = col.varchar_limit == 0
                || u32::try_from(text.chars().count()).is_ok_and(|n| n <= col.varchar_limit);
            if fits {
                Ok(())
            } else {
                Err(SqlDefError::DefaultOutOfRange)
            }
        }
        _ => Err(SqlDefError::DefaultMismatch),
    }
}

fn check_key(
    def: &SqlTableDef,
    key: &KeyConstraint,
    names: &mut BTreeSet<String>,
    primary: bool,
) -> Result<(), SqlDefError> {
    if key.columns.is_empty() || key.columns.len() > def.columns.len() {
        return Err(SqlDefError::BadKeyPosition);
    }
    let mut seen = BTreeSet::new();
    for &pos in &key.columns {
        let col = def
            .columns
            .get(usize::from(pos))
            .ok_or(SqlDefError::BadKeyPosition)?;
        if !seen.insert(pos) {
            return Err(SqlDefError::BadKeyPosition);
        }
        if !col.ty.can_be_key() {
            return Err(SqlDefError::KeyType);
        }
        if primary && !col.not_null {
            return Err(SqlDefError::KeyNullable);
        }
    }
    if key.name.is_empty() || key.name.len() > MAX_NAME {
        return Err(SqlDefError::BadConstraintName);
    }
    if !names.insert(key.name.clone()) {
        return Err(SqlDefError::DuplicateConstraintName);
    }
    Ok(())
}

impl SqlTableDef {
    /// Checks every structural rule of the format. Encoding runs it before writing and decoding
    /// runs it before returning, so a value that passes here has exactly one encoding.
    pub fn validate(&self) -> Result<(), SqlDefError> {
        if self.columns.is_empty() || self.columns.len() > MAX_COLUMNS {
            return Err(SqlDefError::BadColumnCount);
        }
        let mut column_names = BTreeSet::new();
        for col in &self.columns {
            if col.name.is_empty()
                || col.name.len() > MAX_NAME
                || col.name.bytes().any(|b| b.is_ascii_uppercase())
            {
                return Err(SqlDefError::BadName);
            }
            if !column_names.insert(col.name.as_str()) {
                return Err(SqlDefError::DuplicateColumn);
            }
            if col.varchar_limit != 0 && col.ty != SqlType::Varchar {
                return Err(SqlDefError::BadVarcharLimit);
            }
            if let Some(value) = &col.default {
                check_default(col, value)?;
            }
        }
        let mut constraint_names = BTreeSet::new();
        if let Some(pk) = &self.primary_key {
            check_key(self, pk, &mut constraint_names, true)?;
        }
        if self.uniques.len() > MAX_UNIQUES {
            return Err(SqlDefError::TooManyUniques);
        }
        for unique in &self.uniques {
            check_key(self, unique, &mut constraint_names, false)?;
        }
        Ok(())
    }

    /// Encodes the definition body after validating it.
    pub fn encode(&self) -> Result<Vec<u8>, SqlDefError> {
        self.validate()?;
        let mut out = vec![VERSION, LAYOUT_RULE];
        out.extend_from_slice(&(self.columns.len() as u16).to_le_bytes());
        for col in &self.columns {
            put_text(&mut out, &col.name);
            out.push(col.ty.tag());
            out.extend_from_slice(&col.varchar_limit.to_le_bytes());
            out.push(u8::from(col.not_null) | (u8::from(col.default.is_some()) << 1));
            if let Some(value) = &col.default {
                put_default(&mut out, value);
            }
        }
        match &self.primary_key {
            Some(pk) => put_key(&mut out, pk),
            None => out.extend_from_slice(&0u16.to_le_bytes()),
        }
        out.push(self.uniques.len() as u8);
        for unique in &self.uniques {
            put_key(&mut out, unique);
        }
        if out.len() > MAX_BODY {
            return Err(SqlDefError::TooLarge);
        }
        Ok(out)
    }

    /// Decodes a definition body. Total: any input yields a value or an error, and a value
    /// re-encodes to exactly the input.
    pub fn decode(body: &[u8]) -> Result<SqlTableDef, SqlDefError> {
        if body.len() > MAX_BODY {
            return Err(SqlDefError::TooLarge);
        }
        let mut r = Reader { rest: body };
        match r.u8()? {
            VERSION => {}
            v => return Err(SqlDefError::UnsupportedVersion(v)),
        }
        match r.u8()? {
            LAYOUT_RULE => {}
            v => return Err(SqlDefError::UnsupportedLayout(v)),
        }
        let count = usize::from(r.u16()?);
        if count == 0 || count > MAX_COLUMNS {
            return Err(SqlDefError::BadColumnCount);
        }
        // Refuse a count the remaining bytes cannot hold before reading any column.
        if count.saturating_mul(MIN_COLUMN_BYTES) > r.rest.len() {
            return Err(SqlDefError::Truncated);
        }
        let mut columns = Vec::new();
        for _ in 0..count {
            let name = r.text(SqlDefError::BadName)?;
            let tag = r.u8()?;
            let ty = SqlType::from_tag(tag).ok_or(SqlDefError::BadTypeTag(tag))?;
            let varchar_limit = r.u32()?;
            let flags = r.u8()?;
            if flags & !0b11 != 0 {
                return Err(SqlDefError::BadFlags(flags));
            }
            let default = if flags & 0b10 != 0 {
                Some(r.default()?)
            } else {
                None
            };
            columns.push(Column {
                name,
                ty,
                varchar_limit,
                not_null: flags & 1 != 0,
                default,
            });
        }
        let primary_key = r.key(columns.len(), false)?;
        let unique_count = usize::from(r.u8()?);
        if unique_count > MAX_UNIQUES {
            return Err(SqlDefError::TooManyUniques);
        }
        let mut uniques = Vec::new();
        for _ in 0..unique_count {
            uniques.push(
                r.key(columns.len(), true)?
                    .ok_or(SqlDefError::BadKeyPosition)?,
            );
        }
        if !r.rest.is_empty() {
            return Err(SqlDefError::Trailing);
        }
        if bugs::active(Bug::SqlDefColumnsSorted) {
            columns.sort_by(|a, b| a.name.cmp(&b.name));
        }
        let def = SqlTableDef {
            columns,
            primary_key,
            uniques,
        };
        def.validate()?;
        Ok(def)
    }
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_key(out: &mut Vec<u8>, key: &KeyConstraint) {
    out.extend_from_slice(&(key.columns.len() as u16).to_le_bytes());
    for pos in &key.columns {
        out.extend_from_slice(&pos.to_le_bytes());
    }
    put_text(out, &key.name);
}

fn put_default(out: &mut Vec<u8>, value: &DefaultValue) {
    out.push(value.tag());
    match value {
        DefaultValue::Bool(v) => out.push(u8::from(*v)),
        DefaultValue::Int(v) | DefaultValue::Timestamptz(v) => {
            out.extend_from_slice(&v.to_le_bytes());
        }
        DefaultValue::Double(bits) => out.extend_from_slice(&bits.to_le_bytes()),
        DefaultValue::Numeric(text) | DefaultValue::Text(text) | DefaultValue::Jsonb(text) => {
            put_text(out, text);
        }
        DefaultValue::Bytes(bytes) => {
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        DefaultValue::Uuid(bytes) => out.extend_from_slice(bytes),
        DefaultValue::Date(days) => out.extend_from_slice(&days.to_le_bytes()),
    }
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SqlDefError> {
        if self.rest.len() < n {
            return Err(SqlDefError::Truncated);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SqlDefError> {
        self.take(N)?.try_into().map_err(|_| SqlDefError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, SqlDefError> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn u16(&mut self) -> Result<u16, SqlDefError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, SqlDefError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// A length-prefixed byte string; the length is checked against the remaining bytes first.
    fn bytes(&mut self) -> Result<&'a [u8], SqlDefError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// A name: the length is bounded to 1..=255 before any byte is taken.
    fn text(&mut self, bad_length: SqlDefError) -> Result<String, SqlDefError> {
        let len = self.u32()? as usize;
        if len == 0 || len > MAX_NAME {
            return Err(bad_length);
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| SqlDefError::BadUtf8)
    }

    fn string(&mut self) -> Result<String, SqlDefError> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| SqlDefError::BadUtf8)
    }

    fn default(&mut self) -> Result<DefaultValue, SqlDefError> {
        Ok(match self.u8()? {
            1 => match self.u8()? {
                0 => DefaultValue::Bool(false),
                1 => DefaultValue::Bool(true),
                _ => return Err(SqlDefError::BadBool),
            },
            2 => DefaultValue::Int(i64::from_le_bytes(self.array()?)),
            3 => DefaultValue::Double(u64::from_le_bytes(self.array()?)),
            4 => DefaultValue::Numeric(self.string()?),
            5 => DefaultValue::Text(self.string()?),
            6 => DefaultValue::Bytes(self.bytes()?.to_vec()),
            7 => DefaultValue::Uuid(self.array()?),
            8 => DefaultValue::Date(i32::from_le_bytes(self.array()?)),
            9 => DefaultValue::Timestamptz(i64::from_le_bytes(self.array()?)),
            10 => DefaultValue::Jsonb(self.string()?),
            t => return Err(SqlDefError::BadDefaultTag(t)),
        })
    }

    /// A key: positions then a name. The primary key may be empty (count 0, no name); a unique
    /// constraint needs at least one column.
    fn key(
        &mut self,
        columns: usize,
        required: bool,
    ) -> Result<Option<KeyConstraint>, SqlDefError> {
        let count = usize::from(self.u16()?);
        if count == 0 && !required {
            return Ok(None);
        }
        if count == 0 || count > columns {
            return Err(SqlDefError::BadKeyPosition);
        }
        let mut positions = Vec::new();
        for _ in 0..count {
            positions.push(self.u16()?);
        }
        let name = self.text(SqlDefError::BadConstraintName)?;
        Ok(Some(KeyConstraint {
            columns: positions,
            name,
        }))
    }
}

/// Encodes a definition block: `u32` count (1..=1024), then per operation tag 1, `u32` table id,
/// `u32` body length and the body. Table ids must ascend strictly.
pub fn encode_ops(ops: &[DefineTable]) -> Result<Vec<u8>, SqlDefError> {
    if ops.is_empty() {
        return Err(SqlDefError::EmptyDefinitions);
    }
    if ops.len() > MAX_DEFINITIONS {
        return Err(SqlDefError::TooManyDefinitions);
    }
    let mut out = (ops.len() as u32).to_le_bytes().to_vec();
    let mut previous: Option<u32> = None;
    for op in ops {
        check_id(op.table_id, previous)?;
        previous = Some(op.table_id);
        let body = op.definition.encode()?;
        out.push(OP_DEFINE_TABLE);
        out.extend_from_slice(&op.table_id.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
    }
    Ok(out)
}

fn check_id(id: u32, previous: Option<u32>) -> Result<(), SqlDefError> {
    if id > MAX_TABLE_ID {
        return Err(SqlDefError::BadTableId(id));
    }
    match previous {
        Some(p) if p == id => Err(SqlDefError::DuplicateTableId(id)),
        Some(p) if p > id => Err(SqlDefError::NotAscending),
        _ => Ok(()),
    }
}

/// Decodes a definition block from the start of `input` and returns it with the number of bytes
/// it used, so a containing envelope can read the fields that follow.
pub fn decode_ops_prefix(input: &[u8]) -> Result<(Vec<DefineTable>, usize), SqlDefError> {
    let mut r = Reader { rest: input };
    let count = r.u32()? as usize;
    if count == 0 {
        return Err(SqlDefError::EmptyDefinitions);
    }
    if count > MAX_DEFINITIONS {
        return Err(SqlDefError::TooManyDefinitions);
    }
    let mut ops = Vec::new();
    let mut previous: Option<u32> = None;
    for _ in 0..count {
        match r.u8()? {
            OP_DEFINE_TABLE => {}
            t => return Err(SqlDefError::BadOpTag(t)),
        }
        let table_id = r.u32()?;
        check_id(table_id, previous)?;
        previous = Some(table_id);
        let len = r.u32()? as usize;
        if len > MAX_BODY {
            return Err(SqlDefError::TooLarge);
        }
        let definition = SqlTableDef::decode(r.take(len)?)?;
        ops.push(DefineTable {
            table_id,
            definition,
        });
    }
    Ok((ops, input.len() - r.rest.len()))
}

/// Decodes a standalone definition block and refuses trailing bytes.
pub fn decode_ops(input: &[u8]) -> Result<Vec<DefineTable>, SqlDefError> {
    let (ops, used) = decode_ops_prefix(input)?;
    if used != input.len() {
        return Err(SqlDefError::Trailing);
    }
    Ok(ops)
}

#[cfg(test)]
mod tests;
