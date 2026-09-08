# RoleN Roadmap

Status snapshot: 2026-09-08, synced with `REQUIREMENTS.md` / `REQUIREMENTS.json` after the multi-project, provider-cap and wrapped-CLI checkpointing work.

Current requirement counts:

- Functional requirements: **54 done · 15 partial · 3 missing**
- Non-functional requirements: **5 done · 1 partial · 1 unverified**
- Milestones: **M0–M7 and 0.2.0/0.3.0 done · v1.0 pending**

Legend: ✅ done · 🟡 partially implemented · ❌ not implemented / not verified

---

## 1. Done ✅

### Provider, secrets and quota core
- FR-1.1 provider registration/edit/remove for `api`, `cli`, `ollama-local`, `ollama-cloud`, `ollama-remote`.
- FR-1.4 persisted per-provider model capability matrix with editable overrides.
- FR-1.5 per-model pricing with cache-read/write rates, billing kinds and manual model entry.
- FR-2.1/FR-2.2/FR-2.3 OS keychain secrets, age vault fallback, CI env-var injection and backend forcing.
- FR-4.2 CLI token estimation + manual budgets + data-driven CLI quota probes.
- FR-4.3 Ollama local unlimited-but-metered accounting.
- FR-4.4 subscription profiles with cycle/renewal dates, burn rate and exhaustion forecast.
- FR-4.5 quota threshold alerts with notify / switch-rule / pause-role actions.
- FR-4.6 SQLite ledger for every request plus CSV/JSON export.

### Rules, routing and projects
- FR-3.1 built-in + custom roles.
- FR-3.2 YAML-canonical rule DSL with TUI editor and ordered fallback chains.
- FR-3.3 dispatch-time and mid-session rule evaluation with provider migration.
- FR-3.4 rule dry-run/explain in CLI and TUI.
- FR-3.5 project-scoped rules and `rules_override` from `rolen-project.yaml`, merged ahead of global rules in runtime routing, migration and dry-run.
- FR-5.1–FR-5.5 project scaffold, interview-driven REQUIREMENTS.md/REQUIREMENTS.json, diff-previewed AGENTS.md, skill suggestions/install and Requirements→tasks.yaml DAG generation.
- FR-6.1–FR-6.5 clarification engine: question modes, real TUI answer forms, `ask_user` pending questions, dependency blocking, topic→Requirements-section links and REQUIREMENTS.json clarification refresh after answers.

### Orchestrator, parallelism and runtime
- FR-7.1–FR-7.3 single-writer orchestration with write tickets and per-path FIFO/concurrent disjoint-path application.
- FR-7.5 path ownership, overlap blocking and honesty check.
- FR-7.6 atomic temp+rename writes.
- FR-7.7 SQLite ticket journal + git checkpoint per completed task.
- FR-7.8 queue backpressure plus multi-project fairness: queue cap split across projects and round-robin slot rotation.
- FR-7.9 fuzzy 3-way unified-diff patch tickets.
- FR-8.1 parallel DAG execution, multi-project runs in one process via repeated `rolen run --project`, and enforced per-provider session permits.
- FR-8.3 session-per-task event streaming.
- FR-8.5 dependency-aware scheduling gated on completed predecessors and applied tickets.
- FR-12.1–FR-12.3 built-in agent loop, sandboxed tools, ticket-only writes and ledgered context compaction hand-off.
- FR-13.2/FR-13.3 CLI provider overlay+harvest through the write queue and data-driven `cli-adapters.toml`.

### TUI, CLI and storage
- FR-9.1 session dashboard columns including role, provider/model, state, tokens, cost, rate and elapsed.
- FR-9.2 provider panels with quota %, today's tokens/cost, burn rate and exhaustion forecast.
- FR-10.3 rules editor, FR-10.4 projects tree/detail view, FR-10.6 questions tab.
- FR-11.2–FR-11.4 headless run/batch/project execution, CI-safe exit codes and NDJSON event stream.
- FR-14.1–FR-14.4 human-editable config files, migrated SQLite schema, versioned `config.toml`, and config export/import with secrets excluded.

