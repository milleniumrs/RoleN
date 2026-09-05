# RoleN — Product Requirements Document

| Field | Value |
|---|---|
| Project | **RoleN** (`rolen`) |
| Type | Cross-platform TUI application + headless CLI |
| Language | Rust (stable), UI built on [AppCUI-rs](../AppCUI-rs) |
| Version | PRD v0.2 (implementation-synced) |
| Date | 2026-09-02 (v0.1 draft: 2026-08-16) |
| Status | Synced with code at v0.3.0; per-story finish states added |

---

## 1. Vision

RoleN is a **conductor for LLM-powered development**. A developer owns several LLM
subscriptions and tools (Claude, Kimi, GLM, OpenAI, Ollama cloud, Ollama local, CLI agents
such as `claude code`, `codex`, `gemini-cli`, …). Each has different strengths, prices and
remaining quotas. RoleN lets the developer:

1. **Register** every provider/subscription once (credentials, quotas, capabilities).
2. **Assign roles** (planner, summarizer, coder, tool-runner, image reader/writer,
   doc reader/writer, reviewer, …) to providers via declarative **rules** that react to
   remaining quota, cost, latency and task type.
3. **Describe a project** and be *interrogated* by RoleN until the PRD, `AGENTS.md` and
   skill suggestions are unambiguous — before any code is written.
4. **Run many projects in parallel**, and split one project into parallel tasks, while a
   central **single-writer orchestrator** guarantees no two agents ever write the same file
   at the same time.
5. **Watch everything**: live sessions, token burn per provider, quota forecasts, costs,
   alerts — in an AppCUI TUI, or scripted via a headless CLI.

RoleN is a **router + built-in agent runtime**: it dispatches work to external CLI agents
*and* speaks directly to HTTP APIs (OpenAI-compatible, Anthropic, Ollama) through its own
agent loop.

---

## 2. Goals / Non-Goals

### 2.1 Goals
- G1 — First-class support for **three provider categories**: API providers, CLI-subprocess
  providers, Ollama (local + cloud + remote-via-SSH). ✅
- G2 — Declarative **role → provider routing rules** with quota-aware fallbacks. ✅
- G3 — **Hybrid quota tracking**: API usage parsing, billing endpoints when available,
  CLI output parsing, manual budgets, local estimation — unified in one dashboard. 🟡 (estimation + manual budgets done; billing endpoints and CLI output parsing missing)
- G4 — **PRD/AGENTS.md/skill generation at the core**, driven by a clarification engine that
  exhaustively questions the user before and during a project. 🟡 (pre-project interview done; mid-project ambiguity detection missing)
- G5 — **Parallelism by design**: multiple projects, and multiple tasks per project, with a
  **single-writer orchestrator** (write queue) as the only component allowed to mutate files. 🟡 (parallel tasks + single writer done; multi-project in one process nominal)
- G6 — **TUI-first** (AppCUI) with a fully scriptable **headless CLI** (CI-friendly). ✅
- G7 — Secrets in the **OS keychain** with an encrypted-file fallback. ✅

### 2.2 Non-Goals (v1)
- NG1 — Not a model provider itself; no inference, no fine-tuning.
- NG2 — No cloud sync / multi-user / team features (single-developer machine scope).
- NG3 — No GUI (desktop/web) front-end; TUI + CLI only. *(An experimental egui/Dear ImGui
  GUI existed during development and was retired in commit `8c51951`.)*
- NG4 — No automatic merging of *semantic* conflicts (orchestrator prevents write races;
  semantic review stays with the user/planner).
- NG5 — No mobile/remote access.

---

## 3. Terminology

