#!/bin/sh
set -eu

# A new Fly volume is mounted over the image's pre-created home directory and
# starts root-owned. Fix only this dedicated mount before permanently dropping
# privileges; buzz-acp and every child runtime then run as the non-root agent.
chown agent:agent /home/agent
chmod 0700 /home/agent

exec su-exec agent /usr/local/bin/sprig-entrypoint "$@"
