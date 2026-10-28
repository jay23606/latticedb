/* Lattice dashboard prototype. Vanilla JS, no build step. Data is in-memory sample data (data.js).
   Views are registered through Lattice.registerView, which is also how plugins add pages. */
(function () {
  const D = window.LatticeData;
  const $ = (s, r = document) => r.querySelector(s);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

  const PATHS = {
    board: '<rect x="3" y="3" width="5" height="18" rx="1"/><rect x="10" y="3" width="5" height="12" rx="1"/><rect x="17" y="3" width="4" height="8" rx="1"/>',
    list: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
    doc: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h6"/>',
    ask: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.4-1 .9-1 1.7M12 17h.01"/>',
    agent: '<rect x="4" y="8" width="16" height="11" rx="3"/><path d="M12 4v4M9 13h.01M15 13h.01M9 16.5h6"/>',
    db: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"/>',
    drift: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17h.01"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    chat: '<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"/>',
    check: '<path d="m5 12 5 5 9-10"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    bolt: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
    lock: '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
    code: '<path d="m8 8-5 4 5 4M16 8l5 4-5 4M14 5l-4 14"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    puzzle: '<path d="M10 4a2 2 0 1 1 4 0v1h4a1 1 0 0 1 1 1v4h-1a2 2 0 1 0 0 4h1v4a1 1 0 0 1-1 1h-4v-1a2 2 0 1 0-4 0v1H6a1 1 0 0 1-1-1v-4h1a2 2 0 1 0 0-4H5V6a1 1 0 0 1 1-1h4z"/>',
    link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1"/>',
    file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
    story: '<path d="M7 3h10v18l-5-4-5 4z"/>', bug: '<circle cx="12" cy="13" r="5"/><path d="M12 8V5M8 9 5 7M16 9l3-2M7 13H3M17 13h4M8 18l-3 2M16 18l3 2"/>',
    task: '<path d="m5 12 5 5 9-10"/>', epic: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
  };
  const icon = (n, s = 16, extra = "") => `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" ${extra}>${PATHS[n] || ""}</svg>`;

  const rel = (t) => {
    const m = Math.max(0, Math.round((D.now + (performance.now() - T0) - t) / 60000));
    if (m < 1) return "just now"; if (m < 60) return m + "m ago"; const h = Math.round(m / 60);
    if (h < 24) return h + "h ago"; return Math.round(h / 24) + "d ago";
  };
  const T0 = performance.now();
  const stamp = () => D.now + (performance.now() - T0);
  const person = (ref) => D.people[ref] || { name: "Unassigned", kind: "none", color: "#98a2b3" };
  const initials = (n) => n.split(/[\s-]+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
  const avatar = (ref, cls = "") => {
    const p = person(ref);
    if (p.kind === "none") return `<span class="av ${cls}" style="background:var(--line-2);color:var(--mut)" title="Unassigned">?</span>`;
    const st = p.kind === "agent" ? `<span class="st ${p.status}"></span>` : "";
    return `<span class="av ${p.kind === "agent" ? "agent" : ""} ${cls}" style="background:${p.color || ""}" title="${esc(p.name)}${p.kind === "agent" ? " (agent, " + p.status + ")" : ""}">${p.kind === "agent" ? icon("agent", cls === "lg" ? 20 : 14) : esc(initials(p.name))}${st}</span>`;
  };
  const who = (ref) => `${avatar(ref)} <span>${esc(person(ref).name)}</span>${person(ref).kind === "agent" ? ' <span class="chip agent">agent</span>' : ""}`;
  const typeIcon = (t) => `<span class="ti ${t}" title="${t}">${icon(t, 11)}</span>`;
  const byKey = (k) => D.tickets.find((t) => t.key === k);

  const state = { view: "board", layout: "board", open: null, filter: { who: "all", type: "all", epic: "all", drift: false }, doc: "spec", q: D.questions[0].id, qTab: "all", editing: false, live: true, typing: {}, pal: { q: "", i: 0 } };
  try { const th = localStorage.getItem("lattice-theme"); if (th) document.documentElement.dataset.theme = th; } catch (e) {}

  /* ---------- registry (plugins use this) ---------- */
  const views = [];
  const Lattice = (window.Lattice = {
    registerView(v) { views.push(v); if (booted) render(); },
    data: D, icon, esc, avatar, who, rel, person, byKey,
    openTicket: (k) => { state.open = k; render(); },
    toast,
  });
  let booted = false;

  const core = [
    { id: "board", title: "Board", icon: "board", group: "Work" },
    { id: "questions", title: "Ask an agent", icon: "ask", group: "Work", count: () => D.questions.filter((q) => q.state !== "answered").length },
    { id: "agents", title: "Agents", icon: "agent", group: "Work" },
    { id: "docs", title: "Docs", icon: "doc", group: "Knowledge" },
    { id: "drift", title: "Drift", icon: "drift", group: "Knowledge", count: () => D.drifts.filter((d) => d.state !== "resolved" && d.state !== "dismissed").length, hot: true },
  ];

  /* ---------- shell ---------- */
  function navItems() {
    const all = [...core, ...views.map((v) => ({ ...v, group: "Plugins" }))];
    const groups = {};
    all.forEach((i) => (groups[i.group] ||= []).push(i));
    return groups;
  }
  function renderSide() {
    const g = navItems();
    $("#side-nav").innerHTML = Object.entries(g).map(([name, items]) => `<h6>${name}</h6>` + items.map((i) => {
      const c = i.count ? i.count() : 0;
      return `<button data-act="nav" data-v="${i.id}" class="${state.view === i.id ? "on" : ""}" ${state.view === i.id ? 'aria-current="page"' : ""}>${icon(i.icon, 17)}<span>${esc(i.title)}</span>${c ? `<span class="count ${i.hot ? "hot" : ""}">${c}</span>` : ""}</button>`;
    }).join("")).join("");
    $("#live-dot").className = "live-dot" + (state.live ? "" : " off");
    $("#live-sw").setAttribute("aria-pressed", String(state.live));
    const mn = $("#mnav");
    mn.innerHTML = Object.values(g).flat().map((i) => `<option value="${i.id}" ${state.view === i.id ? "selected" : ""}>${esc(i.title)}</option>`).join("");
  }
  function renderTop() {
    const v = [...core, ...views].find((x) => x.id === state.view);
    $("#crumb").innerHTML = `<span>Lattice</span><span>/</span><span>Space LAT</span><span>/</span><b>${esc(v ? v.title : "")}</b>`;
  }
  function render() {
    const sc = $("#content").scrollTop;
    renderSide(); renderTop();
    const v = [...core, ...views].find((x) => x.id === state.view);
    const root = $("#content");
    const fn = renderers[state.view];
    if (fn) root.innerHTML = fn();
    else if (v && v.render) { root.innerHTML = '<div class="page" id="plugin-root"></div>'; v.render($("#plugin-root"), Lattice); }
    root.scrollTop = sc;
    renderDrawer();
    wireDnD();
  }

  /* ---------- board ---------- */
  function filtered() {
    const f = state.filter;
    return D.tickets.filter((t) => (f.who === "all" || (f.who === "agent" ? person(t.assignee).kind === "agent" : f.who === "human" ? person(t.assignee).kind === "human" : !t.assignee)) &&
      (f.type === "all" || t.type === f.type) && (f.epic === "all" || t.epic === f.epic) && (!f.drift || t.drift));
  }
  function cardHtml(t) {
    const ep = D.epics[t.epic];
    return `<div class="card" draggable="true" data-key="${t.key}" data-act="open" role="button" tabindex="0" aria-label="${esc(t.key + " " + t.title)}">
      <div class="row">${typeIcon(t.type)}<span class="key">${t.key}</span><span class="pr-${t.priority}" title="${t.priority} priority">${"▲".repeat(t.priority === "high" ? 3 : t.priority === "medium" ? 2 : 1)}</span>
      ${t.drift ? `<span class="chip warn" title="Drift detected">${icon("drift", 11)} drift</span>` : ""}</div>
      <div class="t">${esc(t.title)}</div>
      <div class="row">${ep ? `<span class="epic-tag" style="background:color-mix(in srgb, ${ep.color} 16%, transparent);color:${ep.color}">${esc(ep.title)}</span>` : ""}
      <span class="sp">${t.points ? `<span class="chip">${t.points}</span>` : ""}<span title="comments">${icon("chat", 13)} ${t.comments.length}</span>${avatar(t.assignee)}</span></div></div>`;
  }
  function controls() {
    const f = state.filter;
    const seg = (k, opts) => `<div class="seg" role="group">${opts.map(([v, l]) => `<button data-act="filter" data-k="${k}" data-v="${v}" class="${f[k] === v ? "on" : ""}">${l}</button>`).join("")}</div>`;
    return `<div class="filters">
      ${seg("who", [["all", "Everyone"], ["human", "People"], ["agent", "Agents"], ["none", "Unassigned"]])}
      <select class="sel" data-change="filter" data-k="type" aria-label="Type"><option value="all">All types</option>${["story", "bug", "task"].map((t) => `<option value="${t}" ${f.type === t ? "selected" : ""}>${t}</option>`).join("")}</select>
      <select class="sel" data-change="filter" data-k="epic" aria-label="Epic"><option value="all">All epics</option>${Object.entries(D.epics).map(([k, e]) => `<option value="${k}" ${f.epic === k ? "selected" : ""}>${esc(e.title)}</option>`).join("")}</select>
      <button class="btn sm ${f.drift ? "primary" : ""}" data-act="filter" data-k="drift" data-v="toggle">${icon("drift", 14)} Drift only</button>
      <span style="margin-left:auto" class="seg"><button data-act="layout" data-v="board" class="${state.layout === "board" ? "on" : ""}">${icon("board", 14)} Board</button><button data-act="layout" data-v="list" class="${state.layout === "list" ? "on" : ""}">${icon("list", 14)} List</button></span></div>`;
  }
  function boardView() {
    const list = filtered();
    const working = Object.values(D.people).filter((p) => p.kind === "agent" && p.status === "working").length;
    const head = `<div class="page-head"><h1>Sprint 14 · Bulk export</h1><span class="chip ok">${icon("bolt", 12)} ${working} agents working</span><span class="sub">${list.length} of ${D.tickets.length} items</span>
      <span style="margin-left:auto"><button class="btn primary" data-act="newTicketAsk">${icon("ask", 15)} Ask an agent</button></span></div>`;
    if (state.layout === "list") {
      return `<div class="page">${head}${controls()}<table class="table"><thead><tr><th>Key</th><th>Summary</th><th>Status</th><th>Assignee</th><th>Pts</th><th>Comments</th></tr></thead><tbody>${list.map((t) => `<tr class="clickable" data-act="open" data-key="${t.key}"><td>${typeIcon(t.type)} <span class="key">${t.key}</span></td><td>${esc(t.title)} ${t.drift ? '<span class="chip warn">drift</span>' : ""}</td><td><span class="chip ${t.state === "Done" || t.state === "Released" ? "ok" : t.state === "In Progress" ? "accent" : ""}">${t.state}</span></td><td>${who(t.assignee)}</td><td>${t.points ?? ""}</td><td>${t.comments.length}</td></tr>`).join("")}</tbody></table></div>`;
    }
    return `<div class="page">${head}${controls()}<div class="board">${D.states.map((s) => {
      const col = list.filter((t) => t.state === s);
      return `<section class="col" data-col="${s}" aria-label="${s}"><div class="col-h">${s}<span class="n">${col.length}</span></div>${col.map(cardHtml).join("") || '<div class="empty" style="padding:14px">Nothing here</div>'}</section>`;
    }).join("")}</div></div>`;
  }

  const GUARDS = {
    Ready: (t) => (t.ac.length ? null : "Needs at least one acceptance criterion before it can be Ready."),
    "In Review": (t) => (t.links.some((l) => l.k === "pr") && t.links.some((l) => l.k === "test") ? null : "Needs a linked pull request and linked tests before review."),
    Released: (t) => (t.links.some((l) => l.k === "deploy" && /healthy/i.test(l.l)) ? null : "Needs a linked deployment that is observed healthy."),
  };
  function moveTicket(key, to) {
    const t = byKey(key); if (!t || t.state === to) return;
    const g = GUARDS[to] && GUARDS[to](t);
    if (g) { toast("Workflow guard: " + g); return; }
    t.state = to; t.updated = stamp();
    t.acts.unshift({ at: stamp(), k: "state", text: `You moved this to ${to}` });
    render(); toast(`${key} moved to ${to}`);
  }
  function wireDnD() {
    document.querySelectorAll(".card[draggable]").forEach((c) => {
      c.addEventListener("dragstart", (e) => { e.dataTransfer.setData("text/plain", c.dataset.key); c.classList.add("dragging"); });
      c.addEventListener("dragend", () => c.classList.remove("dragging"));
      c.addEventListener("keydown", (e) => { if (e.key === "Enter") { state.open = c.dataset.key; render(); } });
    });
    document.querySelectorAll(".col").forEach((col) => {
      col.addEventListener("dragover", (e) => { e.preventDefault(); col.classList.add("over"); });
      col.addEventListener("dragleave", () => col.classList.remove("over"));
      col.addEventListener("drop", (e) => { e.preventDefault(); col.classList.remove("over"); moveTicket(e.dataTransfer.getData("text/plain"), col.dataset.col); });
    });
  }

  /* ---------- drawer ---------- */
  function feedFor(t) {
    const items = [...t.comments.map((c) => ({ ...c, kind: "comment" })), ...t.acts.map((a) => ({ ...a, kind: "event" }))].sort((a, b) => b.at - a.at);
    return items.map((i) => {
      if (i.kind === "comment") {
        const p = person(i.by);
        return `<div class="cmt ${p.kind === "agent" ? "agent" : ""}">${avatar(i.by, "lg")}<div class="bubble"><div class="who">${esc(p.name)} ${p.kind === "agent" ? '<span class="chip agent">agent</span>' : ""}<small>${rel(i.at)}</small></div><div>${esc(i.text).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/@([\w-]+)/g, '<strong style="color:var(--accent)">@$1</strong>')}</div></div></div>`;
      }
      const ic = { state: "check", code: "code", tool: "bolt", comment: "chat" }[i.k] || "clock";
      return `<div class="ev ${i.k}"><span class="ic">${icon(ic, 12)}</span><span>${esc(i.text)}</span><span style="margin-left:auto;color:var(--faint)">${rel(i.at)}</span></div>`;
    }).join("") || '<div class="empty">No activity yet</div>';
  }
  function renderDrawer() {
    const wrap = $("#drawer");
    const t = state.open && byKey(state.open);
    wrap.classList.toggle("open", !!t);
    if (!t) { wrap.innerHTML = ""; return; }
    const p = person(t.assignee);
    const ta = $("#cmt-box"); const draft = ta ? ta.value : "";
    const done = t.ac.filter((a) => a.d).length;
    wrap.innerHTML = `<div class="drawer" role="dialog" aria-label="${esc(t.title)}"><div class="dr-head">
      <div style="display:flex;align-items:center;gap:8px">${typeIcon(t.type)}<span class="key">${t.key}</span>${D.epics[t.epic] ? `<span class="chip accent">${esc(D.epics[t.epic].title)}</span>` : ""}${t.drift ? '<span class="chip warn">drift</span>' : ""}
      <button class="btn ghost sm" style="margin-left:auto" data-act="close" aria-label="Close">✕</button></div><h2>${esc(t.title)}</h2></div>
      <div class="dr-body">
        <dl class="meta">
          <dt>Status</dt><dd><select class="sel" data-change="state" aria-label="Status">${D.states.map((s) => `<option ${t.state === s ? "selected" : ""}>${s}</option>`).join("")}</select></dd>
          <dt>Assignee</dt><dd>${who(t.assignee)}
            <select class="sel" data-change="assign" aria-label="Assign"><option value="">Assign…</option><optgroup label="People">${Object.entries(D.people).filter(([, x]) => x.kind === "human").map(([k, x]) => `<option value="${k}">${esc(x.name)}</option>`).join("")}</optgroup><optgroup label="Agents">${Object.entries(D.people).filter(([, x]) => x.kind === "agent").map(([k, x]) => `<option value="${k}">${esc(x.name)} · ${x.role}</option>`).join("")}</optgroup></select></dd>
          ${p.kind === "agent" ? `<dt>Agent</dt><dd><span class="chip ${p.status === "working" ? "ok" : ""}">${p.status}</span><span class="sub">${esc(p.harness)} · trust: ${p.trust}</span></dd>` : ""}
          <dt>Points</dt><dd>${t.points ?? "–"}</dd><dt>Priority</dt><dd class="pr-${t.priority}">${t.priority}</dd><dt>Updated</dt><dd>${rel(t.updated)}</dd></dl>
        <div class="sec">Description</div><div>${esc(t.desc)}</div>
        <div class="sec">Acceptance criteria <span class="chip">${done}/${t.ac.length}</span></div>
        ${t.ac.map((a, i) => `<label class="ac ${a.d ? "done" : ""}"><input type="checkbox" data-change="ac" data-i="${i}" ${a.d ? "checked" : ""}><span>${esc(a.t)}</span></label>`).join("") || '<div class="sub">None yet. A story needs at least one to be Ready.</div>'}
        <div class="sec">Linked</div><div class="links">${t.links.map((l) => `<span class="link">${icon({ pr: "code", test: "check", deploy: "bolt", spec: "doc", doc: "doc", design: "puzzle", incident: "drift" }[l.k] || "link", 13)} ${esc(l.l)}</span>`).join("") || '<span class="sub">No links</span>'}</div>
        <div class="sec">Activity ${state.typing[t.key] ? `<span class="typing">${esc(state.typing[t.key])} is typing <span class="dots"><i></i><i></i><i></i></span></span>` : ""}</div>
        <div class="composer"><textarea id="cmt-box" placeholder="Comment. Mention an agent with @coder-1 to ask it something." aria-label="Comment"></textarea>
          <div class="bar"><span>${icon("agent", 14)} Mentions notify agents live</span><button class="btn primary sm" style="margin-left:auto" data-act="comment">Comment</button></div></div>
        <div class="feed" style="margin-top:16px">${feedFor(t)}</div></div></div>`;
    const nb = $("#cmt-box"); if (nb && draft) nb.value = draft;
  }

  /* ---------- questions (read-only tickets for agents) ---------- */
  const KIND = { how: "How does it work", fix: "Why / how to fix", investigate: "Investigate" };
  function answerFor(q) {
    const txt = q.title + " " + q.body;
    const hit = D.kb.find((k) => k.re.test(txt));
    if (hit) return { html: hit.text, cites: hit.cites, conf: hit.conf };
    const words = txt.toLowerCase().split(/\W+/).filter((w) => w.length > 4);
    const rel = D.tickets.filter((t) => words.some((w) => (t.title + t.desc).toLowerCase().includes(w))).slice(0, 3);
    return { html: rel.length ? `I could not find a definitive explanation, but these items are closely related: ${rel.map((t) => `<strong>${t.key}</strong> ${esc(t.title)}`).join("; ")}. Please add detail or assign a code area to narrow the search.` : "I found nothing relevant in the spaces I can read. Try adding the service or file name.", cites: rel.map((t) => ({ t: "ticket", l: t.key })), conf: rel.length ? 0.4 : 0.15 };
  }
  function startAnswer(q) {
    q.state = "answering"; q.steps = 0; render();
    const total = 3;
    const step = () => { q.steps++; if (q.steps >= total) { q.state = "answered"; q.answer = { ...answerFor(q), at: stamp() }; toast(`${person(q.assignee).name} answered ${q.id} (read-only, no changes made)`); } render(); if (q.steps < total) setTimeout(step, 1100); };
    setTimeout(step, 900);
  }
  function questionsView() {
    const tabs = [["all", "All"], ["open", "Open"], ["answering", "Answering"], ["answered", "Answered"]];
    const list = D.questions.filter((q) => state.qTab === "all" || q.state === state.qTab);
    const q = D.questions.find((x) => x.id === state.q) || list[0];
    const sc = { open: "warn", answering: "agent", answered: "ok" };
    const detail = !q ? '<div class="panel empty">Select a question</div>' : `<div class="panel"><div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><span class="key">${q.id}</span><span class="chip ${sc[q.state]}">${q.state}</span><span class="chip">${KIND[q.kind]}</span><span class="roflag">${icon("lock", 12)} Read-only</span></div>
      <h2 style="margin-top:10px">${esc(q.title)}</h2><div class="sub" style="display:flex;gap:8px;align-items:center">Asked by ${who(q.by)} · ${rel(q.at)} · scope: ${esc(q.scope)}</div>
      <p>${esc(q.body)}</p>
      <div class="sec">Assigned to</div>
      <div style="display:flex;gap:10px;align-items:center;flex-wrap:wrap">${q.assignee ? who(q.assignee) : '<span class="sub">Nobody yet</span>'}
        ${q.state === "open" ? `<select class="sel" data-change="qassign" data-id="${q.id}"><option value="">Pick an agent…</option>${Object.entries(D.people).filter(([, x]) => x.kind === "agent").map(([k, x]) => `<option value="${k}" ${x.grants.some((g) => /no write|read/.test(g)) ? "" : ""}>${esc(x.name)} · ${x.role}</option>`).join("")}</select>` : ""}</div>
      <div class="sub" style="margin-top:8px">The agent receives a temporary <strong>read-only</strong> grant for this scope and cannot edit files, tickets or docs. It can only reply here.</div>
      ${q.state === "answering" ? `<ul class="steps">${["Searching docs, tickets and code", "Reading the most relevant files", "Drafting the answer"].map((s, i) => `<li class="${(q.steps || 0) > i ? "done" : ""}">${(q.steps || 0) > i ? icon("check", 14) : '<span class="dots"><i></i><i></i><i></i></span>'} ${s}</li>`).join("")}</ul>` : ""}
      ${q.answer ? `<div class="answer"><div style="display:flex;gap:8px;align-items:center;margin-bottom:6px">${who(q.assignee)}<span class="sub">answered ${rel(q.answer.at)}</span></div><div>${q.answer.html}</div>
        <div class="cites">${q.answer.cites.map((c) => `<span class="cite">${icon(c.t === "file" ? "file" : c.t === "doc" ? "doc" : "story", 12)} ${esc(c.l)}</span>`).join("")}</div>
        <div style="display:flex;align-items:center;gap:12px;margin-top:12px"><span class="conf">Confidence <span class="bar-meter"><i style="width:${Math.round(q.answer.conf * 100)}%"></i></span> ${Math.round(q.answer.conf * 100)}%</span>
        <span style="margin-left:auto;display:flex;gap:6px"><button class="btn sm" data-act="qticket" data-id="${q.id}">Convert to ticket</button><button class="btn sm" data-act="qdoc" data-id="${q.id}">Save to docs</button></span></div></div>` : ""}</div>`;
    return `<div class="page"><div class="page-head"><h1>Ask an agent</h1><span class="sub">Read-only tickets: agents explain how things work or suggest fixes, they never change anything.</span><span style="margin-left:auto"><button class="btn primary" data-act="newq">${icon("plus", 15)} New question</button></span></div>
      <div class="split"><div><div class="seg" style="margin-bottom:10px">${tabs.map(([v, l]) => `<button data-act="qtab" data-v="${v}" class="${state.qTab === v ? "on" : ""}">${l}</button>`).join("")}</div>
      <div class="listcard">${list.map((x) => `<button class="li ${q && x.id === q.id ? "on" : ""}" data-act="selq" data-id="${x.id}"><div class="row"><span class="key">${x.id}</span><span class="chip ${sc[x.state]}">${x.state}</span></div><div class="t">${esc(x.title)}</div><div class="row">${avatar(x.by)} ${esc(person(x.by).name)} · ${rel(x.at)}</div></button>`).join("") || '<div class="empty">No questions</div>'}</div></div>${detail}</div></div>`;
  }

  /* ---------- docs ---------- */
  function docsView() {
    const groups = {}; D.docs.forEach((d) => (groups[d.group] ||= []).push(d));
    const d = D.docs.find((x) => x.id === state.doc);
    const drift = D.drifts.find((x) => x.doc === d.id && x.state === "proposed");
    return `<div class="page"><div class="wiki"><nav class="tree" aria-label="Pages">${Object.entries(groups).map(([g, ps]) => `<h6>${g}</h6>` + ps.map((p) => `<button data-act="doc" data-id="${p.id}" class="${p.id === d.id ? "on" : ""}">${icon("doc", 15)}<span>${esc(p.title)}</span>${p.fresh ? "" : `<span class="chip warn" style="margin-left:auto">stale</span>`}</button>`).join("")).join("")}</nav>
      <div><div style="display:flex;gap:8px;align-items:center;margin-bottom:10px"><span class="chip ${d.fresh ? "ok" : "warn"}">${d.fresh ? "Fresh" : "Drifting"}</span><span class="sub">Owner ${who(d.owner)} · updated ${rel(d.updated)}</span>
        <span style="margin-left:auto">${state.editing ? '<button class="btn sm primary" data-act="savedoc">Save</button> <button class="btn sm" data-act="canceldoc">Cancel</button>' : '<button class="btn sm" data-act="editdoc">Edit</button>'}</span></div>
        ${drift ? `<div class="banner warn">${icon("drift", 18)}<div class="body"><strong>Drift detected.</strong> ${esc(drift.observed)}<div class="diff"><div class="del">− ${esc(drift.proposal.before)}</div><div class="add">+ ${esc(drift.proposal.after)}</div></div>
          <div style="margin-top:8px;display:flex;gap:6px;align-items:center">${avatar(drift.proposal.by)} <span class="sub">Proposal by ${esc(person(drift.proposal.by).name)}</span><button class="btn sm primary" data-act="accept" data-id="${drift.id}">Accept proposal</button><button class="btn sm" data-act="dismiss" data-id="${drift.id}">Dismiss</button></div></div></div>` : ""}
        <article class="article" id="article" ${state.editing ? 'contenteditable="true"' : ""}><h1>${esc(d.title)}</h1>${d.body}</article></div>
      <aside class="rail"><div class="box"><h5>Claims</h5>${d.claims.map((c) => `<div class="claim"><span class="chip ${c.ok ? "ok" : "bad"}">${c.ok ? "✓" : "!"}</span><div>${esc(c.t)}<div class="sub" style="font-size:12px">${esc(c.s)}${c.observed ? " · " + esc(c.observed) : ""}</div></div></div>`).join("")}</div>
      <div class="box"><h5>Linked work</h5>${d.links.map((k) => { const t = byKey(k); return t ? `<button class="li" style="padding:6px 0" data-act="open" data-key="${k}"><div class="row">${typeIcon(t.type)}<span class="key">${k}</span>${avatar(t.assignee)}</div><div>${esc(t.title)}</div></button>` : ""; }).join("") || '<span class="sub">None</span>'}</div>
      <div class="box"><h5>Freshness</h5><div class="sub">Facts in this page are extracted as claims and compared with what systems report. A mismatch opens a drift item.</div></div></aside></div></div>`;
  }

  /* ---------- agents ---------- */
  function agentsView() {
    const agents = Object.entries(D.people).filter(([, p]) => p.kind === "agent");
    return `<div class="page"><div class="page-head"><h1>Agents</h1><span class="sub">Every agent acts with scoped, expiring grants. Writes that need approval wait for a person.</span></div><div class="grid">${agents.map(([k, p]) => {
      const t = p.current && byKey(p.current);
      return `<div class="acard"><div class="head">${avatar(k, "lg")}<div><h3>${esc(p.name)}</h3><div class="sub">${esc(p.role)}</div></div><span class="chip ${p.status === "working" ? "ok" : ""}" style="margin-left:auto">${p.status}</span></div>
        <div class="sub">${esc(p.harness)} · trust mode <strong>${p.trust}</strong></div>
        ${t ? `<button class="now" data-act="open" data-key="${t.key}">${typeIcon(t.type)}<span class="key">${t.key}</span><span style="flex:1">${esc(t.title)}</span><span class="typing"><span class="dots"><i></i><i></i><i></i></span></span></button>` : '<div class="now sub">No active work</div>'}
        <div class="sec" style="margin-top:14px">Grants</div><ul>${p.grants.map((g) => `<li>${esc(g)}</li>`).join("")}</ul></div>`;
    }).join("")}</div></div>`;
  }

  /* ---------- drift ---------- */
  function driftView() {
    return `<div class="page"><div class="page-head"><h1>Drift</h1><span class="sub">Claims that no longer match what is observed, with proposed fixes.</span></div>${D.drifts.map((d) => `<div class="panel" style="margin-bottom:12px"><div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><span class="chip ${d.sev === "high" ? "bad" : "warn"}">${d.sev}</span><span class="chip">${d.kind}</span><span class="chip ${d.state === "resolved" ? "ok" : d.state === "dismissed" ? "" : "warn"}">${d.state}</span></div>
      <h2 style="margin-top:8px;font-size:16px">${esc(d.title)}</h2><div class="sub">Claim: ${esc(d.claim)}</div><div class="sub">Observed: ${esc(d.observed)}</div>
      <div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap">${d.doc ? `<button class="btn sm" data-act="gotodoc" data-id="${d.doc}">${icon("doc", 14)} Open page</button>` : ""}${d.ticket ? `<button class="btn sm" data-act="open" data-key="${d.ticket}">${icon("story", 14)} ${d.ticket}</button>` : ""}
      ${d.state === "proposed" ? `<button class="btn sm primary" data-act="accept" data-id="${d.id}">Accept proposal</button>` : ""}${d.state !== "resolved" && d.state !== "dismissed" ? `<button class="btn sm" data-act="dismiss" data-id="${d.id}">Dismiss</button>` : ""}</div></div>`).join("")}</div>`;
  }

  const renderers = { board: boardView, questions: questionsView, docs: docsView, agents: agentsView, drift: driftView };

  /* ---------- modal / palette / toast ---------- */
  let toastTimer;
  function toast(msg) { const el = $("#toast"); el.textContent = msg; el.hidden = false; clearTimeout(toastTimer); toastTimer = setTimeout(() => (el.hidden = true), 3600); }
  function modal(title, body, foot) { $("#overlay").classList.add("open"); $("#modal").innerHTML = `<div class="modal-h">${title}</div><div class="modal-b">${body}</div>${foot ? `<div class="modal-f">${foot}</div>` : ""}`; }
  function closeModal() { $("#overlay").classList.remove("open"); $("#modal").innerHTML = ""; state.palOpen = false; }
  function newQuestion() {
    modal("Ask an agent", `<div class="sub">A read-only ticket. The assigned agent gets temporary read access and answers in the ticket.</div>
      <div class="field"><label for="nq-t">Question</label><input id="nq-t" placeholder="How does the export gateway decide who is rate limited?"></div>
      <div class="field"><label for="nq-b">Context</label><textarea id="nq-b" rows="3" placeholder="What are you trying to do? Link tickets or files if you know them."></textarea></div>
      <div class="field"><label for="nq-k">Type</label><select id="nq-k"><option value="how">How does it work</option><option value="fix">Why is this failing and how do I fix it</option><option value="investigate">Investigate</option></select></div>
      <div class="field"><label for="nq-a">Assign to</label><select id="nq-a"><option value="a:analyst-1">analyst-1 · read-only analyst (recommended)</option><option value="a:reviewer-1">reviewer-1 · review agent</option><option value="a:coder-1">coder-1 · implementation agent</option><option value="">Leave unassigned</option></select></div>`,
      `<button class="btn" data-act="closemodal">Cancel</button><button class="btn primary" data-act="submitq">Create read-only ticket</button>`);
    setTimeout(() => $("#nq-t") && $("#nq-t").focus(), 30);
  }

  /* hybrid + temporal search (mocked results, same shape the real planner returns) */
  function search(qs) {
    const q = qs.trim().toLowerCase(); const out = [];
    if (!q) return [{ g: "Try", items: [{ ic: "clock", t: "when did bulk export start being built?", sub: "Temporal search", act: "palfill", arg: "when did bulk export start being built?" }, { ic: "file", t: "when did I create t.go?", sub: "File history", act: "palfill", arg: "when did I create t.go?" }, { ic: "drift", t: "rate limit", sub: "Text + meaning across tickets and docs", act: "palfill", arg: "rate limit" }] }];
    if (/when .*(start|begin|built|building)/.test(q) && /export/.test(q)) out.push({ g: "Timeline", custom: "timeline", items: [] });
    if (/(create|created).*\b([\w-]+\.\w+)\b/.test(q)) { const m = q.match(/\b([\w-]+\.\w+)\b/g).pop(); out.push({ g: "File history", custom: "file", file: m, items: [] }); }
    const tk = D.tickets.filter((t) => (t.key + t.title + t.desc).toLowerCase().includes(q) || q.split(/\s+/).every((w) => (t.title + t.desc).toLowerCase().includes(w))).slice(0, 5);
    if (tk.length) out.push({ g: "Tickets", items: tk.map((t) => ({ ic: t.type, t: t.title, sub: t.key + " · " + t.state, act: "open", key: t.key })) });
    const dc = D.docs.filter((d) => (d.title + d.body.replace(/<[^>]+>/g, " ")).toLowerCase().includes(q)).slice(0, 4);
    if (dc.length) out.push({ g: "Docs", items: dc.map((d) => ({ ic: "doc", t: d.title, sub: d.group, act: "gotodoc", id: d.id })) });
    const qq = D.questions.filter((x) => (x.title + x.body).toLowerCase().includes(q)).slice(0, 3);
    if (qq.length) out.push({ g: "Questions", items: qq.map((x) => ({ ic: "ask", t: x.title, sub: x.id + " · " + x.state, act: "gotoq", id: x.id })) });
    return out.length ? out : [{ g: "No results", items: [{ ic: "ask", t: `Ask an agent: "${qs}"`, sub: "Create a read-only question", act: "askfrom", arg: qs }] }];
  }
  function openPalette() {
    state.palOpen = true; state.pal = { q: "", i: 0 };
    $("#overlay").classList.add("open");
    $("#modal").innerHTML = `<div class="pal-in">${icon("search", 18)}<input id="pal-q" placeholder="Search text, meaning and time. Try: when did bulk export start being built?" autocomplete="off" aria-label="Search"></div><div class="pal-r" id="pal-r"></div>`;
    renderPalette(); setTimeout(() => $("#pal-q").focus(), 20);
  }
  function renderPalette() {
    const groups = search(state.pal.q); let n = 0; state.palItems = [];
    $("#pal-r").innerHTML = groups.map((g) => {
      let html = `<div class="pal-g">${g.g}</div>`;
      if (g.custom === "timeline") {
        html += `<div style="padding:6px 10px"><div class="sub">Relevance found <strong>Bulk export</strong> (spec, epic LAT-100, 11 stories, 14 files). Earliest activity per phase:</div><div class="tl">${D.timeline.map((m) => `<div><strong>${m.phase}</strong> · ${new Date(m.at).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })}<div class="sub">${esc(m.what)} · ${esc(person(m.who).name)} · ${esc(m.ref)}</div></div>`).join("")}</div>
          <div class="sub">Code work started on the <strong>${new Date(D.timeline[2].at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</strong>; the spec existed ${Math.round((D.timeline[2].at - D.timeline[0].at) / 86400000)} days earlier.</div></div>`;
        return html;
      }
      if (g.custom === "file") {
        html += `<div style="padding:6px 10px"><div class="sub">Lineages of <code>${esc(g.file)}</code> created by you:</div><div class="tl"><div><strong>${new Date(D.now - 86400000 * 19).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })}</strong> · created as <code>cmd/export/${esc(g.file)}</code><div class="sub">Change set cs_8f21 by Dev Patel (you)</div></div><div><strong>${new Date(D.now - 86400000 * 11).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</strong> · moved to <code>internal/export/${esc(g.file)}</code><div class="sub">Rename detected, same file identity</div></div></div><div class="sub">Illustrative sample result.</div></div>`;
        return html;
      }
      g.items.forEach((it) => { html += `<div class="pal-i ${n === state.pal.i ? "on" : ""}" data-pal="${n}">${icon(it.ic, 16)}<div><div>${esc(it.t)}</div><div class="sub" style="font-size:12px">${esc(it.sub)}</div></div></div>`; state.palItems.push(it); n++; });
      return html;
    }).join("");
  }
  function palPick(i) {
    const it = state.palItems[i]; if (!it) return;
    if (it.act === "palfill") { $("#pal-q").value = it.arg; state.pal.q = it.arg; state.pal.i = 0; renderPalette(); return; }
    closeModal();
    if (it.act === "open") { state.open = it.key; } else if (it.act === "gotodoc") { state.view = "docs"; state.doc = it.id; } else if (it.act === "gotoq") { state.view = "questions"; state.q = it.id; state.qTab = "all"; } else if (it.act === "askfrom") { newQuestion(); setTimeout(() => ($("#nq-t").value = it.arg), 40); return; }
    render();
  }

  /* ---------- actions ---------- */
  const A = {
    nav: (d) => { state.view = d.v; state.open = null; render(); },
    open: (d) => { state.open = d.key; render(); },
    close: () => { state.open = null; render(); },
    filter: (d) => { if (d.k === "drift") state.filter.drift = !state.filter.drift; else state.filter[d.k] = d.v; render(); },
    layout: (d) => { state.layout = d.v; render(); },
    comment: () => {
      const t = byKey(state.open); const box = $("#cmt-box"); const text = box.value.trim(); if (!text) return;
      t.comments.push({ by: "h:maya", at: stamp(), text }); t.updated = stamp(); box.value = "";
      const m = text.match(/@([\w-]+)/g) || [];
      m.forEach((tag) => { const ref = "a:" + tag.slice(1); const ag = D.people[ref]; if (!ag) return;
        state.typing[t.key] = ag.name; render();
        setTimeout(() => { state.typing[t.key] = null; t.comments.push({ by: ref, at: stamp(), text: ag.grants.some((g) => /no write/.test(g)) ? "I can look into this read-only and reply with what I find." : "On it. I will check the linked code and the acceptance criteria, then report back here." }); render(); }, 2200); });
      render();
    },
    newq: newQuestion, newTicketAsk: newQuestion,
    closemodal: closeModal,
    submitq: () => {
      const title = $("#nq-t").value.trim(); if (!title) { $("#nq-t").focus(); return; }
      const id = "Q-" + (15 + D.questions.length - 3); const assignee = $("#nq-a").value || null;
      const q = { id, title, kind: $("#nq-k").value, by: "h:maya", assignee, state: assignee ? "answering" : "open", at: stamp(), body: $("#nq-b").value.trim() || "(no extra context)", scope: "all spaces", answer: null };
      D.questions.unshift(q); state.view = "questions"; state.q = id; state.qTab = "all"; closeModal(); if (assignee) startAnswer(q); else render();
    },
    qtab: (d) => { state.qTab = d.v; render(); }, selq: (d) => { state.q = d.id; render(); },
    qticket: (d) => { const q = D.questions.find((x) => x.id === d.id); const key = "LAT-" + (300 + D.tickets.length);
      D.tickets.push({ key, type: "task", title: q.title.replace(/\?$/, ""), state: "Backlog", assignee: null, points: null, priority: "medium", epic: "LAT-100", desc: q.body + " (from " + q.id + ")", ac: [], links: [{ k: "doc", l: q.id + " answer" }], comments: [], acts: [{ at: stamp(), k: "state", text: "Created from " + q.id }], drift: false, updated: stamp() });
      toast(`Created ${key} from ${q.id}`); state.view = "board"; state.open = key; render(); },
    qdoc: (d) => toast("Saved to Docs as a draft page (prototype)."),
    doc: (d) => { state.doc = d.id; state.editing = false; render(); }, gotodoc: (d) => { state.view = "docs"; state.doc = d.id; state.open = null; render(); },
    editdoc: () => { state.editing = true; render(); $("#article").focus(); }, canceldoc: () => { state.editing = false; render(); },
    savedoc: () => { const d = D.docs.find((x) => x.id === state.doc); const a = $("#article").cloneNode(true); a.querySelector("h1")?.remove(); d.body = a.innerHTML; d.updated = stamp(); state.editing = false; render(); toast("Saved. Collaborators see the change live."); },
    accept: (d) => { const x = D.drifts.find((y) => y.id === d.id); x.state = "resolved"; const doc = D.docs.find((p) => p.id === x.doc);
      if (doc) { doc.fresh = true; doc.updated = stamp(); doc.body = doc.body.replace("<strong>60 requests per minute</strong>", "<strong>100 requests per minute</strong> on the Team plan and 20 on the Free plan"); doc.claims.forEach((c) => { c.ok = true; c.observed = null; c.t = "Rate limit is 100 requests per minute (Team), 20 (Free)"; }); }
      const t = byKey(x.ticket); if (t) { t.drift = false; t.acts.unshift({ at: stamp(), k: "state", text: "Drift resolved, proposal accepted" }); }
      toast("Proposal merged. Page is fresh again."); render(); },
    dismiss: (d) => { const x = D.drifts.find((y) => y.id === d.id); x.state = "dismissed"; const t = byKey(x.ticket); if (t) t.drift = false; toast("Dismissed. Recorded as feedback for the judge."); render(); },
    palpick: (d) => palPick(+d.i),
    theme: () => { const cur = document.documentElement.dataset.theme || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"); const nx = cur === "dark" ? "light" : "dark"; document.documentElement.dataset.theme = nx; try { localStorage.setItem("lattice-theme", nx); } catch (e) {} },
    live: () => { state.live = !state.live; render(); },
    search: openPalette,
  };
  document.addEventListener("click", (e) => {
    if (e.target.id === "overlay") { closeModal(); return; }
    if (e.target.id === "drawer") { A.close(); return; }
    const pal = e.target.closest("[data-pal]"); if (pal) { palPick(+pal.dataset.pal); return; }
    const el = e.target.closest("[data-act]"); if (!el || !A[el.dataset.act]) return;
    if (e.target.closest("select, textarea, input") ) return;
    A[el.dataset.act](el.dataset, e);
  });
  document.addEventListener("change", (e) => {
    const el = e.target.closest("[data-change]"); if (!el) return; const k = el.dataset.change;
    if (k === "filter") { state.filter[el.dataset.k] = el.value; render(); }
    if (k === "state") moveTicket(state.open, el.value);
    if (k === "assign" && el.value) { const t = byKey(state.open); const prev = person(t.assignee); t.assignee = el.value; t.updated = stamp(); const p = person(el.value);
      t.acts.unshift({ at: stamp(), k: "state", text: `Assigned to ${p.name}` });
      if (p.kind === "agent") { p.status = "working"; p.current = t.key; if (t.state === "Ready" || t.state === "Backlog") t.state = "In Progress"; }
      if (prev.kind === "agent" && prev.current === t.key) { prev.status = "idle"; prev.current = null; }
      toast(`Assigned ${t.key} to ${p.name}`); render(); }
    if (k === "ac") { const t = byKey(state.open); t.ac[+el.dataset.i].d = el.checked ? 1 : 0; render(); }
    if (k === "qassign" && el.value) { const q = D.questions.find((x) => x.id === el.dataset.id); q.assignee = el.value; startAnswer(q); }
  });
  document.addEventListener("input", (e) => { if (e.target.id === "pal-q") { state.pal.q = e.target.value; state.pal.i = 0; renderPalette(); } });
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); openPalette(); return; }
    if (e.key === "Escape") { if ($("#overlay").classList.contains("open")) closeModal(); else if (state.open) A.close(); return; }
    if (state.palOpen) {
      const n = state.palItems ? state.palItems.length : 0;
      if (e.key === "ArrowDown") { e.preventDefault(); state.pal.i = Math.min(n - 1, state.pal.i + 1); renderPalette(); }
      if (e.key === "ArrowUp") { e.preventDefault(); state.pal.i = Math.max(0, state.pal.i - 1); renderPalette(); }
      if (e.key === "Enter") { e.preventDefault(); palPick(state.pal.i); }
    }
  });
  $("#mnav").addEventListener("change", (e) => { state.view = e.target.value; state.open = null; render(); });

  /* ---------- live simulation: agents keep working while you watch ---------- */
  const LINES = {
    "a:coder-1": [["code", "coder-1 edited src/export/gateway.rs (+18 -3)"], ["tool", "coder-1 ran cargo test: 133 passed"], ["code", "coder-1 edited src/plan/limits.rs (+9 -0)"], ["tool", "coder-1 ran cargo clippy: no warnings"]],
    "a:reviewer-1": [["tool", "reviewer-1 read src/notify/export_mail.rs"], ["tool", "reviewer-1 compared PR #482 with acceptance criteria"], ["comment", "reviewer-1 left an inline note on line 88"]],
    "a:docs-bot": [["tool", "docs-bot read Deployment prod v2.4"], ["tool", "docs-bot re-verified claims in Export runbook"]],
  };
  const idx = {};
  function isTyping() { const a = document.activeElement; return a && /TEXTAREA|INPUT|SELECT/.test(a.tagName) || state.editing; }
  setInterval(() => {
    if (!state.live || isTyping() || $("#overlay").classList.contains("open")) return;
    const refs = Object.keys(LINES); const ref = refs[(idx.n = ((idx.n ?? -1) + 1)) % refs.length]; const ag = D.people[ref];
    if (!ag.current) return; const t = byKey(ag.current); if (!t) return;
    const i = (idx[ref] = ((idx[ref] ?? -1) + 1) % LINES[ref].length); const [k, text] = LINES[ref][i];
    t.acts.unshift({ at: stamp(), k, text }); t.updated = stamp(); render();
  }, 5000);

  /* ---------- boot ---------- */
  document.addEventListener("DOMContentLoaded", () => {
    $("#presence").innerHTML = ["h:maya", "h:omar", "h:lina", "a:coder-1", "a:docs-bot"].map((r) => avatar(r)).join("");
    $("#logo").innerHTML = icon("bolt", 15);
    $("#search-ic").innerHTML = icon("search", 15);
    $("#theme-btn").innerHTML = icon("sun", 15);
    booted = true; render();
  });
})();
