"""Native notification integration against a fake daemon on a private D-Bus.

Run: dbus-run-session -- xvfb-run -a -s '-screen 0 1280x1024x24' python3 tests/notifications_smoke.py
Requires PyGObject (Gio), xdotool, and gdbus. No real desktop notifications.
"""
import json
import os
from pathlib import Path
import subprocess as sp
import tempfile
import time

assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'Run under a private bus and Xvfb'
root = Path(__file__).resolve().parents[1]
work = Path(tempfile.mkdtemp(prefix='mypixels-notifications-'))

# Confirm the bus has no notification daemon before claiming the test name.
owner = sp.run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus', '--object-path',
                '/org/freedesktop/DBus', '--method', 'org.freedesktop.DBus.NameHasOwner',
                'org.freedesktop.Notifications'], capture_output=True, text=True, check=True)
assert 'false' in owner.stdout, 'Refusing to replace a real notification daemon; use dbus-run-session'

server = work / 'daemon.py'
server.write_text(r'''
from gi.repository import Gio, GLib
from pathlib import Path
import json, sys
work = Path(sys.argv[1])
xml = ''' + repr('''<node><interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
<method name="CloseNotification"><arg type="u" direction="in"/></method>
<method name="TestActivate"><arg type="u" direction="in"/></method>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>''') + r'''
conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
conn.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName',
               GLib.Variant('(su)', ('org.freedesktop.Notifications', 4)), None, Gio.DBusCallFlags.NONE, 3000, None)
next_id = 1
path = '/org/freedesktop/Notifications'
def method(connection, sender, object_path, interface, name, args, invocation):
    global next_id
    values = args.unpack()
    if name == 'GetCapabilities':
        invocation.return_value(GLib.Variant('(as)', (['body', 'actions', 'body-markup'],)))
    elif name == 'Notify':
        app, replaces, icon, title, body, actions, hints, timeout = values
        ident = replaces or next_id
        if not replaces: next_id += 1
        with (work / 'calls.jsonl').open('a') as out:
            out.write(json.dumps(dict(id=ident, replaces=replaces, title=title, body=body, actions=actions)) + '\n')
        invocation.return_value(GLib.Variant('(u)', (ident,)))
    elif name == 'CloseNotification':
        conn.emit_signal(None, path, interface, 'NotificationClosed', GLib.Variant('(uu)', (values[0], 3)))
        invocation.return_value(None)
    elif name == 'TestActivate':
        conn.emit_signal(None, path, interface, 'ActionInvoked', GLib.Variant('(us)', (values[0], 'default')))
        invocation.return_value(None)
conn.register_object(path, Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], method, None, None)
(work / 'daemon-ready').touch()
GLib.MainLoop().run()
''')
fixture = work / 'fixture.py'
fixture.write_text(r'''
import os, select, sys, time, tty
from pathlib import Path
work = Path(sys.argv[1])
index = 0
while True:
    try:
        fd = os.open(work / f'tab-{index}', os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        break
    except FileExistsError: index += 1
tty.setraw(0)
os.write(1, f'\x1b]2;notify-{index}\x07\x1b[?25lReady {index}\r\n'.encode())
n = 0
while True:
    path = work / f'command-{index}-{n}'
    if path.exists():
        os.write(1, path.read_bytes())
        (work / f'sent-{index}-{n}').touch()
        n += 1
    if select.select([0], [], [], .02)[0]:
        data = os.read(0, 4096)
        if not data: break
        with (work / f'input-{index}').open('ab') as out: out.write(data)
''')
config = work / 'config.yml'
config.write_text('default_width: 900\ndefault_height: 600\nhide_single_tab: false\nterm_transparency: 0\n')
env = dict(os.environ, HOME=str(work), XDG_CONFIG_HOME=str(work / '.config'), MYPIXELS_CONFIG=str(config), RUST_LOG='mypixels=debug')
env.pop('WAYLAND_DISPLAY', None)
env.pop('MYPIXELS_KEYS', None)
log = (work / 'app.log').open('w')
daemon_log = (work / 'daemon.log').open('w')
daemon = sp.Popen(['python3', str(server), str(work)], env=env, stdout=daemon_log, stderr=daemon_log)
app = None