| Term | Meaning |
|---|---|
| **Provider** | A registered source of LLM capability: `api` (HTTP), `cli` (subprocess agent), `ollama-local`, `ollama-cloud`, `ollama-remote` (SSH tunnel, D8). |
| **Model** | A concrete model exposed by a provider (e.g. `kimi-k3`, `claude-opus-5`, `glm-5.2`, `qwen3:32b`). |
| **Role** | A named function in a workflow: `planner`, `summarizer`, `coder`, `tool-runner`, `image-reader`, `image-writer`, `doc-reader`, `doc-writer`, `reviewer`, `interrogator`, custom… |
| **Rule** | A declarative routing statement: *role + condition (quota/cost/task-type/time) → ordered provider fallback chain*. |
| **Subscription** | A provider account with credentials, plan limits, renewal date, and tracked consumption. |
| **Project** | A unit of work with its own PRD.md/PRD.json, AGENTS.md, skills, workspace directory and task graph. Stored as a directory `~/rolen-workspaces/<id>/` with `rolen-project.yaml`; the **id is a lowercase slug of the name**. |
| **Task** | A node in a project's DAG; assigned a role; executed by exactly one agent session; may depend on other tasks. |
| **Session** | One running agent loop (built-in runtime or wrapped CLI) bound to a task, streaming events and token usage. |
| **Orchestrator** | Central component: schedules tasks, owns the **write queue**, and is the *only* writer to project files. |
| **Write Ticket** | An atomic file-mutation request (path, op, content/patch, base-hash, task-id) submitted to the orchestrator. |
| **Interrogator** | The clarification engine role that generates questions from PRD gaps/ambiguities. |
| **Skill** | A reusable capability package (à la `SKILL.md`) suggested or installed for a project. |

---

## 4. Personas & Primary Use Cases

**Persona — “Solo power developer” (primary).** Owns 3–8 LLM subscriptions, codes daily,
wants maximum throughput per dollar, works on several repos at once.

Story status legend: ✅ implemented · 🟡 partial · ❌ not implemented

- UC1 ✅ — Register a new provider in < 2 minutes (guided wizard, keychain storage, quota probe).
  *Also: `rolen provider detect --register` auto-discovers Ollama and CLI agents.*
- UC2 ✅ — Write a rule: *“Use Kimi-K3 for planning while its monthly quota > 20 %, else
  Claude-Opus-5, else Ollama-cloud `deepseek-v3`”* — via TUI rule editor or YAML.
- UC3 ✅ — Start a new project: RoleN interviews the user, produces PRD.md → PRD.json →
  AGENTS.md → suggested skills, then a task DAG for approval
  (`rolen project new` → `project interview` → `project build`).
- UC4 🟡 — Run 3 projects in parallel; each project splits into 2–6 parallel tasks; watch a
  dashboard of sessions, tokens, quota forecasts; receive alerts at 80 %/95 % burn.
  *Parallel tasks within one spec work (`rolen batch`); one process runs one spec at a time;
  alerts are notify-only (no auto rule-switch/pause); no quota-forecast yet.*
- UC5 🟡 — Run `rolen run --role coder --task "…" --json` / `rolen batch --spec tasks.yaml
  --watch` from CI. *CI-safe exit codes and JSON/NDJSON outputs exist; the originally
  planned `--project P --task T --headless` semantics do not.*
- UC6 ✅ — Mid-project, a provider hits its limit: routing falls back automatically per rules;
  sessions migrate with context hand-off; user is notified, not blocked.

---

## 5. Functional Requirements

Priority: **P0** = MVP must-have, **P1** = v1.0 should-have, **P2** = later could-have.

Status legend: ✅ implemented · 🟡 partial · ❌ not implemented (verified against code @ v0.3.0, 2026-09-02)

### FR-1 Provider Management — P0 — ✅ (minor gaps)
- FR-1.1 ✅ Register/edit/remove providers of types `api`, `cli`, `ollama-local`, `ollama-cloud` — **plus `ollama-remote`** (SSH port-forward, D8).
- FR-1.2 🟡 Provider wizard: endpoint/CLI-path detection (`which claude`, `ollama list`), auth test, model discovery; capability probing only fills `context_tokens`/`streaming` for Ollama — `vision`/`tools` are never probed for API providers.
- FR-1.3 🟡 Health checks on demand (`provider health`); **no timer-based probes**, no latency history.
- FR-1.4 ✅ Model capability matrix persisted per provider (editable overrides).
- FR-1.5 ✅ *P1* **done beyond spec**: cost table per model incl. cache-read/write rates, billing kinds (Free/PerToken/Subscription), manual model entry (`pricing.toml`, Model Prices window).

