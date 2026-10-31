"""Checks for the MCP examples and the README of integrations/claude-code.

The stdio example runs `lattice mcp`, so its flags are checked against the usage text the proxy
prints, read from the source of crates/lattice-cli/src/mcp_proxy.rs. The README table must list
every file in the pack, and neither example may hold a token.
"""

import json
import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PACK = REPO_ROOT / "integrations" / "claude-code"
HTTP_EXAMPLE = PACK / "mcp.example.json"
STDIO_EXAMPLE = PACK / "mcp.stdio.example.json"
README = PACK / "README.md"
PROXY_SOURCE = REPO_ROOT / "crates" / "lattice-cli" / "src" / "mcp_proxy.rs"

USAGE_RE = re.compile(r'const USAGE: &str = "\\\n(?P<body>.*?)";', re.DOTALL)
FLAG_RE = re.compile(r"--[a-z][a-z-]*")
PLACEHOLDER_RE = re.compile(r"\A\$\{[A-Z][A-Z0-9_]*\}\Z")
LOOPBACK_MCP_URL_RE = re.compile(r"\Ahttp://(127\.0\.0\.1|localhost|\[::1\]):[0-9]+/mcp\Z")
EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}")


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def proxy_usage_flags():
    source = PROXY_SOURCE.read_text(encoding="utf-8")
    match = USAGE_RE.search(source)
    if match is None:
        raise AssertionError("USAGE constant not found in mcp_proxy.rs")
    body = match.group("body")
    return body, set(FLAG_RE.findall(body))


def pack_files():
    for path in sorted(PACK.rglob("*")):
        if not path.is_file() or path == README:
            continue
        if "__pycache__" in path.parts or path.suffix == ".pyc":
            continue
        yield path


class Examples(unittest.TestCase):
    def test_both_examples_parse_and_name_the_lattice_server(self):
        for path in (HTTP_EXAMPLE, STDIO_EXAMPLE):
            with self.subTest(path=path.name):
                self.assertIn("lattice", load_json(path)["mcpServers"])


class StdioExample(unittest.TestCase):
    def setUp(self):
        self.entry = load_json(STDIO_EXAMPLE)["mcpServers"]["lattice"]
        self.args = self.entry["args"]

    def test_runs_the_lattice_mcp_subcommand(self):
        self.assertEqual(self.entry["command"], "lattice")
        self.assertEqual(self.args[0], "mcp")

    def test_has_no_transport_type_so_it_is_read_as_stdio(self):
        self.assertEqual(self.entry.get("type", "stdio"), "stdio")

    def test_every_flag_appears_in_the_proxy_usage_text(self):
        body, usage_flags = proxy_usage_flags()
        self.assertIn("usage: lattice mcp", body)
        example_flags = [arg for arg in self.args if arg.startswith("--")]
        self.assertEqual(example_flags, ["--url", "--token-file"])
        for flag in example_flags:
            with self.subTest(flag=flag):
                self.assertIn(flag, usage_flags)

    def test_url_is_loopback_http_on_the_mcp_path(self):
        url = self.args[self.args.index("--url") + 1]
        self.assertRegex(url, LOOPBACK_MCP_URL_RE)

    def test_token_comes_from_a_file_placeholder_and_never_a_literal(self):
        token_value = self.args[self.args.index("--token-file") + 1]
        self.assertRegex(token_value, PLACEHOLDER_RE)
        self.assertNotIn("--token", self.args)
        for arg in self.args:
            self.assertNotIn("Bearer", arg)
            self.assertNotIn("Authorization", arg)
        for value in self.entry.get("env", {}).values():
            self.assertRegex(value, PLACEHOLDER_RE)


class HttpExample(unittest.TestCase):
    def test_bearer_header_holds_only_a_placeholder(self):
        headers = load_json(HTTP_EXAMPLE)["mcpServers"]["lattice"].get("headers", {})
        for value in headers.values():
            scheme, _, token = value.partition(" ")
            self.assertEqual(scheme, "Bearer")
            self.assertRegex(token, PLACEHOLDER_RE)


class PackHygiene(unittest.TestCase):
    def test_readme_table_lists_every_file_in_the_pack(self):
        readme = README.read_text(encoding="utf-8")
        missing = []
        for path in pack_files():
            relative = path.relative_to(PACK).as_posix()
            if f"`{relative}`" not in readme:
                missing.append(relative)
        self.assertEqual(missing, [], "files missing from the README table")

    def test_no_email_address_or_absolute_user_path_in_the_pack(self):
        for path in pack_files():
            text = path.read_text(encoding="utf-8")
            with self.subTest(path=path.name):
                self.assertIsNone(EMAIL_RE.search(text))
                self.assertNotIn("/Users/", text)


if __name__ == "__main__":
    unittest.main()
