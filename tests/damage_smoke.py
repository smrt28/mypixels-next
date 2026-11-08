"""Pixel-compare retained partial redraws with the full-redraw diagnostic oracle.

Run: xvfb-run -a python3 tests/damage_smoke.py
Requires xdotool, ImageMagick import, and Pillow. Uses isolated terminal children.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time

from PIL import Image, ImageChops

root = Path(__file__).resolve().parents[1]
binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
assert os.environ.get('DISPLAY'), 'Run under xvfb-run'

FIXTURE = r'''
import os, time, sys
from pathlib import Path
work = Path(sys.argv[1])
index = 0
while True:
    try:
        fd = os.open(work / f'started-{index}', os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        break
    except FileExistsError:
        index += 1
def emit(s): os.write(1, s.encode())
emit(f'\x1b]2;damage-{index}\x07\x1b[?25l\x1b[2J\x1b[H')
for row in range(1, 21):
    emit(f'\x1b[{row};1H\x1b[38;2;80;180;240mTab {index} row {row:02}: '
         '\x1b[0mretained text -> == != 界 e\u0301  colored tail')
n = 0
while True:
    command = work / f'command-{index}-{n}'
    if not command.exists():
        time.sleep(.01)
        continue
    emit(command.read_text())
    (work / f'ready-{index}-{n}').touch()
    n += 1
'''


def session(full):
    work = Path(tempfile.mkdtemp(prefix='mypixels-damage-full-' if full else 'mypixels-damage-partial-'))
    config = work / 'config.yml'
    config.write_text('''term_font: Cascadia Code 12
tablist_font: Terminus (TTF) 12
default_width: 1000
default_height: 600
default_maximize: false
hide_single_tab: false
term_transparency: 0
background_color: '#122e2e'
tab_overview_scale: 0.35
''')
    fixture = work / 'fixture.py'
    fixture.write_text(FIXTURE)
    env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
               MYPIXELS_CONFIG=str(config), MYPIXELS_TRACE='1', RUST_LOG='info')
    for name in ('WAYLAND_DISPLAY', 'MYPIXELS_NOCACHE', 'MYPIXELS_FULL_REDRAW', 'MYPIXELS_KEYS'):
        env.pop(name, None)
    if full:
        env['MYPIXELS_FULL_REDRAW'] = '1'
    log_path = work / 'app.log'
    log = log_path.open('w')
    app = sp.Popen([binary, '-s', 'python3', str(fixture), str(work)], env=env, stdout=log, stderr=log)

    def run(*args):
        return sp.run(args, env=env, check=True, stdout=sp.PIPE, stderr=sp.PIPE, timeout=10).stdout

    def wait(check):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            assert app.poll() is None, log_path.read_text()
            value = check()
            if value:
                return value
            time.sleep(.05)
        raise AssertionError('Timed out: ' + log_path.read_text()[-3000:])

    counters = [0, 0]

    def output(index, text):
        n = counters[index]
        (work / f'command-{index}-{n}').write_text(text)
        wait(lambda: (work / f'ready-{index}-{n}').exists())
        counters[index] += 1
        time.sleep(.15)

    shots = {}

    def shot(name, delay=.4):
        time.sleep(delay)
        path = work / f'{name}.png'
        run('import', '-window', wid, str(path))
        image = Image.open(path).convert('RGB')
        # Exclude the tab strip's wall clock, retaining the full terminal grid.
        shots[name] = image.crop((0, strip, image.width, image.height))

    try:
        wid = wait(lambda: sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)],
                                 env=env, stdout=sp.PIPE).stdout.decode().splitlines())[-1]
        run('xdotool', 'windowfocus', '--sync', wid)
        wait(lambda: (work / 'started-0').exists() and 'strip = ' in log_path.read_text())
        strip = round(float(re.search(r'strip = ([0-9.]+)px', log_path.read_text()).group(1)))
        shot('initial', 1)
        output(0, '\x1b[2;16H\x1b[31mEDIT\x1b[0m\x1b[16;8Hsparse update')
        shot('sparse')
        output(0, '\x1b[6;1H\x1b[2K\x1b[6;4H界 e\u0301\x1b[8;7H\x1b[0K')
        shot('erase-wide')
        output(0, '\x1b[2;16H\x1b[38;2;70;255;80mEDIT\x1b[0m')
        shot('color-only')
        output(0, '\x1b[4;18r\x1b[18;1H\n\n\x1b[r')
        shot('scroll-region')
        output(0, '\x1b[?1049h\x1b[3;5H\x1b[44mALT SCREEN 界\x1b[0m')
        shot('alternate')
        output(0, '\x1b[?1049l')
        shot('restore')
        # Selection damage must clear both the newly selected and old rows.
        run('xdotool', 'mousemove', '--window', wid, '40', str(strip + 150))
        run('xdotool', 'mousedown', '1')
        run('xdotool', 'mousemove', '--window', wid, '500', str(strip + 230))
        run('xdotool', 'mouseup', '1')
        shot('selection')
        run('xdotool', 'click', '1')
        shot('selection-clear')
        run('xdotool', 'key', 'ctrl+shift+t')
        wait(lambda: (work / 'started-1').exists())
        shot('second-tab', .8)
        # Multiple changes reach an inactive tab before its next preview batch.
        output(0, '\x1b[2;1H\x1b[2K\x1b[31mBACKGROUND FIRST\x1b[0m')
        output(0, '\x1b[16;1H\x1b[2K\x1b[32mBACKGROUND LAST\x1b[0m')
        run('xdotool', 'keydown', 'Alt_L')
        shot('background-previews', 1.4)
        run('xdotool', 'keyup', 'Alt_L')
        run('xdotool', 'key', 'alt+1')
        shot('return-to-first', .8)
        run('xdotool', 'windowsize', wid, '900', '540')
        shot('resize', .8)
        output(0, '\x1b]4;1;rgb:ff/99/22\x07\x1b[2;1H\x1b[31mBACKGROUND FIRST\x1b[0m')
        shot('palette')
        if not full:
            trace = log_path.read_text()
            assert re.search(r'damage [1-9]\d* rows', trace), 'partial path never used'
            assert 'damage 0 rows' in trace, 'idle frames never became clean'
        return work, shots
    finally:
        app.terminate()
        try:
            app.wait(5)
        except sp.TimeoutExpired:
            app.kill()
            app.wait()
        log.close()


oracle_work, oracle = session(True)
partial_work, partial = session(False)
for name, expected in oracle.items():
    actual = partial[name]
    assert expected.size == actual.size, (name, expected.size, actual.size)
    diff = ImageChops.difference(expected, actual)
    diff.save(partial_work / f'{name}-diff.png')
    changed = sum(pixel != (0, 0, 0) for pixel in diff.get_flattened_data())
    # A tiny tolerance accommodates animated switcher edges; one stale text
    # row or a blank terminal exceeds it by orders of magnitude.
    fraction = changed / (expected.width * expected.height)
    assert fraction < .001, f'{name}: {fraction:.2%} pixels differ; artifacts: {partial_work}'
    print(f'PASS {name}: {fraction:.3%} pixels differ', flush=True)
print('Artifacts:', oracle_work, partial_work)
