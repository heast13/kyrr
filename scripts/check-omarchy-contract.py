#!/usr/bin/env python3
"""Checks packaging/omarchy/contract.json against kyrr's code.

On Omarchy kyrr has no window: the bar menu (heast13/omarchy-kyrr) is all of
it, so it must do everything the window does. The contract is what the two
share, and this check keeps it true:

- `commands`: every line kyrr's socket takes from the menu, no more, no less.
- `parity`: every callback of the window's AppState, and every AppState
  property the window writes, with its way in the menu (`plugin`), why the
  menu needs none (`windowOnly`), or what the menu still lacks (`gap`).
  A new setting in the window without an entry fails here, so it cannot
  ship without its counterpart in the menu. `--release` also fails on gaps.
- `state`: a full sample of the state file. Its keys must be exactly the
  keys kyrr writes, so the menu's tests run against what kyrr really sends.

The menu's own CI reads this file from kyrr's main branch and checks that
the menu sends only commands listed here and uses every `plugin` command.

Usage: check-omarchy-contract.py [--release]
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "packaging/omarchy/contract.json"
APP = ROOT / "crates/kyrr-app"
SLINT = APP / "ui/app.slint"
MAIN = APP / "src/main.rs"
SERVERS = APP / "src/servers.rs"
MEMOS = APP / "src/memos.rs"
INSTANCE = ROOT / "crates/kyrr-linux/src/instance.rs"

errors = []


def fail(message):
    errors.append(message)


def function(path, name, indent=""):
    """The text of `fn name` up to the next function at the same indent."""
    text = path.read_text(encoding="utf-8")
    start = re.search(rf"^{indent}(pub )?fn {name}\b", text, re.M)
    if not start:
        sys.exit(f"{path.name}: fn {name} not found (the check needs updating)")
    end = re.compile(rf"^{indent}(pub )?fn \w+|^{indent}}}$", re.M).search(text, start.end())
    return text[start.start() : end.start() if end else len(text)]


def arms(body, pattern):
    """Each match arm in `body` as (names, arm text up to the next arm)."""
    found = list(re.finditer(pattern, body, re.M))
    for i, m in enumerate(found):
        end = found[i + 1].start() if i + 1 < len(found) else len(body)
        yield m, body[m.start() : end]


def handled_commands():
    """Every command line kyrr acts on, with the code that handles it."""
    commands = {}
    command = function(MAIN, "omarchy_command", "    ")
    for m, text in arms(command, r'^\s*\("(\w+)", (?:"([\w.]+)"|(\w+))\)(?: if [^=]+)? =>'):
        verb, what, variable = m.groups()
        if verb == "set":
            continue
        commands[f"{verb} {what}" if what else f"{verb} <{variable}>"] = text
    for body in (function(MAIN, "omarchy_set", "    "), function(SERVERS, "widget_set")):
        for m, text in arms(body, r'^\s*((?:"[\w.]+"(?: \| )?)+)(?: if [^=]+)? =>'):
            for name in re.findall(r'"([\w.]+)"', m.group(1)):
                commands[f"set {name}"] = text
    instance = INSTANCE.read_text(encoding="utf-8")
    raw = re.findall(r'^const \w+: &\[u8\] = b"([\w-]+)";', instance, re.M)
    raw += re.findall(r'\(b"([\w-]+)", Request::', instance)
    for word in raw:
        commands[word] = ""
    if "page-" in instance:
        commands["page-<1-5>"] = ""
    words = re.search(r"const LINE_WORDS: \[&str; \d+\] = \[(.*?)\];", instance, re.S)
    line_words = re.findall(r'"(\w+) "', words.group(1)) if words else []
    return commands, line_words


def app_state():
    """AppState's callbacks, and its properties the window's controls write."""
    text = SLINT.read_text(encoding="utf-8")
    start = text.index("export global AppState {")
    end = re.compile(r"^}", re.M).search(text, start).start()
    block, rest = text[start:end], text[:start] + text[end:]
    callbacks = re.findall(r"^\s*(?:pure )?callback ([\w-]+)", block, re.M)
    written = []
    for name in re.findall(r"^\s*in-out property <[^>]+> ([\w-]+)", block, re.M):
        if re.search(rf"AppState\.{name}\s*(=[^=]|<=>)|<=>\s*AppState\.{name}\b", rest):
            written.append(name)
    return callbacks, written


def snake(name):
    return name.replace("-", "_")


def json_keys(text):
    keys = set(re.findall(r'"(\w+)":', text))
    keys |= set(re.findall(r'insert\(\s*"(\w+)"\.into\(\)', text))
    return keys


def sample_keys(value, skip=()):
    """Every key name in a sample, leaving out the parts under `skip` paths."""
    keys = set()

    def walk(v, path):
        if path in skip:
            return
        if isinstance(v, dict):
            for k, child in v.items():
                keys.add(k)
                walk(child, f"{path}.{k}" if path else k)
        elif isinstance(v, list):
            for child in v:
                walk(child, f"{path}[]")

    walk(value, "")
    return keys


def compare(what, written, sample):
    for key in sorted(written - sample):
        fail(f"state: kyrr writes `{key}` ({what}), but the sample in contract.json lacks it")
    for key in sorted(sample - written):
        fail(f"state: the sample has `{key}` ({what}), but kyrr no longer writes it")


def main():
    release = "--release" in sys.argv[1:]
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))

    commands, line_words = handled_commands()
    listed = contract["commands"]
    for name in sorted(set(commands) - set(listed)):
        fail(f"commands: kyrr handles `{name}`, add it to contract.json")
    for name in sorted(set(listed) - set(commands)):
        fail(f"commands: contract.json lists `{name}`, but kyrr does not handle it")
    for name in listed:
        verb = name.split(" ")[0]
        if " " in name and verb not in line_words:
            fail(f"commands: `{name}` starts with `{verb}`, which the socket drops (LINE_WORDS)")

    callbacks, written = app_state()
    entries = {f"callback {c}": c for c in callbacks} | {f"property {p}": p for p in written}
    parity = contract["parity"]
    for entry in sorted(set(entries) - set(parity)):
        fail(
            f"parity: the window has `{entry}` but contract.json does not say how the "
            "Omarchy menu does the same (plugin), why it needs nothing (windowOnly) "
            "or that it is missing there (gap)"
        )
    for entry in sorted(set(parity) - set(entries)):
        fail(f"parity: contract.json has `{entry}`, which the window no longer has")
    gaps = []
    for entry, way in sorted(parity.items()):
        if entry not in entries:
            continue
        kinds = [k for k in ("plugin", "windowOnly", "gap") if k in way]
        if len(kinds) != 1:
            fail(f"parity: `{entry}` needs exactly one of plugin, windowOnly, gap")
            continue
        if "gap" in way:
            gaps.append(f"{entry}: {way['gap']}")
            continue
        if "windowOnly" in way:
            continue
        name = snake(entries[entry])
        marks = [rf"\binvoke_{name}\b", rf"\bset_{name}\b"] + ([re.escape(way["via"])] if "via" in way else [])
        cmds = way["plugin"]
        for cmd in cmds:
            if cmd not in commands:
                fail(f"parity: `{entry}` names `{cmd}`, which kyrr does not handle")
        if not any(re.search(mark, commands.get(cmd, "")) for cmd in cmds for mark in marks):
            fail(
                f"parity: none of {cmds} reaches `{entry}` (looked for {', '.join(marks)}); "
                "give the command that does, or `via` with the function it calls"
            )

    state = contract["state"]
    if state.get("v") != 2:
        fail("state: the sample must be version 2 (`v`)")
    main_text = function(MAIN, "tell_widget") + function(
        ROOT / "crates/kyrr-linux/src/omarchy.rs", "state_json"
    )
    main_keys = json_keys(main_text) | {"v", "pid", "mode"}
    compare(
        "tell_widget",
        main_keys,
        sample_keys(state, {"ai", "memos.sheet.rework", "memos.list[]"}),
    )
    ai_text = function(SERVERS, "widget_state") + SERVERS.read_text(encoding="utf-8").split(
        "impl ServerShown", 1
    )[1].split("\n}\n", 1)[0]
    compare("servers::widget_state", json_keys(ai_text), sample_keys(state["ai"]))
    compare(
        "servers::widget_memo_rework",
        json_keys(function(SERVERS, "widget_memo_rework")),
        sample_keys(state["memos"]["sheet"]["rework"]),
    )
    compare(
        "memos::widget_list",
        json_keys(function(MEMOS, "widget_list", "    ")),
        sample_keys(state["memos"]["list"]),
    )

    if gaps:
        print("Open gaps, the Omarchy menu cannot do these yet:")
        for gap in gaps:
            print(f"  {gap}")
        if release:
            fail(f"{len(gaps)} parity gap(s): a release needs the menu to do everything the window does")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        sys.exit(1)
    print(
        f"Omarchy contract holds: {len(commands)} commands, {len(entries)} window controls, "
        f"{len(gaps)} gap(s)."
    )


if __name__ == "__main__":
    main()
