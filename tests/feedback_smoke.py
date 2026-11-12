"""Visual feedback switches, live effects, Save and Cancel on private X11.

Run on an isolated X11/XWayland display with MYPIXELS_ISOLATED_FEEDBACK_TEST=1.
Requires Pillow, libX11, libXtst and Gamescope screenshot support (or import).
Uses native Super key events and FIFO settings clicks; never the desktop display.
"""
import ctypes as C
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time
from PIL import Image, ImageChops

assert os.environ.get('MYPIXELS_ISOLATED_FEEDBACK_TEST') == '1', 'Use an isolated display'
root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-feedback-'))
config = work / 'config.yml'
config.write_text('''wal_palette: ''
term_font: DejaVu Sans Mono 12
default_width: 1100
default_height: 750
default_maximize: false
term_transparency: 0
background_color: '#101820'
bg_tint_color: '#101820'
''')
fixture = work / 'fixture.py'
fixture.write_text('''import os, time, tty
from pathlib import Path
tty.setraw(0)
os.write(1, b'\\x1b[2 q\\x1b[2J\\x1b[HVisual feedback test\\x1b[12;24H')
work=Path(os.environ['FEEDBACK_WORK'])
while True:
    if (work/'bell').exists(): os.write(1,b'\\a')
    time.sleep(.03)
''')
keys = work / 'keys'
os.mkfifo(keys)
env = dict(os.environ, HOME=str(work), MYPIXELS_CONFIG=str(config),
           MYPIXELS_KEYS=str(keys), FEEDBACK_WORK=str(work), RUST_LOG='info')
env.pop('WAYLAND_DISPLAY', None)
logpath = work / 'app.log'
log = logpath.open('w')
binary = str(Path(os.environ.get('MYPIXELS_BIN', root / 'target/debug/mypixels')).resolve())
app = sp.Popen([binary, '-s', 'python3', str(fixture)], env=env, stdout=log, stderr=log)


def wait(check, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        assert app.poll() is None, logpath.read_text()
        if check(): return
        time.sleep(.05)
    raise AssertionError(message + '\n' + logpath.read_text())


def key(text):
    fd = os.open(keys, os.O_WRONLY | os.O_NONBLOCK)
    with os.fdopen(fd, 'w') as out: out.write(text + '\n')
    time.sleep(.15)


capture = 0

def shot(label):
    global capture
    capture += 1
    path = work / f'{label}-{capture}.png'
    command = ['gamescopectl', 'screenshot', str(path)] if env.get('GAMESCOPE_WAYLAND_DISPLAY') else ['import', '-window', str(wid), str(path)]
    sp.run(command, env=env, check=True, capture_output=True, timeout=5)
    image = None
    def ready():
        nonlocal image
        try:
            image = Image.open(path).convert('RGB')
            return True
        except (OSError, FileNotFoundError): return False
    wait(ready, 'screenshot missing')
    return image


x = C.CDLL('libX11.so.6')
xt = C.CDLL('libXtst.so.6')
x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [C.c_char_p], C.c_void_p
x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [C.c_void_p], C.c_ulong
x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
x.XInternAtom.argtypes, x.XInternAtom.restype = [C.c_void_p, C.c_char_p, C.c_int], C.c_ulong
x.XGetWindowProperty.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long, C.c_int, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ubyte))]
x.XFree.argtypes = [C.c_void_p]
x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
x.XKeysymToKeycode.argtypes, x.XKeysymToKeycode.restype = [C.c_void_p, C.c_ulong], C.c_ubyte
x.XFlush.argtypes = x.XCloseDisplay.argtypes = [C.c_void_p]
xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
display = x.XOpenDisplay(env['DISPLAY'].encode())
assert display
pid_atom = x.XInternAtom(display, b'_NET_WM_PID', 0)


def app_window():
    pending = [x.XDefaultRootWindow(display)]
    while pending:
        window = pending.pop()
        kind, fmt, count, remaining = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong()
        data = C.POINTER(C.c_ubyte)()
        x.XGetWindowProperty(display, window, pid_atom, 0, 1, 0, 0, C.byref(kind), C.byref(fmt), C.byref(count), C.byref(remaining), C.byref(data))
        pid = C.cast(data, C.POINTER(C.c_ulong))[0] if data and count.value else None
        if data: x.XFree(data)
        if pid == app.pid: return window
        ancestor, parent, children, size = C.c_ulong(), C.c_ulong(), C.POINTER(C.c_ulong)(), C.c_uint()
        if x.XQueryTree(display, window, C.byref(ancestor), C.byref(parent), C.byref(children), C.byref(size)):
            pending.extend(children[i] for i in range(size.value))
            if children: x.XFree(children)
    return None


