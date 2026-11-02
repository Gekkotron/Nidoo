CREATE TABLE hub_restart (
    started_at timestamptz PRIMARY KEY,
    down_secs  bigint,
    stop       text NOT NULL CHECK (stop IN ('first', 'clean', 'unexpected'))
);
