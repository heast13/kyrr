# kyrr for Omarchy's bar

A widget for the bar of [Omarchy](https://omarchy.org) 4 that shows what
[kyrr](https://github.com/heast13/kyrr) is doing: local noise suppression and
dictation for Linux and Windows.

- The waveform lights up in your theme's accent while kyrr listens or writes,
  and in red while it records a voice memo. Dimmed: kyrr is not running.
- Click: open kyrr (starts it when it is not running).
- Middle click: start or stop dictating.

The widget only shows kyrr's state and starts it; it does not install kyrr.
Install kyrr first, see [Installing kyrr](https://github.com/heast13/kyrr/blob/main/docs/install.md).

## Install

```sh
omarchy plugin add https://github.com/heast13/omarchy-kyrr.git --enable
```

Or in the Omarchy menu: Setup > Plugins > Add. Remove it with
`omarchy plugin remove heast13.kyrr`.

## How it works

While kyrr runs on Omarchy it keeps one word in `$XDG_RUNTIME_DIR/kyrr/state`
(`idle`, `listening`, `writing` or `recording`) and deletes the file when it
quits. The widget reads that file; nothing else is read or sent.

## License

MIT, see [LICENSE](LICENSE).
