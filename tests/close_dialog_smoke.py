#!/usr/bin/env python3
"""Close-dialog policies and irreversible-close safety on a private display.

Run: dbus-run-session -- xvfb-run -a -s '-screen 0 1600x1000x24' python3 tests/close_dialog_smoke.py
"""
import os
from pathlib import Path
import subprocess as sp
import tempfile
import time

from PIL import Image
from x11_helpers import request_close


root = Path(__file__).resolve().parents[1]
binary = str(Path(os.environ.get('MYPIXELS_BIN', root / 'target/debug/mypixels')).resolve())
work = Path(tempfile.mkdtemp(prefix='mypixels-close-dialog-'))


class Session:
    def __init__(self, name, policy=None, daemon=False):
        self.dir = work / name
        self.cleaned = False
        self.dir.mkdir()
        config = self.dir / 'config.yml'
        config.write_text('''wal_palette: ''
default_width: 1000
default_height: 700
default_maximize: false
background_color: '#101614'
bg_tint_color: '#101614'
accent_color: '#b9e600'
term_transparency: 0
workspaces_enabled: true
''' + (f'close_window_dialog: {policy}\n' if policy else ''))
        shell = self.dir / 'shell'
        shell.write_text('''#!/usr/bin/python3
import os, tty, threading, time
from pathlib import Path
work = Path(os.environ['CLOSE_TEST_WORK'])
index = len(list(work.glob('pid-*')))
(work / f'pid-{index}').write_text(str(os.getpid()))
tty.setraw(0)
os.write(1, f'\\x1b]2;terminal-{index}\\x07Window close safety test\\r\\n'.encode())
def background():
    while True:
        (work / f'alive-{index}').touch()
        if (work / f'stop-{index}').exists(): os._exit(0)
        time.sleep(.05)
threading.Thread(target=background, daemon=True).start()
while True:
    data = os.read(0, 65536)
    if not data: break
    with (work / f'input-{index}').open('ab') as out: out.write(data)
''')
        shell.chmod(0o755)
        self.keys = self.dir / 'keys'
        os.mkfifo(self.keys)
        self.env = dict(os.environ, HOME=str(self.dir), XDG_CONFIG_HOME=str(self.dir / '.config'),
                        MYPIXELS_CONFIG=str(config), MYPIXELS_KEYS=str(self.keys),
                        MYPIXELS_BUS='com.github.volca.mypixels.close_test',
                        MYPIXELS_MCP=str(self.dir / 'mcp.sock'), SHELL=str(shell),
                        CLOSE_TEST_WORK=str(self.dir), RUST_LOG='warn,mypixels=info')
        self.env.pop('WAYLAND_DISPLAY', None)
        self.env.pop('MYPIXELS_WINDOW', None)
        self.log = (self.dir / 'app.log').open('w')
        self.app = sp.Popen([binary, '--daemon' if daemon else '-s'], env=self.env,
                            stdout=self.log, stderr=self.log)
        if daemon:
            self.wait(lambda: (self.dir / 'mcp.sock').exists(), 'daemon missing')
            sp.run([binary], env=self.env, check=True, capture_output=True, timeout=10)
        self.wait(lambda: (self.dir / 'alive-0').exists(), 'terminal missing')
        self.wait(self.windows, 'window missing')
        self.key('alt+2')
        self.wait(lambda: (self.dir / 'alive-1').exists(), 'second terminal missing')

    def wait(self, check, message, seconds=12):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = check()
            if value:
                return value
            assert self.app.poll() is None, (self.dir / 'app.log').read_text()
            time.sleep(.05)
        raise AssertionError(message + '\n' + (self.dir / 'app.log').read_text())

    def windows(self):
        return sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(self.app.pid)],
                      env=self.env, capture_output=True).stdout.splitlines()

    def key(self, value):
        fd = os.open(self.keys, os.O_WRONLY | os.O_NONBLOCK)
        with os.fdopen(fd, 'w') as out:
            out.write(value + '\n')
        time.sleep(.2)

    def shot(self, wid, name):
        time.sleep(.2)
        path = self.dir / f'{name}.png'
        sp.run(['import', '-window', wid, str(path)], env=self.env, check=True)
        return Image.open(path).convert('RGB')

    def click(self, wid, pos):
        sp.run(['xdotool', 'windowraise', wid], env=self.env, check=True)
        sp.run(['xdotool', 'windowfocus', '--sync', wid], env=self.env, check=True)
        sp.run(['xdotool', 'mousemove', '--window', wid, str(pos[0]), str(pos[1]), 'click', '1'],
               env=self.env, check=True)
        time.sleep(.2)

    def alive(self, index):
        path = self.dir / f'alive-{index}'
        before = path.stat().st_mtime_ns
        time.sleep(.2)
        return path.stat().st_mtime_ns > before

    def clean(self):
        if self.cleaned:
            return
        self.cleaned = True
        if self.app.poll() is None:
            self.app.terminate()
            self.app.wait(timeout=5)
        for path in self.dir.glob('pid-*'):
            try:
                os.kill(int(path.read_text()), 15)
            except ProcessLookupError:
                pass
        self.log.close()


