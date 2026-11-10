"""Opaque-start transparency transitions and brightness independence.

Run in a private Gamescope headless display (black background):
python3 tests/transparency_smoke.py [--x11]
Also supports an isolated compositor with grim when
MYPIXELS_ISOLATED_GRAPHICS_TEST=1 is set. Uses a private FIFO, never desktop input.
Checks rendered pixels and, on Wayland, opaque-region updates for both windows.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import sys
import tempfile
import time
from PIL import Image

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-transparency-'))
assert os.environ.get('GAMESCOPE_WAYLAND_DISPLAY') or (
    os.environ.get('MYPIXELS_ISOLATED_GRAPHICS_TEST') == '1' and os.environ.get('WAYLAND_DISPLAY')
), 'Run inside a private headless compositor with MYPIXELS_ISOLATED_GRAPHICS_TEST=1'
x11 = '--x11' in sys.argv
config = work / 'config.yml'
base = '''wal_palette: ''
default_width: 900
default_height: 600
default_maximize: false
background_color: '#80a0c0'
bg_tint_color: '#80a0c0'
foreground_color: '#101820'
'''
config.write_text(base + 'term_transparency: 0\n')
fixture = work / 'fixture.py'
fixture.write_text("import os, tty\ntty.setraw(0)\nos.write(1, b'\\x1b[?25l\\x1b[2J')\nwhile os.read(0, 4096): pass\n")
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(keys), RUST_LOG='info')
for name in ['MYPIXELS_TRANSPARENCY', 'MYPIXELS_OPACITY', 'SPIKE_TRANSPARENCY', 'SPIKE_OPACITY']:
    env.pop(name, None)
if x11:
    env.pop('WAYLAND_DISPLAY', None)
    env.pop('WAYLAND_SOCKET', None)
else:
    env['WAYLAND_DEBUG'] = 'client'
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
with log_path.open('w') as log:
    app = sp.Popen([binary, '-s', 'python3', str(fixture)], env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        assert app.poll() is None, log_path.read_text()[-6000:]
        time.sleep(.1)
    raise AssertionError(message + '\n' + log_path.read_text()[-6000:])


def send(value, window=1):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(f'window:{window}:{value}\n')
    time.sleep(.2)


capture_index = 0


def screenshot(label):
    global capture_index
    capture_index += 1
    path = work / f'{label}-{capture_index}.png'
    command = ['gamescopectl', 'screenshot'] if env.get('GAMESCOPE_WAYLAND_DISPLAY') else ['grim']
    sp.run(command + [str(path)], env=os.environ, check=True, stdout=sp.PIPE, stderr=sp.PIPE)
    def ready():
        try:
            image = Image.open(path)
            image.load()
            return image.convert('RGB')
        except (FileNotFoundError, OSError):
            return None
    return wait(ready, 'screenshot missing')


def near(actual, expected):
    return all(abs(a - e) <= 2 for a, e in zip(actual, expected))


def matches(alpha, brightness, label):
    image = screenshot(label)
    pixel = image.getpixel((image.width - 30, image.height - 30))
    expected = tuple(round(min(255, c * (1 + brightness / 100)) * alpha) for c in (128, 160, 192))
    return image if near(pixel, expected) else None


try:
    wait(lambda: 'background alpha' in log_path.read_text(), 'initial frame missing')
    wait(lambda: matches(1, 0, 'opaque-start'), 'opaque startup pixel mismatch')
    send('ctrl+shift+n')
    wait(lambda: log_path.read_text().count('alpha: surface offers') >= 2, 'second window missing')
    time.sleep(.4)
    previous = 0.0
    for transparency, brightness in [(0.5, 0), (1, 0), (0, 0), (0.5, -40), (0.5, 40), (0, 0)]:
        before = len(log_path.read_text())
        config.write_text(base + f'term_transparency: {transparency}\nterminal_brightness: {brightness}\n')
        send('ctrl+shift+r', 2)
        wait(lambda: matches(1 - transparency, brightness, f'transparency-{transparency}-brightness-{brightness}'),
             f'transparency {transparency} brightness {brightness} did not reach the output')
        if not x11 and (transparency > 0) != (previous > 0):
            def updated_hints():
                lines = log_path.read_text()[before:].splitlines()
                matches = [re.search(r'wl_surface(?:#|@)(\d+)\.set_opaque_region\(([^)]*)\)', line) for line in lines]
                surfaces = {m[1] for m in matches if m and ((m[2] in ('nil', 'null')) == (transparency > 0))}
                return len(surfaces) >= 2
            wait(updated_hints, 'opaque-region hints did not update in both windows')
        previous = transparency
    print(f'Transparency checks passed ({"X11" if x11 else "Wayland"}). Artifacts:', work)
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
