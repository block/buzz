SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE channels VALIDATE CONSTRAINT channels_fork_source_paired;
