CREATE TABLE IF NOT EXISTS embed_sessions_v2 (
  id TEXT PRIMARY KEY,
  workbench_session_id TEXT NOT NULL,
  code_hash TEXT NOT NULL UNIQUE,
  target_path TEXT NOT NULL,
  audience TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  used_at INTEGER,
  status TEXT NOT NULL CHECK (status IN ('pending', 'used', 'expired', 'revoked')),
  created_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO embed_sessions_v2
  SELECT * FROM embed_sessions;

DROP TABLE embed_sessions;
ALTER TABLE embed_sessions_v2 RENAME TO embed_sessions;

CREATE INDEX IF NOT EXISTS idx_embed_sessions_expiry
  ON embed_sessions (status, expires_at);

CREATE INDEX IF NOT EXISTS idx_embed_sessions_workbench_session
  ON embed_sessions (workbench_session_id, created_at);
