-- The assistant's turns: one row per sentence or answer, kept 90 days.
CREATE TABLE assist_turn (
    seq      bigserial,
    id       text PRIMARY KEY,
    at       timestamptz NOT NULL,
    client   text NOT NULL,
    text     text NOT NULL,
    simulate boolean NOT NULL,
    outcome  text NOT NULL,
    reply    text NOT NULL,
    question jsonb,
    trace    jsonb NOT NULL,
    wrong    boolean NOT NULL DEFAULT false
);

CREATE INDEX assist_turn_at ON assist_turn (at DESC, seq DESC);
