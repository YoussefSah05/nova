-- Country is a first-class browse filter for live channels, but until now the
-- only channel indexes covered (source_id) and (source_id, category). Filtering
-- by country therefore fell back to a full table scan on large playlists
-- (tens of thousands of channels). Mirror the category index so the country
-- chip is as fast as the genre chip.
CREATE INDEX IF NOT EXISTS idx_channels_source_country ON channels(source_id, country);
