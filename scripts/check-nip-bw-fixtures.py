#!/usr/bin/env python3
"""Check the BW documentation corpus, not production workflow semantics."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / 'docs/nips/NIP-BW.fixtures.json'
RECORDS = {
    'role-policy', 'issue-update', 'triage-action', 'issue-state',
    'issue-relation', 'release-pipeline', 'release-set', 'build-request',
    'build-run', 'test-ready', 'member-verdict',
}
COVERAGE = {
    'wrong-id', 'wrong-signature', 'wrong-role', 'wrong-repo', 'wrong-root',
    'unknown-tag', 'duplicate-tag', 'arity', 'oversize', 'stale-previous',
    'fork', 'missing-reference', 'out-of-order', 'replay', 'cutover',
    'policy-boundary', 'legacy-ignored', 'assignment-change', 'old-writer',
    'missing-implemented', 'wrong-stream', 'platform-mix', 'non-leaf',
    'commit-ancestry', 'duplicate-member', 'active-membership', 'build-lock',
    'retry', 'dedupe', 'wrong-run', 'wrong-head', 'artifact-digest',
    'wrong-tester', 'foreign-verdict', 'old-verdict', 'partial-reject',
    'resolved-sibling', 'rework-new-set', 'new-artifact-no-inheritance',
    'attestation-preserved', 'attestation-negative', 'set-failed', 'set-aborted',
    'handoff-order-convergence', 'handoff-equal-time', 'backlog-role',
    'historical-handoff-acceptance', 'acceptance-conflict-order',
    'forged-acceptance', 'forged-acceptance-order', 'technical-conflict-not-rejection',
    'acceptance-survives-technical-conflict', 'completed-set',
    'completed-set-handoff-order', 'completed-set-close-order', 'terminal-close-conflict-order', 'completed-set-conflict-order',
    'terminal-set-no-build',
    'new-artifact-unreviewed', 'later-test-does-not-reopen',
    'new-followup-bug', 'new-followup-change',
}
OUTCOMES = {'accept', 'reject', 'pending', 'conflict', 'replay', 'ignore'}
STAGES = {'envelope', 'id', 'signature', 'shape', 'attestation', 'references',
          'policy', 'role', 'causality', 'external', 'projection'}


def require(ok, message):
    """Raise a useful corpus error instead of accepting incomplete inputs."""
    if not ok:
        raise ValueError(message)


def unique_object(pairs):
    """Reject duplicate JSON keys instead of silently keeping the last."""
    result = {}
    for key, value in pairs:
        require(key not in result, f'duplicate JSON key: {key}')
        result[key] = value
    return result


def load(path):
    """Read strict UTF-8 JSON with duplicate-key and non-integer rejection."""
    def invalid(value):
        raise ValueError(f'non-integer JSON number: {value}')
    return json.loads(Path(path).read_bytes().decode('utf-8'),
                      object_pairs_hook=unique_object,
                      parse_float=invalid, parse_constant=invalid)


def compact(value):
    """Produce NIP-01 compact JSON bytes without changing content strings."""
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')


def matches(pattern, value):
    return isinstance(value, str) and re.fullmatch(pattern, value) is not None


def check_type(value, kind, data, field=''):
    """Validate only the documented closed primitive/object shape language."""
    if kind.startswith('?'):
        if value is not None:
            check_type(value, kind[1:], data, field)
    elif kind.startswith('['):
        require(isinstance(value, list), f'{field}: array required')
        lo, hi = data['arrays'].get(field, data['arrays']['default'])
        require(lo <= len(value) <= hi, f'{field}: array bounds')
        require(len({compact(v) for v in value}) == len(value), f'{field}: duplicates')
        for item in value:
            check_type(item, kind[1:-1], data, field)
        if field == 'members':
            for key in ('issue', 'implemented'):
                require(len({m[key] for m in value}) == len(value), f'members: duplicate {key}')
    elif '|' in kind:
        require(value in kind.split('|'), f'{field}: enum {kind}')
    elif kind in data['types']:
        check_object(value, data['types'][kind], data)
        if kind == 'patch':
            require(bool(value), 'patch: empty')
    elif kind == 'bool':
        require(type(value) is bool, f'{field}: boolean required')
    elif kind in ('time', 'positive'):
        require(type(value) is int, f'{field}: integer required')
        lo, hi = (0, 4294967295) if kind == 'time' else (1, 2147483647)
        require(lo <= value <= hi, f'{field}: integer bounds')
    elif kind in ('id', 'pubkey', 'git'):
        require(matches('[0-9a-f]{40}' if kind == 'git' else '[0-9a-f]{64}', value), f'{field}: {kind}')
    elif kind in ('text', 'longtext', 'title', 'note'):
        require(isinstance(value, str), f'{field}: string required')
        lo = 0 if kind == 'note' else 1
        hi = {'text': 2048, 'longtext': 8192, 'title': 256, 'note': 2048}[kind]
        require(lo <= len(value.encode('utf-8')) <= hi, f'{field}: text bounds')
    elif kind == 'url':
        require(isinstance(value, str) and len(value.encode()) <= 2048, f'{field}: URL bounds')
        url = urlsplit(value)
        require(url.scheme == 'https' and url.hostname and not url.username
                and not url.password and not url.fragment, f'{field}: HTTPS URL')
    elif kind == 'repo':
        require(matches('30617:[0-9a-f]{64}:[a-z0-9][a-z0-9_-]{0,63}', value), f'{field}: repository')
    elif kind == 'stream':
        require(matches('[a-z0-9][a-z0-9._/-]{0,127}', value), f'{field}: stream')
        require('..' not in value and '@{' not in value and not value.endswith(('/', '.'))
                and all(p and not p.startswith('.') and not p.endswith('.lock') for p in value.split('/')),
                f'{field}: invalid branch')
    elif kind == 'release':
        require(matches('[a-z0-9][a-z0-9._+-]{0,63}', value), f'{field}: release label')
    elif kind == 'runid':
        require(matches('[A-Za-z0-9._:-]{1,128}', value), f'{field}: run ID')
    elif kind == 'path':
        require(isinstance(value, str) and 1 <= len(value.encode()) <= 256
                and all(p not in ('', '.', '..') for p in value.split('/'))
                and '\\' not in value, f'{field}: relative path')
    else:
        raise ValueError(f'unknown schema type: {kind}')


def check_object(value, schema, data):
    require(isinstance(value, dict), 'content/object must be a JSON object')
    required, optional = schema['required'], schema['optional']
    require(set(required) <= set(value) <= set(required) | set(optional), 'closed object keys')
    for field, item in value.items():
        check_type(item, (required | optional)[field], data, field)


def check_shape(event, data):
    """Check BW record/artifact shapes; unrelated NIP inputs keep their own rules."""
    require(len(compact(event)) <= 65536, 'event size')
    require(len(event['content'].encode()) <= 32768, 'content size')
    tags = event['tags']
    require(len(tags) <= 16, 'tag count')
    require(all(isinstance(t, list) and t and all(isinstance(s, str) and len(s.encode()) <= 2048 for s in t)
                for t in tags), 'tag arrays')
    if event['kind'] not in (46100, 1063):
        return
    names = [tag[0] for tag in tags]
    require(len(set(names)) == len(names), 'duplicate tag')
    tagmap = {tag[0]: tag for tag in tags}
    if event['kind'] == 46100:
        require('record' in tagmap and len(tagmap['record']) == 2, 'record tag')
        typ = tagmap['record'][1]
        require(typ in RECORDS, 'unknown record')
        schema = data['schemas'][typ]
        content = json.loads(event['content'], object_pairs_hook=unique_object)
        required = {'record', 'a'} | set(schema['tags'])
        if typ != 'role-policy' or 'previous' in tagmap:
            required.add('policy')
        allowed = required | set(schema['optional_tags']) | {'auth'}
        require(required <= set(names) <= allowed, 'tag allowlist/cardinality')
        check_object(content, schema, data)
    else:
        required = {'a', 'set', 'run', 'platform', 'release', 'url', 'm', 'x', 'size'}
        require(required <= set(names) <= required | {'auth'}, 'artifact tags')
        check_type(event['content'], 'text', data)
    for name, tag in tagmap.items():
        require(len(tag) == (4 if name == 'auth' else 2), f'{name}: arity')
        if name == 'auth':
            check_type(tag[1], 'pubkey', data)
            require(len(tag[2].encode()) <= 256 and matches('[0-9a-f]{128}', tag[3]), 'auth shape')
        elif name in ('issue', 'policy', 'previous', 'delegation', 'set', 'run', 'x'):
            check_type(tag[1], 'id', data)
        elif name == 'a':
            check_type(tag[1], 'repo', data)
        elif name == 'size':
            require(matches('[1-9][0-9]*', tag[1]) and int(tag[1]) <= 2147483647, 'artifact size')
        elif name == 'm':
            require(matches('[a-z0-9.+-]+/[a-z0-9.+-]+', tag[1]), 'MIME type')
        elif name in ('url', 'release'):
            check_type(tag[1], name, data)
        elif name == 'platform':
            require(tag[1] in ('windows', 'android', 'ios', 'macos'), 'artifact platform')


def validate(data):
    """Validate integrity, shape, explicit scenario inputs and minimum coverage."""
    require(set(data) == {'format', 'workflow_kind', 'provenance', 'schemas', 'types',
                          'arrays', 'events', 'cases'}, 'top-level keys')
    require(data['format'] == 'nip-bw-fixtures-v1' and data['workflow_kind'] == 46100, 'format/kind')
    require(data['provenance']['test_only'] is True, 'test-only identities required')
    require(set(data['schemas']) == RECORDS, 'record coverage in schema')
    for record in data['schemas'].values():
        require(set(record) == {'required', 'optional', 'tags', 'optional_tags'}, 'schema keys')
        require(not set(record['required']) & set(record['optional']), 'schema overlap')
    events = data['events']
    for label, item in events.items():
        require(set(item) == {'event', 'crypto', 'shape'}, f'{label}: fixture event keys')
        e = item['event']
        require(set(e) == {'id', 'pubkey', 'created_at', 'kind', 'tags', 'content', 'sig'}, f'{label}: event keys')
        for key in ('id', 'pubkey'):
            check_type(e[key], key, data)
        require(matches('[0-9a-f]{128}', e['sig']), f'{label}: signature shape')
        check_type(e['created_at'], 'time', data)
        require(type(e['kind']) is int and 0 <= e['kind'] <= 65535, f'{label}: kind')
        digest = hashlib.sha256(compact([0, e['pubkey'], e['created_at'], e['kind'], e['tags'], e['content']])).hexdigest()
        require(set(item['crypto']) == {'id', 'signature'} and all(type(v) is bool for v in item['crypto'].values()), f'{label}: crypto expectations')
        require((digest == e['id']) == item['crypto']['id'], f'{label}: ID expectation')
        require(type(item['shape']) is bool, f'{label}: shape expectation')
        try:
            check_shape(e, data)
            valid_shape = True
        except (ValueError, TypeError, KeyError):
            valid_shape = False
        require(valid_shape == item['shape'], f'{label}: shape expectation mismatch')
    names, coverage, used = set(), set(), set()
    permutation_projections = {}
    for case in data['cases']:
        require(set(case) == {'name', 'coverage', 'mode', 'now', 'trust', 'external', 'input', 'expected'}, 'case keys')
        require(case['name'] not in names, 'duplicate case name')
        names.add(case['name'])
        permutation_key = compact([sorted(case['input']), case['now'], case['trust'], case['external']])
        projection = case['expected'][-1]['projection'] if case['expected'] else None
        if permutation_key in permutation_projections:
            require(permutation_projections[permutation_key] == projection, 'permutation end projections differ')
        permutation_projections[permutation_key] = projection
        coverage.update(case['coverage'])
        require(case['mode'] in ('crypto', 'semantic'), 'case mode')
        check_type(case['now'], 'time', data)
        require(set(case['trust']) == {'community', 'repo', 'owner'}, 'trust keys')
        check_type(case['trust']['community'], 'url', data)
        check_type(case['trust']['repo'], 'repo', data)
        check_type(case['trust']['owner'], 'pubkey', data)
        require(set(case['external']) == {'git_readbacks', 'git_ancestry', 'provider_readbacks', 'downloads', 'host_authorization'}, 'external inputs')
        for row in case['external']['git_readbacks']:
            check_type(row, 'readback', data)
        for row in case['external']['git_ancestry']:
            require(set(row) == {'ancestor', 'descendant', 'is_ancestor'}, 'ancestry keys')
            check_type(row['ancestor'], 'git', data)
            check_type(row['descendant'], 'git', data)
            check_type(row['is_ancestor'], 'bool', data)
        for row in case['external']['provider_readbacks']:
            require('idempotency_key' in row, 'provider idempotency key')
            check_type(row['idempotency_key'], 'id', data)
            check_object({k: v for k, v in row.items() if k != 'idempotency_key'},
                         data['schemas']['build-run'], data)
        host = case['external']['host_authorization']
        require(set(host) == {'allowed', 'current_policy'}, 'Host authorization keys')
        check_type(host['allowed'], 'bool', data)
        check_type(host['current_policy'], 'id', data)
        require(case['input'] and len(case['input']) == len(case['expected']), 'per-step expectations')
        resolved_assertions = set()
        for label, expected in zip(case['input'], case['expected']):
            require(label in events, f'missing corpus event: {label}')
            used.add(label)
            require(set(expected) == {'outcome', 'stage', 'code', 'projection'}, 'expectation keys')
            require(expected['outcome'] in OUTCOMES and expected['stage'] in STAGES, 'expectation outcome/stage')
            require(isinstance(expected['code'], str) and expected['code'] and isinstance(expected['projection'], dict), 'expectation details')
            # Oracle consistency only: do not infer a human verdict from event claims.
            issues = expected['projection'].get('issues', {})
            require(isinstance(issues, dict), 'issues projection must be an object')
            for issue_id, state in issues.items():
                check_type(issue_id, 'id', data)
                require(state in {'triage', 'backlog', 'ready', 'in-development',
                                  'implemented', 'resolved', 'rework', 'closed'}, 'issue projection state')
                require(issue_id not in resolved_assertions or state == 'resolved',
                        'resolved projection regressed')
                if state == 'resolved':
                    resolved_assertions.add(issue_id)
            if expected['outcome'] in ('accept', 'replay'):
                require(all(events[label]['crypto'].values()) and events[label]['shape'], f'{label}: accepted invalid input')
        for download in case['external']['downloads']:
            require(set(download) == {'url', 'bytes_hex', 'immutable', 'tailnet', 'ephemeral'}, 'download keys')
            check_type(download['url'], 'url', data)
            for flag in ('immutable', 'tailnet', 'ephemeral'):
                check_type(download[flag], 'bool', data)
            require(matches('(?:[0-9a-f]{2})*', download['bytes_hex']), 'download bytes')
    required = COVERAGE | {f'{r}-{polarity}' for r in RECORDS | {'artifact'} for polarity in ('positive', 'negative')}
    require(required <= coverage, f'missing coverage: {sorted(required - coverage)}')
    require(set(events) <= used, f'unused events: {sorted(set(events) - used)}')
    return len(events), len(names)


def main():
    try:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument('path', nargs='?', type=Path, default=DEFAULT)
        parser.add_argument('--bip340-reference', type=Path)
        args = parser.parse_args()
        path, data = args.path, load(args.path)
        count, cases = validate(data)
        if args.bip340_reference:
            reference = args.bip340_reference
            require(hashlib.sha256(reference.read_bytes()).hexdigest() ==
                    '4b1d4ad9e60820df4a6d239733d52b014bc03e4c47afd757a2fe4d3b586d892c',
                    'BIP-340 reference digest mismatch')
            spec = importlib.util.spec_from_file_location('bw_bip340_reference', reference)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            for label, item in data['events'].items():
                event = item['event']
                actual = module.schnorr_verify(bytes.fromhex(event['id']),
                                                bytes.fromhex(event['pubkey']),
                                                bytes.fromhex(event['sig']))
                require(actual == item['crypto']['signature'], f'{label}: Schnorr expectation')
            print(f'PASS: BIP-340 reference verification of {count} signature expectations')
        print(f'PASS: {count} events, {cases} cases; structure and ID bytes, no BW semantic parser')
        if not args.bip340_reference:
            print('Schnorr verification not requested')
        print('Fixture SHA-256:', hashlib.sha256(path.read_bytes()).hexdigest())
    except (ValueError, TypeError, KeyError, OSError) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
