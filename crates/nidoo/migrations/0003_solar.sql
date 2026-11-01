CREATE TABLE solar_forecast_days (
    date         date             PRIMARY KEY,
    estimate_kwh double precision NOT NULL,
    low_kwh      double precision NOT NULL,
    high_kwh     double precision NOT NULL,
    made_at      timestamptz      NOT NULL
);
