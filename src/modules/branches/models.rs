use uuid::Uuid;

/// Row of the `branches` table (write side).
#[derive(Debug, Clone)]
pub struct Branch {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
}
