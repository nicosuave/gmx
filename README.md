# gmx

gmx is a terminal multiplexer for ghostty, built on zmx and Ghostty's recently exposed Applescript capabilities. Native Ghostty splits + persistent sessions in plain old Ghostty.

It aims to be vaguely tmux compatible but does not aim to be 100% tmux compatible.

## Install

```
brew tap nicosuave/tap/gmx
```

This installs zmx as a dependency.

## Usage

```
gmx - Ghostty Multiplexer with zmx session persistence

Usage: gmx <command> [args]

Commands:
  [n]ew <name> [--tab] [--remote R] [--dir D]   Create a new session
  [a]ttach <name> [--tab]                       Reattach to a session
  [k]ill <name>                                 Kill a session and its zmx sessions
  [l]s                                          List sessions
  [s]plit [right|down]                           Add a split to the current session
  [r]ename <old> <new>                           Rename a session
  [c]onfig remote <name> <host>                  Configure a remote host
  key[b]inds install|uninstall|show              Manage Ghostty keybindings

By default, new and attach work in the current terminal.
Use --tab to open in a new Ghostty tab instead.
```

Create a session and start working:

```bash
gmx n myproject              # creates zmx session, attaches in current terminal
gmx s right                  # split right, new zmx pane
gmx s down                   # split down, another pane
```

Close the tab. Sessions persist. Come back later:

```bash
gmx l                        # list sessions
gmx a myproject              # reattach in current terminal
gmx a myproject --tab        # or recreate full layout in new Ghostty tab
```

## Keybindings

gmx can install Ghostty keybindings for you:

```bash
gmx b install                # ctrl+shift+d/e/t/a/x/s
gmx b install --prefix ctrl+b   # tmux-style: ctrl+b then d/e/c/a/x/s
gmx b uninstall              # remove them
gmx b show                   # dry run
```

The `text:` keybindings send commands to your shell, so they work at a prompt but not inside vim/htop. For that, use [Hammerspoon](https://www.hammerspoon.org) global hotkeys:

```lua
-- ~/.hammerspoon/init.lua
hs.hotkey.bind({'ctrl','shift'}, 'd', function()
  hs.execute('/opt/homebrew/bin/gmx split right', true)
end)
```

## Remote sessions

Configure a remote host, then create sessions that SSH in and run zmx on the remote:

```bash
gmx c remote nicbook nicbook --transport ssh
gmx n backend --remote nicbook --dir ~/Code/backend
gmx s right                  # splits also SSH to remote
```

Mosh is supported too: `gmx c remote myhost myhost.local --transport mosh`

Sessions persist on the remote via zmx. Close the tab, reattach later, the remote shell state is still there.

## How it works

gmx uses Ghostty's AppleScript API to create tabs and splits with explicit surface configurations (environment variables, working directory). Each pane runs `zmx attach <session>` for persistence. Sessions are named `<name>.1`, `<name>.2`, etc.

No shell wrapper. No Ghostty fork. Stock Ghostty + zmx.
