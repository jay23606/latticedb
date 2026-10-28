"""Build index.html, engineering.html and spec.html from the markdown docs (markdown + mermaid rendered in the browser)."""
import json
import pathlib
import re

root = pathlib.Path(__file__).parent
out_dir = root / 'site'
out_dir.mkdir(exist_ok=True)


def slug(text):
    return re.sub(r"^-+|-+$", "", re.sub(r"[^a-z0-9]+", "-", text.lower()))


def spec_markdown():
    """Concatenate spec/*.md into one page, rewriting cross links to in-page anchors."""
    files = sorted((root / "spec").glob("*.md"), key=lambda p: (p.name != "README.md", p.name))
    anchors = {}
    bodies = {}
    for f in files:
        text = f.read_text()
        bodies[f.name] = text
        first = next(line for line in text.splitlines() if line.startswith("# "))
        anchors[f.name] = slug(first[2:])
    out = []
    for name, text in bodies.items():
        for fname, anchor in anchors.items():
            text = text.replace(f"({fname})", f"(#{anchor})")
        text = text.replace("(../ARCHITECTURE.md)", "(index.html)").replace("(../ENGINEERING.md)", "(engineering.html)")
        out.append(text)
    return "\n\n---\n\n".join(out)


def reference_markdown():
    """Conventions, policies, ADRs and the RFC process as one page."""
    parts = [(root / "conventions.md").read_text()]
    for f in sorted((root / "policies").glob("*.md")):
        parts.append(f.read_text())
    for f in sorted((root / "adr").glob("[0-9]*.md")):
        parts.append(f.read_text())
    parts.append((root / "rfc" / "README.md").read_text())
    return "\n\n---\n\n".join(parts)


PAGES = [
    ("ARCHITECTURE.md", "index.html", "Lattice v2 Architecture", (1, 2)),
    ("ENGINEERING.md", "engineering.html", "Lattice Engineering", (1, 2)),
    (None, "spec.html", "Lattice Specification", (1,)),
    ("REFERENCE", "reference.html", "Lattice Reference", (1,)),
]
TEMPLATE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>__TITLE__</title>
<style>
:root{--bg:#fff;--fg:#1c1c1e;--mut:#6b6b70;--line:#e3e3e8;--card:#f7f7f9}
@media (prefers-color-scheme:dark){:root{--bg:#121214;--fg:#ececf0;--mut:#9a9aa3;--line:#2a2a30;--card:#1a1a1e}}
body{background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,sans-serif;margin:0}
nav.top{border-bottom:1px solid var(--line);padding:10px 16px;display:flex;gap:16px;font-size:14px;flex-wrap:wrap}
nav.top a{color:var(--mut);text-decoration:none}nav.top a.on{color:var(--fg);font-weight:600}
main{max-width:1000px;margin:0 auto;padding:24px 16px 80px}
h1,h2,h3{line-height:1.25}h2{margin-top:2.4em;border-bottom:1px solid var(--line);padding-bottom:.3em}
table{border-collapse:collapse;width:100%;font-size:14px;display:block;overflow-x:auto}
th,td{border:1px solid var(--line);padding:6px 10px;text-align:left;vertical-align:top}th{background:var(--card)}
.mermaid{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:16px;overflow-x:auto;text-align:center;margin:16px 0}
code{background:var(--card);padding:1px 5px;border-radius:4px}
pre code{display:block;padding:12px;overflow-x:auto}
details.toc{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:10px 16px;margin-bottom:24px}
details.toc summary{cursor:pointer;font-weight:600}details.toc ul{margin:8px 0 0;padding-left:18px;columns:2}
details.toc li.l2{margin-left:14px;font-size:14px;color:var(--mut)}
hr{border:0;border-top:1px solid var(--line);margin:3em 0}
</style></head><body>
<nav class="top"><a href="index.html" class="__A1__">Architecture</a><a href="engineering.html" class="__A2__">Engineering</a><a href="spec.html" class="__A3__">Specification</a><a href="reference.html" class="__A4__">Reference</a><a href="../../web/index.html">Dashboard prototype</a></nav>
<main id="out"></main>
<script type="text/markdown" id="src">__MD__</script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/marked/12.0.2/marked.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/mermaid/10.9.1/mermaid.min.js"></script>
<script>
const md = JSON.parse(document.getElementById('src').textContent);
const out = document.getElementById('out');
out.innerHTML = marked.parse(md);
const slug = t => t.toLowerCase().replace(/[^a-z0-9]+/g,'-').replace(/^-+|-+$/g,'');
const seen = {};
out.querySelectorAll('h1,h2,h3').forEach(h => { let s = slug(h.textContent); seen[s] = (seen[s]||0)+1; h.id = seen[s] > 1 ? s+'-'+seen[s] : s; });
const levels = __LEVELS__;
const heads = [...out.querySelectorAll('h1,h2')].filter(h => levels.includes(+h.tagName[1]));
if (heads.length > 3) {
  const d = document.createElement('details'); d.className = 'toc'; d.open = true;
  d.innerHTML = '<summary>Contents</summary><ul>' + heads.map(h => `<li class="l${h.tagName[1]}"><a href="#${h.id}">${h.textContent}</a></li>`).join('') + '</ul>';
  out.prepend(d);
}
out.querySelectorAll('code.language-mermaid').forEach(c => {
  const d = document.createElement('div'); d.className = 'mermaid'; d.textContent = c.textContent;
  c.parentElement.replaceWith(d);
});
const dark = matchMedia('(prefers-color-scheme: dark)').matches;
mermaid.initialize({startOnLoad:false, theme: dark ? 'dark' : 'default', securityLevel:'loose'});
mermaid.run({querySelector:'.mermaid'}).then(()=>{window.__done=true});
</script></body></html>"""

for md_name, html_name, title, levels in PAGES:
    if md_name is None:
        md = spec_markdown()
    elif md_name == "REFERENCE":
        md = reference_markdown()
    else:
        md = (root / md_name).read_text()
    payload = json.dumps(md).replace("</", "<\\/")
    page = (TEMPLATE.replace("__TITLE__", title)
            .replace("__A1__", "on" if html_name == "index.html" else "")
            .replace("__A2__", "on" if html_name == "engineering.html" else "")
            .replace("__A3__", "on" if html_name == "spec.html" else "")
            .replace("__A4__", "on" if html_name == "reference.html" else "")
            .replace("__LEVELS__", json.dumps(list(levels)))
            .replace("__MD__", payload))
    (out_dir / html_name).write_text(page)
    print("wrote", html_name)
