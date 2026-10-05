-- Automatic playlist refresh becomes opt-in. The stored interval defaulted to
-- 24 hours but was never acted on, so no playlist has ever auto-refreshed and a
-- deliberate "Daily" cannot be told apart from that default. Every playlist
-- starts on Manual; users enable an interval per playlist in Settings.
UPDATE sources SET auto_refresh_hours = 0;