def click(px, py): key(f'click:{px},{py}')


def settings():
    key('alt+shift+c')
    wait(lambda: 'settings nav: ' in logpath.read_text(), 'settings navigation missing')
    line = [line for line in logpath.read_text().splitlines() if 'settings nav: ' in line][-1]
    nav = {name: (float(px), float(py)) for name, px, py in re.findall(r'(\w+)=(-?[0-9.]+),(-?[0-9.]+)', line)}
    click(*nav['Terminal'])
    cell = nav['Window'][0] - nav['Appearance'][0] - 8
    control_x = nav['Appearance'][0] - cell / 2 + 204
    return nav, control_x, nav['Appearance'][1] + 188


def check_bell(enabled):
    baseline = shot('bell-idle').getpixel((40, height - 50))
    (work / 'bell').touch()
    try:
        time.sleep(.12)
        delta = max(max(abs(a - b) for a, b in zip(shot('bell-active').getpixel((40, height - 50)), baseline)) for _ in range(3))
        assert (delta > 8) if enabled else (delta <= 2), (enabled, delta)
    finally:
        (work / 'bell').unlink(missing_ok=True)
        time.sleep(.25)


def check_cursor(enabled):
    x.XSetInputFocus(display, wid, 2, 0)
    x.XFlush(display)
    time.sleep(.2)
    baseline = shot('cursor-idle')
    delta = 0
    code = x.XKeysymToKeycode(display, 0xffeb)  # Super_L (Win).
    for _ in range(3):
        xt.XTestFakeKeyEvent(display, code, 1, 0)
        x.XFlush(display)
        try:
            time.sleep(.025)
            difference = ImageChops.difference(shot('cursor-highlight'), baseline)
            # Key activity can change the ordinary cursor's blink phase. Mask
            # its cell so this check measures only the surrounding highlight.
            difference.paste((0, 0, 0), tuple(map(round, (cx - cell_w, cy - cell_h, cx + cell_w, cy + cell_h))))
            difference = difference.crop(cursor_box)
            delta = max(delta, max(channel[1] for channel in difference.getextrema()))
        finally:
            xt.XTestFakeKeyEvent(display, code, 0, 0)
            x.XFlush(display)
            time.sleep(.22)
    assert (delta > 5) if enabled else (delta <= 2), (enabled, delta)


try:
    wait(app_window, 'window missing')
    wid = app_window()
    key('spy')
    info = logpath.read_text()
    width, height = map(int, re.findall(r'surf=(\d+)x(\d+)', info)[-1])
    cell_w, cell_h = map(float, re.search(r'cell = ([0-9.]+) x ([0-9.]+)', info).groups())
    strip = float(re.search(r'strip = ([0-9.]+)px', info).group(1))
    cx, cy = (23.5 * cell_w, strip + 11.5 * cell_h)
    cursor_box = tuple(map(round, (cx - 3 * cell_h, cy - 3 * cell_h, cx + 3 * cell_h, cy + 3 * cell_h)))
    check_bell(True)
    check_cursor(True)
    nav, toggle_x, bell_y = settings()
    shot('settings-enabled')
    click(toggle_x, bell_y)
    click(toggle_x, bell_y + 36)
    shot('settings-disabled')
    click(nav['Window'][0], height - 35)
    wait(lambda: 'visual_bell: false' in config.read_text() and 'cursor_highlight: false' in config.read_text(), 'Save did not persist feedback switches')
    saved = config.read_text()
    check_bell(False)
    check_cursor(False)
    nav, toggle_x, bell_y = settings()
    click(toggle_x, bell_y)
    click(toggle_x, bell_y + 36)
    check_bell(True)
    check_cursor(True)
    click(nav['Appearance'][0], height - 35)
    wait(lambda: 'configuration and colors reloaded' in logpath.read_text(), 'Cancel did not reload feedback switches')
    assert config.read_text() == saved, 'Cancel changed the saved values'
    check_bell(False)
    check_cursor(False)
    print('PASS: visual bell and repeated Win cursor highlights, live toggles, Save and Cancel; artifacts:', work, flush=True)
finally:
    x.XCloseDisplay(display)
    app.terminate()
    try: app.wait(timeout=5)
    except sp.TimeoutExpired: app.kill(); app.wait(timeout=5)
    log.close()