def wait(check):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        assert daemon.poll() is None, (work / 'daemon.log').read_text()
        if app:
            assert app.poll() is None, (work / 'app.log').read_text()
        value = check()
        if value:
            return value
        time.sleep(.04)
    raise AssertionError('Timed out; artifacts: ' + str(work) + '\n' + (work / 'app.log').read_text()[-4000:])


def calls():
    path = work / 'calls.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


counters = [0, 0]
def output(index, text):
    n = counters[index]
    (work / f'command-{index}-{n}').write_text(text)
    wait(lambda: (work / f'sent-{index}-{n}').exists())
    counters[index] += 1


def run(*args):
    return sp.run(args, env=env, check=True, capture_output=True, timeout=10).stdout


def input_(index):
    path = work / f'input-{index}'
    return path.read_bytes() if path.exists() else b''


try:
    wait(lambda: (work / 'daemon-ready').exists())
    binary = os.environ.get('MYPIXELS_BIN', str(root / 'target/debug/mypixels'))
    app = sp.Popen([binary, '-s', 'python3', str(fixture), str(work)], env=env, stdout=log, stderr=log)
    wid = wait(lambda: sp.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)], env=env, stdout=sp.PIPE).stdout.decode().splitlines())[-1]
    run('xdotool', 'windowfocus', '--sync', wid)
    wait(lambda: (work / 'tab-0').exists())
    output(0, '\x1b]9;Approval requested: cargo test; echo done\x1b\\')
    wait(lambda: len(calls()) == 1)
    assert calls()[0]['title'] == 'Approval requested: cargo test; echo done'
    run('xdotool', 'key', 'ctrl+shift+t')
    wait(lambda: (work / 'tab-1').exists())
    output(0, '\x1b]99;i=build:d=0:a=report:c=1;Build done\x1b\\')
    output(0, '\x1b]99;i=build:p=body;A < B & all tests passed\x1b\\')
    wait(lambda: len(calls()) == 2)
    note = calls()[1]
    assert note['title'] == 'Build done' and note['body'] == 'A &lt; B &amp; all tests passed', note
    # Replacing an existing id stays scoped to its originating tab.
    output(0, '\x1b]99;i=build:a=report:c=1;Updated build\x1b\\')
    wait(lambda: len(calls()) == 3)
    assert calls()[2]['id'] == note['id'] and calls()[2]['replaces'] == note['id']
    run('gdbus', 'call', '--session', '--dest', 'org.freedesktop.Notifications', '--object-path', '/org/freedesktop/Notifications', '--method', 'org.freedesktop.Notifications.TestActivate', str(note['id']))
    wait(lambda: b'\x1b]99;i=build;\x1b\\' in input_(0))
    run('xdotool', 'key', 'x')
    wait(lambda: input_(0).endswith(b'x'))
    assert input_(1) == b'', 'click activated the wrong tab'
    output(0, '\x1b]99;i=build:p=close;\x1b\\')
    wait(lambda: b'\x1b]99;i=build:p=close;\x1b\\' in input_(0))
    output(0, '\x1b]99;i=query:p=?;\x1b\\')
    wait(lambda: b'i=query:p=?;' in input_(0))
    # The focused occasion must suppress the next message.
    before = len(calls())
    output(0, '\x1b]99;o=unfocused;Do not show\x1b\\')
    time.sleep(.3)
    assert len(calls()) == before
    output(0, '\x1b]777;notify;Tests;All passed\x07')
    wait(lambda: len(calls()) == before + 1)
    assert calls()[-1]['body'] == 'All passed'
    print('PASS: OSC 9/99/777, chunks, escaped text, replacement, click-to-tab, activation/close replies, queries, focus filtering; artifacts:', work, flush=True)
finally:
    for process in (app, daemon):
        if process:
            process.terminate()
            try:
                process.wait(5)
            except sp.TimeoutExpired:
                process.kill()
                process.wait()
    log.close()
    daemon_log.close()
