-- Track the last TOTP time-step that was successfully consumed.
-- Used to reject replay of a code within the same 30-second window.
ALTER TABLE local_totp ADD COLUMN last_used_step BIGINT;
ALTER TABLE local_totp_trusted_devices ADD COLUMN last_used_step BIGINT;
