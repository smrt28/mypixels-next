"""Open real windows with the keyboard shortcut; verify rendering, input, and workspaces.

Run after cargo build on an isolated graphical display:
python3 tests/menu_window_smoke.py [--x11]
The private FIFO injects egui pointer events, not desktop keyboard/mouse input.
Set MYPIXELS_MENU_SCREENSHOT=1 for grim/gamescopectl visual artifacts.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import sys
import tempfile
import time

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-menu-window-'))
config = work / 'config.yml'
config.write_text('''wal_palette: ''
default_width: 900
default_height: 600
default_maximize: false
term_transparency: 0
workspaces_enabled: true
workspace_letter_keys: true
''')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import os, sys, tty
from pathlib import Path
work = Path(sys.argv[1])
index = 0
while True:
    path = work / f'input-{index}'
    try:
        path.touch(exist_ok=False)
        break
    except FileExistsError:
        index += 1
tty.setraw(0)
os.write(1, f'\x1b]2;Terminal {index}\x07Window terminal {index}\r\n'.encode())
while True:
    data = os.read(0, 65536)
    if not data: break
    with path.open('ab') as out: out.write(data)
    os.write(1, b'Received: ' + data + b'\r\n')
''')
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(keys), RUST_LOG='info')
if '--x11' in sys.argv:
    env.pop('WAYLAND_DISPLAY', None)
    env.pop('WAYLAND_SOCKET', None)
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
with log_path.open('w') as log:
    app = sp.Popen([binary, '-s', 'python3', str(fixture), str(work)], env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        assert app.poll() is None, log_path.read_text()[-7000:]
        time.sleep(.06)
    raise AssertionError(message + '\n' + log_path.read_text()[-7000:])


def send(value, window=1):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(f'window:{window}:{value}\n')
    time.sleep(.16)


def spy(window):
    count = log_path.read_text().count('SPY active=')
    send('spy', window)
    wait(lambda: log_path.read_text().count('SPY active=') > count, 'spy response missing')
    line = [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    return (list(map(int, re.findall(r'\d+\[:(\d+)\]', line))),
            int(re.search(r'SPY active=(\d+)', line)[1]),
            int(re.search(r'workspace=(\d+)', line)[1]),
            int(re.search(r'windows=(\d+)', line)[1]),
            'menu=true' in line)


def expect(window, ids, active, workspace, total):
    value = spy(window)
    assert value[:4] == (ids, active, workspace, total), value


def shot(name):
    if not os.environ.get('MYPIXELS_MENU_SCREENSHOT'):
        return
    time.sleep(.3)
    path = work / f'{name}.png'
    command = ['gamescopectl', 'screenshot'] if os.environ.get('GAMESCOPE_WAYLAND_DISPLAY') else ['grim']
    sp.run(command + [str(path)], check=True, stdout=sp.PIPE, stderr=sp.PIPE)
    wait(lambda: path.exists() and path.stat().st_size > 0, 'screenshot missing')


def new_window(source, total, tab_id):
    if '--x11' in sys.argv:
        # Xvfb has no window manager to focus newly mapped windows. Focus only
        # the test's own source window before injecting its shortcut.
        windows = sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)],
                         env=env, check=True, stdout=sp.PIPE).stdout.splitlines()
        window = sorted(windows, key=int)[source - 1].decode()
        sp.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
        time.sleep(.2)
    send('ctrl+shift+n', source)
    wait(lambda: (work / f'input-{tab_id}').exists(), 'shortcut did not create a terminal')
    expect(total, [tab_id], 0, 0, total)
    shot(f'new-window-{total}-tab-{tab_id}')


try:
    wait(lambda: (work / 'input-0').exists(), 'initial terminal missing')
    new_window(1, 2, 1)
    send('text:new-window-input', 2)
    wait(lambda: (work / 'input-1').read_bytes() == b'new-window-input', 'new window input malfunctioned')
    send('text:original-window-input', 1)
    wait(lambda: (work / 'input-0').read_bytes() == b'original-window-input', 'original window input changed')
    # Open from the older window while another window is active.
    new_window(1, 3, 2)
    send('alt+5', 3)
    wait(lambda: (work / 'input-6').exists(), 'new window numbered tabs failed')
    expect(3, [2, 3, 4, 5, 6], 4, 0, 3)
    send('alt+w', 3)
    wait(lambda: (work / 'input-7').exists(), 'new window workspace failed')
    expect(3, [7], 0, 1, 3)
    send('text:workspace-input', 3)
    wait(lambda: (work / 'input-7').read_bytes() == b'workspace-input', 'workspace input malfunctioned')
    send('alt+q', 3)
    expect(3, [2, 3, 4, 5, 6], 4, 0, 3)
    expect(2, [1], 0, 0, 3)
    # Removing an earlier window changes indices, not identities or renderers.
    send('ctrl+alt+x', 1)
    expect(1, [1], 0, 0, 2)
    expect(2, [2, 3, 4, 5, 6], 4, 0, 2)
    new_window(1, 3, 8)
    send('text:after-close-input', 3)
    wait(lambda: (work / 'input-8').read_bytes() == b'after-close-input', 'window after closure malfunctioned')
    shot('final-window-input')
    assert 'panicked at' not in log_path.read_text()
    print('Shortcut-created window checks passed. Artifacts:', work)
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
