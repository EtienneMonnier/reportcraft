//! Schema discovery: what a data source contains (tables, views, columns, keys, relationships),
//! so people and agents can write queries without guessing. Every connector that has a schema
//! implements `SchemaSource`; the types here are shared and serialisable.

use reportcraft_core::{FieldType, Result, RowSet};
use serde::{Deserialize, Serialize};

/// A source whose structure can be listed.
pub trait SchemaSource {
    /// Every table and view, with columns and keys, sorted by name.
    fn schema(&self) -> Result<Schema>;
    /// The first `limit` rows of one table or view (capped by the source).
    fn preview(&self, table: &str, limit: usize) -> Result<RowSet>;
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Schema {
    pub tables: Vec<Table>,
}

impl Schema {
    /// A table or view by name (case-insensitive).
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<ColumnInfo>,
    /// Relationships from this table's columns to another table's.
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    /// The type exactly as the source declares it (may be empty).
    pub declared_type: String,
    /// How ReportCraft reads it.
    pub field_type: FieldType,
    pub nullable: bool,
    /// Position in the primary key (1-based), if part of it.
    #[serde(default)]
    pub primary_key: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub columns: Vec<String>,
    pub references_table: String,
    pub references_columns: Vec<String>,
}
