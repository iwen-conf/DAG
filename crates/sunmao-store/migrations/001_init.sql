-- sunmao full schema (D-19 multi-project: all domain tables hang project_id)

CREATE TABLE project (
    id          text PRIMARY KEY,
    name        text NOT NULL,
    repo_path   text NOT NULL UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE node (
    id              text NOT NULL,
    project_id      text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    graph_version   bigint NOT NULL,
    parent_id       text,
    path            text NOT NULL,
    kind            text NOT NULL CHECK (kind IN ('package','task')),
    layer           text,
    role            text,
    title           text NOT NULL,
    spec            jsonb NOT NULL DEFAULT '{}',

    task_state      text CHECK (task_state IN
                      ('todo','ready','claimed','running','review','done','failed','cancelled')),
    ready           boolean NOT NULL DEFAULT false,
    priority        int NOT NULL DEFAULT 0,
    required_caps   text[] NOT NULL DEFAULT '{}',
    write_scope     text[] NOT NULL DEFAULT '{}',
    inputs          jsonb NOT NULL DEFAULT '[]',
    validators      text[] NOT NULL DEFAULT '{}',
    max_attempts    int NOT NULL DEFAULT 3,

    owner           text,
    lease_token     uuid,
    lease_expires   timestamptz,

    scope_state     text NOT NULL DEFAULT 'active'
                      CHECK (scope_state IN ('active','paused','closed','archived')),
    permanent       boolean NOT NULL DEFAULT false,
    scope_reason    text,
    scope_actor     text,
    scope_until     timestamptz,
    plan_state      text NOT NULL DEFAULT 'draft'
                      CHECK (plan_state IN ('draft','planning','planned')),

    needs_replan    boolean NOT NULL DEFAULT false,

    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (project_id, id),
    CONSTRAINT task_fields CHECK (kind != 'task' OR task_state IS NOT NULL)
);

CREATE TABLE edge (
    project_id text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    from_id    text NOT NULL,
    to_id      text NOT NULL,
    PRIMARY KEY (project_id, from_id, to_id),
    CHECK (from_id != to_id)
);

CREATE TABLE event (
    id          text PRIMARY KEY,
    project_id  text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    seq         bigserial UNIQUE,
    node_id     text,
    actor       text NOT NULL,
    kind        text NOT NULL,
    payload     jsonb NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE attempt (
    id            text PRIMARY KEY,
    project_id    text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    node_id       text NOT NULL,
    seq_no        int  NOT NULL,
    owner         text NOT NULL,
    started_at    timestamptz NOT NULL DEFAULT now(),
    ended_at      timestamptz,
    outcome       text CHECK (outcome IN
                    ('done','validation_failed','lease_expired','cancelled','error')),
    failure       jsonb,
    handover      jsonb,
    UNIQUE (project_id, node_id, seq_no)
);

CREATE TABLE artifact (
    id           text PRIMARY KEY,
    project_id   text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    node_id      text NOT NULL,
    attempt_id   text NOT NULL REFERENCES attempt(id),
    paths        text[] NOT NULL,
    commit_hash  text NOT NULL,
    digest       text NOT NULL,
    version      text,
    published_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE graph_version (
    project_id  text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    version     bigint NOT NULL,
    planner     text NOT NULL,
    summary     text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, version)
);

-- Contract major pending
CREATE TABLE contract_pending (
    project_id   text NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    artifact_id  text NOT NULL,
    version      text NOT NULL,
    bump         text NOT NULL,
    node_id      text NOT NULL,
    approved     boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, artifact_id, version)
);

CREATE INDEX idx_node_claim   ON node (project_id, priority DESC, id) WHERE kind='task' AND ready;
CREATE INDEX idx_node_path    ON node (project_id, path text_pattern_ops);
CREATE INDEX idx_node_lease   ON node (lease_expires) WHERE task_state IN ('claimed','running','review');
CREATE INDEX idx_edge_to      ON edge (project_id, to_id);
CREATE INDEX idx_event_node   ON event (node_id, seq);
CREATE INDEX idx_event_proj   ON event (project_id, seq);
CREATE INDEX idx_attempt_node ON attempt (project_id, node_id, seq_no);

-- NOTIFY trigger for SSE
CREATE OR REPLACE FUNCTION sunmao_notify_event() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('sunmao_events', json_build_object(
    'seq', NEW.seq,
    'project_id', NEW.project_id,
    'kind', NEW.kind,
    'node_id', NEW.node_id
  )::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_event_notify
AFTER INSERT ON event
FOR EACH ROW EXECUTE FUNCTION sunmao_notify_event();

CREATE VIEW v_task_blocked AS
SELECT t.project_id, t.id,
       CASE WHEN anc.id IS NOT NULL THEN 'scope:' || anc.scope_state
            ELSE 'deps:' || array_to_string(unmet.ids, ',') END AS reason
FROM node t
LEFT JOIN LATERAL (
    SELECT p.id, p.scope_state FROM node p
    WHERE p.project_id = t.project_id AND p.kind='package' AND p.scope_state != 'active'
      AND t.path LIKE p.path || '.%'
    ORDER BY length(p.path) DESC LIMIT 1
) anc ON true
LEFT JOIN LATERAL (
    SELECT array_agg(e.from_id) AS ids FROM edge e
    JOIN node up ON up.project_id = e.project_id AND up.id = e.from_id
    WHERE e.project_id = t.project_id AND e.to_id = t.id
      AND (up.kind='task' AND up.task_state != 'done' OR up.kind='package')
) unmet ON true
WHERE t.kind='task' AND t.task_state IN ('todo','ready')
  AND (anc.id IS NOT NULL OR unmet.ids IS NOT NULL);

CREATE VIEW v_package_progress AS
SELECT p.project_id, p.id,
       count(*) FILTER (WHERE t.task_state='done') AS done,
       count(*) FILTER (WHERE t.task_state IN ('claimed','running','review')) AS active,
       count(*) AS total
FROM node p
JOIN node t ON t.project_id = p.project_id AND t.kind='task' AND t.path LIKE p.path || '.%'
WHERE p.kind='package'
GROUP BY p.project_id, p.id;
