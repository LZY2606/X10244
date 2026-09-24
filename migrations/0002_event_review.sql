-- v2: review annotations attached without restructuring any provenance table.
ALTER TABLE events ADD COLUMN actor_label TEXT;
ALTER TABLE raw_points ADD COLUMN note TEXT NOT NULL DEFAULT '';
