"""Rendered terminal brightness, truecolor, ANSI, OSC overrides, and previews.

Run after cargo build on an isolated Wayland display:
python3 tests/brightness_smoke.py
Requires Pillow and grim (or gamescopectl inside a private Gamescope display).
Uses a private FIFO; never sends desktop keyboard input or reads the clipboard.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time
from PIL import Image

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-brightness-'))
config = work / 'config.yml'
base = '''wal_palette: ''
term_font: DejaVu Sans Mono 12
default_width: 900
default_height: 600
default_maximize: false
term_transparency: 0
background_color: '#203040'
bg_tint_color: '#203040'
foreground_color: '#8090a0'
'''
config.write_text(base + 'terminal_brightness: 0\n')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import os, sys, tty
from pathlib import Path
tty.setraw(0)
def paint():
    os.write(1, b'\x1b[?25l\x1b[H\x1b[2J\x1b]4;1;#406080\x07')
    for row, style in [(2, '48;2;64;96;128'), (3, '41'), (4, '48;5;1'), (5, '48;5;59')]:
        os.write(1, (f'\x1b[{row};1H\x1b[{style}m' + ' ' * 20 + '\x1b[0m').encode())
    os.write(1, ('\x1b[7;1H\x1b[38;2;64;96;128m' + '█' * 20 + '\x1b[0m').encode())
paint()
Path(sys.argv[1]).touch()
while True:
    data = os.read(0, 65536)
    if not data: break
    if b'redraw' in data: paint()
    if b'override' in data: os.write(1, b'\x1b]11;#304050\x07\x1b]4;1;#506070\x07')
''')
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(keys), RUST_LOG='info')
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
with log_path.open('w') as log:
    app = sp.Popen([binary, '-s', 'python3', str(fixture), str(work / 'ready')],
                   env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        assert app.poll() is None, log_path.read_text()
        time.sleep(.08)
    raise AssertionError(message + '\n' + log_path.read_text())


def send(key):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(key + '\n')
    time.sleep(.2)


shot_index = 0


def screenshot(label):
    global shot_index
    shot_index += 1
    path = work / f'{label}-{shot_index}.png'
    command = ['gamescopectl', 'screenshot'] if env.get('GAMESCOPE_WAYLAND_DISPLAY') else ['grim']
    sp.run(command + [str(path)], env=env, check=True, stdout=sp.PIPE, stderr=sp.PIPE)
    def ready():
        try:
            image = Image.open(path)
            image.load()
            return image.convert('RGB')
        except (FileNotFoundError, OSError):
            return None
    return wait(ready, 'screenshot missing')


def scaled(color, percent):
    return tuple(min(255, round(channel * (1 + percent / 100))) for channel in color)


def near(actual, expected):
    return all(abs(a - e) <= 1 for a, e in zip(actual, expected))


try:
    wait(lambda: (work / 'ready').exists(), 'terminal missing')
    time.sleep(.4)
    log = log_path.read_text()
    cw, ch = map(float, re.search(r'cell = ([0-9.]+) x ([0-9.]+)', log).groups())
    strip = float(re.search(r'strip = ([0-9.]+)px', log)[1])
    # Redraw after the compositor's initial resize has settled.
    send('text:redraw')
    for percent in [0, -40, 40, 0]:
        config.write_text(base + f'terminal_brightness: {percent}\n')
        send('ctrl+shift+r')
        def matches():
            image = screenshot(str(percent))
            bg = image.getpixel((image.width - 30, image.height - 30))
            if not near(bg, scaled((32, 48, 64), percent)):
                return False
            for row, color in [(1, (64, 96, 128)), (2, (64, 96, 128)),
                               (3, (64, 96, 128)), (4, (95, 95, 95)), (6, (64, 96, 128))]:
                pixel = image.getpixel((round(cw * 2.5), round(strip + (row + .5) * ch)))
                assert near(pixel, scaled(color, percent)), (percent, row, pixel, scaled(color, percent))
            return image
        wait(matches, f'brightness {percent} did not reach rendered pixels')
    # Runtime palette/background overrides use the same display adjustment.
    config.write_text(base + 'terminal_brightness: -40\n')
    send('ctrl+shift+r')
    send('text:override')
    def overridden():
        image = screenshot('osc-override')
        return (near(image.getpixel((image.width - 30, image.height - 30)), scaled((48, 64, 80), -40))
                and near(image.getpixel((round(cw * 2.5), round(strip + 2.5 * ch))), scaled((80, 96, 112), -40)))
    wait(overridden, 'runtime overrides did not follow brightness')
    send('ctrl+shift+t')
    send('switcher')
    time.sleep(.4)
    screenshot('darkened-previews')
    send('alt_up')
    print('Brightness pixel checks passed. Artifacts:', work)
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
