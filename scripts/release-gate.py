#!/usr/bin/env python3
"""kyrr's release gate, the mechanical part: the rules a review by feel kept
missing, checked the same way every time. Each check prints `PASS name`,
`FAIL name: reason` or `WARN name: reason`, with the places it found under it
(file:line). The exit code is 1 when any check FAILs. Nothing is changed.

Default mode runs on every pull request. --release adds the checks that only
hold once a release PR has bumped the version: one version everywhere, the
NEWS rules, and the Omarchy contract without gaps. --online requests every
external link (FAIL only on 404 and 410). --plugin DIR checks a checkout of
heast13/omarchy-kyrr: its manifest, and a version bump after changes.

  python3 scripts/release-gate.py
  python3 scripts/release-gate.py --release --online --plugin ../omarchy-kyrr

The rules are described in docs/release-gate.md.
"""

import argparse
import concurrent.futures
import json
import re
import subprocess
import sys
import tomllib
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APP_SLINT = ROOT / "crates/kyrr-app/ui/app.slint"
PO = ROOT / "crates/kyrr-app/ui/lang/de/LC_MESSAGES/kyrr.po"
CARGO = ROOT / "Cargo.toml"
LOCK = ROOT / "Cargo.lock"
METAINFO = ROOT / "packaging/linux/io.github.heast13.kyrr.metainfo.xml"
CHANGELOG = ROOT / "CHANGELOG.md"
NEWS = ROOT / "NEWS.md"
README = ROOT / "README.md"
SITE_DE = ROOT / "site/de/index.html"
OMARCHY = ROOT / "scripts/check-omarchy-contract.py"
PLUGIN_ID = "io.github.heast13.kyrr"

# Flo's word rules. Rauschunterdrückung is a FAIL. Diktat for the feature is
# only a WARN, because a dictated text can be called "Diktat" too.
RAUSCH = re.compile("Rauschunterdrückung")
DIKTAT = re.compile(r"\bDiktat(?:e|es|en|s)?\b")
FLATPAK = re.compile("flatpak", re.I)
# App texts for normal users: no file paths and no shell commands.
PATHS_AND_COMMANDS = re.compile(r"/home/|~/|C:\\|%APPDATA%|\.config/|\bsudo |\bpacman |\bapt |\bsystemctl ")
# The text Windows sees must never say that nothing changes there.
NOT_FOR_WINDOWS = {
    "English": re.compile(
        r"nothing (?:changes|is changed|is different)|changes nothing|windows (?:is )?(?:unchanged|not affected)"
        r"|\bonly (?:on )?linux\b",
        re.I,
    ),
    "Deutsch": re.compile(
        r"(?:ändert|verändert) sich (?:\w+ )?nichts|nichts (?:ändert sich|verändert sich|bleibt)"
        r"|windows (?:ist )?nicht betroffen|\bnur (?:unter |auf )?linux\b",
        re.I,
    ),
}
TR = re.compile(r'@tr\(\s*"((?:[^"\\]|\\.)*)"')
PLACEHOLDER = re.compile(r"\{[^{}]*\}")
MARK = re.compile(r"^- \((Windows|Linux)\) ")
# href, src and markdown links and images; the URL is group 1 or group 2.
LINK = re.compile(r"""\b(?:href|src)\s*=\s*["']([^"']+)["']|\]\(\s*<?([^)\s>]+)""")

results = []  # the verdict of each check, for the summary


def rel(path):
    return Path(path).relative_to(ROOT).as_posix()


def report(status, name, reason="", details=()):
    """Prints one check's verdict and, under it, the places it found."""
    results.append(status)
    print(f"{status} {name}" + (f": {reason}" if reason else ""))
    for line in details:
        print(f"    {line}")


def conclude(name, problems, why, warn=False):
    """PASS when nothing was found, else FAIL (or WARN) with the places."""
    if problems:
        report("WARN" if warn else "FAIL", name, f"{len(problems)} {why}", problems)
    else:
        report("PASS", name)


def unescape(text):
    """Text between quotes, as Slint and gettext both write it: \\n, \\t, \\"
    and \\\\. Normalised to NFC so that umlauts compare equal."""

    def one(m):
        esc = m.group(1)
        if esc.startswith("u{"):
            return chr(int(esc[2:-1], 16))
        return {"n": "\n", "t": "\t"}.get(esc, esc)

    return unicodedata.normalize("NFC", re.sub(r"\\(u\{[0-9a-fA-F]+\}|.)", one, text, flags=re.S))


