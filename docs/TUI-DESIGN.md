# RoleN — TUI Design

Companion to the design decisions (FR-10). All wireframes are schematic mock-ups of AppCUI
windows/controls (rendered as vector figures in the PDF build). AppCUI model: a **desktop** hosts multiple
**windows**; a bottom/top **app bar** shows global status; every action is reachable by keyboard, menu and mouse.

---

## 1. Design Principles

1. **One Mission Control, many satellites.** The main window (tabs) is for global state;
   every *project* and every *session* can open as its own satellite window so parallel
   work is visible side by side on the desktop.
2. **Live by default.** Every list/gauge subscribes to the event bus; no refresh buttons.
3. **Nothing hidden.** Pending questions, write-queue depth and quota alarms are always
   one glance away (app bar + dashboard ticker).
4. **Keyboard-first.** Global hotkeys work from any window; every form is tab-navigable.
5. **Same core for TUI and CLI** — the TUI is a view; `rolen-cli` can do everything.

---

## 2. Global Chrome

<!-- tikz: chrome -->
```
┌ RoleN ───────────────────────────────────────────────────────────────────────┐
│  File  Project  Providers  Rules  Sessions  View  Tools  Help                    │ ← menu bar
├──────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│                            (desktop: windows live here)                          │
│                                                                                  │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ● 4 sessions │ 61.2k tok today │ $1.84 │ queue: 3 │ ❓2 questions │ ⚠1 │ 14:32   │ ← app bar
└──────────────────────────────────────────────────────────────────────────────────┘
```

- **App bar segments** (left→right): active sessions • tokens today • cost today •
  write-queue depth • pending clarifications (click → Interrogation Center) • active
  alerts (click → alerts log) • clock. Segments are clickable buttons.
- **Menu map**:
  - *File*: New Project, Open Workspace, Import/Export Config, Quit
  - *Project*: Run/Pause/Resume, New Task, Interview, Regenerate AGENTS.md, Skills…
  - *Providers*: Add Provider, Detect CLIs, Health Check All, Quota Report
  - *Rules*: New Rule, Edit Rule, Delete Rule, Dry-Run, Import YAML
  - *Sessions*: Quick Chat, New Task Session, Pause All, Transcripts…
  - *View*: Mission Control, Tile Windows, Theme, Focus Mode
  - *Tools*: Settings, Config Doctor, Logs, Sandbox Allow-Lists
  - *Help*: Shortcuts, About

---

## 3. Mission Control (main window, opens maximized)

Tab control with 6 fixed tabs: **Dashboard · Projects · Providers · Rules · Questions · Activity**.

### 3.1 Dashboard tab

<!-- tikz: dashboard -->
```
├─[Dashboard]─[Projects]─[Providers]─[Rules]─[Questions (2)]─[Activity]──────────┤
│ ┌─ Active Sessions ─────────────────────┐ ┌─ Providers ──────────────────────┐ │
│ │ ▶ #12 coder    kimi-k2.7   shop   12k │ │ Claude    ████████░░ 78% ~12d    │ │
│ │ ▶ #13 planner  kimi-k3     shop  3.1k │ │ Kimi      █████░░░░░ 51% ~6d ⚠   │ │
│ │ ⏸ #14 doc-wr   glm-5.2     blog   890 │ │ GLM       █████████░ 92%         │ │
│ │ ▶ #15 tool     ollama:qwen3 shop  45k │ │ Ollama☁   unmetered  45k today   │ │
│ │ ▶ #16 img-rd   claude-opus blog  2.2k │ │ Ollama⛁   local      12k today   │ │
│ └───────────────────────────────────────┘ └──────────────────────────────────┘ │
│ ┌─ Write Queue ─────────────────────────┐ ┌─ Alerts ─────────────────────────┐ │
│ │ depth 3 · applied 1,204 · rejected 2  │ │ ⚠ Kimi burn 2.1× → ~6 days left  │ │
│ │ ▇▇▅▃▅▆ (last 60s)                     │ │ ❓ 2 question batches unanswered   │ │
│ └───────────────────────────────────────┘ └──────────────────────────────────┘ │
```

- Sessions listview: double-click → session window; right-click → pause/steer/kill/migrate.
- Provider gauges: progress bars + projected-exhaustion estimate; click → provider window.
- Write queue: depth + applied/rejected counters + sparkline (GraphView); click → ticket log.

### 3.2 Projects tab

<!-- tikz: projects -->
```
├─[…]─[Projects]─────────────────────────────────────────────────────────────────┤
│ ┌─ Projects ────────────────────────────────────────────────────────────────┐  │
│ │  name           state      tasks      tok today   cost    questions        │  │
│ │  shop-redesign  ▶ running  2/6 done   60.3k      $1.42    2 pending ⚠     │  │
│ │  blog-engine    ⏸ paused   1/4 done   3.1k       $0.41    —               │  │
│ │  scraper        ✅ done    5/5 done   —          $2.07    —               │  │
│ └────────────────────────────────────────────────────────────────────────────┘  │
│ [Enter] open window   [Ctrl+N] new project   [Del] archive   [I]nterview        │
```

