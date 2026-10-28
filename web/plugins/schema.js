/* DB Schema plugin. Registers one page through Lattice.registerView.
   It shows the schema of the app's own database (introspected and stored in Lattice as a SchemaSnapshot),
   with an ER diagram, per-table details, change history and the tickets that touched each table.
   A second source shows Lattice's own workspace types, using the same renderer. */
(function () {
  const L = window.Lattice;
  const { esc, icon, person } = L;
  const ago = (d) => L.data.now - d * 86400000;

  const SOURCES = {
    app: {
      label: "App database",
      name: "app-db · PostgreSQL 16",
      version: "migration 0042",
      introspected: ago(0.4),
      tables: [
        { n: "users", cols: [["id", "uuid", "pk"], ["email", "text", "uniq"], ["name", "text"], ["created_at", "timestamptz"]], idx: ["users_email_key (email) unique"] },
        { n: "workspaces", cols: [["id", "uuid", "pk"], ["name", "text"], ["plan", "text"], ["created_at", "timestamptz"]], idx: [] },
        { n: "reports", cols: [["id", "uuid", "pk"], ["workspace_id", "uuid", "fk", "workspaces"], ["name", "text"], ["query", "jsonb"], ["row_estimate", "bigint", "null"]], idx: ["reports_ws (workspace_id)"] },
        { n: "exports", cols: [["id", "uuid", "pk"], ["workspace_id", "uuid", "fk", "workspaces"], ["report_id", "uuid", "fk", "reports"], ["requested_by", "uuid", "fk", "users"], ["format", "text"], ["status", "text"], ["row_count", "bigint", "null"], ["created_at", "timestamptz"], ["finished_at", "timestamptz", "null"]], idx: ["exports_ws_created (workspace_id, created_at)"] },
        { n: "export_chunks", cols: [["export_id", "uuid", "fk", "exports"], ["seq", "int", "pk"], ["blob_ref", "text"], ["bytes", "bigint"]], idx: ["export_chunks_pk (export_id, seq)"] },
        { n: "rate_buckets", cols: [["workspace_id", "uuid", "fk", "workspaces"], ["window_start", "timestamptz", "pk"], ["count", "int"], ["rate_bucket", "text", "null"]], idx: ["rate_buckets_pk (workspace_id, window_start)"] },
        { n: "audit_log", cols: [["id", "bigint", "pk"], ["actor_id", "uuid", "fk", "users"], ["action", "text"], ["resource", "text"], ["at", "timestamptz"]], idx: ["audit_actor_at (actor_id, at)"] },
      ],
      history: [
        { v: "0042", by: "a:coder-1", at: ago(0.2), s: "add rate_buckets.rate_bucket (nullable)", t: "rate_buckets", ticket: "LAT-104" },
        { v: "0041", by: "h:dev", at: ago(3), s: "add exports.format", t: "exports", ticket: "LAT-101" },
        { v: "0040", by: "a:coder-1", at: ago(8), s: "create export_chunks", t: "export_chunks", ticket: "LAT-102" },
        { v: "0039", by: "h:dev", at: ago(20), s: "create audit_log", t: "audit_log", ticket: "LAT-112" },
      ],
      ticketsFor: { exports: ["LAT-101", "LAT-106", "LAT-107"], export_chunks: ["LAT-102"], rate_buckets: ["LAT-104"], audit_log: ["LAT-108", "LAT-112"], reports: ["LAT-106"] },
      src: (t) => `CREATE TABLE ${t.n} (\n${t.cols.map((c) => `  ${c[0]} ${c[1]}${c[2] === "pk" ? " PRIMARY KEY" : c[2] === "uniq" ? " UNIQUE" : c[2] === "null" ? "" : " NOT NULL"}${c[2] === "fk" ? ` REFERENCES ${c[3]}(id)` : ""}`).join(",\n")}\n);`,
    },
    lattice: {
      label: "Lattice types",
      name: "Lattice workspace · types and facets",
      version: "schema hash 7c1e…a90",
      introspected: ago(0.1),
      tables: [
        { n: "Epic", cols: [["id", "ulid", "pk"], ["title", "text"], ["color", "text", "null"]], idx: [] },
        { n: "Story", cols: [["id", "ulid", "pk"], ["epic", "ref<Epic>", "fk", "Epic"], ["title", "text"], ["points", "u16", "null"], ["state", "enum"], ["assignee", "principal", "null"]], idx: ["(epic, state)"] },
        { n: "Spec", cols: [["id", "ulid", "pk"], ["title", "text"], ["status", "enum"]], idx: [] },
        { n: "StorySpec", cols: [["story", "ref<Story>", "fk", "Story"], ["spec", "ref<Spec>", "fk", "Spec"]], idx: [] },
        { n: "Doc", cols: [["id", "ulid", "pk"], ["title", "text"], ["body", "richtext"], ["freshness", "enum"]], idx: [] },
        { n: "Comment", cols: [["id", "ulid", "pk"], ["on", "ref<any>", "fk", "Story"], ["author", "principal"], ["text", "richtext"]], idx: [] },
        { n: "WorkItem", cols: [["id", "ulid", "pk"], ["kind", "enum"], ["subject", "ref<any>", "fk", "Story"], ["state", "enum"], ["leased_by", "principal", "null"]], idx: [] },
      ],
      history: [{ v: "v7", by: "h:dev", at: ago(2), s: "Story.points optional", t: "Story", ticket: "LAT-104" }, { v: "v6", by: "a:docs-bot", at: ago(9), s: "Doc.freshness added", t: "Doc", ticket: "LAT-111" }],
      ticketsFor: { Story: ["LAT-104"], Doc: ["LAT-111"] },
      src: (t) => `type ${t.n} {\n${t.cols.map((c) => `  ${c[0]}: ${c[1]}${c[2] === "null" ? " optional" : ""}`).join("\n")}\n}`,
    },
  };

  const st = { src: "app", sel: "exports", q: "" };

  function layout(tables) {
    const W = 232, HEAD = 30, ROW = 22, GX = 64, GY = 28, PER = 3;
    const colH = Array(PER).fill(24); const pos = {};
    tables.forEach((t, i) => { const c = i % PER; const h = HEAD + t.cols.length * ROW + 8; pos[t.n] = { x: 24 + c * (W + GX), y: colH[c], w: W, h, c }; colH[c] += h + GY; });
    return { pos, W, HEAD, ROW, width: 24 + PER * (W + GX), height: Math.max(...colH) + 10 };
  }

  function diagram(src) {
    const { pos, W, HEAD, ROW, width, height } = layout(src.tables);
    const hl = (a, b) => st.sel === a || st.sel === b;
    const edges = [];
    src.tables.forEach((t) => t.cols.forEach((c, ci) => { if (c[2] === "fk" && pos[c[3]] && c[3] !== t.n) edges.push({ from: t.n, ci, to: c[3] }); }));
    const paths = edges.map((e) => {
      const a = pos[e.from], b = pos[e.to]; const ay = a.y + HEAD + e.ci * ROW + ROW / 2; const by = b.y + HEAD / 2;
      let ax, bx, d;
      if (a.x < b.x) { ax = a.x + W; bx = b.x; const mx = (ax + bx) / 2; d = `M${ax},${ay} C${mx},${ay} ${mx},${by} ${bx},${by}`; }
      else if (a.x > b.x) { ax = a.x; bx = b.x + W; const mx = (ax + bx) / 2; d = `M${ax},${ay} C${mx},${ay} ${mx},${by} ${bx},${by}`; }
      else { ax = a.x + W; bx = b.x + W; d = `M${ax},${ay} C${ax + 46},${ay} ${bx + 46},${by} ${bx},${by}`; }
      return `<path class="rel ${hl(e.from, e.to) ? "hl" : ""}" d="${d}" marker-end="url(#arr)"/>`;
    }).join("");
    const boxes = src.tables.map((t) => {
      const p = pos[t.n]; const on = st.sel === t.n; const dim = st.q && !t.n.toLowerCase().includes(st.q.toLowerCase());
      return `<g class="${on ? "tbl-sel" : ""} ${dim ? "dim" : ""}" data-act="schemasel" data-t="${t.n}" style="cursor:pointer" tabindex="0" role="button" aria-label="Table ${t.n}">
        <rect class="tbl-box" x="${p.x}" y="${p.y}" width="${W}" height="${p.h}" rx="8"/><path class="tbl-head" d="M${p.x},${p.y + HEAD} V${p.y + 8} a8,8 0 0 1 8,-8 h${W - 16} a8,8 0 0 1 8,8 V${p.y + HEAD} z"/>
        <text class="tbl-name" x="${p.x + 12}" y="${p.y + 20}">${esc(t.n)}</text>
        ${t.cols.map((c, i) => `<text class="col-n" x="${p.x + 12}" y="${p.y + HEAD + i * ROW + 16}">${c[2] === "pk" ? "🔑 " : c[2] === "fk" ? "↗ " : ""}${esc(c[0])}</text><text class="col-t" x="${p.x + W - 12}" y="${p.y + HEAD + i * ROW + 16}" text-anchor="end">${esc(c[1])}${c[2] === "null" ? "?" : ""}</text>`).join("")}</g>`;
    }).join("");
    return `<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-label="Entity relationship diagram"><defs><marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="var(--faint)"/></marker></defs>${paths}${boxes}</svg>`;
  }

  function details(src) {
    const t = src.tables.find((x) => x.n === st.sel) || src.tables[0];
    const refs = src.tables.filter((o) => o.cols.some((c) => c[2] === "fk" && c[3] === t.n)).map((o) => o.n);
    const hist = src.history.filter((h) => h.t === t.n);
    const tk = (src.ticketsFor[t.n] || []).map((k) => L.byKey(k)).filter(Boolean);
    return `<div class="box"><h5>${esc(t.n)}</h5><table class="cols">${t.cols.map((c) => `<tr><td>${c[2] === "pk" ? "🔑 " : c[2] === "fk" ? "↗ " : ""}${esc(c[0])}${c[2] === "fk" ? ` <span class="sub">→ ${esc(c[3])}</span>` : ""}</td><td>${esc(c[1])}${c[2] === "null" ? "?" : ""}</td></tr>`).join("")}</table></div>
      ${t.idx.length ? `<div class="box"><h5>Indexes</h5>${t.idx.map((i) => `<div class="sub" style="font-family:var(--mono);font-size:12px">${esc(i)}</div>`).join("")}</div>` : ""}
      ${refs.length ? `<div class="box"><h5>Referenced by</h5>${refs.map((r) => `<button class="chip accent" style="border:0;margin:0 4px 4px 0;cursor:pointer" data-act="schemasel" data-t="${r}">${esc(r)}</button>`).join("")}</div>` : ""}
      <div class="box"><h5>Changes</h5>${hist.map((h) => `<div class="claim"><span class="chip">${esc(h.v)}</span><div>${esc(h.s)}<div class="sub" style="font-size:12px">${esc(person(h.by).name)} · ${L.rel(h.at)} · ${esc(h.ticket)}</div></div></div>`).join("") || '<div class="sub">No recorded changes in the retained window.</div>'}</div>
      <div class="box"><h5>Linked work</h5>${tk.map((x) => `<button class="li" style="padding:6px 0" data-act="open" data-key="${x.key}"><div class="row"><span class="key">${x.key}</span>${L.avatar(x.assignee)}</div><div>${esc(x.title)}</div></button>`).join("") || '<div class="sub">No linked tickets.</div>'}</div>
      <div class="box"><h5>${st.src === "app" ? "DDL" : "LSL"}</h5><pre class="src">${esc(src.src(t))}</pre></div>`;
  }

  function render(root) {
    const src = SOURCES[st.src];
    if (!src.tables.some((t) => t.n === st.sel)) st.sel = src.tables[0].n;
    root.innerHTML = `<div class="page-head"><h1>Database schema</h1><span class="chip ok">${esc(src.version)}</span><span class="sub">${esc(src.name)} · introspected ${L.rel(src.introspected)}</span>
      <span style="margin-left:auto" class="seg">${Object.entries(SOURCES).map(([k, s]) => `<button data-act="schemasrc" data-s="${k}" class="${st.src === k ? "on" : ""}">${esc(s.label)}</button>`).join("")}</span></div>
      <div class="schema"><div class="tree"><input class="sel" style="width:100%;margin:2px 0 8px" id="schema-q" placeholder="Filter tables" value="${esc(st.q)}" aria-label="Filter tables">
        ${src.tables.map((t) => `<button data-act="schemasel" data-t="${t.n}" class="${t.n === st.sel ? "on" : ""}">${icon("db", 15)}<span>${esc(t.n)}</span><span class="sub" style="margin-left:auto">${t.cols.length}</span></button>`).join("")}</div>
        <div class="canvas" id="schema-canvas">${diagram(src)}</div><aside class="rail">${details(src)}</aside></div>`;
    const q = root.querySelector("#schema-q");
    q.addEventListener("input", () => { st.q = q.value; const c = root.querySelector("#schema-canvas"); c.innerHTML = diagram(src); });
  }

  let mounted = null;
  function mount(root) { mounted = root; render(root); }
  document.addEventListener("click", (e) => {
    const el = e.target.closest("[data-act]"); if (!el || !mounted || !document.body.contains(mounted)) return;
    if (el.dataset.act === "schemasel") { st.sel = el.dataset.t; render(mounted); }
    if (el.dataset.act === "schemasrc") { st.src = el.dataset.s; st.sel = ""; st.q = ""; render(mounted); }
  });
  document.addEventListener("keydown", (e) => { if (e.key === "Enter" && mounted && e.target.matches && e.target.matches("g[data-act=schemasel]")) { st.sel = e.target.dataset.t; render(mounted); } });

  L.registerView({ id: "schema", title: "DB Schema", icon: "db", render: mount });
})();
