-- EPG boundary (M24): each programme belongs to the XMLTV feed that supplied
-- it. A feed's refresh replaces only its own rows, so two feeds covering the
-- same channel ID (a provider feed and a community one, or two sources) no
-- longer overwrite each other; deleting a source (→ its feeds) removes its
-- programmes. Rows written before this migration have no owner and are
-- replaced by the next refresh that covers their channel.
ALTER TABLE programs ADD COLUMN epg_source_id INTEGER REFERENCES epg_sources(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_programs_feed ON programs(epg_source_id);
