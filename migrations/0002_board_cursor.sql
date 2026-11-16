-- Board state needs three values the original schema did not name: the attempt
-- count, the hits from the last successful sweep, and the page cursor the sweep
-- uses to skip an unchanged second page. SQLite cannot add a column with a
-- non-constant default, so the two counters start at 0.
ALTER TABLE board_health ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE board_health ADD COLUMN hits_last_sweep INTEGER NOT NULL DEFAULT 0;
ALTER TABLE board_health ADD COLUMN cursor_json TEXT;