### FR-2 Credentials & Secrets — P0 — ✅
- FR-2.1 ✅ Store secrets in OS keychain (Windows Credential Manager, macOS Keychain,
  Secret Service) via `keyring`; config stores only key references.
- FR-2.2 ✅ Encrypted fallback vault (age-encrypted `vault.age`, master password via `ROLEN_VAULT_PASSWORD`).
- FR-2.3 ✅ Env-var injection for headless/CI (`ROLEN_KEY_<KEY_REF>` — sanitized key ref, not provider id), never logged. Extra: `ROLEN_SECRETS_BACKEND=vault|keychain` forces a backend.
- FR-2.4 🟡 Secrets never appear in config/logs by construction (only key refs), but there is **no active redaction layer** — wrapped-CLI PTY transcripts are written verbatim.

### FR-3 Roles & Routing Rules — P0 — ✅
- FR-3.1 ✅ Built-in role catalog + user-defined roles.
- FR-3.2 ✅ Rule DSL (**YAML canonical**, `rules.yaml` + TUI editor): conditions on
  `quota_remaining_%`, `cost_so_far`, `task_type`, `project`, `time_of_day`,
  `provider_health`; result = ordered fallback chain of `provider/model`.
- FR-3.3 ✅ Rule evaluation at task dispatch **and** mid-session on quota-exceeded events (session migration with `Migrated` event).
- FR-3.4 ✅ Dry-run / explain: `rolen rule dry-run --role X` + TUI editor dry-run panel.
- FR-3.5 🟡 *P1*: rule-level `project_scope` works, but `Project.rules_override` is never read; *P2* learned cost/latency heuristics ❌ absent.

### FR-4 Quota & Token Tracking — P0 — 🟡
- FR-4.1 🟡 API providers: `usage` parsing from responses (incl. cache buckets) ✅; **billing-endpoint polling**: data-driven via per-provider `quota_url` + `quota_json_path` in providers.toml, polled on demand with `rolen provider sync-quota` ✅ (no automatic timer).
- FR-4.2 ✅ CLI providers: chars/4 token estimation + manual budget ✅; **CLI quota-output parsing** via the adapter's `quota_args`/`quota_regex` in cli-adapters.toml, run by `rolen provider sync-quota` (named groups `used`/`limit` or capture groups 1/2).
- FR-4.3 ✅ Ollama local: treated as unlimited-but-metered (still counted for stats).
- FR-4.4 ✅ Subscription profiles: plan limit, **cycle start + renewal dates** (`rolen provider budget --cycle-start/--renewal YYYY-MM-DD`), projected exhaustion via burn rate ("empty in Nd" in the Providers tab). Synced numbers (Api/Parsed source) override ledger estimates in quota computation.
- FR-4.5 ✅ Alerts at configurable thresholds (default 80 %/95 %); all actions implemented: **notify** (TUI popup), **switch-rule** (provider auto-suspended → routing fallbacks engage; `rolen provider suspend/resume`), **pause-role** (roles routed through the provider are paused until `rolen rule resume --role X`).
- FR-4.6 ✅ Persistent SQLite ledger of every request (tokens incl. cache buckets, cost, latency, task, session) **plus CSV/JSON export** via `rolen export --what ledger|sessions|tickets --format csv|json`.

### FR-5 Project Core: PRD, AGENTS.md, Skills — P0 — ✅ (minor gaps)
- FR-5.1 ✅ Project scaffolding wizard: name, workspace dir, repo init (git), language/stack.
- FR-5.2 ✅ **PRD builder**: PRD.md drafted from interview answers, compiled to schema-versioned PRD.json with `rolen prd --validate`; **TUI review before writing** — Build shows a preview (full content on first build, unified diff on rebuild) with Apply/Discard.
- FR-5.3 ✅ **AGENTS.md generator** from PRD.json + stack + skills; regeneration is **diff-previewed** (`patch::simple_diff`) and requires Apply before anything is overwritten. *(Still manual Build-triggered — no automatic change detection.)*
- FR-5.4 ✅ **Skill suggestions** from a local skill registry (built-in + user dirs); one-click install into the project.
- FR-5.5 ✅ *P1* **done**: PRD sections → initial task DAG proposal (`tasks.yaml` via daggen).

