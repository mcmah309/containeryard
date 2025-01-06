#!/bin/sh
set -e
set -u

touch /root/.bash_history

apt-get install -y bsdmainutils

apt-get install -y hstr

cat > "/root/.bashrc" <<- 'EOM'
# ensure synchronization between bash memory and history file
export PROMPT_COMMAND="history -a; history -n; export HISTFILE=/root/.bash_history"
shopt -s histappend              # append new history items to .bash_history

function git_branch() {
    branch=$(git branch 2>/dev/null | grep '^*' | colrm 1 2 || echo "")
    if [ ! -z "$branch" ]; then
        echo " ($branch)"
    fi
}
# PS1 is the prompt (e.g. "user@host:directory (branch)$ ")
export PS1="\[\033[36m\]\u@c-\h\[\033[00m\]:\[\033[33m\]\w\[\033[32m\]\$(git_branch)\[\033[00m\]\$ "

### For hstr
alias hh=hstr                    # hh to be alias for hstr
export HSTR_CONFIG=hicolor       # get more colors
export HISTCONTROL=ignorespace   # leading space hides commands from history
export HISTFILESIZE=10000        # increase history file size (default is 500)
export HISTSIZE=${HISTFILESIZE}  # increase history size (default is 500)
# if this is interactive shell, then bind hstr to Ctrl-r (for Vi mode check doc)
if [[ $- =~ .*i.* ]]; then bind '"\C-r": "\C-a hstr -- \C-j"'; fi
# if this is interactive shell, then bind 'kill last command' to Ctrl-x k
if [[ $- =~ .*i.* ]]; then bind '"\C-xk": "\C-a hstr -k \C-j"'; fi
export HSTR_TIOCSTI=y

alias l='ls -lah'
EOM