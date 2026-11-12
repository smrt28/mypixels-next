"""Headless named-session launcher regression: python3 tests/persistent_ipc.py.

A private mock session owns the actual flock/socket contract. No display needed.
"""
import fcntl
import os
from pathlib import Path
import socket
import subprocess
import time
import uuid

binary = Path(os.environ.get('MYPIXELS_BIN', 'target/debug/mypixels')).resolve()
env = dict(os.environ)
env.pop('DISPLAY', None)
env.pop('WAYLAND_DISPLAY', None)
for args in (['-S'], ['-S', ''], ['-S', 'x' * 33], ['-S', 'a', '-S', 'b'], ['-S', 'a', '-s']):
    result = subprocess.run([str(binary), *args], env=env, capture_output=True, timeout=5)
    assert result.returncode == 2, (args, result.stderr)

name = 'ipc-' + uuid.uuid4().hex[:20]
directory = Path(f'/tmp/mypixels-sessions-{os.geteuid()}')
directory.mkdir(mode=0o700, exist_ok=True)
key = name.encode().hex()
path = directory / (key + '.sock')
lockpath = directory / (key + '.lock')
try:
    with lockpath.open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        # Launch before bind to exercise concurrent creation/startup retries.
        clients = [subprocess.Popen([str(binary), '-S', name, 'command-must-not-run'],
                    env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(3)]
        time.sleep(.15)
        with socket.socket(socket.AF_UNIX) as server:
            server.bind(str(path))
            server.listen()
            server.settimeout(5)
            for _ in clients:
                connection, _ = server.accept()
                with connection:
                    connection.sendall(b'\x01')
            for client in clients:
                stdout, stderr = client.communicate(timeout=5)
                assert client.returncode == 0, stderr
        # A lock owner without a functioning listener must report failure,
        # rather than starting a competing session or a fresh terminal.
        with socket.socket(socket.AF_UNIX) as server:
            path.unlink()
            server.bind(str(path))
            server.listen()
            server.settimeout(5)
            client = subprocess.Popen([str(binary), '-S', name], env=env,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            connection, _ = server.accept()
            connection.close()
            _, stderr = client.communicate(timeout=5)
            assert client.returncode == 2, stderr
finally:
    path.unlink(missing_ok=True)
    lockpath.unlink(missing_ok=True)
print('PASS: invalid arguments, concurrent attach during startup, no command respawn, broken session failure')
