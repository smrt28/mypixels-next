"""Workspace names, colors, icons, button display, Save, and Cancel.

Run under an isolated X server: xvfb-run -a python3 tests/workspace_settings_smoke.py
Requires xdotool, ImageMagick import, and Pillow. Uses an isolated HOME/config.
"""
import os
from pathlib import Path
import re
import subprocess as sp
import tempfile
import time

from PIL import Image

root = Path(__file__).resolve().parents[1]
binary = Path(os.environ.get('MYPIXELS_BIN', root / 'target/debug/mypixels')).resolve()
assert os.environ.get('DISPLAY'), 'Run under xvfb-run'
work = Path(tempfile.mkdtemp(prefix='mypixels-workspace-settings-'))
config = work / 'config.yml'
config.write_text('''# Preserve this comment when saving.
wal_palette: ''
term_font: Cascadia Code 12
tablist_font: Terminus (TTF) 12
default_width: 1280
default_height: 850
default_maximize: false
term_opacity: 0
hide_single_tab: false
background_color: '#0b1216'
foreground_color: '#dce4e7'
accent_color: '#00dbe6'
workspaces_enabled: true
workspace_genre_1: Existing
''')
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'),
           MYPIXELS_CONFIG=str(config), RUST_LOG='info')
env.pop('WAYLAND_DISPLAY', None)
env.pop('MYPIXELS_KEYS', None)
log_path = work / 'app.log'
log = log_path.open('w')
app = sp.Popen([str(binary), '-s', 'python3', str(root / 'tests/pty_fixture.py'), str(work)],
               env=env, stdout=log, stderr=log)


def run(*args):
    return sp.run(args, env=env, check=True, stdout=sp.PIPE, stderr=sp.PIPE, timeout=5).stdout


def wait(check, message):
    until = time.monotonic() + 10
    while time.monotonic() < until:
        value = check()
        if value:
            return value
        assert app.poll() is None, log_path.read_text()
        time.sleep(.08)
    raise AssertionError(message + '\n' + log_path.read_text())


def shot(name):
    time.sleep(.4)
    path = work / (name + '.png')
    run('import', '-window', wid, str(path))
    return Image.open(path).convert('RGB')


def click(x, y):
    run('xdotool', 'mousemove', '--window', wid, str(round(x)), str(round(y)), 'click', '1')
    time.sleep(.15)


def saved_value(key):
    match = re.search(rf"(?m)^{re.escape(key)}: (.*)$", config.read_text())
    return match.group(1).strip().strip("'\"") if match else None


try:
    def windows():
        return sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)],
                      env=env, stdout=sp.PIPE).stdout.decode().splitlines()
    wid = wait(windows, 'window did not appear')[-1]
    wait(lambda: list(work.glob('input-*')), 'PTY missing')
    run('xdotool', 'windowfocus', '--sync', wid)
    run('xdotool', 'windowsize', wid, '1280', '1000')
    time.sleep(.5)
    width, height = 1280, 1000
    left = width - 448
    run('xdotool', 'key', 'alt+shift+c')
    time.sleep(.5)
    # Four wheel notches move the workspace card above the fold.
    run('xdotool', 'mousemove', '--window', wid, str(width - 80), '620',
        'click', '--repeat', '4', '--delay', '100', '5')
    editor = shot('workspace-editor')
    # Anchor the editor to the red palette swatch rather than the page header:
    # section heights above it can change without moving the target controls.
    red = [(x, y) for y in range(250, height - 100) for x in range(left + 20, left + 60)
           if editor.getpixel((x, y)) == (240, 123, 114)]
    assert red, 'workspace color palette missing'
    color_y = (min(y for x, y in red) + max(y for x, y in red)) / 2
    selector_y = color_y - 138
    click(left + 200, selector_y)
    click(left + 200, selector_y + 80)  # Workspace 2 in the dropdown.
    click(left + 200, color_y - 69)
    run('xdotool', 'key', 'ctrl+a')
    run('xdotool', 'type', '--clearmodifiers', 'Research')
    click(left + 262, color_y)  # Purple preset.
    click(left + 380, color_y + 115)  # Cube, fifth icon.
    click(left + 280, color_y + 209)  # Button display.
    click(left + 280, color_y + 209 + 38 + 3 * 42)  # Icon and name.
    click(left + 280, color_y + 245)  # Button position.
    click(left + 280, color_y + 245 + 38)  # Left.
    shot('edited-workspace')
    click(left + 330, height - 35)
    wait(lambda: saved_value('workspace_icon_2') == 'cube', 'workspace icon was not saved')
    saved = config.read_text()
    for key, expected in {'workspace_genre_1': 'Existing', 'workspace_genre_2': 'Research',
                          'workspace_color_2': '#b792ed', 'workspace_button_display': 'icon_and_name',
                          'workspace_button_position': 'left'}.items():
        assert saved_value(key) == expected, f'{key} did not save {expected}:\n{saved}'
    run('xdotool', 'key', 'alt+shift+2')
    current = shot('workspace-button')
    assert any(current.getpixel((x, 14)) == (183, 146, 237) for x in range(45, 160)), 'workspace button did not preview its saved color'
    run('xdotool', 'key', 'alt+shift+c')
    shot('reopened-workspace')
    click(left + 68, color_y + 115)  # Change icon to Code, then discard it.
    click(left + 118, height - 35)
    wait(lambda: 'configuration and colors reloaded' in log_path.read_text(), 'Cancel did not reload settings')
    assert config.read_text() == saved, 'Cancel changed saved workspace settings'
    assert all(not p.read_bytes() for p in work.glob('input-*')), 'workspace settings input leaked into a terminal'
    print('PASS: workspace selector, name, palette, icon, button display and position, Save and Cancel; artifacts:', work, flush=True)

finally:
    app.terminate()
    try:
        app.wait(timeout=3)
    except sp.TimeoutExpired:
        app.kill()
        app.wait()
    log.close()