### 3.3 Providers tab

<!-- tikz: providers -->
```
├─[…]─[Providers]────────────────────────────────────────────────────────────────┤
│ ┌───────────────────────────────────────────────────────────────────────────┐  │
│ │  name      type      status   models   quota        latency   cost today  │  │
│ │  Claude    api       ● ok     4        78% left     380ms     $0.92       │  │
│ │  Kimi      api       ● ok     3        51% left ⚠   520ms     $0.44       │  │
│ │  claude    cli       ● ok     —        parsed: 61%  —         —           │  │
│ │  Ollama⛁   ollama-l  ● ok     7        unmetered    90ms      $0.00       │  │
│ └───────────────────────────────────────────────────────────────────────────┘  │
│ [+ Add]  [Detect CLIs]  [Health Check]  double-click → Provider window          │
```

**Provider window** (double-click): tabs *Details · Models · Quota · Adapter*.
- Details: type, endpoint/CLI path, key reference (never the secret — `[••• stored in keychain]`,
  [Change] [Test] buttons).
- Models: capability matrix (context, vision, tools, streaming, $/M tok) — editable grid.
- Quota: plan limit, cycle, renewal date, source picker (`api/parsed/manual/estimated`),
  burn graph, alert thresholds.
- Adapter (CLI type only): invocation template, output parser, quota parser, overlay mode.

### 3.4 Rules tab

<!-- tikz: rules -->
```
├─[…]─[Rules]────────────────────────────────────────────────────────────────────┤
│ ┌─ rules ────────────────────────────────┐ ┌─ editor ────────────────────────┐ │
│ │ ● coder → kimi/glm/ollama              │ │ role:   [coder          ▾]      │ │
│ │ ○ planner → kimi-k3 → claude-opus      │ │ when:   [quota_remaining% < 20] │ │
│ │ ○ summarizer → kimi-k3 / opus (quota)  │ │         [on provider Kimi   ▾]  │ │
│ │ ○ tool-runner → ollama local           │ │ then:   1. [kimi-k2.7      ▾]   │ │
│ │ ○ img-reader → claude-opus             │ │         2. [glm-5.2        ▾]   │ │
│ └────────────────────────────────────────┘ │         3. [ollama☁/qwen3  ▾]   │ │
│                                            │  [▲ up] [▼ down] [+ add] [− del] │ │
│                                            │ scope: (•) all projects ( ) shop │ │
│                                            │ [Dry-run ▶] → “coder right now → │ │
│                                            │   kimi-k2.7 (Kimi 51% left)”     │ │
│                                            │ [Save] [Cancel]   YAML ⇄ view    │ │
│                                            └──────────────────────────────────┘ │
```

- Left: rule list (one per role+scope); the editor opens as a modal dialog
  (Rules ▸ New Rule / Edit Rule, or `Enter` on a row) with a condition builder
  (comboboxes + validated value field) and an **ordered fallback chain** editor
  (▲ up / ▼ down / + add / − remove).
- **Dry-run panel** in the editor evaluates the current (unsaved) form values
  against live quota state and explains the choice.
- "YAML ⇄" toggle shows the canonical YAML (decision D2) with live two-way sync.

### 3.5 Questions tab — Interrogation Center

<!-- tikz: questions -->
```
├─[…]─[Questions (2)]────────────────────────────────────────────────────────────┤
│ ┌─ pending batches ──────────────────────┐ ┌─ batch: shop-redesign · auth ───┐ │
│ │ ▶ shop-redesign · auth (4q)  blocks: 2 │ │ 1. Session store?               │ │
│ │   tasks #4,#7                          │ │    (•) JWT  ( ) cookies  ( ) ?  │ │
│ │   blog-engine · seo (2q)   advisory    │ │ 2. Password reset via email?    │ │
│ └────────────────────────────────────────┘ │    [x] yes → provider? [smtp ▾] │ │
│                                            │ 3. …                            │ │
│                                            │ [Answer & Continue] [Later]     │ │
│                                            └──────────────────────────────────┘ │
```

Answering unblocks paused tasks automatically (FR-6.3). "Later" keeps tasks paused but
never loses the batch.

### 3.6 Activity tab

Chronological ledger stream (filterable by project/provider/type): tickets applied,
rule decisions, quota events, migrations, alerts. Export CSV/JSON buttons.

---

## 4. Project Window (satellite, one per project)

Opened from Projects tab (`Enter`) or `rolen project open shop`.

