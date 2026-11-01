CREATE TABLE climate_decision (
    time      timestamptz      NOT NULL,
    seq       bigserial,
    zone      text             NOT NULL,
    entity_id text             NOT NULL,
    mode      text             NOT NULL,
    setpoint  double precision,
    source    text             NOT NULL,
    reason    text             NOT NULL,
    applied   boolean          NOT NULL
);
SELECT create_hypertable('climate_decision', by_range('time', INTERVAL '30 days'));
CREATE INDEX climate_decision_zone_time ON climate_decision (zone, time DESC, seq DESC);
CREATE INDEX climate_decision_entity_time ON climate_decision (entity_id, time DESC, seq DESC);
ALTER TABLE climate_decision SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'zone',
    timescaledb.compress_orderby = 'time DESC, seq DESC'
);
SELECT add_compression_policy('climate_decision', INTERVAL '30 days');
