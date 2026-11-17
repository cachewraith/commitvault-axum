//! SQL fragments reused by several repositories. Everything here is a
//! compile-time constant — user input is only ever passed as bind parameters.

/// JSON object describing a commit (`c` must be the commits alias in scope).
pub const COMMIT_BRIEF_JSON: &str = "jsonb_build_object(\
    'id', c.id, 'short_hash', c.short_hash, 'commit_hash', c.commit_hash, \
    'commit_message', c.commit_message, 'branch_name', c.branch_name, 'commit_date', c.commit_date)";

/// JSON object describing a project (`p` must be the projects alias in scope).
pub const PROJECT_REF_JSON: &str = "jsonb_build_object('id', p.id, 'name', p.name, 'slug', p.slug)";

/// JSON object describing an environment (`e` must be the environments alias in scope).
pub const ENVIRONMENT_REF_JSON: &str =
    "jsonb_build_object('id', e.id, 'name', e.name, 'slug', e.slug, 'color', e.color)";

/// Correlated sub-select producing the tags of one row as a JSON array.
/// `join_table`/`fk_column` come from `TagTarget` (constants); `owner` is the
/// qualified id column of the row in scope, e.g. `c.id`.
pub fn tags_json(join_table: &str, fk_column: &str, owner: &str) -> String {
    format!(
        "(SELECT coalesce(jsonb_agg(jsonb_build_object('id', t.id, 'name', t.name, 'slug', t.slug, 'color', t.color) \
         ORDER BY t.name), '[]'::jsonb) FROM {join_table} jt JOIN tags t ON t.id = jt.tag_id WHERE jt.{fk_column} = {owner})"
    )
}