### FR-6 Clarification Engine (Interrogator) — P0 — 🟡
- FR-6.1 ✅ Before planning, the interrogator role produces targeted questions covering:
  scope edges, data models, error handling, auth, performance targets, platforms,
  i18n, accessibility, testing strategy, deployment, licensing, “definition of done”.
- FR-6.2 ✅ Answers recorded into PRD.json (`clarifications[]`) with timestamps; **TUI forms with real controls** — radio buttons when a question has options, text field otherwise, explicit "answer later" (defer) — used by both the interview and the Questions tab. *(No datepicker questions yet — none of the 14 topics generate dates.)*
- FR-6.3 ✅ **Implemented**: `ask_user` records a pending question (stamped with `task_id`) into the project's `rolen-project.yaml` — visible in the TUI Questions tab; the scheduler **pauses tasks whose (transitive) dependencies have unanswered questions**; the asking task itself proceeds with a documented assumption (non-blocking).
- FR-6.4 ✅ Question budget & modes: `thorough` (default, 14 topics), `balanced` (8), `minimal` (4).
- FR-6.5 🟡 Question ↔ answer traceable via PRD.json ✅; **`linked_prd_path` always null, no question ↔ task linkage ❌**.

### FR-7 Orchestrator & Single-Writer File System — P0 — ✅ (two gaps)
- FR-7.1 ✅ **The orchestrator is the only component that writes project files.**
  Agents never touch the disk directly; they emit **write tickets**.
- FR-7.2 ✅ Write ticket: `{task_id, path, op: create|patch|replace|delete|rename, content|diff, base_hash}` (the `priority` field was dropped in implementation).
- FR-7.3 ✅ **Global write queue** with *per-path serialization*: FIFO per file; disjoint files apply concurrently.
- FR-7.4 🟡 **Optimistic concurrency** via `base_hash` (stale → rejected → agent re-reads/re-issues) ✅; **orchestrator-mediated read-your-writes ❌** (reads go straight to disk).
- FR-7.5 ✅ **Task ownership by design**: scheduler enforces non-overlapping path claims; overlapping claims block; “honesty check” fails tasks whose claimed files were never written.
- FR-7.6 ✅ **Atomic writes**: temp-file + rename.
- FR-7.7 ✅ **Audit & rollback**: every ticket journaled in SQLite; git auto-commit checkpoints per completed task.
- FR-7.8 🟡 **Backpressure done**: `parallelism.queue_cap` (default 1000) blocks submitters when the queue is full; **live queue depth in the TUI appbar** (from the cross-process ticket journal). *Remaining*: per-project fairness quotas (one process runs one project today, so this matters once multi-project runs land).
- FR-7.9 ✅ *P1* **done**: unified-diff `patch` tickets applied with fuzzy 3-way context matching (declared position, then unique-match search; ambiguous/failed hunks reject the whole ticket).

### FR-8 Parallel Execution — P0 — 🟡
- FR-8.1 🟡 Parallel DAG execution with configurable caps ✅ (CPU heuristic `max(2, logical_cpus/2)`, per-provider limits); **N projects concurrently in one process ❌** (one spec per `batch` run).
- FR-8.2 🟡 Workspace isolation: separate directories under `~/rolen-workspaces` ✅; *P1* git-worktree mode ❌.
- FR-8.3 ✅ Session-per-task; sessions stream events to the orchestrator bus.
- FR-8.4 🟡 **Cancel**: Ctrl+C on `rolen run`/`batch` stops agents gracefully between steps; cancelled sessions are marked `interrupted` and keep a **context snapshot** (`snapshots/<session>.json`), resumable via `rolen run --resume`. **Pause/resume**: cooperative pause flag pauses agents between steps (session marked `paused`, snapshot written). *Remaining*: TUI pause buttons (in-TUI runs are still stubs) and wrapped-CLI session checkpointing (P1).
- FR-8.5 ✅ Dependency-aware scheduling: a task unblocks only when its DAG predecessors completed *and* their write tickets are fully applied.

