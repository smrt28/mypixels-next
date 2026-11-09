#!/usr/bin/env python3
"""Graveyard smoke: closed tabs keep running, the panel shows and raises
them, the limit evicts the oldest, and the last window ends the session
with the burials.

Run with a display big enough for the configured window:

    xvfb-run -a -s "-screen 0 1600x900x24" python3 tests/graveyard_smoke.py

"""
import os
import subprocess as sp
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parents[1]
assert os.environ.get('DISPLAY'), 'Run under xvfb-run'
work = Path(tempfile.mkdtemp(prefix='mypixels-graveyard-'))
config = work / 'config.yml'
config.write_text('''wal_palette: ''
term_font: Cascadia Code 12
tablist_font: Terminus (TTF) 12
default_width: 1400
default_height: 800
default_maximize: false
hide_single_tab: false
term_opacity: 0
bg_tint_color: '#0b1216'
background_color: '#0b1216'
foreground_color: '#dce4e7'
accent_color: '#00dbe6'
graveyard_limit: 1
''')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import os, sys, time, tty, threading
from pathlib import Path
work = Path(sys.argv[1])
tty.setraw(0)
# New tabs re-run the same command, so the index is derived, not passed.
index = len(list(work.glob('input-*')))
path = work / f'input-{index}'
path.touch()
os.write(1, f'\x1b]2;grave-{index}\x07line {index} \x1b[32malive\x1b[0m\r\n'.encode())

def beat():
    while True:
        (work / f'alive-{index}').touch()
        time.sleep(0.3)

threading.Thread(target=beat, daemon=True).start()
while True:
    data = sys.stdin.buffer.read(1)
    if not data:
        break
    with open(path, 'ab') as f:
        f.write(data)
    os.write(1, data)
''')
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), RUST_LOG='info')
env.pop('WAYLAND_DISPLAY', None)
env.pop('MYPIXELS_KEYS', None)
env.pop('MYPIXELS_WINDOW', None)
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
log = log_path.open('w')
# No -s: the instance must host the session bus, so step 5's second launch
# can hand its window request over D-Bus.
app = sp.Popen([binary, 'python3', str(fixture), str(work)], env=env,
               stdout=log, stderr=log)


def run(*args):
    return sp.run(args, env=env, check=True, stdout=sp.PIPE, stderr=sp.PIPE, timeout=10).stdout


def wait(check, message, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        assert app.poll() is None, log_path.read_text()
        time.sleep(.08)
    raise AssertionError(message + '\n' + log_path.read_text())


def key(value):
    run('xdotool', 'key', value)


def shot(name):
    time.sleep(.5)
    path = work / (name + '.png')
    run('import', '-window', wid, str(path))
    return Image.open(path).convert('RGB')


def windows():
    return sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)],
                  env=env, stdout=sp.PIPE).stdout.decode().splitlines()


def alive_after(path, pause=0.9):
    """The heartbeat file must keep moving: the shell is still running."""
    before = path.stat().st_mtime_ns
    time.sleep(pause)
    return path.stat().st_mtime_ns > before


wid = wait(windows, 'window missing')[-1]
wait(lambda: (work / 'input-0').exists(), 'first PTY missing')
run('xdotool', 'windowfocus', '--sync', wid)
time.sleep(.3)

# Second tab, then bury it.
key('ctrl+shift+t')
wait(lambda: (work / 'input-1').exists(), 'second PTY missing')
time.sleep(.5)
key('ctrl+alt+x')
time.sleep(.7)

# 1. Closing does not kill: the buried shell keeps its heartbeat.
assert alive_after(work / 'alive-1'), 'buried tab stopped running'

# 2. The panel opens and paints (accent stroke on the selected card).
key('ctrl+shift+g')


def panel_visible():
    image = shot('graveyard-open')
    # The panel owns the right ~380 px. Selection is opt-in, so look for
    # the card fill (bg nudged toward fg) rather than the accent stroke.
    card = any((lambda p: 20 < p[0] < 40 and 28 < p[1] < 48 and 32 < p[2] < 52)
               (image.getpixel((x, y)))
               for y in range(60, image.height - 40, 3)
               for x in range(image.width - 370, image.width - 10, 3))
    return image if card else None


wait(panel_visible, 'graveyard panel did not paint')
key('Escape')
time.sleep(.3)

# 3. Ctrl+Shift+Z raises the newest burial; it is active and takes keys.
key('ctrl+shift+z')
time.sleep(.7)
key('z')
wait(lambda: b'z' in (work / 'input-1').read_bytes(), 'raised tab does not take keys')
assert (work / 'input-0').read_bytes() == b'', 'keys leaked into the other tab'

# 4. Bury both; the limit is 1, so the oldest burial (grave-1, buried
#    first) is evicted. Closing the last window then ends the session: no
#    headless process holding shells.
key('ctrl+alt+x')  # buries grave-1 (the tab we just raised)
time.sleep(.4)
key('ctrl+alt+x')  # buries grave-0: evicts grave-1, closes the window, exits
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    if app.poll() is not None:
        break
else:
    raise AssertionError('app did not exit with its last window\n' + log_path.read_text())
dead = False
while time.monotonic() < deadline:
    if not alive_after(work / 'alive-1', pause=0.6):
        dead = True
        break
assert dead, 'evicted burial kept running'
assert not alive_after(work / 'alive-0', pause=0.5), 'burial survived the exit'

print(f'graveyard smoke ok ({len(list(work.glob("*.png")))} artifacts in {work})')
