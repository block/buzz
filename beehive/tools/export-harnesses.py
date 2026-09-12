"""Export Desktop static discovery data; run explicitly at a reviewed revision.
No Rust invocation, executable probe, credentials, or package dependency required.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

revision = sys.argv[1]
base = 'desktop/src-tauri/src/managed_agents/discovery/'
rows = []
for name, struct in [('catalog.rs', 'KnownAcpRuntime'), ('presets.rs', 'PresetHarness')]:
    source = subprocess.check_output(['git', 'show', f'{revision}:{base}{name}'], text=True)
    source = source.split('const ' + ('KNOWN_ACP_RUNTIMES' if name == 'catalog.rs' else 'PRESET_HARNESSES'), 1)[1].split('\n];', 1)[0]
    for block in source.split(struct + ' {')[1:]:
        match = re.search(r'\bid: "([^"]+)"', block)
        if not match:
            continue
        def string(field):
            m = re.search(r'\b' + field + r': (?:Some\()?"([^"]+)"', block)
            return m[1] if m else None
        commands = re.search(r'commands: &\[([^]]+)\]', block)
        rows.append(dict(id=match[1], label=string('label'), commands=re.findall(r'"([^"]+)"', commands[1]) if commands else [string('command')], underlyingCli=string('underlying_cli'), modelEnv=string('model_env_var'), providerEnv=string('provider_env_var'), effortEnv=string('thinking_env_var')))
discovery = subprocess.check_output(['git', 'show', f'{revision}:desktop/src-tauri/src/managed_agents/discovery.rs'], text=True)
minimum = [int(v.strip()) for v in re.search(r'MIN_CODEX_ACP_VERSION: \(u64, u64, u64\) = \(([^)]+)\)', discovery)[1].split(',')]
managed = subprocess.check_output(['git', 'show', f'{revision}:desktop/src-tauri/src/managed_agents/managed_node_paths.rs'], text=True)
node_version = re.search(r'BUZZ_MANAGED_NODE_VERSION: &str = "([^"]+)"', managed)[1]
Path('beehive/src/harness-contract.json').write_text(json.dumps(dict(revision=revision, sources=[base+'catalog.rs', base+'presets.rs'], codexMinimum=minimum, managedNodeVersion=node_version, runtimes=rows), indent=2)+'\n')
