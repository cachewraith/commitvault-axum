-- CommitVault initial schema.
-- See docs/DATABASE.md for the reasoning behind each decision.

-- Trigram indexes back the `ILIKE '%term%'` searches (pg_trgm is a trusted
-- extension, so the database owner can create it without superuser rights).
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Keeps `updated_at` correct regardless of which code path writes the row.
CREATE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- Authentication
-- ---------------------------------------------------------------------------

CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Stored lower-cased; the unique index therefore enforces case-insensitive uniqueness.
    email         TEXT        NOT NULL CHECK (email = lower(email) AND char_length(email) BETWEEN 3 AND 254),
    password_hash TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_key ON users (email);
CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Server-side session records (tower-sessions). `id` is the opaque random
-- session id; the browser only ever holds a signed cookie containing it.
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    data        JSONB       NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);

-- ---------------------------------------------------------------------------
-- Projects and environments
-- ---------------------------------------------------------------------------

CREATE TABLE projects (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    slug           TEXT        NOT NULL CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) <= 140),
    description    TEXT,
    repository_url TEXT,
    default_branch TEXT        NOT NULL DEFAULT 'main' CHECK (char_length(default_branch) BETWEEN 1 AND 255),
    status         TEXT        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at    TIMESTAMPTZ,
    -- status and archived_at always agree.
    CONSTRAINT projects_archived_consistency CHECK ((status = 'archived') = (archived_at IS NOT NULL))
);
CREATE UNIQUE INDEX projects_slug_key ON projects (slug);
CREATE INDEX projects_status_updated_idx ON projects (status, updated_at DESC);
CREATE INDEX projects_name_trgm_idx ON projects USING gin (name gin_trgm_ops);
CREATE TRIGGER projects_set_updated_at BEFORE UPDATE ON projects
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE environments (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID        NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name         TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    slug         TEXT        NOT NULL CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) <= 100),
    description  TEXT,
    color        TEXT        NOT NULL DEFAULT '#71717a' CHECK (color ~ '^#[0-9a-f]{6}$'),
    server_label TEXT,
    -- Display order inside the project (user-controlled, see reorder endpoint).
    position     INTEGER     NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT environments_project_slug_key UNIQUE (project_id, slug),
    -- Target of the composite foreign key from deployments (same-project guarantee).
    CONSTRAINT environments_id_project_key UNIQUE (id, project_id)
);
CREATE INDEX environments_project_position_idx ON environments (project_id, position);
CREATE TRIGGER environments_set_updated_at BEFORE UPDATE ON environments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- Commits and deployments
-- ---------------------------------------------------------------------------

