"""Real Yazi image previews: crop reconstruction, navigation, and resizing.

Run after cargo build on an isolated Wayland display:
python3 tests/yazi_image_smoke.py [--x11]
Requires yazi, Pillow, and grim (or gamescopectl in a private Gamescope display).
Uses generated images and a dedicated FIFO, never desktop keyboard input.
"""
import math
import os
from pathlib import Path
import re
import subprocess as sp
import sys
import tempfile
import time
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-yazi-'))
folder = work / 'images'
folder.mkdir()
palettes = [('#e04040', '#40c060', '#4080e0', '#e0c040'),
            ('#d040c0', '#40c0d0', '#d08040', '#8040d0'),
            ('#e04040', '#40c060', '#4080e0', '#e0c040')]
for name, colors, size in zip(['a.png', 'b.png', 'c.png'], palettes,
                              [(320, 240), (320, 240), (960, 1600)]):
    width, height = size
    image = Image.new('RGB', size)
    draw = ImageDraw.Draw(image)
    for rect, color in zip([(0, 0, width // 2 - 1, height // 2 - 1),
                            (width // 2, 0, width - 1, height // 2 - 1),
                            (0, height // 2, width // 2 - 1, height - 1),
                            (width // 2, height // 2, width - 1, height - 1)], colors):
        draw.rectangle(rect, fill=color)
    image.save(folder / name)
config = work / 'config.yml'
config.write_text("wal_palette: ''\ndefault_width: 1000\ndefault_height: 700\nterm_transparency: 0\nterm_font: DejaVu Sans Mono 12\n")
keys = work / 'keys'
os.mkfifo(keys)
dump = work / 'apc.dump'
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           YAZI_CONFIG_HOME=str(work / 'yazi'), MYPIXELS_CONFIG=str(config),
           MYPIXELS_KEYS=str(keys), MYPIXELS_APC_DUMP=str(dump), RUST_LOG='info')
for name in ['TERM_PROGRAM', 'TERM_PROGRAM_VERSION', 'KITTY_WINDOW_ID', 'TMUX', 'ZELLIJ', 'WEZTERM_PANE']:
    env.pop(name, None)
if '--x11' in sys.argv:
    env.pop('WAYLAND_DISPLAY', None)
    env.pop('WAYLAND_SOCKET', None)
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
log_path = work / 'app.log'
with log_path.open('w') as log:
    app = sp.Popen([binary, '-s', 'yazi', str(folder / 'a.png')], env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        assert app.poll() is None, log_path.read_text()[-6000:]
        time.sleep(.1)
    raise AssertionError(message + '\n' + log_path.read_text()[-6000:])


def commands():
    data = dump.read_bytes() if dump.exists() else b''
    found = []
    for match in re.finditer(rb'APCFRAME (\d+) (\d+) (\d+) (\d+)\n', data):
        if match.end() + int(match[4]) > len(data):
            continue
        payload = data[match.end():match.end() + int(match[4])]
        header = payload.lstrip(b'G;').split(b';')[0]
        params = dict(pair.split(b'=', 1) for pair in header.split(b',') if b'=' in pair)
        found.append((int(match[2]), int(match[3]), params))
    return found


def send(value):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out:
        out.write(value + '\n')
    time.sleep(.2)


capture_index = 0


def screenshot(label):
    global capture_index
    capture_index += 1
    path = work / f'{label}-{capture_index}.png'
    command = ['gamescopectl', 'screenshot'] if os.environ.get('GAMESCOPE_WAYLAND_DISPLAY') else ['grim']
    sp.run(command + [str(path)], check=True, stdout=sp.PIPE, stderr=sp.PIPE)
    def ready():
        try:
            image = Image.open(path)
            image.load()
            return image.convert('RGB')
        except (FileNotFoundError, OSError):
            return None
    return wait(ready, 'screenshot missing')


def color(value):
    return tuple(bytes.fromhex(value[1:]))


def matches(actual, expected):
    # Allow small color conversion/decoding differences in Yazi and the compositor.
    return all(abs(a - b) <= 4 for a, b in zip(actual, expected))


def correct_image(which, label, after=0):
    controls = commands()[after:]
    uploads = [(index, p[b'i']) for index, (_, _, p) in enumerate(controls)
               if p.get(b'a') in (b't', b'T') and b'i' in p]
    if not uploads:
        return False
    # Yazi reuses image ids after resize; placements from an older upload
    # describe a different viewport and must not enlarge this rectangle.
    latest, image_id = uploads[-1]
    placements = [(row, col, p) for row, col, p in controls[latest + 1:]
                  if p.get(b'a') == b'p' and p.get(b'i') == image_id]
    if len(placements) < 4:
        return False
    # Wait until all tiles of the preview rectangle have arrived.
    top = min(row for row, _, _ in placements)
    left = min(col for _, col, _ in placements)
    rows = max(row for row, _, _ in placements) - top + 1
    cols = max(col for _, col, _ in placements) - left + 1
    if len(placements) < rows * cols:
        return False
    image = screenshot(label)
    x, y = left * cw, strip + top * ch
    width, height = cols * cw, rows * ch
    expected = list(map(color, palettes[which]))
    for (fx, fy), target in zip([(0.25, .25), (.75, .25), (.25, .75), (.75, .75)], expected):
        point = (round(x + width * fx), round(y + height * fy))
        if point[0] >= image.width or point[1] >= image.height:
            return False
        if not matches(image.getpixel(point), target):
            return False
    # Check every scanline away from the quadrant boundary. Letterboxing each
    # 1x1 tile separately produces dark horizontal seams and must not pass.
    for dy in range(2, round(height) - 2):
        if abs(dy - height / 2) < ch:
            continue
        target = expected[0 if dy < height / 2 else 2]
        if not matches(image.getpixel((round(x + width * .25), round(y + dy))), target):
            return False
    # Check the top and left edges too: a placement cap used to evict the
    # first tiles, leaving a dark rectangle in the image's top-left corner.
    for dx in range(1, round(width) - 1):
        if abs(dx - width / 2) < cw:
            continue
        target = expected[0 if dx < width / 2 else 1]
        if not matches(image.getpixel((round(x + dx), round(y + 1))), target):
            return False
    for dy in range(1, round(height) - 1):
        if abs(dy - height / 2) < ch:
            continue
        target = expected[0 if dy < height / 2 else 2]
        if not matches(image.getpixel((round(x + 1), round(y + dy))), target):
            return False
    return True


try:
    wait(lambda: any(p.get(b'a') == b'p' for _, _, p in commands()), 'Yazi did not send image placements')
    log = log_path.read_text()
    cw, ch = map(float, re.search(r'cell = ([0-9.]+) x ([0-9.]+)', log).groups())
    strip = float(re.search(r'strip = ([0-9.]+)px', log)[1])
    wait(lambda: correct_image(0, 'first-image'), 'first Yazi preview is incorrect or has tile seams')
    offset = len(commands())
    send('text:j')
    wait(lambda: correct_image(1, 'next-image', offset), 'navigation did not replace the image correctly')
    offset = len(commands())
    send('text:j')
    wait(lambda: correct_image(2, 'large-image', offset), 'large preview has missing tiles')
    assert sum(p.get(b'a') == b'p' for _, _, p in commands()[offset:]) > 1024, 'fixture must exceed the old placement limit'
    offset = len(commands())
    send('text:k')
    wait(lambda: correct_image(1, 'back-to-second', offset), 'returning from a large image left stale tiles')
    offset = len(commands())
    send('text:k')
    wait(lambda: correct_image(0, 'previous-image', offset), 'returning to the first image left stale placements')
    offset = len(commands())
    # A row-spacing change resizes the PTY grid on both X11 and Wayland,
    # including compositors that ignore client requests to resize the window.
    config.write_text(config.read_text() + 'line_spacing: 0.1\n')
    ch = math.floor(ch * 1.1 + .5)
    send('ctrl+shift+r')
    wait(lambda: correct_image(0, 'resized-image', offset), 'resizing did not redraw the cropped preview correctly')
    assert 'panicked at' not in log_path.read_text()
    print('Yazi image checks passed. Artifacts:', work, flush=True)
except BaseException:
    import traceback
    traceback.print_exc()
    sys.stderr.flush()
    raise
finally:
    app.terminate()
    try:
        app.wait(timeout=5)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