<!-- tikz: project-window -->
```
┌ Project: shop-redesign ── running ─────────────────────────────────────────────┐
│ [▶ Run] [⏸ Pause] [+ Task] [❓ Interview] [PRD] [AGENTS.md] [Skills]      [⚙]  │ ← toolbar
├─ Tasks (DAG) ────────────────────┬─[Overview]─[Chat]─[Files]─[Ledger]──────────┤
│ ✅ 1 scaffolding                  │ Overview: goal, PRD summary, progress       │
│ ▶ 2 api-layer        coder  ●#12 │ bars, current blockers, next actions        │
│ ▶ 3 frontend         coder  ●#15 │                                             │
│ ⏸ 4 tests        deps: 2,3       │ Chat: live transcript of selected task's    │
│ ◌ 5 docs         deps: 4         │ session + steer input                       │
│ ◌ 6 deploy       deps: 5         │ Files: claimed paths per task + ticket log  │
│                                  │ Ledger: tokens/cost per task & provider     │
└──────────────────────────────────┴─────────────────────────────────────────────┘
```

- Left: **TreeView of the DAG** — state glyphs (✅ done, ▶ running, ⏸ paused/blocked,
  ◌ pending), role, live session id. Selecting a task binds the right-side tabs.
- Toolbar ❓ **Interview** opens the clarification form for this project; **AGENTS.md**
  previews the generated file with diff on regeneration; **Skills** opens the
  suggestion/install panel.
- Right-side tabs (splitter draggable): Overview / Chat / Files / Ledger / (+PRD view).

### 4.1 Chat / Session view (the heart of single-model interaction)

<!-- tikz: chat -->
```
┌ Session #12 — shop-redesign · task "api-layer" ────────────────────────────────┐
│ role: coder   provider: [kimi-k2.7 ▾]   ⏱ 04:12   tok in 8.2k / out 1.4k $0.03 │
├────────────────────────────────────────────────────────────────────────────────┤
│  🤖 I'll add the cart endpoints. Reading src/api/mod.rs…                        │
│  🔧 submit_write src/api/cart.rs  → ticket #1204 ✅ applied (hash ok)           │
│  🔧 run_shell `cargo check` → ok                                                │
│  🤖 Endpoints done. Summary: …                                                  │
│                                                                                 │
│ > steer: also handle out-of-stock errors_                        [Send] [■ Stop]│
├────────────────────────────────────────────────────────────────────────────────┤
│ queue: 1 ticket pending · context 41% · [Migrate provider] [Snapshot] [📜 full] │
└────────────────────────────────────────────────────────────────────────────────┘
```

- Transcript: RichTextField, streamed; tool calls rendered as collapsible lines with
  ticket status pulled live from the write queue.
- Provider combobox: switch model mid-session → triggers structured-snapshot migration
  (decision D5), with confirmation.
- Bottom bar: queue state for this session's tickets, context-window usage, actions
  (Migrate / Snapshot / open full transcript in markdown viewer).

---

## 5. Quick Chat (single LLM, no project)

`Ctrl+Q` or Sessions → Quick Chat. A lightweight window for ad-hoc conversation:

<!-- tikz: quickchat -->
```
┌ Quick Chat ────────────────────────────────────────────┐
│ provider: [Claude ▾]  model: [claude-opus-5 ▾]  78% ⏳ │
├────────────────────────────────────────────────────────┤
│  (transcript)                                          │
│ > _                                      [Send] [📎]   │
│ tok 1.2k · $0.01 · [Promote to task] [Save transcript] │
└────────────────────────────────────────────────────────┘
```

- Token usage is still ledgered against the provider (quotas stay honest).
- **Promote to task**: converts the chat into a project task with the transcript as
  context seed. Attach (📎) adds files/images for vision-capable models.

---

## 6. Settings

Menu Tools → Settings (`F10`). Modal window: left Accordion/ListBox of sections,
right side shows the section's form. **[Save] applies to TOML live; [Defaults] resets.**

### Settings inventory

| Section | Keys |
|---|---|
| **General** | workspace root, default project template, language/locale, autostart behavior, check for updates |
| **Appearance** | theme (dark/light/custom), true-color toggle, font-size hint, dashboard density, confirm-on-exit |
| **Parallelism** | global session cap (default: CPU heuristic, D6), per-provider concurrency limits, write-queue batch size, per-project fairness quota |
| **Quotas & alerts** | warn thresholds (default 80/95%), alert action (notify / auto-switch rule / pause role), burn-rate window, renewal reminders |
| **Rules** | default question mode (thorough/balanced/minimal — FR-6.4), unmatched-role behavior (ask / cheapest / fail), rule conflict precedence |
| **Secrets** | backend: keychain ↔ encrypted vault, vault path, master-password timeout, env-var passthrough allow-list |
| **Sandbox & security** | shell allow-list per project, network access for tool-runner, overlay mode for CLI adapters (D3), redaction level |
| **Projects** | default DAG parallelism, auto git checkpoints on/off, AGENTS.md regeneration policy (ask/auto), skill registry paths |
| **Storage** | config dir, SQLite path, transcript retention days, ledger export format, backup on exit |
| **Notifications** | TUI popups on/off, OS toast on/off, sound, quiet hours |
| **Keybindings** | full remapping table (searchable) |
| **Headless/API** | default `--json`, NDJSON watch toggle, exit-code verbosity |
| **Advanced** | log level, event-bus buffer size, experimental flags, reset all state |