### FR-9 Sessions & Monitoring — P0 — 🟡
- FR-9.1 ✅ Dashboard per session: **role** (new sessions-table column, schema-migrated), provider/model, state, tokens, cost, rate (tokens/min), elapsed. *(No last-event column — transcript covers it.)*
- FR-9.2 ✅ Per-provider panels: quota %, today's tokens/cost, **burn rate (tokens/day) and exhaustion forecast (“empty in Nd”)** for budgeted providers. *(Progress-bar gauges remain text — cosmetic.)*
- FR-9.3 🟡 **Transcripts for every built-in-runtime session** now written to `transcripts/<session>.md` (routed/text/tool/retry/migration/done events) and viewable in the TUI ✅; quick-chat sessions still transcript-less; viewer remains read-only without search/export ❌.
- FR-9.4 🟡 TUI popup notifications ✅; **OS toast on critical quota alerts** (opt-in `general.os_notifications`, Settings checkbox; PowerShell balloon on Windows, osascript/notify-send elsewhere) ✅. *Remaining: toasts for pending questions/task failures.*
- FR-9.5 ❌ *P1*: historical analytics not implemented.

### FR-10 TUI (AppCUI) — P0 — 🟡
Screens implemented as six tabs: Dashboard, Projects, Providers, Rules, Questions, Activity.
- FR-10.1 🟡 **Dashboard**: sessions listview + provider quota (text %) ✅; **no progress-bar gauges, static queue-depth label, no alerts ticker ❌**.
- FR-10.2 🟡 **Providers**: table + wizard dialogs + capability matrix ✅; **no health graph view ❌**.
- FR-10.3 ✅ **Rules editor**: condition builder, fallback-chain editor with up/down ordering, dry-run panel.
- FR-10.4 ✅ **Projects**: **treeview (project → tasks from tasks.yaml → their sessions)**, pending-question count and PRD/AGENTS.md marks on project nodes; Enter opens the **project detail window** (PRD.md / AGENTS.md / skills / clarifications tabs) on a project node or the transcript on a session node.
- FR-10.5 🟡 **Chat/Session view**: Quick Chat window (multi-turn, ledgered) ✅ but **cannot steer a running agent session ❌**; Activity tab shows live PTY output of one wrapped CLI task (read-only); **no ticket/ledger side panel ❌**.
- FR-10.6 ✅ **Interrogation center**: Questions tab lists pending question batches across all projects with answering.
- FR-10.7 🟡 Menus, mouse support, **13 named themes** with live switching + persistence ✅ (beyond spec); **keybindings hardcoded, not configurable ❌**.
- FR-10.8 ❌ *P2*: no split layouts. In-TUI project/batch run is also a stub.

### FR-11 Headless CLI — P0 — 🟡
Binary: `rolen`. Actual subcommands: `tui, config, provider, quota, rule, run, batch, project, prd, cli`.
- FR-11.1 🟡 Implemented: `provider` (add/remove/models/test/detect/health/budget/suspend/resume), `rule` (init/list/add/remove/dry-run/pause/resume), `project` (new/list/interview/build/skills), `prd`, `quota`, `run` (incl. `--resume`), `batch`, `cli`, `config` (incl. `export`/`import`), **`sessions`, `export`**, `tui`. **Missing vs plan: top-level `agents`, `skills` ❌** (skills live under `project skills`; AGENTS.md generation under `project build`).
- FR-11.2 ✅ `rolen run --role R --task "<desc>" [--provider --model --workdir --max-steps --allow-shell --json --resume]`; **`rolen run --project P [--task T]`**: with `--project`, `--task` selects a task id from the project's `tasks.yaml` (role/instructions/claimed paths from the DAG); omitting `--task` runs the whole DAG through the orchestrator; `--headless` accepted for CI parity. Exit codes: 0 ok / 1 failure / 130 cancelled.
- FR-11.3 ✅ All state mutated identically via TUI and CLI (single core library).
- FR-11.4 ✅ *P1* **done**: NDJSON event stream — implemented as `rolen batch --watch`.

