import tempfile
import unittest
from pathlib import Path

import check_install_action_pins as check

PIN = 'taiki-e/install-action@645d1ab48b3cb8591684a8b730b2ccf4cb3f9568 # v2.87.33'


def block_step(tool_line, indent='      '):
    return f'{indent}- uses: {PIN}\n{indent}  with:\n{indent}    {tool_line}\n'


def inline_step(with_line):
    return f'      - uses: {PIN}\n        with: {{ {with_line} }}\n'


class CheckInstallActionPins(unittest.TestCase):
    def test_real_workflows_pin_every_install_action_step(self):
        checked, found = check.check_dir(check.WORKFLOWS)
        self.assertGreater(checked, 0, 'no install-action step was scanned')
        self.assertEqual(found, [])

    def test_block_form_with_exact_version_passes(self):
        text = 'jobs:\n  a:\n    steps:\n' + block_step('tool: cargo-deny@0.20.2')
        self.assertEqual(check.problems_in_text(text, 'a.yml'), (1, []))

    def test_block_form_without_version_is_reported_with_its_line(self):
        text = 'jobs:\n  a:\n    steps:\n' + block_step('tool: cargo-deny')
        self.assertEqual(
            check.problems_in_text(text, 'a.yml')[1],
            ['a.yml:4: `cargo-deny` is not pinned to an exact version; write `name@X.Y.Z`'],
        )

    def test_inline_form_is_checked_too(self):
        self.assertEqual(check.problems_in_text(inline_step('tool: cargo-deny@0.20.2'), 'b.yml'), (1, []))
        self.assertEqual(len(check.problems_in_text(inline_step('tool: cargo-deny'), 'b.yml')[1]), 1)

    def test_latest_and_expressions_are_not_pins(self):
        for value in ('tool: typos@latest', 'tool: typos@1', 'tool: ${{ env.TOOL }}', 'tool: typos@^1.51.1'):
            with self.subTest(value=value):
                problems = check.problems_in_text(block_step(value), 'c.yml')[1]
                self.assertEqual(len(problems), 1)

    def test_comma_separated_tools_are_each_checked(self):
        text = block_step('tool: cargo-deny@0.20.2, typos')
        problems = check.problems_in_text(text, 'd.yml')[1]
        self.assertEqual(len(problems), 1)
        self.assertIn('`typos`', problems[0])

    def test_step_without_tool_input_is_reported(self):
        text = f'      - uses: {PIN}\n        with:\n          other: 1\n'
        self.assertEqual(
            check.problems_in_text(text, 'e.yml')[1],
            ['e.yml:1: install-action step has no `tool:` input'],
        )

    def test_a_tool_input_does_not_leak_into_the_next_step(self):
        text = (
            '    steps:\n'
            f'      - uses: {PIN}\n'
            '        with:\n'
            '          tool: cargo-deny@0.20.2\n'
            f'      - uses: {PIN}\n'
            '        with:\n'
            '          other: 1\n'
        )
        self.assertEqual(
            check.problems_in_text(text, 'f.yml')[1],
            ['f.yml:5: install-action step has no `tool:` input'],
        )

    def test_if_then_uses_layout_is_checked(self):
        text = (
            '      - if: matrix.os == \'ubuntu-latest\'\n'
            f'        uses: {PIN}\n'
            '        with:\n'
            '          tool: typos\n'
        )
        problems = check.problems_in_text(text, 'g.yml')[1]
        self.assertEqual(len(problems), 1)
        self.assertIn('`typos`', problems[0])

    def test_commented_out_install_action_is_ignored(self):
        text = f'      # - uses: {PIN}\n      #   with: {{ tool: typos }}\n'
        self.assertEqual(check.problems_in_text(text, 'h.yml'), (0, []))

    def test_directory_scan_names_the_failing_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'good.yml').write_text('jobs:\n' + block_step('tool: typos@1.51.1'), encoding='utf-8')
            Path(tmp, 'bad.yaml').write_text('jobs:\n' + block_step('tool: typos'), encoding='utf-8')
            checked, found = check.check_dir(tmp)
        self.assertEqual(checked, 2)
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0].startswith('bad.yaml:'))


if __name__ == '__main__':
    unittest.main()
