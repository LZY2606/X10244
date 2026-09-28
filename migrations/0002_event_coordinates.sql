ALTER TABLE events ADD COLUMN coord_basis TEXT;
ALTER TABLE rule_params ADD COLUMN coord_basis TEXT;
UPDATE rule_params SET coord_basis = 'canonical_q:A^-1;legacy_v1' WHERE coord_basis IS NULL;
UPDATE events SET coord_basis = 'event-log;canonical_q:A^-1' WHERE coord_basis IS NULL;