---

## 7. Interaction Flows

### 7.1 First run (onboarding wizard, runs once)
1. Welcome → pick workspace root & theme.
2. **Detect providers**: scans PATH for `claude`/`codex`/`gemini`/`ollama`, probes
   `localhost:11434`; shows found list with checkboxes.
3. **Add API providers**: endpoint + key (stored to keychain, FR-2) → auth test →
   model discovery.
4. **Quota setup**: for each provider pick source (auto / manual budget) + plan limits.
5. **Default rules**: proposed from capabilities (e.g. tool-runner → ollama local);
   user confirms in the rule editor.
6. Done → Mission Control. `rolen config doctor` summary shown.

### 7.2 New project
1. `Ctrl+N` → wizard: name, dir, stack, git init.
2. **Interview** (interrogator role, FR-6): batched forms; answers stream into PRD.json.
3. RoleN drafts **PRD.md** → user reviews in markdown tab → approve → PRD.json compiled.
4. **AGENTS.md + skill suggestions** generated; user checks which skills to install.
5. **Task DAG proposal** shown in the project's tree; user edits/reorders → **Run**.
6. Sessions spawn per rule routing; dashboard shows them; questions may reappear
   mid-flight in the app bar (❓ badge).

### 7.3 Quota-exceeded event (no user action needed)
1. Provider returns 429 / parsed quota hits threshold → rule re-evaluates (FR-3.3).
2. Session migrates via structured snapshot (D5); transcript gains a
   `── migrated kimi-k2.7 → glm-5.2 ──` marker.
3. App bar ⚠ badge + notification; Activity tab records the decision and its reason.

### 7.4 Steering a running agent
Open session window → type in steer box → message injected into agent loop.
`■ Stop` pauses with snapshot; resume continues with context intact.

---

## 8. Keybindings (defaults, remappable)

| Key | Action |
|---|---|
| `Ctrl+Q` | Quick Chat |
| `Ctrl+N` | New Project |
| `F10` | Settings |
| `F1..F6` | Mission Control tabs (Dashboard…Activity) |
| `Ctrl+Enter` | Send (chat/steer) |
| `Ctrl+P` | Command palette (jump to project/provider/rule/session) |
| `Ctrl+D` | Dry-run rule under cursor |
| `Ctrl+T` | Tile all satellite windows |
| `Esc` | Close dialog / blur input |
| `?` | Shortcuts overlay |

---

## 9. AppCUI Control Mapping

| Need | AppCUI control |
|---|---|
| Tabs everywhere | `Tab` |
| Sessions/providers/projects tables | `ListView` (columns, sort) |
| Task DAG | `TreeView` + custom glyphs |
| Quota gauges | `ProgressBar` + `GraphView` (burn sparklines) |
| PRD view | `Markdown` viewer |
| Chat transcript | `RichTextField` (read-only, appended) |
| Steer/chat input | `TextField` (multiline) + `Button` |
| Rule builder | `ComboBox`, `NumericField`, `ListBox` + up/down |
| Settings sections | `Accordion` / `ListBox` + panel swap |
| Project window panes | `Splitter` |
| Global status | `AppBar` segments |
| Wizards/onboarding | modal `Window`s + `Button`s, `CheckBox`, `RadioButton` |
| Questions forms | generated `RadioButton`/`CheckBox`/`TextField`/`DatePicker` |
| Toasts/alerts | notification dialogs + app bar badges |
| Theming | built-in themes, true-color |

---

## 10. Window Inventory

| Window | Kind | Opened from |
|---|---|---|
| Mission Control | main (maximized) | startup |
| Onboarding wizard | modal | first run / Tools |
| Project window | satellite | Projects tab, `rolen project open` |
| Session/Chat window | satellite | dashboard/chat tab, Quick Chat promote |
| Quick Chat | satellite | `Ctrl+Q` |
| Provider window | modal-ish | Providers tab |
| Provider wizard | modal | Providers → Add |
| Rule editor | tab panel | Rules tab |
| Interrogation form | panel/modal | Questions tab, app bar ❓ |
| Settings | modal | `F10` |
| Transcript viewer | satellite | session → 📜 full |
| File/picker dialogs | stock AppCUI | wizards |
