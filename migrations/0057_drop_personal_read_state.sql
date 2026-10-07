-- Reverts migration 0056 (private read-state accessory API, #7906), which was
-- removed before being deployed. 0056 stays in the history because some
-- environments already applied it; this migration undoes it forward-only.
-- Children first: frontiers reference accounts. Dropping a table also drops
-- its community write fence trigger.
DROP TABLE IF EXISTS personal_read_frontiers;
DROP TABLE IF EXISTS personal_read_accounts;
