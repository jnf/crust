#!/bin/sh
# MPD convenience aliases and functions via mpc.
# Source from ~/.bashrc on the Pi (or any shell, with MPD_HOST set; see README):
#   . ~/.config/bars/mpc-aliases.sh

# --- Playback ---
alias ms='mpc status'       # what's playing + play state, progress, volume
alias mc='mpc current'      # just the current track name
alias mp='mpc toggle'       # play/pause toggle
alias mn='mpc next'         # next track
alias mb='mpc cdprev'       # back (previous track)
alias mx='mpc stop'         # stop playback

# --- Queue ---
alias mq='mpc playlist'     # show current queue
alias mclear='mpc clear'    # clear the queue
alias mshuffle='mpc shuffle' # shuffle the queue in place

# --- Volume ---
alias mvo='mpc volume'      # mvo 80  →  set absolute; mvo +5 / mvo -5  →  relative
alias mup='mpc volume +5'   # volume up by 5%
alias mdn='mpc volume -5'   # volume down by 5%

# --- Library browsing ---
alias mls='mpc ls'          # list library root; mls "Artist/Album" to drill down

# mfind [tag] <query>  — search library and print matching tracks
#   mfind radiohead              →  search all fields
#   mfind artist radiohead       →  search by tag (artist, album, title, etc.)
mfind() {
    if [ $# -eq 1 ]; then
        mpc search any "$1"
    else
        mpc search "$@"
    fi
}

# madd [tag] <query>  — search and add matching tracks to the current queue
#   madd "ok computer"           →  add all matches for "ok computer"
#   madd album "ok computer"     →  add by album tag specifically
madd() {
    if [ $# -eq 1 ]; then
        mpc searchadd any "$1"
    else
        mpc searchadd "$@"
    fi
}

# mwatch  — print current track whenever it changes; runs until killed (Ctrl-C)
#   mwatch                       →  print to terminal on each player event
#   mwatch title                 →  set terminal title bar instead of printing
mwatch() {
    if [ "${1}" = "title" ]; then
        mpc idleloop player | while read -r _; do
            printf '\033]0;%s\007' "$(mpc current)"
        done
    else
        mpc current
        mpc idleloop player | while read -r _; do
            mpc current
        done
    fi
}

# mplay [tag] <query>  — clear queue, add matching tracks, start playback
#   mplay "in rainbows"          →  clear + add all matches + play
#   mplay artist "radiohead"     →  clear + add by artist + play
mplay() {
    mpc clear
    if [ $# -eq 1 ]; then
        mpc searchadd any "$1"
    else
        mpc searchadd "$@"
    fi
    mpc play
}

# --- Output (which USB card plays; see bars-output on the jukebox) ---

# mout [name|list|auto|use <name>]  — show or switch the playback card
#   mout                         →  list cards: present? enabled? preferred?
#   mout Cans                    →  switch to the cans and remember the choice
#   mout auto                    →  let bars-output pick (what udev and boot run)
# On the jukebox it runs bars-output directly; anywhere else it goes over ssh to
# $MPD_HOST (any password@ prefix and :port stripped).
mout() {
    case "${1:-}" in
        "") set -- list ;;
        list|auto|use) ;;
        *) set -- use "$1" ;;
    esac
    if command -v bars-output >/dev/null 2>&1; then
        bars-output "$@"
    else
        _host=${MPD_HOST:-localhost}; _host=${_host#*@}; _host=${_host%%:*}
        ssh "$_host" bars-output "$@"
    fi
}
