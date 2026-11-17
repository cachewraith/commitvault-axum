use uuid::Uuid;

/// Row of the `environments` table (write side).
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: String,
    pub server_label: Option<String>,
}