def quoted(text):
    """The unescaped contents of one quoted gettext string."""
    return unescape(text.strip()[1:-1])


def short(text):
    """A message for the report: quoted and cut to 60 characters."""
    cut = "..." if len(text) > 60 else ""
    return '"' + text.replace("\n", " ")[:60] + cut + '"'


def scan(units, regex):
    """`place: excerpt` for each (place, text) unit where a regex hits, once per unit."""
    hits = []
    for place, text in units:
        m = regex.search(text)
        if m:
            start, end = max(m.start() - 40, 0), m.end() + 40
            hits.append(f"{place}: ...{' '.join(text[start:end].split())}...")
    return hits


def text_units(paths):
    """(place, line) for every line of the text files under the paths."""
    for root in paths:
        for path in sorted(root.rglob("*") if root.is_dir() else [root]):
            if not path.is_file():
                continue
            try:
                lines = path.read_text(encoding="utf-8").splitlines()
            except UnicodeDecodeError:
                continue  # images, videos, archives
            for no, line in enumerate(lines, 1):
                yield f"{rel(path)}:{no}", unicodedata.normalize("NFC", line)


def deutsch_units(path):
    """(place, line) of the newest `### Deutsch` block of NEWS.md: the notes of
    released versions stay as they were published."""
    inside = False
    sections = 0
    for no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.startswith("## "):
            inside = False
            sections += 1
            if sections > 1:
                return
        elif line.strip() == "### Deutsch":
            inside = True
        elif inside:
            yield f"{rel(path)}:{no}", unicodedata.normalize("NFC", line)


def new_entry():
    return {"msgid": None, "msgstr": "", "fuzzy": False, "line": 0, "msgstr_line": 0}


def read_po():
    """kyrr.po's entries with msgid, msgstr, fuzzy flag and line numbers.
    Obsolete (#~) entries and the header (empty msgid) are left out."""
    entries, cur, field = [], new_entry(), None
    for no, raw in enumerate(PO.read_text(encoding="utf-8").splitlines() + [""], 1):
        line = raw.strip()
        if not line or (line.startswith("msgid ") and cur["msgid"] is not None):
            if cur["msgid"]:
                entries.append(cur)
            cur, field = new_entry(), None
        if not line or line.startswith("#~") or (line.startswith("#") and not line.startswith("#,")):
            continue
        if line.startswith("#,"):
            cur["fuzzy"] = cur["fuzzy"] or "fuzzy" in line
        elif line.startswith("msgid "):
            field, cur["msgid"], cur["line"] = "msgid", quoted(line[6:]), no
        elif line.startswith("msgstr "):
            field, cur["msgstr"], cur["msgstr_line"] = "msgstr", quoted(line[7:]), no
        elif line.startswith('"') and field:
            cur[field] += quoted(line)
    return entries


def slint_strings():
    """(line, msgid) for every @tr in app.slint."""
    text = APP_SLINT.read_text(encoding="utf-8")
    return [(text.count("\n", 0, m.start()) + 1, unescape(m.group(1))) for m in TR.finditer(text)]


def check_translations():
    """Every @tr string has a finished German entry, and every entry a @tr."""
    problems, po = [], {}
    for entry in read_po():
        if entry["msgid"] in po:
            problems.append(f"{rel(PO)}:{entry['line']}: duplicate entry for {short(entry['msgid'])}")
        po[entry["msgid"]] = entry
    trs = {}
    for no, msgid in slint_strings():
        trs.setdefault(msgid, no)
    for msgid, no in trs.items():
        place = f"{rel(APP_SLINT)}:{no}"
        entry = po.get(msgid)
        if entry is None:
            problems.append(f"{place}: no entry in kyrr.po for {short(msgid)}")
        elif not entry["msgstr"].strip():
            problems.append(f"{place}: empty msgstr for {short(msgid)}")
        else:
            if entry["fuzzy"]:
                problems.append(f"{place}: fuzzy entry for {short(msgid)}")
            if len(PLACEHOLDER.findall(msgid)) != len(PLACEHOLDER.findall(entry["msgstr"])):
                problems.append(f"{place}: placeholder count differs for {short(msgid)}")
    for msgid, entry in po.items():
        if msgid not in trs:
            problems.append(f"{rel(PO)}:{entry['line']}: no @tr in app.slint for {short(msgid)}")
    conclude("translations", problems, "translation problem(s)")


