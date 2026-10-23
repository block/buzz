import copy
import json
import time
from pathlib import Path
import sys
import selectors
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid

import bridge


class BridgeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cfg = {'relay': 'wss://example.test', 'buzz_command': ['buzz'],
                    'codex_command': ['codex'], 'routes': [{
                        'channel': str(uuid.uuid4()), 'thread': str(uuid.uuid4()),
                        'authors': ['a' * 64], 'worktrees': [str(self.root)]}]}
        self.route = self.cfg['routes'][0]
        self.path = self.root / 'state.sqlite'
        self.event = {'id': 'b' * 64, 'pubkey': 'a' * 64, 'kind': 9,
                      'tags': [['h', self.route['channel']]], 'content': 'complete'}
        self.db = bridge.open_db(self.path, self.cfg)
        self.addCleanup(self.db.close)

    def test_replay_after_restart_delivers_only_once(self):
        bridge.ingest(self.db, self.route, self.event)
        with patch('bridge.run_queue') as run:
            run.return_value = 0
            bridge.deliver(self.db, self.cfg)
            self.assertEqual(run.call_count, 1)
            args = run.call_args.args[0]
            self.assertEqual(args[:4], ['codex', 'queue', '--thread', self.route['thread']])
            self.assertIn('external data', args[-1])
            self.assertIn(self.event['id'], args[-1])
            reopened = bridge.open_db(self.path, self.cfg)
            try:
                bridge.ingest(reopened, self.route, self.event)
                bridge.deliver(reopened, self.cfg)
                self.assertEqual(run.call_count, 1)
            finally:
                reopened.close()

    def test_oversized_report_is_journaled_and_does_not_block_replay_or_summary(self):
        with patch('bridge.run_queue') as run:
            run.return_value = 0
            oversized = dict(self.event, content='x' * 60001)
            bridge.ingest(self.db, self.route, oversized)
            bridge.deliver(self.db, self.cfg)
            run.assert_not_called()
            self.assertEqual(self.db.execute('SELECT id FROM rejected').fetchone()[0], oversized['id'])
            reopened = bridge.open_db(self.path, self.cfg)
            try:
                bridge.ingest(reopened, self.route, oversized)
                summary = dict(self.event, id='c' * 64, content='short summary')
                bridge.ingest(reopened, self.route, summary)
                bridge.deliver(reopened, self.cfg)
                self.assertEqual(run.call_count, 1)
                self.assertIn('short summary', run.call_args.args[0][-1])
                self.assertEqual(reopened.execute('SELECT count(*) FROM rejected').fetchone()[0], 1)
            finally:
                reopened.close()

    def test_ambiguous_submission_is_not_retried(self):
        bridge.ingest(self.db, self.route, self.event)
        with patch('bridge.run_queue', side_effect=TimeoutError('queue timed out')):
            with self.assertRaises(TimeoutError):
                bridge.deliver(self.db, self.cfg)
        with self.assertRaisesRegex(RuntimeError, 'ambiguous'):
            bridge.open_db(self.path, self.cfg)

    def test_rejects_unregistered_author_and_channel(self):
        for changed in [{'pubkey': 'c' * 64}, {'tags': [['h', str(uuid.uuid4())]]}, {'kind': 20001}]:
            with self.assertRaises(ValueError):
                bridge.ingest(self.db, self.route, dict(self.event, **changed))
        self.assertEqual(self.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 0)

    def test_changed_config_cannot_redirect_pending_reports(self):
        cfg = copy.deepcopy(self.cfg)
        cfg['routes'][0]['thread'] = str(uuid.uuid4())
        with self.assertRaisesRegex(ValueError, 'configuration changed'):
            bridge.open_db(self.path, cfg)

    def test_multiple_worktrees_per_task_but_no_shared_worktree(self):
        second = self.root / 'other-repo'
        second.mkdir()
        self.route['worktrees'].append(str(second))
        path = self.root / 'routes.json'
        path.write_text(json.dumps(self.cfg))
        self.assertEqual(len(bridge.load_config(path)['routes'][0]['worktrees']), 2)
        duplicate = dict(self.route, channel=str(uuid.uuid4()), thread=str(uuid.uuid4()))
        self.cfg['routes'].append(duplicate)
        path.write_text(json.dumps(self.cfg))
        with self.assertRaisesRegex(ValueError, 'must not share'):
            bridge.load_config(path)

    def test_real_queue_subprocess_gets_literal_message_without_shell(self):
        capture = self.root / 'argv.json'
        script = self.root / 'queue.py'
        script.write_text('import json,sys\nfrom pathlib import Path\nPath(sys.argv[1]).write_text(json.dumps(sys.argv[2:]))\n')
        cfg = copy.deepcopy(self.cfg)
        cfg['codex_command'] = [sys.executable, str(script), str(capture)]
        self.event['content'] = '$(touch should-not-exist); `echo nope`'
        bridge.ingest(self.db, self.route, self.event)
        bridge.deliver(self.db, cfg)
        args = json.loads(capture.read_text())
        self.assertEqual(args[0], 'queue')
        self.assertIn(self.event['content'], args[-1])
        self.assertEqual(self.db.execute('SELECT state FROM deliveries').fetchone()[0], 'sent')

    def process_tree_command(self):
        child = self.root / 'child.py'
        child.write_text('import os,signal,time\nfrom pathlib import Path\n'
                         'signal.signal(signal.SIGTERM, signal.SIG_IGN)\n'
                         'Path(' + repr(str(self.root / 'child.pid')) + ').write_text(str(os.getpid()))\n'
                         'time.sleep(60)\n')
        wrapper = self.root / 'wrapper.py'
        wrapper.write_text('import subprocess,sys,time\n'
                           'subprocess.Popen([sys.executable, ' + repr(str(child)) + '])\n'
                           'time.sleep(60)\n')
        return [sys.executable, str(wrapper)]

    def assert_child_stopped(self):
        pid = int((self.root / 'child.pid').read_text())
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            status = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True)
            if status.returncode != 0 or status.stdout.strip().startswith('Z'):
                return
            time.sleep(0.02)
        self.fail('wrapper descendant is still running')

    def test_stop_kills_wrapper_descendant_that_ignores_termination(self):
        process = subprocess.Popen(self.process_tree_command(), stdout=subprocess.PIPE, start_new_session=True)
        try:
            deadline = time.monotonic() + 5
            while not (self.root / 'child.pid').exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertTrue((self.root / 'child.pid').exists())
            bridge.stop(process)
            self.assert_child_stopped()
        finally:
            if process.poll() is None:
                bridge.stop(process)

    def test_queue_timeout_kills_descendants(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            bridge.run_queue(self.process_tree_command(), timeout=2)
        self.assert_child_stopped()

    def test_foreground_bridge_streams_and_stops_its_watcher(self):
        watcher = self.root / 'watcher.py'
        watcher.write_text('import json,time\n' +
                          'event = ' + repr(self.event) + '\n' +
                          'print(json.dumps({"type":"event","event":event}), flush=True)\n' +
                          'print(json.dumps({"type":"ready"}), flush=True)\n' +
                          'time.sleep(30)\n')
        cfg = copy.deepcopy(self.cfg)
        cfg['buzz_command'] = [sys.executable, str(watcher)]
        cfg['codex_command'] = [sys.executable, '-c', 'pass']
        config = self.root / 'integration.json'
        config.write_text(json.dumps(cfg))
        process = subprocess.Popen([sys.executable, str(Path(bridge.__file__)),
                                    '--config', str(config), '--state', str(self.root / 'integration.sqlite')],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(timeout=10), 'report was not delivered')
            record = json.loads(process.stdout.readline())
            self.assertEqual(record['delivered'], self.event['id'])
            process.send_signal(signal.SIGINT)
            _, errors = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, errors.decode())
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()
            process.stderr.close()


if __name__ == '__main__':
    unittest.main()