### FR-12 Built-in Agent Runtime — P0 — ✅ (one gap)
- FR-12.1 ✅ Agent loop: chat with tool calls → tools executed → results fed back, until done (with retry, tool-call salvaging, completion nudges).
- FR-12.2 ✅ Standard tools: `read_file`, `search`, `list_dir`, `run_shell` (sandboxed, allow-listed), `submit_write` (→ write ticket), `ask_user` (stub, see FR-6.3). *No `write_file` tool exists.*
- FR-12.3 ✅ Context management: approaching the context limit, the middle of the history is **summarized by the LLM into a structured hand-off note** (GOAL / DECISIONS / FILES TOUCHED / OPEN QUESTIONS / NEXT STEPS); falls back to drop-middle with a marker when the model is unreachable. The summary call is ledgered like any other.
- FR-12.4 ❌ *P1*: no MCP client anywhere in the codebase.

### FR-13 CLI-Provider Wrapping — P0 — ✅ (minor gaps)
- FR-13.1 🟡 Spawn CLI agents (`claude`, `codex`, `gemini`, `kimi`, …) as PTY subprocesses (`portable-pty`/ConPTY) ✅; output forwarded as **raw chunks — no semantic stream-parsing ❌**.
- FR-13.2 ✅ **Overlay + harvest**: wrapped CLI agents run against a staging-copy overlay; their writes are diffed, harvested and **re-applied through the orchestrator write queue**.
- FR-13.3 ✅ **Data-driven adapters**: `cli-adapters.toml` (`[[adapter]] match = "<stem-substring>", args = [...]`) extends/overrides the built-in templates with no recompile (NFR-6); built-ins remain for claude/codex/gemini/kimi + generic `-p` fallback. *Remaining*: per-CLI quota parsers ❌.

### FR-14 Configuration & Storage — P0 — 🟡
- FR-14.1 ✅ Human-editable config (`config.toml`, `providers.toml`, **`rules.yaml`** (D2), `subscriptions.toml`, **`pricing.toml`** (extra)) under `~/.config/rolen` (XDG) / `%APPDATA%\rolen`, plus `skills/` library dir.
- FR-14.2 ✅ SQLite state DB (`ledger.sqlite3`: ledger, sessions, tickets journal, PRD cache) with schema migrations.
- FR-14.3 🟡 `rolen config doctor` validates ✅; SQLite schema versioned/migrated ✅; **`config.toml` itself has no schema-version field or migrations ❌**.
- FR-14.4 ✅ **Implemented**: `rolen config export [--out file]` writes a JSON bundle of config/providers/rules/subscriptions/pricing (secrets excluded — keychain refs only); `rolen config import --from file` restores with `.bak` backups of existing files.

---

## 6. Non-Functional Requirements

- NFR-1 **Platforms**: Windows 10+, macOS 13+, Linux (AppCUI backends); x64 + arm64. ✅ (CI builds all targets)
- NFR-2 **Performance**: TUI ≥ 30 fps feel, < 100 ms input latency; orchestrator sustains
  ≥ 20 concurrent sessions and ≥ 200 write tickets/s on a mid-range laptop. *(not benchmarked)*
- NFR-3 **Reliability**: crash-safe journaling; on restart, incomplete tickets are
  reconciled, sessions marked recoverable/interrupted. ✅ (journaled tickets)
- NFR-4 **Security**: secrets only in keychain/vault (FR-2); shell tool sandboxed with
  per-project allow-lists; no telemetry by default (opt-in only). ✅
- NFR-5 **Testability**: core logic (`rolen-core`) UI-free; ≥ 70 % line coverage on
  routing, write-queue, and ledger; golden-file tests for PRD/AGENTS generators. *(coverage not measured)*
- NFR-6 **Extensibility**: provider adapters and skills are data-driven (drop-in TOML/MD),
  no recompile needed. ✅ *(CLI adapters: `cli-adapters.toml`; skills: `SKILL.md` registry)*
- NFR-7 **Localization-ready**: strings externalized (AppCUI supports Unicode).

---

## 7. Architecture Overview

