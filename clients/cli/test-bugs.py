#!/usr/bin/env python3
"""Exercise the executable bug CLI against HTTP, asserting requests and failures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CLI = str(Path(__file__).with_name('takomo'))

class BugsCliTest(unittest.TestCase):
    def setUp(self):
        self.requests = []
        self.status = 200
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def request(self):
                body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                owner.requests.append((self.command, self.path, json.loads(body) if body else None, self.headers.get('Idempotency-Key')))
                self.send_response(owner.status)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps({'id': 'tp-bug', 'code': 'test.refused', 'message': 'Test response'}).encode())
            do_GET = do_POST = do_PATCH = do_PUT = request
            def log_message(self, *args): pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)

    def run_cli(self, *args):
        return subprocess.run(['bash', CLI, 'bug', *args], text=True, capture_output=True, timeout=10,
            env={**os.environ, 'TAKOMO_URL': f'http://127.0.0.1:{self.server.server_port}', 'TAKOMO_TOKEN': 'test-token', 'TAKOMO_PROJECT': 'tp', 'TAKOMO_RETRIES': '0'})

    def test_report_is_one_ticket_no_research_and_preserves_literal_text(self):
        with tempfile.NamedTemporaryFile(mode='w') as file:
            file.write('Expected $5; got `something`\n$(do not execute)'); file.flush()
            for _ in range(2):
                result = self.run_cli('new', 'Wrong total', '--body-file', file.name, '--request-id', 'same-report')
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout)['id'], 'tp-bug')
        self.assertEqual(len(self.requests), 2)
        for method, path, body, key in self.requests:
            self.assertEqual((method, path, key), ('POST', '/v1/tickets', 'same-report'))
            self.assertEqual(body['type'], 'bug')
            self.assertEqual(body['body'], 'Expected $5; got `something`\n$(do not execute)')

    def test_explicit_research_steer_cancel_and_filters(self):
        commands = [('research', 'tp-bug', '--request-id', 'r1'), ('steer', 'aj-1', '--message', 'Check dates', '--request-id', 's1'), ('cancel', 'aj-1'), ('ls', '--severity', 'high', '--offset', '50', '--all')]
        for args in commands:
            result = self.run_cli(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.requests[0][:3], ('POST', '/v1/bugs/tp-bug/research', {'request_id': 'r1', 'message': None}))
        self.assertEqual(self.requests[1][:3], ('POST', '/v1/agent-jobs/aj-1/steer', {'message': 'Check dates', 'request_id': 's1'}))
        self.assertEqual(self.requests[2][:3], ('POST', '/v1/agent-jobs/aj-1/cancel', {}))
        self.assertIn('all=true', self.requests[3][1])
        self.assertIn('offset=50', self.requests[3][1])

    def test_refusal_does_not_claim_success_or_retry(self):
        self.status = 409
        result = self.run_cli('research', 'tp-bug', '--request-id', 'r1')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')
        self.assertIn('test.refused', result.stderr)
        self.assertEqual(len(self.requests), 1)

    def test_invalid_arguments_send_nothing(self):
        result = self.run_cli('ls', '--limit', '0')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests, [])

if __name__ == '__main__': unittest.main()
