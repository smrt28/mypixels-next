# mypixels shell integration (bash): command marks + a bell on failure.
#
#   source /path/to/this/file   (e.g. from ~/.bashrc)
#
# Emits OSC 133 marks - D (exit status of the last command) and A (new
# prompt) from PROMPT_COMMAND, B at the end of PS1, C via PS0 - which
# mypixels folds into per-tab command blocks for Ctrl+Alt+Up/Down command
# navigation. The failed-command BEL stays: mypixels escalates an unseen
# bell into a compositor attention request plus a red pulse on the tab's
# cell, and it works over ssh because it is all just bytes in the stream.
__mypixels_precmd() {
    local rc=$1
    printf '\e]133;D;%s\a' "$rc"
    if [ "$rc" -ne 0 ]; then
        printf '\a'
    fi
    printf '\e]133;A\a'
    # Existing prompt hooks must still see the command's exit status.
    return "$rc"
}
__mypixels_prompt_end() {
    local rc=$?
    # Prompt themes often replace PS1 in PROMPT_COMMAND. Add B only after
    # those hooks have finished, so the input anchor survives each redraw.
    local input_start='\[\e]133;B\a\]'
    if [[ $PS1 != *"$input_start" ]]; then
        PS1+=$input_start
    fi
    return "$rc"
}
__mypixels_preexec() {
    # PS0 fires after a line is read and before it runs (C). The previous
    # command's status belongs to PROMPT_COMMAND instead. The write goes to
    # the tty, not stdout: PS0 runs this inside a command substitution, and
    # stdout bytes emitted there are captured as the expansion's value and
    # thrown away, never reaching the terminal.
    printf '\e]133;C\a' > /dev/tty
}
__mypixels_install_prompt_hooks() {
    local hook
    local -a hooks=()
    # A scalar expands as one array element, preserving compound commands.
    # Remove our own entries before rebuilding, including the old scalar
    # prefix, so re-sourcing either entry point upgrades without duplicates.
    for hook in "${PROMPT_COMMAND[@]}"; do
        case $hook in
            '__mypixels_precmd $?' | '__mypixels_prompt_end') continue ;;
            '__mypixels_precmd $?;'*) hook=${hook#'__mypixels_precmd $?;'} ;;
        esac
        [[ -n $hook ]] && hooks+=("$hook")
    done
    PROMPT_COMMAND=('__mypixels_precmd $?' "${hooks[@]}" '__mypixels_prompt_end')
}
__mypixels_install_prompt_hooks
unset -f __mypixels_install_prompt_hooks
# Preserve an existing PS0 prompt and append the execution marker once.
case ${PS0-} in
    *'$(__mypixels_preexec)'*) ;;
    *) PS0="${PS0-}"'$(__mypixels_preexec)' ;;
esac
