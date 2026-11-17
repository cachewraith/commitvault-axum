use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Row of the `commits` table (write side).
#[derive(Debug, Clone)]
pub struct Commit {
    pub id: Uuid,
    pub project_id: Uuid,
    pub branch_name: String,
    pub commit_hash: String,
    pub short_hash: String,
    pub commit_message: String,
    pub description: Option<String>,
    pub commit_date: DateTime<Utc>,
    pub author_name: Option<String>,
    pub external_url: Option<String>,
}
