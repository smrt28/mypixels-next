"""Real interactive Bash regression checks, without a display or user rc files.

Run: python3 tests/bash_shell_integration.py
Checks prompt-hook chaining and the OSC 133 boundaries used for command capture.
"""
import errno
import os
from pathlib import Path
import pty
import re
import select
import signal
import tempfile
import time

root = Path(__file__).resolve().parents[1]
marker = re.compile(rb'\x1b\]133;([ABCD])(?:;([0-9]+))?\x07')


class Shell:
    def __init__(self, work, hooks):
        self.work = work
        rc = work / 'bashrc'
        rc.write_text(r'''
PS1='before> '
PS2='continuation> '
PS0=''
__test_prompt() {
    local rc=$?
    printf '%s\n' "$rc" >> "$HOME/statuses"
    printf 'first\n' >> "$HOME/hook-order"
    PS1='界 project\n❯ '
}
__test_second() { printf 'second\n' >> "$HOME/hook-order"; }
''' + hooks + '\n' + '\n'.join(
            f'source "{path}"' for path in (
                root / 'scripts/shell_integration.sh',
                root / 'scripts/shell_integration.bash',
                root / 'scripts/shell_integration.sh',
            )) + '\n')
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ.update(HOME=str(work), HISTFILE='/dev/null',
                              TERM='xterm-256color', INPUTRC='/dev/null')
            os.execvp('bash', ['bash', '--noprofile', '--rcfile', str(rc), '-i'])
        try:
            self.prompt = self.read_prompt()
        except BaseException:
            self.close()
            raise

    def read_prompt(self):
        output = b''
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], .1)[0]:
                try:
                    output += os.read(self.fd, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if b'\x1b]133;B\x07' in output:
                    return output
        raise AssertionError(f'prompt-end marker missing: {output!r}')

    def command(self, command):
        os.write(self.fd, command.encode() + b'\n')
        return self.read_prompt()

    def close(self):
        os.close(self.fd)
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(self.pid, 0)


def check(hooks, array=False):
    with tempfile.TemporaryDirectory(prefix='mypixels-bash-') as directory:
        work = Path(directory)
        shell = Shell(work, hooks)
        try:
            assert marker.findall(shell.prompt) == [(b'D', b'0'), (b'A', b''), (b'B', b'')], shell.prompt
            for command, status in [
                ("printf 'captured output\\n'", b'0'),
                ('false', b'1'),
                ("printf '%s\\n' '界 command with spaces'", b'0'),
                ('true; false', b'1'),
                ('', b'1'),
            ]:
                output = shell.command(command)
                expected = ([(b'C', b'')] if command else []) + [(b'D', status), (b'A', b''), (b'B', b'')]
                assert marker.findall(output) == expected, (command, output)
                if command:
                    # Input must reach the terminal before C snapshots it.
                    assert command.encode() in output.split(b'\x1b]133;C\x07')[0], output
                assert (work / 'statuses').read_text().splitlines()[-1] == status.decode(), output
                if status == b'1':
                    assert b'\x1b]133;D;1\x07\x07' in output, 'failure bell lost'
            order = (work / 'hook-order').read_text().splitlines()
            assert order == (['first', 'second'] if array else ['first']) * 6, order
            # A DEBUG trap already installed by another integration is untouched.
            output = shell.command("trap -p DEBUG > \"$HOME/debug-trap\"")
            if 'trap ' in hooks:
                assert "trap -- ':' DEBUG" in (work / 'debug-trap').read_text()
            # Existing PS0 content and a prompt replacement at runtime survive too.
            shell.command(f'PS0="preexec> "; source "{root / "scripts/shell_integration.sh"}"')
            output = shell.command('true')
            assert b'preexec> ' in output, output
            assert output.count(b'\x1b]133;C\x07') == 1, output
        finally:
            shell.close()


with tempfile.TemporaryDirectory(prefix='mypixels-bash-default-') as directory:
    shell = Shell(Path(directory), 'unset PROMPT_COMMAND')
    try:
        assert marker.findall(shell.command('true')) == [(b'C', b''), (b'D', b'0'), (b'A', b''), (b'B', b'')]
    finally:
        shell.close()

check('PROMPT_COMMAND=__test_prompt')
check("PROMPT_COMMAND='__test_prompt; __test_second'", array=True)
check("PROMPT_COMMAND=(__test_prompt __test_second)\ntrap ':' DEBUG", array=True)
# Re-sourcing upgrades the previous integration's scalar hook rather than
# nesting it and producing duplicate command-end/prompt-start markers.
check("PROMPT_COMMAND='__mypixels_precmd $?;__test_prompt'")
print('PASS: scalar/compound/array prompt hooks, dynamic multiline prompts, exit statuses, failure bells, DEBUG trap, PS0 and repeat sourcing')