CREATE TABLE commits (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id     UUID        NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    branch_name    TEXT        NOT NULL CHECK (char_length(branch_name) BETWEEN 1 AND 255),
    -- Lower-case hex, 7..64 chars (abbreviated, SHA-1 or SHA-256 object names).
    commit_hash    TEXT        NOT NULL CHECK (commit_hash ~ '^[0-9a-f]{7,64}$'),
    short_hash     TEXT        NOT NULL CHECK (short_hash ~ '^[0-9a-f]{7,12}$'),
    commit_message TEXT        NOT NULL CHECK (char_length(commit_message) BETWEEN 1 AND 2000),
    description    TEXT,
    commit_date    TIMESTAMPTZ NOT NULL,
    author_name    TEXT,
    external_url   TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A project never contains the same commit twice. Also the lookup index
    -- for commits(project_id, commit_hash).
    CONSTRAINT commits_project_hash_key UNIQUE (project_id, commit_hash),
    CONSTRAINT commits_id_project_key UNIQUE (id, project_id)
);
CREATE INDEX commits_project_branch_idx ON commits (project_id, branch_name);
CREATE INDEX commits_project_date_idx ON commits (project_id, commit_date DESC);
CREATE INDEX commits_date_idx ON commits (commit_date DESC);
CREATE INDEX commits_hash_prefix_idx ON commits (commit_hash text_pattern_ops);
CREATE INDEX commits_message_trgm_idx ON commits USING gin (commit_message gin_trgm_ops);
CREATE TRIGGER commits_set_updated_at BEFORE UPDATE ON commits
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- A deployment is a *recorded event*: "the user noted that this commit was
-- deployed to this environment at this time". One commit may have many.
CREATE TABLE deployments (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       UUID        NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    environment_id   UUID        NOT NULL,
    commit_id        UUID        NOT NULL,
    deployment_title TEXT        NOT NULL CHECK (char_length(deployment_title) BETWEEN 1 AND 200),
    deployment_notes TEXT,
    deployed_at      TIMESTAMPTZ NOT NULL,
    status           TEXT        NOT NULL DEFAULT 'successful'
                     CHECK (status IN ('successful', 'failed', 'rolled_back', 'pending')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Composite keys guarantee the environment and the commit belong to the
    -- same project as the deployment. NO ACTION (not RESTRICT): the check runs
    -- at end of statement, so deleting a project can cascade through all three
    -- tables, while deleting a commit/environment that still has deployment
    -- history fails unless the application removes that history explicitly.
    CONSTRAINT deployments_environment_fkey FOREIGN KEY (environment_id, project_id)
        REFERENCES environments (id, project_id) ON DELETE NO ACTION,
    CONSTRAINT deployments_commit_fkey FOREIGN KEY (commit_id, project_id)
        REFERENCES commits (id, project_id) ON DELETE NO ACTION
);
CREATE INDEX deployments_project_env_date_idx ON deployments (project_id, environment_id, deployed_at DESC);
CREATE INDEX deployments_environment_date_idx ON deployments (environment_id, deployed_at DESC);
CREATE INDEX deployments_commit_idx ON deployments (commit_id);
CREATE INDEX deployments_date_idx ON deployments (deployed_at DESC);
CREATE TRIGGER deployments_set_updated_at BEFORE UPDATE ON deployments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- Releases
-- ---------------------------------------------------------------------------

CREATE TABLE releases (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id         UUID        NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    -- Free-form: semantic versions are supported but not enforced.
    version            TEXT        NOT NULL CHECK (char_length(version) BETWEEN 1 AND 64),
    title              TEXT        NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    description        TEXT,
    -- Tiptap document (validated against an allow-list before it is stored).
    release_notes      JSONB,
    -- Plain text extracted from release_notes; used for search only.
    release_notes_text TEXT        NOT NULL DEFAULT '',
    released_at        TIMESTAMPTZ,
    status             TEXT        NOT NULL DEFAULT 'draft'
                       CHECK (status IN ('draft', 'released', 'deprecated')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT releases_project_version_key UNIQUE (project_id, version),
    CONSTRAINT releases_id_project_key UNIQUE (id, project_id)
);
CREATE INDEX releases_project_released_idx ON releases (project_id, released_at DESC NULLS FIRST);
CREATE TRIGGER releases_set_updated_at BEFORE UPDATE ON releases
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE release_commits (
    release_id UUID        NOT NULL,
    commit_id  UUID        NOT NULL,
    project_id UUID        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (release_id, commit_id),
    -- Same-project guarantee, as for deployments. Association rows disappear
    -- with either side.
    CONSTRAINT release_commits_release_fkey FOREIGN KEY (release_id, project_id)
        REFERENCES releases (id, project_id) ON DELETE CASCADE,
    CONSTRAINT release_commits_commit_fkey FOREIGN KEY (commit_id, project_id)
        REFERENCES commits (id, project_id) ON DELETE CASCADE
);
CREATE INDEX release_commits_commit_idx ON release_commits (commit_id);

-- ---------------------------------------------------------------------------
-- Technical notes
-- ---------------------------------------------------------------------------

CREATE TABLE notes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- NULL = global note (not tied to a project).
    project_id   UUID        REFERENCES projects (id) ON DELETE CASCADE,
    title        TEXT        NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    -- Tiptap document; the source of truth. HTML is never stored.
    content_json JSONB       NOT NULL,
    -- Plain text extracted from content_json; used for search only.
    content_text TEXT        NOT NULL DEFAULT '',
    excerpt      TEXT        NOT NULL DEFAULT '',
    category     TEXT        CHECK (category IS NULL OR char_length(category) BETWEEN 1 AND 60),
    is_pinned    BOOLEAN     NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX notes_project_updated_idx ON notes (project_id, updated_at DESC);
CREATE INDEX notes_updated_idx ON notes (updated_at DESC);
CREATE INDEX notes_category_idx ON notes (category) WHERE category IS NOT NULL;
CREATE INDEX notes_title_trgm_idx ON notes USING gin (title gin_trgm_ops);
CREATE INDEX notes_content_trgm_idx ON notes USING gin (content_text gin_trgm_ops);
CREATE TRIGGER notes_set_updated_at BEFORE UPDATE ON notes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- Tags
-- ---------------------------------------------------------------------------

CREATE TABLE tags (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 50),
    slug       TEXT        NOT NULL CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) <= 60),
    color      TEXT        NOT NULL DEFAULT '#71717a' CHECK (color ~ '^#[0-9a-f]{6}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX tags_slug_key ON tags (slug);
CREATE TRIGGER tags_set_updated_at BEFORE UPDATE ON tags
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- One join table per taggable entity keeps real foreign keys on both sides
-- (a polymorphic "taggable_type/taggable_id" table could not).
CREATE TABLE project_tags (
    project_id UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    tag_id     UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, tag_id)
);
CREATE INDEX project_tags_tag_idx ON project_tags (tag_id);

CREATE TABLE commit_tags (
    commit_id UUID NOT NULL REFERENCES commits (id) ON DELETE CASCADE,
    tag_id    UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (commit_id, tag_id)
);
CREATE INDEX commit_tags_tag_idx ON commit_tags (tag_id);

CREATE TABLE deployment_tags (
    deployment_id UUID NOT NULL REFERENCES deployments (id) ON DELETE CASCADE,
    tag_id        UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (deployment_id, tag_id)
);
CREATE INDEX deployment_tags_tag_idx ON deployment_tags (tag_id);

CREATE TABLE release_tags (
    release_id UUID NOT NULL REFERENCES releases (id) ON DELETE CASCADE,
    tag_id     UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (release_id, tag_id)
);
CREATE INDEX release_tags_tag_idx ON release_tags (tag_id);

CREATE TABLE note_tags (
    note_id UUID NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    tag_id  UUID NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, tag_id)
);
CREATE INDEX note_tags_tag_idx ON note_tags (tag_id);