def check_words():
    """German texts: no Rauschunterdrückung (FAIL), no Diktat for the feature (WARN)."""
    german = [(f"{rel(PO)}:{e['msgstr_line']}", e["msgstr"]) for e in read_po()]
    german += list(text_units([SITE_DE, README])) + list(deutsch_units(NEWS))
    conclude("words", scan(german, RAUSCH), "line(s) with Rauschunterdrückung (write Geräuschunterdrückung)")
    conclude("words-diktat", scan(german, DIKTAT), "Diktat word(s) to look at (the feature is Diktierfunktion)", warn=True)


def check_flatpak():
    """Flatpak was dropped for good: not in the README, NEWS, site or docs."""
    # The gate's own briefs state the rule, so they name it.
    units = text_units([README, NEWS, ROOT / "site", ROOT / "docs"])
    hits = scan((u for u in units if not u[0].startswith("docs/release-gate")), FLATPAK)
    conclude("flatpak", hits, "line(s) with Flatpak, which was dropped for good")


def check_texts():
    """App texts for normal users: no file paths and no shell commands (WARN)."""
    units = [(f"{rel(APP_SLINT)}:{no}", msgid) for no, msgid in slint_strings()]
    units += [(f"{rel(PO)}:{e['msgstr_line']}", e["msgstr"]) for e in read_po()]
    conclude("texts", scan(units, PATHS_AND_COMMANDS), "text(s) with a path or a command", warn=True)


def link_files():
    return [README, *sorted((ROOT / "docs").glob("*.md")), ROOT / "site/index.html", SITE_DE]


def links_in(path):
    """(line, url) for every href, src and markdown link in a file."""
    text = path.read_text(encoding="utf-8")
    for m in LINK.finditer(text):
        yield text.count("\n", 0, m.start()) + 1, (m.group(1) or m.group(2)).strip()


def link_answer(url):
    """(HTTP status or None, text): HEAD first, GET when HEAD gets no answer."""
    answer = (None, "no answer")
    for method in ("HEAD", "GET"):
        request = urllib.request.Request(url, method=method, headers={"User-Agent": "kyrr-release-gate"})
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                return response.status, f"HTTP {response.status}"
        except urllib.error.HTTPError as err:
            answer = (err.code, f"HTTP {err.code}")
        except (OSError, ValueError) as err:  # URLError and timeouts are OSErrors
            answer = (None, f"no answer: {err}")
    return answer


def check_links(online):
    """Relative links and images must exist; with --online, external ones answer."""
    links = [(path, no, url) for path in link_files() for no, url in links_in(path)]
    broken = []
    for path, no, url in links:
        if re.match(r"[a-z][a-z0-9+.-]*:|//", url, re.I) or url.startswith("#"):
            continue  # http, mailto and the like, or an anchor in the same page
        target = urllib.parse.unquote(url.split("#")[0].split("?")[0])
        if not target:
            continue
        base = ROOT if target.startswith("/") else path.parent
        if not (base / target.lstrip("/")).exists():
            broken.append(f"{rel(path)}:{no}: {url} does not exist")
    conclude("links", broken, "broken relative link(s)")
    if online:
        check_online(links)


def check_online(links):
    """Each external URL once: 404 and 410 FAIL, other failures WARN."""
    urls = {}
    for path, no, url in links:
        if url.startswith(("http://", "https://")):
            urls.setdefault(url, []).append(f"{rel(path)}:{no}")
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        answers = list(pool.map(link_answer, urls))
    dead, unsure = [], []
    for (url, places), (code, text) in zip(urls.items(), answers):
        line = f"{', '.join(dict.fromkeys(places))}: {url} ({text})"
        if code in (404, 410):
            dead.append(line)
        elif code is None or code >= 400:
            unsure.append(line)
    if dead:
        report("FAIL", "links-online", f"{len(dead)} dead external link(s)", dead + unsure)
    else:
        conclude("links-online", unsure, "external link(s) that did not answer", warn=True)


def check_omarchy(release):
    """The Omarchy contract; in release mode its gaps fail too."""
    name = "omarchy (release)" if release else "omarchy"
    cmd = [sys.executable, str(OMARCHY), *(["--release"] if release else [])]
    run = subprocess.run(cmd, capture_output=True, text=True)
    output = (run.stdout + run.stderr).strip().splitlines()
    if run.returncode:
        report("FAIL", name, f"the contract check exited {run.returncode}", output)
    else:
        report("PASS", name, details=output)


