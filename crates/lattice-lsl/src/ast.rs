//! Syntax tree for LSL v0.

/// A field type with its refinements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// `bool`.
    Bool,
    /// `int`, optionally restricted with `in lo..=hi`.
    Int(Option<(i64, i64)>),
    /// `text`, optionally with `max <chars>`.
    Text(Option<u64>),
    /// `bytes`, optionally with `max <bytes>`.
    Bytes(Option<u64>),
    /// `ref<TypeName>`: the id of another entity.
    Ref(String),
    /// `list<T>`, optionally with `max <items>`.
    List(Box<Ty>, Option<u64>),
    /// A named enum.
    Enum(String),
}

/// One field of a facet or type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: Ty,
    /// Whether the field may be absent.
    pub optional: bool,
    /// 1-based source line, for diagnostics.
    pub line: u32,
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// `enum Name { a, b }`.
    Enum {
        /// Enum name.
        name: String,
        /// Variants in declaration order.
        variants: Vec<String>,
        /// Source line.
        line: u32,
    },
    /// `facet Name { fields }`: a reusable group of fields.
    Facet {
        /// Facet name.
        name: String,
        /// Fields.
        fields: Vec<FieldDef>,
        /// Source line.
        line: u32,
    },
    /// `type Name = FacetA + FacetB { fields }`: an entity type composed from facets.
    Type {
        /// Type name.
        name: String,
        /// Composed facets.
        facets: Vec<String>,
        /// Own fields.
        fields: Vec<FieldDef>,
        /// Source line.
        line: u32,
    },
}

/// A parsed schema file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schema {
    /// Declarations in order.
    pub items: Vec<Item>,
}