def red_button(image):
    """Locate the broad red button, including its muted disabled fill."""
    hits = []
    for y in range(image.height):
        start = None
        for x in range(image.width):
            r, g, b = image.getpixel((x, y))
            red = r > 45 and r > 1.4 * g and r > 1.2 * b
            if red and start is None:
                start = x
            if not red and start is not None:
                if x - start > 175:
                    hits.append((start, x, y))
                start = None
    assert hits, 'red close button missing'
    return ((min(h[0] for h in hits) + max(h[1] for h in hits)) // 2,
            (min(h[2] for h in hits) + max(h[2] for h in hits)) // 2)


session = None
try:
    # Never is an explicit opt-out, including when the last window loses tabs.
    session = Session('never', 'never')
    request_close(session.windows()[0])
    session.wait(lambda: session.app.poll() is not None, 'Never policy still displayed a dialog')
    session.clean()

    # Loss-only skips the dialog when a daemon can retain the window.
    session = Session('loss-daemon', 'loss', daemon=True)
    request_close(session.windows()[0])
    session.wait(lambda: not session.windows(), 'Loss policy prompted for recoverable closure')
    assert session.alive(0) and session.alive(1), 'recoverable terminals died'
    session.clean()

    # Loss-only warns on the last foreground window. Neither disabled clicks,
    # ordinary typing, nor timer expiry may terminate it or reach its PTYs.
    session = Session('loss-foreground', 'loss')
    wid = session.windows()[0]
    request_close(wid)
    time.sleep(.3)
    session.key('return')  # Default is Cancel.
    assert len(session.windows()) == 1 and session.alive(1)
    request_close(wid)
    started = time.monotonic()
    disabled = session.shot(wid, 'danger-disabled')
    session.click(wid, red_button(disabled))
    session.key('x')
    session.key('ctrl+shift+g')
    assert session.app.poll() is None, 'destructive close fired during the delay'
    assert not (session.dir / 'input-1').exists(), 'modal input leaked to a terminal'
    time.sleep(max(0, 5.3 - (time.monotonic() - started)))
    assert session.app.poll() is None, 'countdown auto-confirmed permanent close'
    ready = session.shot(wid, 'danger-ready')
    session.click(wid, red_button(ready))
    session.wait(lambda: session.app.poll() is not None, 'explicit permanent close failed')
    assert not session.alive(0) and not session.alive(1), 'permanent close left terminals running'
    session.clean()

    # Always is the default. Another window supplies recovery, but if that
    # window exits, the visible modal must switch to a fresh danger warning.
    session = Session('always')
    source = session.windows()[0]
    session.key('ctrl+shift+n')
    session.wait(lambda: len(session.windows()) == 2, 'second window missing')
    session.wait(lambda: (session.dir / 'alive-2').exists(), 'second window child missing')
    request_close(source)
    session.shot(source, 'recoverable-other-window')
    session.key('window:1:return')
    assert len(session.windows()) == 2, 'default Enter closed a recoverable window'
    request_close(source)
    time.sleep(.3)
    (session.dir / 'stop-2').touch()
    session.wait(lambda: len(session.windows()) == 1, 'other window did not exit')
    danger = session.shot(source, 'recovery-host-lost')
    session.click(source, red_button(danger))
    assert session.app.poll() is None and session.alive(0), 'stale recovery choice destroyed terminals'
    session.key('escape')
    assert session.alive(1)
    # Explicit permanent closure also works when another window could have
    # retained the terminals: the dead window must never enter the graveyard.
    session.key('ctrl+shift+n')
    session.wait(lambda: len(session.windows()) == 2, 'replacement recovery host missing')
    session.wait(lambda: (session.dir / 'alive-3').exists(), 'replacement child missing')
    request_close(source)
    time.sleep(.3)
    session.key('window:1:tab')  # Cancel -> Move to Graveyard.
    session.key('window:1:tab')  # Move -> Close permanently…
    session.key('window:1:return')
    time.sleep(5.2)
    danger = session.shot(source, 'permanent-with-survivor')
    session.click(source, red_button(danger))
    session.wait(lambda: len(session.windows()) == 1, 'permanent close with a survivor failed')
    assert not session.alive(0) and not session.alive(1) and session.alive(3)
    session.key('ctrl+alt+shift+z')
    assert len(session.windows()) == 1, 'permanently closed window was buried instead'
    print(f'close dialog smoke ok (artifacts in {work})')
finally:
    if session is not None:
        session.clean()
