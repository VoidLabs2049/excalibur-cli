# Excalibur - Fish Shell Integration
#
# Function name: exh (excalibur history)
#
# Thin alias over `ex` (see ex.fish), which owns the exit-code protocol.

function exh --description "Interactive command history browser (Excalibur)"
    ex h
end

# Bind to Ctrl+R (overwrites default Fish history search)
bind \cr exh

# Optional: Bind to Ctrl+H as well
# bind \ch exh