def cargo_version():
    return tomllib.loads(CARGO.read_text(encoding="utf-8"))["workspace"]["package"]["version"]


def crate_names():
    """The workspace crates' names, from crates/*/Cargo.toml."""
    paths = sorted((ROOT / "crates").glob("*/Cargo.toml"))
    return [tomllib.loads(p.read_text(encoding="utf-8"))["package"]["name"] for p in paths]


def changelog_problems(version):
    """No open Unreleased section; the newest version heading with a valid date."""
    text = CHANGELOG.read_text(encoding="utf-8")
    heads = list(re.finditer(r"^## (.+?)[ \t]*$", text, re.M))
    problems = []
    for i, head in enumerate(heads):
        end = heads[i + 1].start() if i + 1 < len(heads) else len(text)
        if head.group(1) == "Unreleased" and text[head.end():end].strip():
            problems.append("CHANGELOG.md: the Unreleased section is not empty")
    top = next((h.group(1) for h in heads if h.group(1) != "Unreleased"), "")
    match = re.fullmatch(r"(\d+\.\d+\.\d+) \((\d{4}-\d{2}-\d{2})\)", top)
    if match is None:
        return problems + [f"CHANGELOG.md: the newest heading is `## {top}`, not `## X.Y.Z (YYYY-MM-DD)`"]
    if match.group(1) != version:
        problems.append(f"CHANGELOG.md: the newest version is {match.group(1)}, expected {version}")
    try:
        date.fromisoformat(match.group(2))
    except ValueError:
        problems.append(f"CHANGELOG.md: {match.group(2)} is not a valid date")
    return problems


def check_versions(version):
    """One version everywhere: Cargo.toml, Cargo.lock, metainfo, CHANGELOG, NEWS."""
    problems = []
    lock = tomllib.loads(LOCK.read_text(encoding="utf-8")).get("package", [])
    lock = [p for p in lock if "source" not in p]  # the workspace's own crates
    for crate in crate_names():
        found = [p["version"] for p in lock if p["name"] == crate]
        if found != [version]:
            problems.append(f"Cargo.lock: {crate} is {found or 'missing'}, expected {version}")
    newest = ET.parse(METAINFO).getroot().find(".//release")
    if newest is None:
        problems.append("metainfo: no <release> entry")
    else:
        if newest.get("version") != version:
            problems.append(f"metainfo: the newest release is {newest.get('version')}, expected {version}")
        description = newest.find("description")
        if description is None or not "".join(description.itertext()).strip():
            problems.append("metainfo: the newest release has no description")
        if not (newest.findtext("url[@type='details']") or "").strip():
            problems.append("metainfo: the newest release has no details url")
    problems += changelog_problems(version)
    news = re.search(r"^## (\S+)[ \t]*$", NEWS.read_text(encoding="utf-8"), re.M)
    if news is None or news.group(1) != version:
        found = news.group(1) if news else "nothing"
        problems.append(f"NEWS.md: the first section is {found}, expected {version}")
    conclude("versions", problems, "version problem(s)")


def check_screenshots(version):
    """Screenshots should point at the tag vX.Y.Z, not at a commit (WARN)."""
    images = re.findall(r"<image>\s*(\S+?)\s*</image>", METAINFO.read_text(encoding="utf-8"))
    loose = [f"{rel(METAINFO)}: {url}" for url in images if f"/kyrr/v{version}/" not in url]
    conclude("screenshots", loose, f"screenshot(s) not at tag v{version}", warn=True)


def item_marks(body):
    """The mark of each item of a NEWS block: "Windows", "Linux" or None."""
    marks = []
    for line in body.splitlines():
        if line.startswith("- "):
            mark = MARK.match(line)
            marks.append(mark.group(1) if mark else None)
    return marks


def for_system(body, system):
    """The lines of a NEWS block that one system sees, as kyrr reads them: an
    item marked for the other system goes with its continuation lines."""
    out, keep = [], True
    for line in body.splitlines():
        if line.startswith("- "):
            mark = MARK.match(line)
            keep = mark is None or mark.group(1) == system
        elif not line.strip():
            keep = True
        if keep:
            out.append(line)
    return "\n".join(out)


