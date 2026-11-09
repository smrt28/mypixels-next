"""Workspace regression check using a dedicated FIFO and isolated terminal child.

Run: python3 tests/workspace_smoke.py (requires a graphical display).
No desktop keyboard input or clipboard access is used. Artifacts stay in /tmp.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-workspaces-'))
config = work / 'config.yml'
config.write_text('''wal_palette: ''
default_width: 900
default_height: 500
default_maximize: false
hide_single_tab: false
tab_overview_on_alt: false
tab_overview_delay_ms: 60
show_tab_titles: false
''')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import os, sys, tty, threading, time
from pathlib import Path
work = Path(sys.argv[1])
index = len(list(work.glob('input-*')))
while True:
    path = work / f'input-{index}'
    try:
        path.touch(exist_ok=False)
        break
    except FileExistsError:
        index += 1
tty.setraw(0)
os.write(1, f'\x1b]2;Terminal {index}\x07Workspace terminal {index}\r\n'.encode())
def background():
    trigger = work / f'trigger-{index}'
    emitted = False
    while True:
        if (work / f'stop-{index}').exists(): os._exit(0)
        if trigger.exists() and not emitted:
            os.write(1, b'Background output\r\n\x1b[6n')
            emitted = True
        time.sleep(.05)
threading.Thread(target=background, daemon=True).start()
while True:
    data = os.read(0, 65536)
    if not data: break
    with path.open('ab') as out: out.write(data)
    if b'EXIT' in data: break
''')
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(keys), RUST_LOG='info')
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
with log_path.open('w') as log:
    app = sp.Popen([binary, '-s', 'python3', str(fixture), str(work)],
                   env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        assert app.poll() is None, log_path.read_text()
        time.sleep(.05)
    raise AssertionError(message + '\n' + log_path.read_text())


target_window = 1


def send(value):
    # Nonblocking open keeps startup failures from hanging the test.
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(f'window:{target_window}:{value}\n')
    time.sleep(.12)


def spy():
    count = log_path.read_text().count('SPY active=')
    send('spy')
    wait(lambda: log_path.read_text().count('SPY active=') > count, 'spy missing')
    line = [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    return (int(re.search(r'workspace=(\d+)', line)[1]),
            list(map(int, re.findall(r'\d+\[:(\d+)\]', line))),
            int(re.search(r'SPY active=(\d+)', line)[1]))


def expect(workspace, ids, active):
    state = spy()
    assert state == (workspace, ids, active), state


def new_tab(index):
    send('ctrl+shift+t')
    wait(lambda: (work / f'input-{index}').exists(), 'new terminal missing')


try:
    wait(lambda: (work / 'input-0').exists(), 'initial terminal missing')
    expect(0, [0], 0)
    # Disabled at startup, then enabled without a restart: exercise the actual
    # Alt-hold timer, not just the direct switcher hook.
    send('alt_hold')
    time.sleep(.2)
    spy()
    assert 'switcher=false' in [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    send('switcher')
    spy()
    assert 'switcher=false' in [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    config.write_text(config.read_text().replace('tab_overview_on_alt: false', 'tab_overview_on_alt: true')
                      .replace('show_tab_titles: false', 'show_tab_titles: true'))
    send('ctrl+shift+r')
    send('alt_hold')
    def switcher_visible():
        spy()
        return 'switcher=true' in [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    wait(switcher_visible, 'enabling the switcher did not activate the hold timer')
    send('alt_up')
    new_tab(1)
    new_tab(2)
    send('alt+2')
    expect(0, [0, 1, 2], 1)
    send('alt+w')
    wait(lambda: (work / 'input-3').exists(), 'workspace terminal missing')
    expect(1, [3], 0)
    new_tab(4)
    send('alt+q')
    expect(0, [0, 1, 2], 1)
    # A hidden terminal keeps receiving output and protocol replies.
    (work / 'trigger-3').touch()
    wait(lambda: (work / 'input-3').read_bytes().endswith(b'R'), 'hidden DSR reply missing')
    assert b'\x1b[' in (work / 'input-3').read_bytes()
    # Switching while the Alt-held tab switcher is open must dismiss it.
    send('switcher')
    send('alt+w')
    expect(1, [3, 4], 1)
    send('alt+1')
    send('alt+q')
    (work / 'stop-3').touch()
    time.sleep(.5)
    send('alt+w')
    expect(1, [4], 0)
    send('ctrl+alt+x')
    expect(0, [0, 1, 2], 1)
    # All ten shortcuts, lazy creation, and remembered tabs on return.
    next_id = 5
    for index, key in enumerate('ertyuiop', start=2):
        send('alt+' + key)
        wait(lambda n=next_id: (work / f'input-{n}').exists(), 'workspace not created')
        expect(index, [next_id], 0)
        send('alt+' + key)
        expect(index, [next_id], 0)
        next_id += 1
    send('alt+q')
    expect(0, [0, 1, 2], 1)
    send('alt+3')
    send('text:hello')
    wait(lambda: (work / 'input-2').read_bytes() == b'hello', 'input routed to wrong terminal')
    assert (work / 'input-0').read_bytes() == b''
    assert (work / 'input-1').read_bytes() == b''
    if os.environ.get('MYPIXELS_WORKSPACE_SCREENSHOT'):
        sp.run(['grim', str(work / 'workspaces.png')], check=True)
        send('alt+o')
        expect(8, [11], 0)
        sp.run(['grim', str(work / 'workspace-W9.png')], check=True)
        send('alt+q')
        send('alt+shift+c')
        time.sleep(.3)
        sp.run(['grim', str(work / 'workspace-colors-settings.png')], check=True)
        send('alt+shift+c')
    # New windows start with their own first workspace.
    send('ctrl+shift+n')
    wait(lambda: (work / f'input-{next_id}').exists(), 'new window missing')
    target_window = 2
    expect(0, [next_id], 0)
    send('alt+w')
    next_id += 1
    wait(lambda: (work / f'input-{next_id}').exists(), 'new window workspace missing')
    expect(1, [next_id], 0)
    target_window = 1
    expect(0, [0, 1, 2], 2)
    send('alt+p')
    expect(9, [12], 0)
    # Two existing tabs: requesting tab 5 appends tabs 3, 4, and 5 even
    # when tab 1 is active and normal new-tab placement would insert beside it.
    send('alt+q')
    send('ctrl+alt+x')
    expect(0, [0, 1], 1)
    send('alt+1')
    send('alt+5')
    expect(0, [0, 1, 15, 16, 17], 4)
    wait(lambda: (work / 'input-17').exists(), 'numbered shortcut did not spawn all terminals')
    send('alt+5')
    expect(0, [0, 1, 15, 16, 17], 4)
    send('alt+2')
    expect(0, [0, 1, 15, 16, 17], 1)
    # A configured Ctrl+5 works too. Automatic creation must also ignore
    # the "first" placement preference used by manual tab creation.
    with config.open('a') as out:
        out.write("new_tab_position: first\nbindings:\n  goto_tab_4: CONTROL+_5\n")
    send('ctrl+shift+r')
    send('alt+p')
    expect(9, [12], 0)
    send('switcher')
    send('alt+0')
    expect(9, [12] + list(range(18, 27)), 9)
    wait(lambda: (work / 'input-26').exists(), 'tab 10 terminals missing')
    send('alt+esc')
    expect(9, [12] + list(range(18, 27)), 0)
    send('alt+0')
    expect(9, [12] + list(range(18, 27)), 9)
    send('alt+w')
    expect(1, [27], 0)
    wait(lambda: (work / 'input-27').exists(), 'workspace terminal missing')
    send('ctrl+5')
    expect(1, list(range(27, 32)), 4)
    wait(lambda: (work / 'input-31').exists(), 'custom numbered shortcut terminals missing')
    send('alt+q')
    expect(0, [0, 1, 15, 16, 17], 1)
    target_window = 2
    expect(1, [14], 0)
    for index in range(15, 32):
        assert (work / f'input-{index}').read_bytes() == b'', 'tab shortcut leaked into a terminal'
    # Disabling via a live config reload dismisses an already open switcher.
    send('switcher')
    assert switcher_visible()
    config.write_text(config.read_text().replace('tab_overview_on_alt: true', 'tab_overview_on_alt: false')
                      .replace('show_tab_titles: true', 'show_tab_titles: false'))
    send('ctrl+shift+r')
    spy()
    assert 'switcher=false' in [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    send('alt_hold')
    time.sleep(.2)
    send('switcher')
    spy()
    assert 'switcher=false' in [line for line in log_path.read_text().splitlines() if 'SPY active=' in line][-1]
    send('ctrl+5')
    expect(1, [14, 32, 33, 34, 35], 4)
    wait(lambda: (work / 'input-35').exists(), 'numbered shortcut failed with the switcher disabled')
    if os.environ.get('MYPIXELS_WORKSPACE_SCREENSHOT'):
        sp.run(['grim', str(work / 'number-only-tabs.png')], check=True)
    print('Workspace, tab preference, and numbered tab checks passed. Artifacts:', work)
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
