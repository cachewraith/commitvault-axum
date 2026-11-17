use uuid::Uuid;

/// Row of the `notes` table (write side).
#[derive(Debug, Clone)]
pub struct Note {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub title: String,
    pub content_json: serde_json::Value,
    pub content_text: String,
    pub excerpt: String,
    pub category: Option<String>,
    pub is_pinned: bool,
}