def check_news(version):
    """The NEWS.md section of this version, under the rules of its intro."""
    text = NEWS.read_text(encoding="utf-8")
    section = re.search(rf"^## {re.escape(version)}[ \t]*$\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if section is None:
        conclude("news", [f"NEWS.md has no `## {version}` section"], "problem(s)")
        return
    parts = re.split(r"^### Deutsch[ \t]*$", section.group(1), maxsplit=1, flags=re.M)
    problems = [] if len(parts) == 2 else ["NEWS.md: no `### Deutsch` block"]
    blocks = {"English": parts[0].strip(), "Deutsch": parts[1].strip() if len(parts) == 2 else ""}
    for lang, body in blocks.items():
        if not body:
            problems.append(f"{lang}: no items")
            continue
        if len(body) > 2000:
            problems.append(f"{lang}: {len(body)} characters, the limit is 2000")
        if "-->" in body:
            problems.append(f"{lang}: contains '-->'")
        marks = item_marks(body)
        for system in ("Windows", "Linux"):
            if not any(mark in (None, system) for mark in marks):
                problems.append(f"{lang}: no item that {system} sees")
        for line in for_system(body, "Windows").splitlines():
            if NOT_FOR_WINDOWS[lang].search(line):
                problems.append(f"{lang}, Windows sees: {line.strip()}")
    en, de = item_marks(blocks["English"]), item_marks(blocks["Deutsch"])
    if len(en) != len(de):
        problems.append(f"{len(en)} English items, but {len(de)} Deutsch items")
    elif en != de:
        problems.append("the items are marked differently in English and Deutsch")
    conclude("news", problems, "NEWS problem(s) for this version")


def git(directory, *args):
    """The output of a git command in the plugin checkout, or None on failure."""
    run = subprocess.run(["git", "-C", str(directory), *args], capture_output=True, text=True)
    return run.stdout.strip() if run.returncode == 0 else None


def check_plugin(directory):
    """The Omarchy plugin checkout: a valid manifest, and a version bump after changes."""
    try:
        manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    except (OSError, ValueError) as err:
        conclude("plugin", [f"{directory / 'manifest.json'}: {err}"], "problem(s) with manifest.json")
        return
    if not isinstance(manifest, dict):
        manifest = {}
    version = manifest.get("version")
    problems = []
    if manifest.get("id") != PLUGIN_ID:
        problems.append(f"manifest.json: the id is not {PLUGIN_ID}")
    if not isinstance(version, str) or not version:
        problems.append("manifest.json: no version")
    conclude("plugin", problems, "problem(s) with manifest.json")
    # The bump check compares with the manifest at the last tag.
    tag = git(directory, "describe", "--tags", "--abbrev=0") or ""
    old = git(directory, "show", f"{tag}:manifest.json") if tag else None
    try:
        old_version = json.loads(old).get("version") if old else None
    except (ValueError, AttributeError):
        old_version = None
    if not isinstance(version, str) or old_version is None:
        report("WARN", "plugin-bump", f"no tag with a manifest in {directory}, the version bump was not checked")
        return
    changed = git(directory, "rev-list", f"{tag}..HEAD")
    if changed and old_version == version:
        report("WARN", "plugin-bump", f"{len(changed.split())} commit(s) since {tag}, version still {version}")
    else:
        report("PASS", "plugin-bump")


def main():
    parser = argparse.ArgumentParser(description="kyrr's release gate: the mechanical checks.")
    parser.add_argument("--release", action="store_true", help="add the release checks: versions, NEWS, Omarchy gaps")
    parser.add_argument("--online", action="store_true", help="also request every external link")
    parser.add_argument("--plugin", metavar="DIR", help="a checkout of heast13/omarchy-kyrr to check")
    args = parser.parse_args()
    print(f"release gate, {'release' if args.release else 'default'} mode")
    check_translations()
    check_words()
    check_flatpak()
    check_texts()
    check_links(args.online)
    check_omarchy(args.release)
    if args.release:
        version = cargo_version()
        check_versions(version)
        check_screenshots(version)
        check_news(version)
    if args.plugin:
        check_plugin(Path(args.plugin).resolve())
    counts = {status: results.count(status) for status in ("PASS", "WARN", "FAIL")}
    print(f"\nsummary: {counts['PASS']} PASS, {counts['WARN']} WARN, {counts['FAIL']} FAIL")
    sys.exit(1 if counts["FAIL"] else 0)


if __name__ == "__main__":
    main()
