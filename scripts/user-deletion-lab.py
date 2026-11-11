#!/usr/bin/env python3
"""Prepare a disposable local deletion lab. Default action only prints the plan.

Setup creates data, including signed relay events; it never invokes a deletion
handler. Review docs/user-deletion-lab.md before running any action.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
STATE = ROOT / '.local/user-deletion'
TARGET = '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798'
SURVIVOR = 'c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5'
RELAY = 'f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9'
URL = 'http://localhost:58300'
COMPOSE = ['docker', 'compose', '-p', 'buzz-deletion-lab', '-f',
           str(ROOT / 'docker-compose.user-deletion.yml')]


def environment():
    # Fixed service identities prevent accidentally inheriting a real database,
    # S3 bucket, replica, or relay from the caller's shell.
    env = {k: v for k, v in os.environ.items() if not k.startswith(
        ('BUZZ_', 'PG', 'AWS_', 'REDIS_', 'DATABASE_', 'READ_DATABASE_', 'RELAY_'))}
    env.update({
        'DATABASE_URL': 'postgres://buzz:buzz_lab@127.0.0.1:57431/buzz_deletion_lab',
        'BUZZ_POSTGRES_ADMIN_URL': 'postgres://buzz:buzz_lab@127.0.0.1:57431/postgres',
        'PGHOST': '127.0.0.1', 'PGPORT': '57431', 'PGUSER': 'buzz',
        'PGPASSWORD': 'buzz_lab', 'PGDATABASE': 'buzz_deletion_lab',
        'PGOPTIONS': '-c timezone=UTC', 'PGCONNECT_TIMEOUT': '5',
        'PGSCHEMA_PLAN_HOST': '127.0.0.1', 'PGSCHEMA_PLAN_PORT': '57431',
        'PGSCHEMA_PLAN_DB': 'buzz_deletion_lab', 'PGSCHEMA_PLAN_USER': 'buzz',
        'PGSCHEMA_PLAN_PASSWORD': 'buzz_lab',
        'REDIS_URL': 'redis://127.0.0.1:57432',
        'RELAY_URL': 'ws://localhost:58300',
        'BUZZ_S3_ENDPOINT': 'http://127.0.0.1:59010',
        'BUZZ_S3_BUCKET': 'buzz-deletion-lab', 'BUZZ_S3_REGION': 'us-east-1',
        'BUZZ_S3_ACCESS_KEY': 'buzz_lab', 'BUZZ_S3_SECRET_KEY': 'buzz_lab_password',
        'BUZZ_S3_ADDRESSING_STYLE': 'path',
        'AWS_ACCESS_KEY_ID': 'buzz_lab', 'AWS_SECRET_ACCESS_KEY': 'buzz_lab_password',
        'AWS_DEFAULT_REGION': 'us-east-1', 'AWS_EC2_METADATA_DISABLED': 'true',
        'AWS_PAGER': '', 'BUZZ_BIND_ADDR': '127.0.0.1:58300',
        'BUZZ_HEALTH_PORT': '58301', 'BUZZ_METRICS_PORT': '58302',
        'BUZZ_AUTO_MIGRATE': 'false', 'BUZZ_PUSH_ENABLED': 'false',
        'BUZZ_RELAY_PRIVATE_KEY': f'{3:064x}', 'RELAY_OWNER_PUBKEY': SURVIVOR,
        'BUZZ_REQUIRE_RELAY_MEMBERSHIP': 'true',
        'BUZZ_MEDIA_BASE_URL': URL + '/media',
        'TARGET_SECRET_KEY': f'{1:064x}', 'SURVIVOR_SECRET_KEY': f'{2:064x}',
        'RUST_LOG': 'warn',
    })
    env['BUZZ_MEDIA_SOURCE_DATABASE_URL'] = env['DATABASE_URL']
    return env


ENV = environment()


def run(args, *, input_text=None, log=None, expected=0):
    result = subprocess.run([str(a) for a in args], cwd=ROOT, env=ENV,
                            input=input_text, text=True, capture_output=True, timeout=300)
    if log:
        # Keep every attempt; a retry must not overwrite earlier evidence.
        path = STATE / f'{time.time_ns()}-{log}'
        path.write_text(result.stdout + '\n' + result.stderr)
        print(path.relative_to(ROOT), flush=True)
    if result.returncode != expected:
        raise RuntimeError(f'{Path(str(args[0])).name} exited {result.returncode}, '
                           f'expected {expected}: {result.stderr[-3000:]}')
    return result.stdout


def sql(query):
    return run([ENV.get('PSQL', 'psql'), '-X', '-qAt', '-v', 'ON_ERROR_STOP=1'],
               input_text=query).strip()


def fixture(kind, action, *extra):
    return run([sys.executable, ROOT / f'scripts/user-deletion-{kind}-fixture.py',
                action, '--manifest', STATE / f'{kind}.json', *extra],
               log=f'{kind}-{action}.log')


def s3(operation, *extra):
    return run([ENV.get('AWS', 'aws'), '--endpoint-url', ENV['BUZZ_S3_ENDPOINT'],
                's3api', operation, '--bucket', ENV['BUZZ_S3_BUCKET'], *extra])


def admin(*args, log=None, expected=0):
    return run([ROOT / 'target/debug/buzz-admin', 'user-deletions', *args],
               log=log, expected=expected)


def live(action, phase='before'):
    return run([ROOT / 'target/debug/examples/user_deletion_live_seed', action,
                '--relay-url', URL, '--manifest', STATE / 'live.json',
                '--phase', phase], log=f'live-{action}-{phase}.log')


def save_new(path, value):
    with path.open('x') as output:
        json.dump(value, output, indent=2, sort_keys=True)
        output.write('\n')


def setup():
    for name in ('docker', ENV.get('PSQL', 'psql'), ENV.get('AWS', 'aws')):
        if not shutil.which(name):
            raise RuntimeError(f'{name} is required; see the lab documentation')
    for name in ('buzz-admin', 'buzz-relay', 'examples/user_deletion_live_seed'):
        if not (ROOT / 'target/debug' / name).is_file():
            raise RuntimeError(f'Build target/debug/{name} first; see the lab documentation')
    if (STATE / 'lab.json').exists():
        raise RuntimeError('lab.json exists; inspect the existing run instead of reseeding')
    for port in (58300, 58301, 58302):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', port))
    STATE.mkdir(parents=True, exist_ok=True)
    communities = sorted([str(uuid.uuid4()), str(uuid.uuid4())])
    lab = {'seed_complete': False, 'target': TARGET, 'survivor': SURVIVOR,
           'relay_pubkey': RELAY, 'communities': communities, 'relay_url': URL}
    save_new(STATE / 'lab.json', lab)
    # Reserved manifest makes an interrupted run visible and prevents retries
    # from silently duplicating only some of the fixtures.
    run(COMPOSE + ['up', '-d', '--wait'], log='services.log')
    if sql("SELECT count(*) FROM information_schema.tables WHERE table_schema='public' AND table_name='communities'") == '1':
        if sql('SELECT count(*) FROM communities') != '0':
            raise RuntimeError('lab database already contains communities; refusing to seed')
    run([ROOT / 'bin/pgschema', 'apply', '--file', 'schema/schema.sql', '--auto-approve'],
        log='schema.log')
    run([ENV.get('PSQL', 'psql'), '-X', '-v', 'ON_ERROR_STOP=1', '-f',
         'scripts/reconcile-schema-after-pgschema.sql'], log='reconcile.log')
    for attempt in range(30):
        try:
            buckets = json.loads(run([ENV.get('AWS', 'aws'), '--endpoint-url',
                ENV['BUZZ_S3_ENDPOINT'], '--cli-connect-timeout', '1',
                '--cli-read-timeout', '1', 's3api', 'list-buckets']))
            break
        except RuntimeError:
            if attempt == 29:
                raise
            time.sleep(1)
    if ENV['BUZZ_S3_BUCKET'] not in [b['Name'] for b in buckets['Buckets']]:
        s3('create-bucket')
    listing = json.loads(s3('list-object-versions'))
    if listing.get('Versions') or listing.get('DeleteMarkers'):
        raise RuntimeError('lab bucket is not empty; refusing to mix fixture runs')
    s3('put-bucket-versioning', '--versioning-configuration', 'Status=Enabled')
    statements = ['BEGIN;']
    for community, host in zip(communities, ('localhost:58300', '127.0.0.1:58300')):
        statements.append(f"INSERT INTO communities(id,host) VALUES('{community}','{host}');")
        for actor, name, role in ((TARGET, 'Departing fixture user', 'member'),
                                  (SURVIVOR, 'Surviving fixture user', 'owner')):
            statements.append(f"INSERT INTO users(community_id,pubkey,display_name) VALUES('{community}',decode('{actor}','hex'),'{name}');")
            statements.append(f"INSERT INTO relay_members(community_id,pubkey,role) VALUES('{community}','{actor}','{role}');")
    sql('\n'.join(statements + ['COMMIT;']))
    flags = ['--target', TARGET, '--survivor', SURVIVOR, '--community', communities[0],
             '--other-community', communities[1]]
    for kind in ('content', 'media', 'account'):
        fixture(kind, 'seed', *flags)
    # Export only fixture settings, never the inherited shell or real secrets.
    settings = {k: v for k, v in ENV.items() if k.startswith(
        ('BUZZ_', 'PG', 'AWS_', 'REDIS_', 'DATABASE_', 'RELAY_', 'TARGET_', 'SURVIVOR_'))}
    (STATE / 'env').write_text(''.join(f'export {k}={shlex.quote(v)}\n' for k,v in settings.items()))
    (STATE / 'env').chmod(0o600)
    with (STATE / 'relay.log').open('a') as output:
        relay = subprocess.Popen([ROOT / 'target/debug/buzz-relay'], cwd=ROOT,
                                 env=ENV, stdout=output, stderr=output)
        try:
            for _ in range(100):
                if relay.poll() is not None:
                    raise RuntimeError('lab relay exited; inspect relay.log')
                try:
                    with urllib.request.urlopen('http://127.0.0.1:58301/_readiness', timeout=1) as response:
                        if response.status == 200:
                            break
                except OSError:
                    time.sleep(0.2)
            else:
                raise RuntimeError('lab relay did not become healthy')
            live('seed')
            live('verify')
        finally:
            # This is the child we started, never a PID taken from a stale file.
            if relay.poll() is None:
                relay.send_signal(signal.SIGTERM)
                try:
                    relay.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    relay.kill()
                    relay.wait()
    lab['seed_complete'] = True
    (STATE / 'lab.json').write_text(json.dumps(lab, indent=2) + '\n')
    print('Seed complete. Relay stopped. No deletion request or handler was run.')


def prepare():
    lab = json.loads((STATE / 'lab.json').read_text())
    if not lab['seed_complete']:
        raise RuntimeError('setup did not complete')
    if any((STATE / name).exists() for name in ('request.json', 'rehearsal.json')):
        raise RuntimeError('request manifest already exists; inspect it instead of replacing baselines')
    request = json.loads(admin('submit', '--pubkey', TARGET,
        '--requested-by', 'local-lab', '--reason', 'Disposable local workflow verification',
        log='submit.json'))
    admin('approve', request['id'], '--approved-by', 'local-lab',
          '--inventory-digest', request['inventory_digest'], log='approve.json')
    save_new(STATE / 'request.json', request)
    rehearsal = json.loads(admin('submit', '--pubkey', TARGET, '--read-only',
        '--requested-by', 'local-lab', '--reason', 'Disposable local workflow rehearsal',
        log='submit-rehearsal.json'))
    admin('approve', rehearsal['id'], '--approved-by', 'local-lab',
          '--inventory-digest', rehearsal['inventory_digest'], log='approve-rehearsal.json')
    save_new(STATE / 'rehearsal.json', rehearsal)
    for kind in ('content', 'media', 'account'):
        fixture(kind, 'snapshot')
    print('Separate deletion/rehearsal requests approved and baselines saved. No handler was run.')


def preview():
    request = json.loads((STATE / 'request.json').read_text())
    for stage in ('purge_media', 'purge_content', 'purge_account'):
        admin('run', request['id'], '--read-only', '--stage', stage,
              '--media-base', URL + '/media', '--relay-pubkey', RELAY,
              log=f'preview-{stage}.jsonl')
        for kind in ('content', 'media', 'account'):
            fixture(kind, 'verify', '--phase', 'preview')
    print('All three read-only stages passed their fixture comparisons.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', nargs='?', default='plan',
                        choices=('plan', 'setup', 'prepare', 'preview'))
    args = parser.parse_args()
    if args.action == 'plan':
        print('setup: isolated services/schema, two communities and users, SQL fixtures, signed relay activity; stops relay.')
        print('prepare: submit/approve the synthetic request and save verification baselines.')
        print('preview: run three read-only handlers and compare baselines. No delete action exists in this script.')
        print('See docs/user-deletion-lab.md for prerequisites and separately approved deletion commands.')
    else:
        globals()[args.action]()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
