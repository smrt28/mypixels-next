"""Headless named-session launcher regression: python3 tests/persistent_ipc.py.

A private mock session owns the actual flock/socket contract. No display needed.
"""
import fcntl
import json
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
for args in (['-S'], ['-S', ''], ['-S', 'a', '-S', 'b'], ['-S', 'a', '-s'], ['-A', '-S', 'a'], ['-S', 'a', '-A'], ['-A', '-s']):
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
        statepath = path.with_suffix('.state')
        statepath.write_text(json.dumps(dict(name=name, attached=False, pending=0)))
        before = statepath.read_bytes()
        for options in (['-l'], ['-l', '-A'], ['-l', '-S', 'must-not-create']):
            result = subprocess.run([str(binary), *options], env=env, capture_output=True,
                                    text=True, timeout=5)
            assert result.returncode == 0, result.stderr
            assert f'{name}\tdetached' in result.stdout.splitlines(), result.stdout
            assert statepath.read_bytes() == before, 'listing changed session state'
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
    result = subprocess.run([str(binary), '-l'], env=env, capture_output=True,
                            text=True, timeout=5)
    assert result.returncode == 0, result.stderr
    assert not any(line.startswith(name + '\t') for line in result.stdout.splitlines()), result.stdout
finally:
    path.unlink(missing_ok=True)
    lockpath.unlink(missing_ok=True)
    path.with_suffix('.state').unlink(missing_ok=True)
# A newly created session must forward initialization errors to the launcher,
# release its lifetime lock and leave no background owner behind. No display is
# present, so winit is expected to reject initialization before any terminal starts.
failed_name = 'bg-' + uuid.uuid4().hex[:20]
failed_key = failed_name.encode().hex()
try:
    result = subprocess.run([str(binary), '-S', failed_name], env=env,
                            capture_output=True, text=True, timeout=10)
    assert result.returncode != 0, 'startup failure was reported as success'
    assert 'event loop' in result.stderr.lower(), result.stderr
    with (directory / (failed_key + '.lock')).open('r') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    assert not any(line.startswith(failed_name + '\t') for line in
        subprocess.check_output([str(binary), '-l'], env=env, text=True, timeout=5).splitlines())
finally:
    for suffix in ('.sock', '.state', '.lock'):
        (directory / (failed_key + suffix)).unlink(missing_ok=True)
print('PASS: validation, concurrent attach, listing, startup errors and released background ownership')