```
┌──────────────────────────── RoleN ──────────────────────────────┐
│  rolen-tui (AppCUI)          rolen-cli (headless)               │
│        │                            │                           │
│        └───────────┬────────────────┘                           │
│                    ▼                                            │
│             rolen-core                                          │
│  ┌─────────────┬───────────────┬───────────────┬─────────────┐  │
│  │ Rule Engine │ Orchestrator  │ Quota/Ledger  │ Interrogator│  │
│  │  (FR-3)     │ (FR-7, FR-8)  │   (FR-4)      │   (FR-6)    │  │
│  └─────┬───────┴──────┬────────┴───────┬───────┴──────┬──────┘  │
│        ▼              ▼                ▼              ▼         │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                 Event Bus + SQLite journal               │   │
│  └──────────────────────────────────────────────────────────┘   │
│  ┌──────────────┐  ┌──────────────────┐  ┌───────────────────┐  │
│  │ Agent Runtime│  │ CLI adapters(PTY)│  │ Provider HTTP API │  │
│  │  (FR-12)     │  │    (FR-13)       │  │   clients (FR-1)  │  │
│  └──────────────┘  └──────────────────┘  └───────────────────┘  │
│        └──────────► write tickets ──► ORCHESTRATOR (sole writer)│
└─────────────────────────────────────────────────────────────────┘
```

Key crates (workspace): `rolen-core`, `rolen-orchestrator`, `rolen-providers`,
`rolen-runtime`, `rolen-cliadapters`, `rolen-tui`, `rolen-cli`.
Depends on AppCUI from crates.io, pinned (`appcui = "=0.4.17"`).

### Implemented beyond this PRD (v0.2.0–v0.3.0)
- **Quick Chat** window (Ctrl+Q): multi-turn ledgered conversation, any provider.
- **Settings window** (F10): workspace root, theme, question mode, parallelism caps, alert thresholds.
- **13 named colour themes** with live switching and persistence; offscreen theme-verification tooling (`docs/theme_report.py`).
- **Model Prices window**: cache-read/write rates, billing kinds (Free/PerToken/Subscription), manual model entry.
- **Activity tab**: live PTY output of a wrapped CLI task.
- **Anthropic OAuth subscription import** from opencode `auth.json` with auto-refresh (D9).
- **ollama-remote** SSH tunnel manager (D8).
- CLI extras: `provider detect [--register]`, `provider budget`, `provider test`, `rule init`, `rule dry-run`, `batch`, `cli run`, `config path`.
- Agent-loop robustness: tool-call JSON salvaging, completion guard, scheduler “honesty check”.

---

## 8. Data Model (essentials, as implemented)

- `Provider { id, type: api|cli|ollama-local|ollama-cloud|ollama-remote, endpoint|cli_path, key_ref, models[], suspended, capabilities, cost_table? }`
- `Subscription { provider_id, plan_limit, used, cycle_start, renewal, source: api|parsed|manual|estimated }` *(cycle_start/renewal currently unused)*
- `Rule { id, role, conditions[], fallback_chain[], priority, project_scope? }`
- `Project { id /* slug of name */, name, dir, prd_json, agents_md_hash, skills[], tasks[], rules_override? /* unused */ }` — stored as `rolen-project.yaml`; DAG lives in `tasks.yaml`
- `Task { id, project_id, role, title, deps[], claimed_paths[], state, session_id? }`
- `Session { id, task_id, provider_id, model, state, tokens_in, tokens_out, cost, started, transcript_path /* set only for wrapped-CLI sessions */ }`
- `WriteTicket { id, task_id, path, op, payload, base_hash, state: queued|applied|rejected, ts }` *(no priority field)*
- `LedgerEntry { id, session_id, provider_id, tokens_in, tokens_out, cache_read, cache_write_5m, cache_write_1h, cost, latency_ms, ts }`
- `Clarification { id, project_id, task_id?, question, options?, answer?, status, linked_prd_path /* currently always null */, ts }`

---

## 9. Milestones

