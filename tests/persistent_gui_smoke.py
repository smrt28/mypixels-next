"""Named sessions and the upstream windowless daemon on a private compositor.

Run only on an isolated display with MYPIXELS_ISOLATED_SESSION_TEST=1.
Uses the FIFO keyboard harness; no xdotool or desktop session is required.
"""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess as sp
import sys
import tempfile
import time
import uuid

assert os.environ.get('MYPIXELS_ISOLATED_SESSION_TEST') == '1', 'Use an isolated display'
root = Path(__file__).resolve().parents[1]
binary = str(Path(os.environ.get('MYPIXELS_BIN', root / 'target/debug/mypixels')).resolve())
work = Path(tempfile.mkdtemp(prefix='mypixels-persistent-merge-'))
config = work / 'config.yml'
config.write_text('''wal_palette: ''
default_width: 900
default_height: 600
default_maximize: false
workspaces_enabled: true
graveyard_limit: 1
close_window_dialog: always
bindings:
  close_window: CONTROL+SHIFT+w
''')
fixture = work / 'shell'
fixture.write_text('''#!/usr/bin/python3
import json, os, threading, time, tty
from pathlib import Path
work = Path(os.environ['SESSION_TEST_WORK'])
index = 0
while True:
    try:
        fd = os.open(work / f'pid-{index}', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(fd, json.dumps(dict(pid=os.getpid(), app=os.getppid())).encode())
        os.close(fd)
        break
    except FileExistsError: index += 1
tty.setraw(0)
os.write(1, (f'\\x1b]2;terminal-{index}\\x07\\x1b[?25l' +
             ''.join(f'line {n}: terminal {index}\\r\\n' for n in range(300))).encode())
def background():
    sent = False
    while True:
        (work / f'alive-{index}').touch()
        if (work / f'query-{index}').exists() and not sent:
            os.write(1, b'\\x1b[16t')
            sent = True
        time.sleep(.05)
threading.Thread(target=background, daemon=True).start()
while True:
    data = os.read(0, 65536)
    if not data: break
    with (work / f'input-{index}').open('ab') as out: out.write(data)
''')
fixture.chmod(0o755)
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(keys),
           SHELL=str(fixture), SESSION_TEST_WORK=str(work),
           MYPIXELS_BUS='com.github.volca.mypixels.merge_test',
           MYPIXELS_MCP=str(work / 'mcp.sock'))
env.pop('MYPIXELS_WINDOW', None)
if '--x11' in sys.argv:
    env.pop('WAYLAND_DISPLAY', None)
name = 'merge-' + uuid.uuid4().hex[:16]
state = Path(f'/tmp/mypixels-sessions-{os.geteuid()}/{name.encode().hex()}.state')
app_pid = None
daemon = None


def wait(check, message):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if check(): return
        time.sleep(.05)
    raise AssertionError(message)


def launch(*args):
    result = sp.run([binary, *args], env=env, capture_output=True, timeout=40)
    assert result.returncode == 0, result.stderr.decode()


def key(value):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out: out.write(value + '\n')
    time.sleep(.2)


def heartbeat(index):
    path = work / f'alive-{index}'
    before = path.stat().st_mtime_ns
    time.sleep(.15)
    assert path.stat().st_mtime_ns > before, f'terminal {index} stopped'


def windows():
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(5)
        sock.connect(env['MYPIXELS_MCP'])
        sock.sendall(json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                                'params': {'name': 'list_windows'}}).encode() + b'\n')
        reply = json.loads(sock.makefile('rb').readline())
    return json.loads(reply['result']['content'][0]['text'])[0]


try:
    # The launcher returns, but the detached owner and its terminals stay alive.
    launch('-S', name)
    wait(lambda: (work / 'alive-0').exists(), 'first terminal missing')
    app_pid = json.loads((work / 'pid-0').read_text())['app']
    assert os.getsid(app_pid) == app_pid, 'session still belongs to the invoking terminal'
    key('window:1:alt+2')
    wait(lambda: (work / 'alive-1').exists(), 'second tab missing')
    key('window:1:ctrl+shift+n')
    wait(lambda: (work / 'alive-2').exists(), 'second window missing')
    pids = sorted(p.read_text() for p in work.glob('pid-*'))
    key('window:2:ctrl+shift+w')
    assert json.loads(state.read_text())['attached'], 'closing one window detached the session'
    key('window:1:ctrl+shift+w')
    wait(lambda: not json.loads(state.read_text())['attached'], 'named close showed a dialog or failed to detach')
    # More windows than the graveyard limit must survive named-session detach.
    for index in range(3): heartbeat(index)
    (work / 'query-1').touch()
    wait(lambda: (work / 'input-1').exists() and b'\x1b[6;' in (work / 'input-1').read_bytes(),
         'windowless terminal protocol reply missing')
    launch('-S', name)
    assert sorted(p.read_text() for p in work.glob('pid-*')) == pids, 'reattach respawned terminals'
    assert json.loads(state.read_text())['attached']
    key('window:1:z')
    wait(lambda: b'z' in (work / 'input-1').read_bytes(), 'tab selection/order or restored input changed')
    key('window:2:z')
    wait(lambda: (work / 'input-2').exists() and b'z' in (work / 'input-2').read_bytes(), 'restored window order changed')
    if env.get('WAYLAND_DISPLAY') or env.get('GAMESCOPE_WAYLAND_DISPLAY'):
        capture = ['gamescopectl', 'screenshot'] if env.get('GAMESCOPE_WAYLAND_DISPLAY') else ['grim']
        sp.run(capture + [str(work / 'restored.png')], env=env, check=True, timeout=10)
        wait(lambda: (work / 'restored.png').exists(), 'screenshot missing')

    # Stop the named owner before reusing the FIFO for a different app process.
    os.kill(app_pid, signal.SIGTERM)
    wait(lambda: not Path(f'/proc/{app_pid}/fd/0').exists(), 'named owner did not stop')
    app_pid = None

    # The upstream --daemon stays windowless, accepts a window, and restores it.
    never = work / 'daemon.yml'
    never.write_text(config.read_text().replace('close_window_dialog: always', 'close_window_dialog: never'))
    env['MYPIXELS_CONFIG'] = str(never)
    log = (work / 'daemon.log').open('w')
    daemon = sp.Popen([binary, '--daemon'], env=env, stdout=log, stderr=log)
    wait(lambda: (work / 'mcp.sock').exists(), 'daemon did not register')
    assert windows() == [], 'daemon spawned a window at startup'
    launch('--new-tab')
    wait(lambda: len(windows()) == 1 and (work / 'alive-3').exists(), 'daemon window missing')
    before = windows()[0]
    key('window:1:ctrl+shift+w')
    wait(lambda: not windows(), 'daemon last window did not close')
    heartbeat(3)
    launch('--restore-window')
    wait(lambda: len(windows()) == 1, 'daemon window did not restore')
    assert windows()[0]['tabs'] == before['tabs'], 'daemon restore changed tabs'
    print('PASS: background launch, named multi-window detach/restore, live input and protocol replies, daemon restore; artifacts:', work)
finally:
    if app_pid:
        try: os.kill(app_pid, signal.SIGTERM)
        except ProcessLookupError: pass
    if daemon:
        daemon.terminate()
        daemon.wait(timeout=5)
    for path in work.glob('pid-*'):
        try: os.kill(json.loads(path.read_text())['pid'], signal.SIGTERM)
        except ProcessLookupError: pass
    for suffix in ('.sock', '.state', '.lock'):
        state.with_suffix(suffix).unlink(missing_ok=True)
