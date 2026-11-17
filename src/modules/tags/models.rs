use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Row of the `tags` table.
#[derive(Debug, Clone)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The entity kinds a tag can be attached to. Each maps to its own join table
/// so both sides keep real foreign keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTarget {
    Project,
    Commit,
    Deployment,
    Release,
    Note,
}

impl TagTarget {
    /// `(join table, foreign key column)` — compile-time constants, safe to
    /// interpolate into SQL.
    pub const fn table(self) -> (&'static str, &'static str) {
        match self {
            Self::Project => ("project_tags", "project_id"),
            Self::Commit => ("commit_tags", "commit_id"),
            Self::Deployment => ("deployment_tags", "deployment_id"),
            Self::Release => ("release_tags", "release_id"),
            Self::Note => ("note_tags", "note_id"),
        }
    }

    /// SQL sub-select yielding the tags of the row identified by `owner`.
    pub fn json_sql(self, owner: &str) -> String {
        let (table, column) = self.table();
        crate::shared::sql::tags_json(table, column, owner)
    }
}
