#!/usr/bin/env python3
"""Serve the local PoR dashboard and bounded, read-only event-stream pages.
No dependencies. Does not launch tests or validators. Binds to loopback only.
"""
import argparse
import functools
import json
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

SCENARIOS = frozenset(('honest', 'unequal', 'isolated', 'correlated'))

class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

    def do_GET(self):
        parsed = urlsplit(self.path)
        if parsed.path != '/api/por-events':
            return super().do_GET()
        query = parse_qs(parsed.query)
        try:
            scenario = query.get('scenario', ['honest'])[0]
            if scenario not in SCENARIOS:
                raise ValueError('Unknown scenario')
            cursor = int(query.get('cursor', ['0'])[0])
            if cursor < 0:
                raise ValueError('Invalid cursor')
            path = self.server.event_root / (scenario + '.events.jsonl')
            if not path.is_file():
                return self.respond({'events': [], 'cursor': 0, 'run_id': None, 'waiting': True})
            events = []
            # Keep one descriptor per request; no polling loop holds up another client.
            with path.open('rb') as stream:
                first = stream.readline(1024 * 1024)
                if not first.endswith(b'\n'):
                    return self.respond({'events': [], 'cursor': 0, 'run_id': None, 'waiting': True})
                header = json.loads(first)
                run_id = header['run_id']
                requested = query.get('run_id', [''])[0]
                reset = requested != run_id or cursor > path.stat().st_size
                if reset:
                    cursor = 0
                stream.seek(cursor)
                for _ in range(256):
                    position = stream.tell()
                    line = stream.readline(1024 * 1024)
                    if not line or not line.endswith(b'\n'):
                        stream.seek(position)
                        break  # The writer may still be appending this record.
                    record = json.loads(line)
                    if record.get('run_id') != run_id:
                        raise ValueError('Stream changed while reading; reconnect')
                    events.append(record)
                cursor = stream.tell()
            return self.respond({'events': events, 'cursor': cursor, 'run_id': run_id, 'reset': reset, 'waiting': not events})
        except (ValueError, KeyError, OSError) as error:
            return self.respond({'error': str(error)}, 400)

    def respond(self, value, status=200):
        body = json.dumps(value).encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        if not self.path.startswith('/api/por-events'):
            super().log_message(fmt, *args)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--events', type=Path, default=Path(__file__).resolve().parents[2] / 'docs' / 'demos' / 'traces')
    parser.add_argument('--port', type=int, default=8765)
    args = parser.parse_args()
    docs = Path(__file__).resolve().parents[2] / 'docs'
    server = ThreadingHTTPServer(('127.0.0.1', args.port), functools.partial(Handler, directory=str(docs)))
    server.event_root = args.events.resolve()
    print(f'PoR event cinema: http://localhost:{args.port}/demos/por-weighted-slashing.html#animation', flush=True)
    print(f'Reading events from {server.event_root}. No tests are launched.', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()

if __name__ == '__main__':
    main()