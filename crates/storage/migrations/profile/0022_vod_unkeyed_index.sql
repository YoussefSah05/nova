-- P07 follow-up: make the canonical-catalog upgrade check O(pending) on every
-- profile open. Only pre-0021 movie rows lack a binding key; imports key rows
-- as they insert them, so this partial index is empty in steady state.
CREATE INDEX IF NOT EXISTS idx_vod_unkeyed ON vod_items(source_id) WHERE binding_key IS NULL;
