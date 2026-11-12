#!/usr/bin/env bash
# mypixels shell integration for Bash 4.4+.
# Source from ~/.bashrc after prompt setup:
#   source /path/to/mypixels/scripts/shell_integration.sh
# Reuse the Bash hooks so both entry points emit the same OSC 133 command
# markers and failure bell. Sourcing either file repeatedly is harmless.
case ${BASH_SOURCE[0]} in
    */*) source "${BASH_SOURCE[0]%/*}/shell_integration.bash" ;;
    *) source ./shell_integration.bash ;;
esac
