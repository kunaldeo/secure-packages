DO $$ BEGIN
    CREATE TYPE ecosystem AS ENUM ('pypi', 'npm', 'cargo', 'go');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TYPE analysis_status AS ENUM ('pending', 'analyzing', 'approved', 'rejected', 'needs_review', 'failed');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS packages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ecosystem       ecosystem NOT NULL,
    name            TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(ecosystem, normalized_name)
);

CREATE TABLE IF NOT EXISTS package_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id      UUID NOT NULL REFERENCES packages(id),
    version         TEXT NOT NULL,
    source_sha256   TEXT,
    status          analysis_status NOT NULL DEFAULT 'pending',
    error_message   TEXT,
    metadata        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(package_id, version)
);

CREATE TABLE IF NOT EXISTS analysis_records (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_version_id  UUID NOT NULL REFERENCES package_versions(id),
    analysis_type       TEXT NOT NULL,
    static_scan         JSONB,
    llm_result          JSONB,
    diff_summary        JSONB,
    verdict             TEXT NOT NULL,
    risk_score          REAL,
    reasoning           TEXT,
    model_used          TEXT,
    prompt_tokens       INTEGER,
    completion_tokens   INTEGER,
    previous_version_id UUID REFERENCES package_versions(id),
    analyzed_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS audit_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor           TEXT NOT NULL,
    action          TEXT NOT NULL,
    target_type     TEXT,
    target_id       UUID,
    details_json    JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pv_status ON package_versions(status);
CREATE INDEX IF NOT EXISTS idx_pv_package ON package_versions(package_id);
CREATE INDEX IF NOT EXISTS idx_packages_eco_name ON packages(ecosystem, normalized_name);
CREATE INDEX IF NOT EXISTS idx_analysis_version ON analysis_records(package_version_id);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_log(created_at);
