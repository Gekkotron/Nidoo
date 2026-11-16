-- The sensor supervisor's problems: open ones (closed_at NULL) survive a restart; closed ones
-- are kept 30 days so a past outage stays out of the sensor's baseline.
CREATE TABLE supervisor_problem (
    entity_id  text        NOT NULL,
    check_kind text        NOT NULL,
    opened_at  timestamptz NOT NULL,
    closed_at  timestamptz,
    name       text        NOT NULL,
    alert_id   text,
    evidence   jsonb       NOT NULL,
    PRIMARY KEY (entity_id, check_kind, opened_at)
);

CREATE INDEX supervisor_problem_open ON supervisor_problem (entity_id) WHERE closed_at IS NULL;
