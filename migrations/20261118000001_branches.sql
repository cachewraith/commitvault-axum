-- Branches become records of their own: a commit references a branch row of
-- its project instead of carrying a free-text name, so a branch can be listed,
-- described and renamed, and a typo can no longer create a second "branch".

CREATE TABLE branches (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID        NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    -- Same rule set as the former commits.branch_name (validated in the API).
    name        TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 255),
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT branches_project_name_key UNIQUE (project_id, name),
    -- Target of the composite foreign key from commits (same-project guarantee).
    CONSTRAINT branches_id_project_key UNIQUE (id, project_id)
);
CREATE INDEX branches_name_trgm_idx ON branches USING gin (name gin_trgm_ops);
CREATE TRIGGER branches_set_updated_at BEFORE UPDATE ON branches
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- One row per branch name already used by a commit…
INSERT INTO branches (project_id, name, created_at)
SELECT project_id, branch_name, min(created_at)
FROM commits
GROUP BY project_id, branch_name;

-- …and the default branch of every project, so recording a commit in a new
-- project does not require creating a branch first.
INSERT INTO branches (project_id, name)
SELECT id, default_branch FROM projects
ON CONFLICT (project_id, name) DO NOTHING;

ALTER TABLE commits ADD COLUMN branch_id UUID;

UPDATE commits c
SET branch_id = b.id
FROM branches b
WHERE b.project_id = c.project_id AND b.name = c.branch_name;

-- NO ACTION (not RESTRICT), as for deployments: deleting a project cascades
-- through branches and commits in one statement, while deleting a branch that
-- still has commits fails unless the application removes them explicitly.
ALTER TABLE commits
    ALTER COLUMN branch_id SET NOT NULL,
    ADD CONSTRAINT commits_branch_fkey FOREIGN KEY (branch_id, project_id)
        REFERENCES branches (id, project_id) ON DELETE NO ACTION;

DROP INDEX commits_project_branch_idx;
ALTER TABLE commits DROP COLUMN branch_name;

-- Branch detail (commits of a branch, newest first) and per-branch counts.
CREATE INDEX commits_branch_date_idx ON commits (branch_id, commit_date DESC);
