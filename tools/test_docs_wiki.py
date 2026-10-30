import tempfile
import unittest
from pathlib import Path

import docs_summary as ds
import docs_wiki as dw
from test_docs_summary import write

REPO_ROOT = Path(__file__).resolve().parent.parent
PAGE_NAMES = [
    "Agents-and-live-collaboration", "Architecture-overview", "Getting-started", "Gates-and-board",
    "Glossary", "Home", "How-to-contribute", "Roadmap", "SQL-subset", "Security-model", "Verification",
]


def fixture(root: Path) -> None:
    write(root, "LICENSE", "x")
    write(root, "CONTRIBUTING.md", "# Contributing\n\nSee `docs/x.md` and [gl](docs/glossary.md).\n\n## Setup\n")
    write(root, "docs/wiki/home.md", "![b](../assets/b.png)\n\n# Lattice\n\n[road](../roadmap.md) [lic](../../LICENSE) [c](../../CONTRIBUTING.md#setup)\n")
    write(root, "docs/wiki/getting-started.md", "# Getting started\n\nSee [home](home.md).\n")
    write(root, "docs/wiki/architecture.md", "# Architecture\n\n## Layers\n\ntext\n\n## Later\n\n```\n## not a heading\n```\n")
    for name in ("sql", "agents", "security", "verification"):
        write(root, f"docs/wiki/{name}.md", f"# {name}\n")
    write(root, "docs/roadmap.md", "# Roadmap\n\n[gate](gates/phase-1.md)\n")
    write(root, "docs/glossary.md", "# Glossary\n")
    write(root, "docs/gates/phase-1.md", "# Phase 1 status\n")
    write(root, "docs/gates/phase-10.md", "# Phase 10 status\n")
    write(root, "docs/gates/phase-2.md", "# Phase 2 status\n")


class DocsWikiTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name).resolve()
        fixture(self.root)

    def tearDown(self):
        self._tmp.cleanup()

    def test_pages_and_banner(self):
        files, errors = dw.generate(self.root)
        self.assertEqual(errors, [])
        self.assertEqual(
            sorted(files),
            sorted([f"{name}.md" for name in PAGE_NAMES] + ["_Sidebar.md"]),
        )
        for name, content in files.items():
            if name != "_Sidebar.md":
                self.assertTrue(content.startswith("> **Generated from docs/ in the main repository, edit there.**"), name)

    def test_home_drops_banner_image_and_maps_links(self):
        files, _ = dw.generate(self.root)
        home = files["Home.md"]
        self.assertNotIn("b.png", home)
        self.assertIn("(Roadmap)", home)
        self.assertIn("(How-to-contribute#setup)", home)
        self.assertIn(f"({ds.REPO_URL}/blob/main/LICENSE)", home)

    def test_wiki_source_links_resolve_to_wiki_pages(self):
        files, _ = dw.generate(self.root)
        self.assertIn("See [home](Home).", files["Getting-started.md"])

    def test_roadmap_links_to_gate_become_urls(self):
        files, _ = dw.generate(self.root)
        self.assertIn(f"({ds.REPO_URL}/blob/main/docs/gates/phase-1.md)", files["Roadmap.md"])

    def test_architecture_page_is_the_whole_source(self):
        files, _ = dw.generate(self.root)
        arch = files["Architecture-overview.md"]
        self.assertIn("## Later", arch)
        self.assertIn("## not a heading", arch)

    def test_gates_page_is_numerically_ordered(self):
        files, _ = dw.generate(self.root)
        page = files["Gates-and-board.md"]
        self.assertLess(page.index("phase-1.md"), page.index("phase-2.md"))
        self.assertLess(page.index("phase-2.md"), page.index("phase-10.md"))
        self.assertIn(dw.BOARD_URL, page)

    def test_anchor_missing_from_the_wiki_page_goes_to_github(self):
        write(self.root, "docs/roadmap.md", "# Roadmap\n\n[a](wiki/architecture.md#missing) [b](wiki/architecture.md#layers)\n")
        files, errors = dw.generate(self.root)
        self.assertEqual(errors, [])
        road = files["Roadmap.md"]
        self.assertIn(f"({ds.REPO_URL}/blob/main/docs/wiki/architecture.md#missing)", road)
        self.assertIn("(Architecture-overview#layers)", road)

    def test_symlinked_gate_page_is_skipped(self):
        (self.root / "docs" / "gates" / "phase-9.md").symlink_to(self.root / "docs" / "gates" / "phase-1.md")
        files, _ = dw.generate(self.root)
        self.assertNotIn("phase-9.md", files["Gates-and-board.md"])

    def test_broken_link_is_an_error(self):
        write(self.root, "docs/glossary.md", "# Glossary\n\n[x](nope.md)\n")
        _, errors = dw.generate(self.root)
        self.assertEqual(len(errors), 1)

    def test_main_writes_and_replaces(self):
        out = self.root / "out"
        out.mkdir()
        (out / "stale.md").write_text("old")
        self.assertEqual(dw.main(["--root", str(self.root), "--out", str(out)]), 0)
        self.assertFalse((out / "stale.md").exists())
        self.assertTrue((out / "Home.md").exists())

    def test_main_does_not_write_on_error(self):
        write(self.root, "docs/glossary.md", "# Glossary\n\n[x](nope.md)\n")
        out = self.root / "out"
        self.assertEqual(dw.main(["--root", str(self.root), "--out", str(out)]), 1)
        self.assertFalse(out.exists())

    def test_strip_leading_images_keeps_other_text(self):
        text = "![a](x.png)\nkeep me\n\n# T\n"
        self.assertEqual(dw.strip_leading_images(text), "keep me\n# T\n")

    def test_real_sources_generate_without_errors(self):
        files, errors = dw.generate(REPO_ROOT)
        self.assertEqual(errors, [])
        self.assertEqual(sorted(files), sorted([f"{name}.md" for name in PAGE_NAMES] + ["_Sidebar.md"]))


if __name__ == "__main__":
    unittest.main()
