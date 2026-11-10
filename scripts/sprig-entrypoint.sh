#!/bin/bash
set -euo pipefail

# buzz-acp owns ephemeral Git setup for both native and MCP shells.
# The harness must receive Kubernetes' termination signal directly.
exec buzz-acp "$@"
