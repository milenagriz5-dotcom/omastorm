"""Start an isolated engine, select one source and print one line per state.

    python3 scripts/tooling/probe.py inpe-brazil --seconds 60

Live check for a source adapter: the daemon runs under a scratch
XDG_RUNTIME_DIR (never the installed engine's), its log is printed on exit,
and it is always stopped. Exit status 1 when no complete frame arrived.
"""
from pathlib import Path
import argparse, json, os, re, socket, subprocess, sys, tempfile, time

ROOT = Path(__file__).resolve().parents[2]
USAGE = 'Start an isolated engine, select one source and print one line per state.'


def summary(state):
    frame = state.get('frame') or {}
    timeline = state.get('timeline') or []
    complete = sum(1 for entry in timeline if entry.get('status') == 'complete')
    connection = state.get('connection') or {}
    return (f"{state.get('mode')} {(state.get('selection') or {}).get('sourceId')} "
            f"{connection.get('status')} age={connection.get('ageSeconds')}s "
            f"frame={frame.get('id')} {frame.get('status')} timeline={complete}/{len(timeline)}")


def connect(daemon, path):
    deadline = time.monotonic() + 30
    while True:
        if daemon.poll() is not None: raise SystemExit(f'engine exited {daemon.returncode}')
        client = socket.socket(socket.AF_UNIX)
        try:
            client.connect(str(path)); return client
        except OSError:
            client.close()
            if time.monotonic() > deadline: raise SystemExit('engine did not listen within 30 s')
            time.sleep(0.05)


def main():
    parser = argparse.ArgumentParser(description=USAGE)
    parser.add_argument('source', help='source id from hello, e.g. inpe-brazil')
    parser.add_argument('--seconds', type=float, default=60, help='how long to watch (default 60)')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/omastorm-engine')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='omastorm-probe-', dir='/tmp') as runtime:
        log = Path(runtime) / 'probe.log'
        env = {k: v for k, v in os.environ.items() if k != 'OMASTORM_ARCHIVE'}
        env['XDG_RUNTIME_DIR'] = runtime
        with log.open('w') as out:
            daemon = subprocess.Popen([args.binary, 'serve'], env=env, stdout=out, stderr=out)
        sock_path = Path(runtime) / 'omastorm/engine.sock'
        complete = False
        try:
            client = connect(daemon, sock_path)
            client.settimeout(1)
            start = time.monotonic(); last = None; pending = b''; closed = False
            while time.monotonic() - start < args.seconds and not closed:
                if b'\n' not in pending:
                    try: chunk = client.recv(1 << 16)
                    except socket.timeout: continue
                    if not chunk: print('engine closed the connection'); closed = True
                    pending += chunk
                    continue
                line, pending = pending.split(b'\n', 1)
                message = json.loads(line)
                if message.get('type') == 'hello':
                    ids = [s['id'] for s in message.get('sources', [])]
                    print('sources:', ' '.join(ids))
                    if args.source not in ids: raise SystemExit(f'{args.source} is not a source')
                    client.sendall((json.dumps({'type': 'select_source', 'id': args.source}) + '\n').encode())
                elif message.get('type') == 'state':
                    text = summary(message)
                    complete = complete or (message.get('frame') or {}).get('status') == 'complete'
                    # Age ticks every second; print only when anything else changes.
                    if re.sub(r' age=\S+', '', text) != last:
                        print(f'{time.monotonic() - start:6.1f}s {text}')
                    last = re.sub(r' age=\S+', '', text)
                else:
                    print(f"{time.monotonic() - start:6.1f}s {message.get('type')}")
        finally:
            daemon.terminate()
            try: daemon.wait(5)
            except subprocess.TimeoutExpired: daemon.kill(); daemon.wait()
            print('--- engine log ---'); print(log.read_text(), end='')
    sys.exit(0 if complete else 1)


if __name__ == '__main__': main()
