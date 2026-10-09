"""One-shot API client, embedded in the custom TUI; never runs as a daemon."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import socket
import sys
import time

home = Path(pwd.getpwuid(os.getuid()).pw_dir)
session = sys.argv[1]
session_dir = home / '.config/herdr'
if session != 'default':
    session_dir = session_dir / 'sessions' / session
socket_path = Path(sys.argv[2]) if len(sys.argv) > 2 else session_dir / 'herdr.sock'
socket_path = socket_path.resolve()
lock_dir = home / '.local/state/herdr-project'
lock_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
lock_key = hashlib.sha256(str(socket_path).encode()).hexdigest()[:20]


def rpc(method, params):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(10)
        connection.connect(str(socket_path))
        connection.sendall((json.dumps({'id': 'herdr-project:home', 'method': method, 'params': params}) + '\n').encode())
        response = json.loads(connection.makefile('rb').readline())
    if 'error' in response:
        raise RuntimeError(response['error']['message'])
    return response['result']


with (lock_dir / ('home-' + lock_key + '.lock')).open('a') as lock:
    deadline = time.monotonic() + 12
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            if time.monotonic() >= deadline:
                raise TimeoutError('another client is creating the home workspace')
            time.sleep(.1)
    snapshot = rpc('session.snapshot', {})['snapshot']
    if not any(workspace['label'] == '~' for workspace in snapshot['workspaces']):
        rpc('workspace.create', {'cwd': str(home), 'label': '~', 'focus': False})