### Verified non-functional
- NFR-1 platform matrix in CI.
- NFR-3 crash-safe journaling and interrupted-session recovery.
- NFR-4 keychain/vault secrets, sandboxed shell tool, no default telemetry.
- NFR-6 data-driven skills and CLI adapters.
- NFR-7 localization-ready externalized strings/Unicode.

---

## 2. Partially implemented 🟡

- FR-1.2 provider wizard: detection/auth/model discovery work; API `vision`/`tools` capability probing still missing.
- FR-1.3 health checks: on-demand only; no timer-based probes or latency history.
- FR-2.4 secret hygiene: secrets are absent from config/logs by construction; no active redaction layer for wrapped-CLI PTY transcripts.
- FR-4.1 API quota sync: response usage parsing and on-demand billing-endpoint polling exist; no automatic sync timer.
- FR-7.4 optimistic concurrency: stale `base_hash` rejection works; orchestrator-mediated read-your-writes is still missing.
- FR-8.2 workspace isolation: separate workspace directories work; git-worktree mode is still missing.
- FR-8.4 cancel/pause/resume: built-in runtime and wrapped CLI cancel/snapshot/resume now work; wrapped CLI pause is pre-spawn only; TUI pause buttons remain.
- FR-9.3 transcripts: built-in runtime sessions are transcripted and viewable; quick-chat sessions are not; viewer has no search/export.
- FR-9.4 notifications: TUI popups and opt-in OS toasts for critical quota alerts exist; pending-question and task-failure toasts remain.
- FR-10.1 dashboard: sessions and text quota work; gauges, live queue-depth label and alerts ticker remain.
- FR-10.2 providers screen: table/wizard/capability matrix work; health graph view remains.
- FR-10.5 chat/session view: Quick Chat works; running-agent steering, Activity-tab interaction and ticket/ledger side panel remain.
- FR-10.7 menus/mouse/themes work; keybindings are still hardcoded.
- FR-11.1 headless CLI: broad command coverage exists; top-level `agents` and `skills` commands are still absent.
- FR-13.1 CLI wrapping: PTY spawn/streaming works; output is still raw chunks without semantic stream parsing.

---

## 3. Left undone ❌ / not verified

### Missing features
- FR-9.5 historical analytics.
- FR-10.8 split layouts and real in-TUI project/batch run.
- FR-12.4 MCP client.

### Not yet proven
- NFR-2 performance: `<100 ms` input latency, `>=20` concurrent sessions, `>=200` write tickets/s are still unbenchmarked.
- NFR-5 testability: routing/write-queue/ledger coverage target (`>=70%`) is not measured.
- v1.0 exit criteria: soak test of **3 projects × 4 tasks for 4 hours** has not run.
- Store packaging (`winget`, `brew`) remains deferred.

---

## 4. Recommended path to v1.0

1. **Prove the new execution model**
   - Run a small live smoke with two projects sharing one process.
   - Run the v1.0 soak test and record NFR-2 numbers while it runs.
2. **Close execution robustness gaps**
   - FR-7.4 orchestrator-mediated read-your-writes.
   - FR-8.2 git-worktree isolation.
   - FR-8.4 TUI pause buttons once in-TUI runs exist.
3. **Close provider/quota gaps**
   - FR-1.2 API capability probing.
   - FR-1.3 timer-based health probes + latency history.
   - FR-4.1 automatic quota sync timer.
4. **Add v1.0 integrations**
   - FR-12.4 MCP client.
   - FR-9.5 historical analytics.
   - FR-13.1 semantic CLI stream parsing where adapters support it.
5. **TUI completion**
   - FR-10.1/10.2/10.5/10.7/10.8 dashboard gauges, health graph, session steering, configurable keybindings and split layouts.
6. **Release hardening**
   - NFR-5 coverage measurement and targeted test backfill.
   - `winget`/`brew` packaging.
   - Final Requirements/status re-sync and release tag.

---

## 5. Recent completed milestones

- `8e82fbe` — multi-project DAG execution with fair queue sharing.
- `0d40ee3` — runtime-enforced per-provider session caps.
- Latest chunk — wrapped-CLI checkpointing, config schema migration, project rule overrides and question linkage.
