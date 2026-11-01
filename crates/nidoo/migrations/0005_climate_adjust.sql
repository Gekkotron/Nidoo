-- The offset the room-thermometer correction applied to a device's command: the stored
-- setpoint is the target, the command was `setpoint + adjust`.
ALTER TABLE climate_decision ADD COLUMN adjust double precision;
