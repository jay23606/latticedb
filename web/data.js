/* Sample workspace for the Lattice dashboard prototype. Everything here stands in for entities
   that the real lattice-api would serve (Story, Doc, Drift, WorkItem, Agent ...). */
(function () {
  const MIN = 60000;
  const now = Date.now();
  const ago = (m) => now - m * MIN;

  const people = {
    "h:maya": { name: "Maya Chen", role: "Project manager", kind: "human", color: "#7c3aed" },
    "h:omar": { name: "Omar Haddad", role: "Business analyst", kind: "human", color: "#0369a1" },
    "h:lina": { name: "Lina Sørensen", role: "Designer", kind: "human", color: "#be185d" },
    "h:dev": { name: "Dev Patel", role: "Developer", kind: "human", color: "#b45309" },
    "a:coder-1": {
      name: "coder-1", role: "Implementation agent", kind: "agent", harness: "Claude Code plugin",
      status: "working", current: "LAT-104", trust: "assist",
      grants: ["write  space LAT  (code, issues)", "approval required  deploy, release"],
    },
    "a:reviewer-1": {
      name: "reviewer-1", role: "Review agent", kind: "agent", harness: "Claude Code plugin",
      status: "working", current: "LAT-103", trust: "assist",
      grants: ["read  space LAT", "comment  issues, pull requests"],
    },
    "a:docs-bot": {
      name: "docs-bot", role: "Docs maintainer", kind: "agent", harness: "MCP client",
      status: "working", current: "LAT-111", trust: "shadow",
      grants: ["write  docs/**  (proposals on a branch)", "read  deployments"],
    },
    "a:analyst-1": {
      name: "analyst-1", role: "Read-only analyst", kind: "agent", harness: "MCP client",
      status: "idle", current: null, trust: "assist",
      grants: ["read  all spaces", "no write access  (answers only)"],
    },
  };

  const states = ["Backlog", "Ready", "In Progress", "In Review", "Done", "Released"];

  const T = (key, type, title, state, assignee, points, priority, epic, desc, ac, links, comments, acts, extra) => ({
    key, type, title, state, assignee, points, priority, epic, desc, ac, links, comments, acts, drift: false,
    updated: ago(30), ...extra,
  });

  const tickets = [
    T("LAT-101", "story", "Export a report as CSV", "Released", "h:dev", 3, "high", "LAT-100",
      "As an analyst I can export any saved report as a CSV file so I can analyse it in a spreadsheet.",
      [{ t: "CSV contains the same columns as the on-screen report", d: 1 }, { t: "Export respects the viewer's permissions", d: 1 }],
      [{ k: "pr", l: "PR #431 merged" }, { k: "test", l: "export_csv_test (12 passing)" }, { k: "deploy", l: "prod v2.3 healthy" }, { k: "spec", l: "Bulk export spec" }],
      [{ by: "h:omar", at: ago(60 * 70), text: "Confirmed with finance that column order must match the screen." }],
      [{ at: ago(60 * 74), k: "state", text: "Moved to Released" }, { at: ago(60 * 99), k: "code", text: "dev-patel merged PR #431" }]),
    T("LAT-102", "story", "Stream large exports without timeouts", "Done", "a:coder-1", 5, "high", "LAT-100",
      "Exports over 100k rows must stream to the client in chunks instead of building the whole file in memory.",
      [{ t: "Memory stays under 200 MB for a 2M row export", d: 1 }, { t: "Client can resume a broken download", d: 1 }],
      [{ k: "pr", l: "PR #455 merged" }, { k: "test", l: "stream_export_test (9 passing)" }, { k: "spec", l: "Bulk export spec" }],
      [{ by: "a:coder-1", at: ago(60 * 30), text: "Implemented chunked writer behind the `export_chunks` table. Resume uses `Range` headers." },
       { by: "h:dev", at: ago(60 * 28), text: "Nice. Can we cap chunk size at 4 MB?" }],
      [{ at: ago(60 * 29), k: "code", text: "coder-1 edited src/export/stream.rs (+142 -18)" }, { at: ago(60 * 29.5), k: "tool", text: "coder-1 ran cargo test: 128 passed" }]),
    T("LAT-103", "story", "Email me when an export finishes", "In Review", "a:reviewer-1", 2, "medium", "LAT-100",
      "When a long export completes the requester gets an email with a download link that expires after 24 hours.",
      [{ t: "Email sent within 60 seconds of completion", d: 1 }, { t: "Link expires after 24 hours", d: 0 }],
      [{ k: "pr", l: "PR #482 open" }, { k: "test", l: "export_email_test (4 passing)" }],
      [{ by: "a:reviewer-1", at: ago(22), text: "Found one issue: the expiry is computed in local time. Suggested a UTC fix on the branch." },
       { by: "h:maya", at: ago(15), text: "@reviewer-1 can you also check the unsubscribe footer?" }],
      [{ at: ago(23), k: "tool", text: "reviewer-1 read src/notify/export_mail.rs" }, { at: ago(21), k: "comment", text: "reviewer-1 requested changes" }],
      { drift: true }),
    T("LAT-104", "story", "Rate limit export requests per workspace", "In Progress", "a:coder-1", 5, "high", "LAT-100",
      "Protect the export service by limiting how many exports a workspace can start per minute.",
      [{ t: "Limit is configurable per plan", d: 1 }, { t: "Exceeding the limit returns 429 with Retry-After", d: 0 }, { t: "Limit documented in the runbook", d: 0 }],
      [{ k: "spec", l: "Bulk export spec" }, { k: "doc", l: "Export runbook" }, { k: "deploy", l: "staging v2.4 rolling out" }],
      [{ by: "h:maya", at: ago(95), text: "Business wants 100 per minute for the Team plan and 20 for Free." },
       { by: "a:coder-1", at: ago(88), text: "Using a sliding window in `rate_buckets`. Will expose the numbers through plan config." }],
      [{ at: ago(4), k: "code", text: "coder-1 edited src/export/gateway.rs (+64 -9)" }, { at: ago(9), k: "tool", text: "coder-1 ran cargo test: 131 passed, 2 failed" }, { at: ago(40), k: "state", text: "Maya assigned to coder-1" }],
      { drift: true }),
    T("LAT-105", "task", "Design export button states and empty state", "In Progress", "h:lina", 2, "medium", "LAT-100",
      "Define idle, loading, disabled and error states for the export button, and the empty state for 'no exports yet'.",
      [{ t: "States documented in the design file", d: 1 }, { t: "Tokens used for all colours", d: 0 }],
      [{ k: "design", l: "Export UX v3 (Figma-class asset)" }],
      [{ by: "h:lina", at: ago(300), text: "Posted three options in the design file. Leaning towards option B." }], [{ at: ago(300), k: "comment", text: "Lina commented" }]),
    T("LAT-106", "bug", "Export fails for reports over 2M rows", "Ready", null, 3, "high", "LAT-100",
      "Users report a 504 when exporting very large reports. Reproduced with a 2.4M row report.",
      [{ t: "2.4M row export completes", d: 0 }],
      [{ k: "incident", l: "INC-77  export timeouts" }], [{ by: "h:dev", at: ago(180), text: "Likely the gateway timeout, not the writer. Needs investigation." }], [{ at: ago(181), k: "state", text: "Moved to Ready" }]),
    T("LAT-107", "story", "Schedule recurring exports", "Ready", null, 8, "medium", "LAT-100",
      "Let users schedule a daily or weekly export that is emailed to a list of recipients.",
      [{ t: "Daily and weekly schedules", d: 0 }, { t: "Recipients can unsubscribe", d: 0 }], [{ k: "spec", l: "Bulk export spec" }], [], [{ at: ago(600), k: "state", text: "Omar refined this story" }]),
    T("LAT-108", "task", "Add export audit events", "Backlog", null, 2, "low", "LAT-100", "Write an audit entry for every export request and download.", [], [], [], []),
    T("LAT-109", "story", "Export to Parquet", "Backlog", null, 8, "low", "LAT-100", "Data team wants Parquet output for large reports.", [], [], [], []),
    T("LAT-110", "bug", "CSV encoding breaks on Arabic headers", "In Progress", "h:dev", 2, "high", "LAT-100",
      "Headers with Arabic characters show as question marks when opened in Excel.",
      [{ t: "UTF-8 BOM added for Excel", d: 1 }, { t: "Regression test with Arabic and Japanese headers", d: 0 }],
      [{ k: "pr", l: "PR #490 draft" }], [{ by: "h:omar", at: ago(240), text: "Reported by two customers in Cairo." }], [{ at: ago(120), k: "code", text: "dev-patel pushed 2 commits" }]),
    T("LAT-111", "task", "Update export runbook", "In Review", "a:docs-bot", 1, "medium", "LAT-100",
      "Keep the export runbook aligned with what is actually deployed.",
      [{ t: "Runbook numbers match production", d: 0 }], [{ k: "doc", l: "Export runbook" }, { k: "deploy", l: "prod v2.4 healthy" }],
      [{ by: "a:docs-bot", at: ago(35), text: "Drift detected: runbook says 60 requests per minute, production allows 100. Proposal ready for review." }],
      [{ at: ago(36), k: "tool", text: "docs-bot compared Export runbook with Deployment prod v2.4" }], { drift: true }),
    T("LAT-112", "story", "Permission check on the export endpoint", "Done", "h:dev", 3, "high", "LAT-100",
      "Only users who can view a report may export it.", [{ t: "403 for users without access", d: 1 }], [{ k: "pr", l: "PR #440 merged" }], [], [{ at: ago(60 * 40), k: "state", text: "Moved to Done" }]),
    T("LAT-201", "story", "Welcome checklist for new workspaces", "In Progress", "h:lina", 5, "medium", "LAT-200",
      "New workspaces see a checklist that guides them through creating a first report and inviting a teammate.",
      [{ t: "Checklist persists per user", d: 0 }], [{ k: "design", l: "Onboarding flow v2" }], [], [{ at: ago(400), k: "comment", text: "Lina started the flow" }]),
    T("LAT-202", "story", "Invite teammates flow", "Backlog", null, 5, "medium", "LAT-200", "Invite by email with role selection.", [], [], [], []),
  ];

  const epics = {
    "LAT-100": { title: "Bulk export", color: "#7c3aed" },
    "LAT-200": { title: "Onboarding revamp", color: "#0e7490" },
  };

  const docs = [
    { id: "overview", title: "Architecture overview", group: "Engineering", owner: "h:dev", updated: ago(60 * 24 * 6), fresh: true, links: [],
      claims: [{ t: "Export service is stateless behind the gateway", s: "src/export/mod.rs", ok: true }],
      body: `<p>The platform is a Rust API behind a gateway. Reports are rendered by the report service; exports are produced by the export service and written to object storage in chunks.</p>
<h2>Services</h2><ul><li><strong>gateway</strong> authentication, rate limits, routing</li><li><strong>report-service</strong> query execution and rendering</li><li><strong>export-service</strong> chunked export writer and download endpoint</li></ul>
<h2>Data</h2><p>All persistent state lives in PostgreSQL. Large files are stored as chunks referenced from the <code>export_chunks</code> table.</p>` },
    { id: "spec", title: "Bulk export spec", group: "Specs", owner: "h:omar", updated: ago(60 * 24 * 2), fresh: true, links: ["LAT-101", "LAT-102", "LAT-104", "LAT-107"],
      claims: [{ t: "Exports over 100k rows stream in chunks", s: "LAT-102", ok: true }, { t: "Free plan limit is 20 exports per minute", s: "LAT-104", ok: true }],
      body: `<p>Analysts need to export reports for use in other tools. The export must be reliable for very large reports and must respect permissions.</p>
<h2>Requirements</h2><ol><li>CSV export of any saved report</li><li>Streaming for reports above 100,000 rows</li><li>Per-workspace rate limits by plan</li><li>Email notification when a long export finishes</li></ol>
<h2>Plan limits</h2><table><thead><tr><th>Plan</th><th>Exports per minute</th></tr></thead><tbody><tr><td>Free</td><td>20</td></tr><tr><td>Team</td><td>100</td></tr></tbody></table>
<h2>Out of scope</h2><p>Parquet output and scheduled exports are tracked separately.</p>` },
    { id: "api", title: "Export API reference", group: "API", owner: "h:dev", updated: ago(60 * 24 * 9), fresh: true, links: ["LAT-104"],
      claims: [{ t: "POST /v1/exports returns 202 with a job id", s: "src/export/api.rs", ok: true }],
      body: `<h2>Create an export</h2><pre><code>POST /v1/exports
{ "report_id": "rpt_123", "format": "csv" }

202 Accepted
{ "export_id": "exp_789", "status": "queued" }</code></pre>
<h2>Errors</h2><ul><li><code>403</code> caller cannot view the report</li><li><code>429</code> rate limit exceeded, see <code>Retry-After</code></li></ul>` },
    { id: "runbook", title: "Export runbook", group: "Runbooks", owner: "a:docs-bot", updated: ago(60 * 24 * 20), fresh: false, links: ["LAT-111", "LAT-104"],
      claims: [{ t: "Rate limit is 60 requests per minute per workspace", s: "this page", ok: false, observed: "Deployment prod v2.4 allows 100 per minute" }],
      body: `<p>Use this runbook when exports are slow, failing or being throttled.</p>
<h2>Throttling</h2><p id="rl">The gateway allows <strong>60 requests per minute</strong> per workspace. If a customer reports 429 responses, check the <code>rate_buckets</code> table for their workspace.</p>
<h2>Slow exports</h2><ol><li>Check the export service dashboard for chunk write latency</li><li>Confirm object storage is healthy</li><li>If needed, restart the export workers</li></ol>` },
    { id: "ux", title: "Export UX notes", group: "Design", owner: "h:lina", updated: ago(60 * 24 * 1), fresh: true, links: ["LAT-105"],
      claims: [{ t: "Button uses the primary token", s: "design tokens", ok: true }],
      body: `<p>The export button has four states: idle, loading, disabled and error. Option B keeps the label visible while loading and shows progress inline.</p>
<h2>Empty state</h2><p>When no exports exist the page explains how exports work and offers a single call to action.</p>` },
  ];

  const drifts = [
    { id: "D-1", kind: "Doc vs deployment", sev: "high", title: "Export runbook says 60 requests per minute", doc: "runbook", ticket: "LAT-111",
      claim: "The gateway allows 60 requests per minute per workspace.", observed: "Deployment prod v2.4 reports a limit of 100 per minute (observed 12 minutes ago).",
      proposal: { by: "a:docs-bot", before: "60 requests per minute", after: "100 requests per minute (Team plan) and 20 (Free plan)" }, state: "proposed" },
    { id: "D-2", kind: "Ticket vs pull request", sev: "low", title: "LAT-103 is In Review but PR #482 has new commits", doc: null, ticket: "LAT-103",
      claim: "Review requested on the latest commit.", observed: "Two commits were pushed after the last review.", proposal: null, state: "detected" },
  ];

  const kb = [
    { re: /rate|limit|429|throttl/i, text: "Rate limiting is enforced in the gateway middleware. Requests are counted per workspace in a one-minute sliding window stored in <code>rate_buckets</code>. Production currently allows 100 per minute on the Team plan and 20 on Free. When the limit is exceeded the gateway returns 429 with a <code>Retry-After</code> header.",
      cites: [{ t: "file", l: "src/export/gateway.rs" }, { t: "ticket", l: "LAT-104" }, { t: "doc", l: "Export runbook" }], conf: 0.86 },
    { re: /stream|chunk|memory|resume/i, text: "Exports above 100k rows are written in chunks. Each chunk is stored as a row in <code>export_chunks</code> (sequence number, blob reference, size). The download endpoint reads chunks in order and supports <code>Range</code> requests, which is how a broken download resumes.",
      cites: [{ t: "file", l: "src/export/stream.rs" }, { t: "ticket", l: "LAT-102" }, { t: "doc", l: "Bulk export spec" }], conf: 0.9 },
    { re: /2m|million|rows|504|timeout|fail/i, text: "The most likely cause is the gateway request timeout (30 seconds) rather than the chunk writer. The writer streams, but the gateway waits for the first byte of a response that is only produced after the query plan finishes. Suggested fix: return 202 immediately and let the client poll, which the API already supports, or raise the timeout for the export route. I could not reproduce it from read access alone, so treat this as a hypothesis to verify with the incident logs.",
      cites: [{ t: "ticket", l: "LAT-106" }, { t: "file", l: "src/export/api.rs" }, { t: "ticket", l: "INC-77" }], conf: 0.62 },
    { re: /encod|arabic|bom|excel|utf/i, text: "Excel does not assume UTF-8 for CSV files without a byte order mark, so non-ASCII headers appear garbled. Writing a UTF-8 BOM at the start of the file fixes it for Excel while remaining valid for other tools.",
      cites: [{ t: "ticket", l: "LAT-110" }, { t: "file", l: "src/export/csv.rs" }], conf: 0.88 },
    { re: /email|mail|notify/i, text: "Completion emails are sent by the notification worker, which listens for the <code>export.finished</code> event. The download link is signed and expires after 24 hours.",
      cites: [{ t: "file", l: "src/notify/export_mail.rs" }, { t: "ticket", l: "LAT-103" }], conf: 0.84 },
  ];

  const questions = [
    { id: "Q-12", title: "How does export streaming work?", kind: "how", by: "h:omar", assignee: "a:analyst-1", state: "answered", at: ago(60 * 5),
      body: "I need to explain to a customer why large exports no longer time out. How does it work end to end?", scope: "all spaces",
      answer: { html: kb[1].text, cites: kb[1].cites, conf: kb[1].conf, at: ago(60 * 5 - 4) } },
    { id: "Q-13", title: "Why do exports over 2M rows return 504?", kind: "fix", by: "h:dev", assignee: "a:analyst-1", state: "answered", at: ago(60 * 3),
      body: "Related to LAT-106. Looking for the cause and a suggested fix. No code changes please.", scope: "space LAT",
      answer: { html: kb[2].text, cites: kb[2].cites, conf: kb[2].conf, at: ago(60 * 3 - 5) } },
    { id: "Q-14", title: "Where is the export rate limit enforced?", kind: "how", by: "h:lina", assignee: null, state: "open", at: ago(40),
      body: "Design needs to know whether the limit is applied per user or per workspace to write the error copy.", scope: "all spaces", answer: null },
  ];

  const timeline = [
    { phase: "Spec", at: ago(60 * 24 * 41), who: "h:omar", what: "Bulk export spec created", ref: "Bulk export spec" },
    { phase: "Ticket", at: ago(60 * 24 * 38), who: "h:maya", what: "Epic LAT-100 and first stories created", ref: "LAT-100" },
    { phase: "Code", at: ago(60 * 24 * 33), who: "h:dev", what: "First code change: src/export/csv.rs", ref: "PR #431" },
    { phase: "Deploy", at: ago(60 * 24 * 27), who: "a:coder-1", what: "First deployment to production (v2.3)", ref: "prod v2.3" },
  ];

  window.LatticeData = { people, states, tickets, epics, docs, drifts, questions, kb, timeline, now };
})();
