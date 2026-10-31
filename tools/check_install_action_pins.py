#!/usr/bin/env python3
"""Fail when a `taiki-e/install-action` step installs a tool without an exact version.

An unversioned `tool:` installs whatever release is current when the job runs, so an upstream change
or a compromised release reaches the gate, the release job or the nightly build without review. Every
tool must be written `name@X.Y.Z`. Dependabot does not read these values, so a new version is an edit
made in a pull request.

`python3 tools/check_install_action_pins.py [DIR]` checks every workflow under DIR (default
`.github/workflows`), prints one line per problem and exits 1 when a step is not pinned or when no
install-action step was found at all. The unit tests in test_check_install_action_pins.py run the
same check over the real workflows and over fixtures.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
INSTALL_ACTION = re.compile(r'uses:\s*taiki-e/install-action@')
TOOL = re.compile(r'(?:^|[\s{,])tool:\s*(?P<value>[^#]*)')
PINNED = re.compile(r'\A[a-z0-9][a-z0-9_-]*@[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?\Z')


def indent_of(line):
    return len(line) - len(line.lstrip(' '))


def step_lines(lines, start):
    """The lines of the mapping that holds the `uses:` line at `start`, up to the next step."""
    key_column = indent_of(lines[start]) + (2 if lines[start].lstrip().startswith('- ') else 0)
    block = [lines[start]]
    for line in lines[start + 1:]:
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        if indent_of(line) < key_column:
            break
        block.append(line)
    return block


def tool_names(block):
    """Every tool named by a `tool:` input in the block; `{ tool: x }` and comma lists both work."""
    names = []
    for line in block:
        match = TOOL.search(line)
        if match:
            value = match.group('value').split('}')[0]
            names.extend(part.strip() for part in value.split(',') if part.strip())
    return names


def problems_in_text(text, label):
    """Return (install-action steps seen, one message per problem) for one workflow file."""
    lines = text.splitlines()
    checked = 0
    found = []
    for index, line in enumerate(lines):
        if line.lstrip().startswith('#') or not INSTALL_ACTION.search(line):
            continue
        checked += 1
        where = f'{label}:{index + 1}'
        names = tool_names(step_lines(lines, index))
        if not names:
            found.append(f'{where}: install-action step has no `tool:` input')
        for name in names:
            if not PINNED.match(name):
                found.append(f'{where}: `{name}` is not pinned to an exact version; write `name@X.Y.Z`')
    return checked, found


def check_dir(directory):
    """Return (install-action steps seen, problems) for every workflow file in `directory`."""
    checked = 0
    found = []
    for path in sorted(Path(directory).glob('*.yml')) + sorted(Path(directory).glob('*.yaml')):
        count, problems = problems_in_text(path.read_text(encoding='utf-8'), path.name)
        checked += count
        found.extend(problems)
    return checked, found


def main(argv):
    directory = Path(argv[1]) if len(argv) > 1 else WORKFLOWS
    checked, found = check_dir(directory)
    for problem in found:
        print(problem, file=sys.stderr)
    if checked == 0:
        print(f'no install-action steps found under {directory}', file=sys.stderr)
        return 1
    if found:
        return 1
    print(f'{checked} install-action steps pinned under {directory}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
