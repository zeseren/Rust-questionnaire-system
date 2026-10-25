CREATE TABLE users (
    id uuid PRIMARY KEY,
    username varchar(32) NOT NULL UNIQUE,
    password_hash text NOT NULL,
    role text NOT NULL CHECK (role IN ('user', 'admin')),
    disabled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL
);
CREATE INDEX users_created ON users(created_at DESC, id DESC);
CREATE TABLE sessions (
    id_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);

CREATE TABLE surveys (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES users(id),
    status text NOT NULL CHECK (status IN ('unpublished', 'collecting', 'closed', 'archived')),
    mode text CHECK (mode IN ('anonymous', 'login_once')),
    deadline timestamptz,
    taken_down boolean NOT NULL DEFAULT false,
    draft jsonb NOT NULL CHECK (jsonb_typeof(draft) = 'object'),
    revision bigint NOT NULL CHECK (revision > 0),
    current_version_id uuid,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CHECK ((status = 'unpublished' AND current_version_id IS NULL)
        OR (status <> 'unpublished' AND current_version_id IS NOT NULL AND mode IS NOT NULL))
);
CREATE INDEX surveys_owner_created ON surveys(owner_id, created_at DESC, id DESC);
CREATE INDEX surveys_owner_updated ON surveys(owner_id, updated_at DESC, id DESC);
CREATE INDEX surveys_created ON surveys(created_at DESC, id DESC);
CREATE TABLE versions (
    id uuid PRIMARY KEY,
    survey_id uuid NOT NULL REFERENCES surveys(id),
    number integer NOT NULL CHECK (number > 0),
    document jsonb NOT NULL CHECK (jsonb_typeof(document) = 'object'),
    published_at timestamptz NOT NULL,
    UNIQUE (survey_id, number),
    UNIQUE (survey_id, id)
);
ALTER TABLE surveys ADD CONSTRAINT survey_current_version
    FOREIGN KEY (id, current_version_id) REFERENCES versions(survey_id, id);

-- 一条记录包含整份已校验答案及成功幂等记录，写入天然原子，不会留下半份答卷。
CREATE TABLE responses (
    id uuid PRIMARY KEY,
    survey_id uuid NOT NULL REFERENCES surveys(id),
    version_id uuid NOT NULL,
    respondent_id uuid REFERENCES users(id),
    scope text NOT NULL,
    key_hash text NOT NULL,
    payload_hash text NOT NULL,
    answers jsonb NOT NULL CHECK (jsonb_typeof(answers) = 'object'),
    submitted_at timestamptz NOT NULL,
    FOREIGN KEY (survey_id, version_id) REFERENCES versions(survey_id, id),
    UNIQUE (survey_id, scope, key_hash),
    CHECK ((respondent_id IS NULL AND scope = 'anonymous')
        OR (respondent_id IS NOT NULL AND scope = respondent_id::text))
);
-- NULL 不参与限次；匿名模式不假装能够识别同一自然人。
CREATE UNIQUE INDEX responses_once_per_account ON responses(survey_id, respondent_id)
    WHERE respondent_id IS NOT NULL;
CREATE INDEX responses_version_time ON responses(survey_id, version_id, submitted_at, id);

CREATE TABLE audits (
    id uuid PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES users(id),
    action text NOT NULL CHECK (action IN ('disable_user', 'enable_user', 'take_down', 'restore_survey')),
    target_id uuid NOT NULL,
    reason varchar(500) NOT NULL CHECK (char_length(btrim(reason)) > 0),
    created_at timestamptz NOT NULL
);
CREATE INDEX audits_created ON audits(created_at DESC, id DESC);
