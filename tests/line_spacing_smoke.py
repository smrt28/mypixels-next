"""Live row spacing, PTY pixel sizes, hidden workspaces, and zoom preservation.

Run on a graphical display after cargo build: python3 tests/line_spacing_smoke.py
Uses isolated terminal children and a private FIFO; no desktop keyboard input.
Set MYPIXELS_SPACING_SCREENSHOT=1 to save desktop screenshots with grim.
"""
import json
import math
import os
from pathlib import Path
import subprocess as sp
import tempfile
import time

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-line-spacing-'))
config = work / 'config.yml'
base_config = '''wal_palette: ''
term_font: DejaVu Sans Mono 12
default_width: 900
default_height: 600
default_maximize: false
hide_single_tab: false
term_transparency: 0
'''
config.write_text(base_config + 'line_spacing: 0\n')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import fcntl, json, os, struct, sys, termios, threading, time, tty
from pathlib import Path
work = Path(sys.argv[1])
index = 0
while True:
    try:
        (work / f'child-{index}').touch(exist_ok=False)
        break
    except FileExistsError:
        index += 1
tty.setraw(0)
os.write(1, (f'\x1b]2;Spacing {index}\x07' + ''.join(f'Row {n:02d}  ABC abc 0123  [] {{}}  gjpqy\r\n' for n in range(18))).encode())
def monitor():
    previous = None
    while True:
        size = struct.unpack('HHHH', fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
        if size != previous:
            temp = work / f'size-{index}.tmp'
            temp.write_text(json.dumps(size))
            temp.replace(work / f'size-{index}.json')
            previous = size
        trigger = work / f'query-{index}'
        if trigger.exists():
            trigger.unlink()
            os.write(1, b'\x1b[16t')
        time.sleep(.04)
threading.Thread(target=monitor, daemon=True).start()
while True:
    data = os.read(0, 65536)
    if not data: break
    with (work / f'input-{index}').open('ab') as out: out.write(data)
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
        time.sleep(.06)
    raise AssertionError(message + '\n' + log_path.read_text())


def send(key, window=1):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(f'window:{window}:{key}\n')
    time.sleep(.16)


def size(index):
    path = work / f'size-{index}.json'
    return json.loads(path.read_text()) if path.exists() else None


def cell(index):
    value = size(index)
    if not value:
        return None
    rows, cols, width, height = value
    return width // cols, height // rows


def shot(name):
    if os.environ.get('MYPIXELS_SPACING_SCREENSHOT'):
        time.sleep(.3)
        sp.run(['grim', str(work / f'{name}.png')], check=True)


def set_spacing(spacing, height, width):
    config.write_text(base_config + f'line_spacing: {spacing}\n')
    send('ctrl+shift+r', 2)
    wait(lambda: all(cell(i) == (width, height) for i in range(4)),
         f'spacing {spacing} did not update every PTY to {width}x{height}')
    # A hidden terminal must report the same cell size as the visible grid.
    reply = f'\x1b[6;{height};{width}t'.encode()
    (work / 'query-0').touch()
    wait(lambda: (work / 'input-0').exists() and reply in (work / 'input-0').read_bytes(),
         'hidden terminal reported a stale cell size')


try:
    wait(lambda: size(0), 'initial terminal missing')
    send('alt+w')
    wait(lambda: size(1), 'hidden workspace terminal missing')
    send('ctrl+shift+n')
    wait(lambda: size(2), 'second window missing')
    send('alt+w', 2)
    wait(lambda: size(3), 'second window workspace missing')
    time.sleep(.4)
    width, height = cell(0)
    assert all(cell(i) == (width, height) for i in range(4))
    original_rows = [size(i)[0] for i in range(4)]
    shot('normal')
    for spacing, label in [(0.1, 'expanded'), (-0.1, 'compressed'), (0, 'restored')]:
        target_height = max(1, math.floor(height * (1 + spacing) + .5))
        set_spacing(spacing, target_height, width)
        if spacing > 0:
            assert all(size(i)[0] < original_rows[i] for i in range(4))
        elif spacing < 0:
            assert all(size(i)[0] > original_rows[i] for i in range(4))
        else:
            assert [size(i)[0] for i in range(4)] == original_rows
        shot(label)
    # A spacing-only edit must preserve the current live font zoom.
    send('alt+shift+plus', 2)
    wait(lambda: cell(0) != (width, height), 'zoom did not update row metrics')
    zoom_width, zoom_height = cell(0)
    wait(lambda: all(cell(i) == (zoom_width, zoom_height) for i in range(4)), 'zoom did not update all windows')
    set_spacing(0.1, math.floor(zoom_height * 1.1 + .5), zoom_width)
    set_spacing(0, zoom_height, zoom_width)
    send('switcher', 2)
    shot('zoomed-switcher')
    send('alt_up', 2)
    print('Line spacing checks passed. Artifacts:', work)
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
