import json
import tempfile
import unittest
from pathlib import Path

import gen_harness_packs as g

REPO_ROOT = Path(__file__).resolve().parent.parent
MCP_TEXT = (REPO_ROOT / g.MCP_SOURCE).read_text(encoding="utf-8")
SERVED = g.served_tool_names(MCP_TEXT)
REMOVED_SUBSCRIBE_ARM = '            Tool::Subscribe => "lattice_subscribe",\n'


def write(root, rel, text):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class ServedToolNames(unittest.TestCase):
    def test_the_real_source_serves_the_three_tools(self):
        self.assertEqual(g.served_tool_names(MCP_TEXT), {"lattice_read", "lattice_write", "lattice_subscribe"})

    def test_planted_pack_names_removed_tool_is_refused(self):
        # PackNamesRemovedTool: the server stops serving lattice_subscribe, a pack still names it.
        planted = MCP_TEXT.replace(REMOVED_SUBSCRIBE_ARM, "", 1)
        self.assertNotEqual(planted, MCP_TEXT)
        served = g.served_tool_names(planted)
        self.assertNotIn("lattice_subscribe", served)
        self.assertEqual(g.unknown_tool_names("Call `lattice_subscribe` first.\n", served), ["lattice_subscribe"])
        with self.assertRaises(ValueError):
            g.render("Call `lattice_subscribe` first.\n", served, "planted")

    def test_source_without_fn_name_is_an_error(self):
        with self.assertRaises(ValueError):
            g.served_tool_names("fn other() {}\n")



class HarnessText(unittest.TestCase):
    def test_every_pack_names_only_served_tools(self):
        packs = g.discover_packs(REPO_ROOT)
        self.assertTrue(packs, "no pack with a harness source")
        for pack in packs:
            text = (pack / g.SOURCE_NAME).read_text(encoding="utf-8")
            self.assertEqual(g.unknown_tool_names(text, SERVED), [], pack.name)

    def test_harness_states_the_write_and_error_cases(self):
        # The write rules and the error steps must survive edits to the source text.
        for pack in g.discover_packs(REPO_ROOT):
            text = (pack / g.SOURCE_NAME).read_text(encoding="utf-8")
            for needle in ("expected_version` 0", "key text PRIMARY KEY", "table_shape", "cursor_too_old", "version conflict"):
                with self.subTest(pack=pack.name, needle=needle):
                    self.assertIn(needle, text)

    def test_generated_text_passes_hygiene(self):
        for pack in g.discover_packs(REPO_ROOT):
            text = (pack / g.OUTPUT_NAME).read_text(encoding="utf-8")
            self.assertEqual(g.hygiene_problems(text), [], pack.name)

    def test_unknown_tool_is_refused(self):
        with self.assertRaises(ValueError):
            g.render("Use `lattice_nope` to read.\n", SERVED, "fixture")

    def test_email_absolute_path_and_token_are_refused(self):
        samples = {
            "email": "Ask owner@example.com.\n",
            "absolute path": "Read /Users/someone/project/notes.\n",
            "bearer token": "Send Bearer abcdefghijklmnopqrstuv.\n",
            "assigned token": "token = abcdefghijklmnop\n",
        }
        for label, source in samples.items():
            with self.subTest(label=label), self.assertRaises(ValueError):
                g.render(source, SERVED, "fixture")

    def test_source_with_generator_marker_is_refused(self):
        with self.assertRaises(ValueError):
            g.render(g.END_MARK + "\n", SERVED, "fixture")

    def test_source_must_end_with_one_newline(self):
        for source in ("no newline", "two newlines\n\n"):
            with self.subTest(source=source), self.assertRaises(ValueError):
                g.render(source, SERVED, "fixture")

    def test_render_wraps_the_source_verbatim(self):
        source = "## Title\n\nUse `lattice_read`.\n"
        block = g.render(source, SERVED, "fixture")
        self.assertEqual(block, f"{g.BEGIN_MARK}\n{source}{g.END_MARK}\n")


class Drift(unittest.TestCase):
    def test_committed_generated_files_match_their_sources(self):
        self.assertEqual(g.drift_problems(REPO_ROOT, g.build(REPO_ROOT)), [])

    def test_stale_and_missing_generated_files_are_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, g.MCP_SOURCE, MCP_TEXT)
            write(root, "integrations/demo/harness.src.md", "Use `lattice_read`.\n")
            expected = g.build(root)
            self.assertEqual(g.drift_problems(root, expected), ["missing: integrations/demo/harness.generated.md"])
            write(root, "integrations/demo/harness.generated.md", "an old copy\n")
            self.assertEqual(g.drift_problems(root, expected), ["stale: integrations/demo/harness.generated.md"])
            write(root, "integrations/demo/harness.generated.md", expected[root / "integrations/demo/harness.generated.md"])
            self.assertEqual(g.drift_problems(root, expected), [])

    def test_edited_source_makes_the_generated_file_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, g.MCP_SOURCE, MCP_TEXT)
            write(root, "integrations/demo/harness.src.md", "Use `lattice_read`.\n")
            out = root / "integrations/demo/harness.generated.md"
            write(root, "integrations/demo/harness.generated.md", g.build(root)[out])
            write(root, "integrations/demo/harness.src.md", "Use `lattice_write`.\n")
            self.assertEqual(g.drift_problems(root, g.build(root)), ["stale: integrations/demo/harness.generated.md"])

    def test_directories_without_a_source_are_not_packs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root, "integrations/board/schema.sql", "-- no harness here\n")
            self.assertEqual(g.discover_packs(root), [])


class McpEntryExample(unittest.TestCase):
    def test_example_entry_uses_an_environment_reference_and_no_literal_token(self):
        packs = [p for p in g.discover_packs(REPO_ROOT) if (p / "mcp.example.json").is_file()]
        self.assertTrue(packs, "no pack ships an MCP entry example")
        for pack in packs:
            with self.subTest(pack=pack.name):
                text = (pack / "mcp.example.json").read_text(encoding="utf-8")
                entry = json.loads(text)["mcpServers"]["lattice"]
                self.assertEqual(entry["type"], "http")
                self.assertTrue(entry["url"].endswith("/mcp"))
                self.assertEqual(entry["headers"]["Authorization"], "Bearer ${LATTICE_TOKEN}")
                self.assertEqual(g.hygiene_problems(text), [])


if __name__ == "__main__":
    unittest.main()
