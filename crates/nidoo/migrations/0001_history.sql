CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE entity_state (
    time      timestamptz      NOT NULL,
    seq       bigserial,
    entity_id text             NOT NULL,
    num       double precision,
    value     jsonb            NOT NULL
);
SELECT create_hypertable('entity_state', by_range('time', INTERVAL '7 days'));
CREATE INDEX entity_state_entity_time ON entity_state (entity_id, time DESC, seq DESC);
ALTER TABLE entity_state SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'entity_id',
    timescaledb.compress_orderby = 'time DESC, seq DESC'
);
SELECT add_compression_policy('entity_state', INTERVAL '7 days');

CREATE TABLE entity_availability (
    time         timestamptz NOT NULL,
    seq          bigserial,
    entity_id    text        NOT NULL,
    availability text        NOT NULL
);
SELECT create_hypertable('entity_availability', by_range('time', INTERVAL '30 days'));
CREATE INDEX entity_availability_entity_time ON entity_availability (entity_id, time DESC, seq DESC);
ALTER TABLE entity_availability SET (timescaledb.compress, timescaledb.compress_segmentby = 'entity_id');
SELECT add_compression_policy('entity_availability', INTERVAL '30 days');

CREATE TABLE entity_latest (
    entity_id text PRIMARY KEY,
    time      timestamptz NOT NULL,
    value     jsonb       NOT NULL
);
CREATE TABLE availability_latest (
    entity_id    text PRIMARY KEY,
    time         timestamptz NOT NULL,
    availability text        NOT NULL
);

CREATE TABLE history_hourly (
    entity_id  text             NOT NULL,
    bucket     timestamptz      NOT NULL,
    covered_us bigint           NOT NULL,
    integral   double precision NOT NULL,
    min        double precision NOT NULL,
    max        double precision NOT NULL,
    last       double precision NOT NULL,
    increase   double precision NOT NULL,
    PRIMARY KEY (entity_id, bucket)
);

CREATE TABLE rollup_state (
    id         smallint PRIMARY KEY CHECK (id = 1),
    done_until timestamptz NOT NULL
);