| # | Milestone | Contents | Exit criteria |
|---|---|---|---|
| M0 | Skeleton ✅ (2026-08-16) | workspace, core types, config, keychain, SQLite, AppCUI hello-window | `rolen config doctor` passes; TUI opens |
| M1 | Providers + Ledger ✅ (2026-08-16) | FR-1, FR-2, FR-4 for API + Ollama | register provider, send test prompt, see tokens in dashboard |
| M2 | Rules + Runtime ✅ (2026-08-16) | FR-3, FR-12 | role→provider routing with quota fallback works headless |
| M3 | Orchestrator ✅ (2026-08-16) | FR-7, FR-8 | 2 parallel agents, 100 % of writes via queue, hash-reject test passes |
| M4 | Project core ✅ (2026-08-16) | FR-5, FR-6 | new project → interview → PRD.md/PRD.json/AGENTS.md/skills |
| M5 | CLI adapters ✅ (2026-08-16)¹ | FR-13 | wrap `claude`/`codex` session in dashboard |
| M6 | TUI complete ✅ (2026-08-16) | FR-9, FR-10 | all screens, alerts, transcripts |
| M7 | Headless polish ✅ (2026-08-16)² | FR-11, packaging (MSI/winget, brew, cargo install) | CI pipeline example runs green |
| 0.2.0/0.3.0 | Post-PRD releases ✅ (2026-08) | pricing model beyond FR-1.5, themes, Quick Chat, Settings; GUI retired | release binaries on all targets |
| v1.0 | — | P1 items: worktrees, MCP, analytics + the remaining ❌/🟡 gaps in §5 (FR-7.8 fairness, FR-8.4 TUI/CLI-adapter checkpointing, FR-14.4 export, …) | soak test: 3 projects × 4 tasks for 4 h |

¹ M5 verified with a mock agent CLI (PTY + overlay + harvest + ledger all exercised) AND live with the real `claude` CLI after user re-auth (2026-08-16): 2 writes harvested via the queue, exit 0.

² M7: `--json` on run/quota, NDJSON `batch --watch`, clippy clean, fmt scoped (CI at .github/workflows/ci.yml), release binary verified. Store packaging (winget/brew) intentionally deferred to v1.0.

---

## 10. Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| CLI tools change output formats → quota parsing breaks | Med | adapter version pinning, parser tests, manual-budget fallback |
| Single-writer queue becomes throughput bottleneck | Med | per-path concurrency, batching, benchmark at M3 (target in NFR-2) |
| Agents produce conflicting *semantic* changes (not file races) | High | planner-level path ownership + reviewer role + git checkpoints |
| Provider APIs differ subtly (tool calling, streaming) | Med | capability matrix + per-provider conformance tests |
| User fatigue from too many questions | Med | question modes (FR-6.4), batching, “answer later” with task pausing |
| Context loss on mid-session provider switch | Med | summarization hand-off (FR-12.3) before migration |
| AppCUI upstream changes | Low | pinned crates.io version `=0.4.17` |

---

## 11. Resolved Decisions (2026-08-16)

| # | Question | Decision |
|---|---|---|
| D1 | License | **MIT** (changed 2026-08-16 from dual MIT OR Apache-2.0) |
| D2 | Rule DSL format | **YAML canonical** (`rules.yaml`); TUI edits round-trip to YAML |
| D3 | CLI agent file writes | **Overlay + harvest** — writes re-applied via orchestrator queue (FR-13.2) |
| D4 | Skill format | **`SKILL.md` convention** (YAML front-matter + resources/) |
| D5 | Session migration format | **Structured snapshot + summary**: goal, decisions-so-far, task state, condensed conversation, file manifests |
| D6 | Default parallelism cap | **CPU heuristic** `max(2, logical_cpus / 2)` **+ per-provider concurrency limits**, user-tunable |
| D7 | Windows PTY | **`portable-pty` (ConPTY on Windows)**, single abstraction for all CLI adapters |
| D8 | Remote Ollama (added 2026-08-16) | **`ollama-remote` provider type**: RoleN manages an `ssh -N -L` port-forward (system ssh, `~/.ssh` keys, BatchMode, accept-new host keys); provider points at the local forwarded port |
| D9 | Anthropic subscription auth (added 2026-08-16) | **OAuth access/refresh tokens** imported from opencode `auth.json`, stored as JSON in the keychain, auto-refreshed via the token endpoint; `anthropic-beta: oauth-2025-04-20` header |
| D10 | Project identity (added 2026-09-02) | **Project id = lowercase slug of the name** (also the workspace directory name); lookup accepts id or case-insensitive display name; no UUID registry |

UI wireframes and interaction flows: see [`docs/TUI-DESIGN.md`](docs/TUI-DESIGN.md).
