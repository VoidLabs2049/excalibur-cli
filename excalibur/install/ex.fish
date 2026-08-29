# Excalibur - Generic Fish Shell Integration
#
# `ex <module> [args...]` runs any Excalibur module and applies whatever
# command the module emits:
#
#   exit 0  -> insert into the command line, leave it editable
#   exit 10 -> insert and execute immediately
#
# This is the single place that knows the exit-code protocol. Per-module
# wrappers (exh, excc) are thin aliases over it; new modules need no new
# fish function at all -- `ex pt`, `ex s`, ... just work.

function ex --description "Run an Excalibur module and apply its emitted command"
    set -l emitted (command excalibur $argv 2>/dev/null)
    set -l code $status

    # Clear any residual output from the TUI
    commandline -f repaint

    # User cancelled, or the module emitted nothing (in-module action only)
    if test -z "$emitted"
        return
    end

    switch $code
        case 0
            # Insert command into command line (user can edit)
            commandline -r -- $emitted
            commandline -f repaint
        case 10
            # Insert and execute immediately
            commandline -r -- $emitted
            commandline -f repaint
            commandline -f execute
    end
end
