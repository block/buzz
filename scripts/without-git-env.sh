#!/usr/bin/env bash
# Git exports repository-local variables (GIT_DIR, GIT_INDEX_FILE,
# GIT_WORK_TREE, ...) to hooks. Pre-push lanes run test suites whose fixtures
# spawn git in temp dirs; with those variables inherited, fixtures operate on
# this checkout instead. Lanes rediscover the repository from their working
# directory.
set -euo pipefail

for var in $(env | sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p'); do
  unset "$var"
done

exec "$@"
