# Plan: L5 Digital Twin — Use the Signal

> **REVISION 2026-05-06 — /autoplan Phase 4 synthesis (CEO + Eng + DX + Codex).** Read **PLAN REVISION NOTES** below before dispatching any subagent. Several tasks reference fields/code that don't exist in the current codebase; subagent hand-off without these patches will produce non-compiling code.

---

## PLAN REVISION NOTES (Phase 4 synthesis)

Four reviewer voices (CEO, Eng, DX, Codex) converged on the following blockers. Auto-decided edits applied inline; taste decisions deferred to user.

### CRITICAL — applied inline

1. **Field-name bugs.** `FeedbackRecord` field is `verdict: Verdict { Accepted, Rejected, Ignored }`, NOT `action: String`. Every `f.action == "accept"` in this plan is a spec lie. Patched in M10-02 + M11-02.
2. **`ErrorRecord.hash` vs `FeedbackRecord.error_hash`.** `ErrorRecord` has field `hash` (not `error_hash`). `FeedbackRecord.error_hash` joins to `ErrorRecord.hash`. Plan callers must reflect this.
3. **Lossy data model (Codex CRIT).** `FeedbackRecord` does not snapshot accepted suggestion *text*; suggestions are stored by `error_hash` keyed file (`suggestion_<error_hash>.json`) and can be **overwritten** when daemon regenerates. Reusing them as "accepted examples" silently mutates training data. **NEW prerequisite: M9.5 — immutable accepted-suggestion snapshots.** Persist `AcceptedSuggestion { suggestion_hash, error_hash, text, ts }` keyed by `suggestion_hash` at feedback-accept time in `ipc.rs`. M10/M11/M12 all read from this immutable table, not the regenerable `suggestion_*.json`.
4. **M11-03 wired to wrong subsystem (Codex CRIT).** Plan says modify `ollama_subscriber.rs`, but feedback events flow through `ipc.rs::handle_feedback`. **Move M11-03 → `crates/daemon/src/ipc.rs`** post-`put_feedback` hook.
5. **Protocol mismatch (Codex HIGH).** Current IPC is `Envelope { method: String, payload: Value }`, NOT enum-typed `IpcRequest::Foo`. Every `IpcRequest::Profile` / `IpcResponse::Profile` reference in plan is fantasy. M10-03 + M14 must add new `method` strings + JSON payload structs, with parser branches in `ipc.rs::dispatch`. Subagent must follow the existing `feedback` / `suggest` method convention, not the planned enum variants.
6. **Falsifiability of "+20% acceptance lift" (CEO + Codex CRIT).** Aggregate acceptance rate is corruptible by error-mix shift, fatigue, cached suggestion replay, and prompt-version drift. **Replaced** by:
   - Primary metric: **per-error-class acceptance rate** (keyed by `ErrorRecord.kind`) — at least 3 classes with ≥10 exposures pre/post.
   - Secondary: **% accepted suggestions that needed no follow-up edit within 5 minutes** (proxy for true usefulness).
   - Cohort: `prompt_version` field added to `Metrics` so M11-on vs M11-off can be split.
7. **M16 = exfiltration vector (Codex CRIT).** Sync ships `last_command`, `raw_excerpt`, `note`, accepted suggestion text — fields known to leak proprietary code. **M16 deferred out of L5.** Re-plan as L7 with explicit allowlist + redactor-at-export, separate doc.

### HIGH — applied inline

8. **M12 mislabeled.** Renamed "Local LoRA fine-tune" → "**M12 — Modelfile prompt-bake**" since v1 path is Ollama Modelfile MESSAGE blocks (no actual gradient updates). True LoRA deferred.
9. **kNN-over-accepts as M11 alternative (CEO HIGH).** Retrieve top-3 nearest accepted (error,suggestion) pairs by `ErrorRecord.kind` + Levenshtein on `raw_excerpt` — no StyleProfile needed for the example-injection path. M10 StyleProfile reduced to a *secondary* signal (terseness + tool prior); examples come from M11 kNN. Cuts M10 scope by ~40%.
10. **TTHW gap (DX CRIT).** Time-to-Hello-World for a new user is 3-7 days (Ollama install + 4.7GB pull + 50 events). **Added** `organism-cli profile --seed-from-history` (mine `~/.zsh_history` for past failures) + `organism-cli doctor` (preflight checks).
11. **Pre-flight threshold raised.** 50 events insufficient for ngram stability. Bumped to **≥300 accepts** with split-half stability check before M11 ship gates open.
12. **PII redactor at trust boundary (Eng + Codex HIGH).** `pub trait Redactor` defined in `cortex::redact`; `build_few_shot_context` takes `&dyn Redactor` and applies before string assembly, not after. Closes "redacted prompt vs unredacted profile" gap.
13. **M14 chat REPL deferred.** CEO + Codex + DX agree: scope creep, Aider/Cursor own this surface. Removed from L5; tracked in BACKLOG.md as candidate L7.
14. **M13 trigger gate too coarse.** `tool_rate >= 0.7` collapses unrelated rust errors. Replaced with **per-`ErrorRecord.kind` accept rate** (same key as new primary metric).
15. **DAG sequencing.** `M15-03 (baseline snapshot) → M11 ship` is a hard edge; added explicitly to DAG diagram below.

### MED — flagged, fix during implementation

- M14 char-count token budget → use `tokenizers` crate (deferred since M14 deferred).
- M15 atomic snapshot writes (tempfile + rename) — required by Codex.
- M15 hourly snapshot too coarse → write on every counter delta, batched to disk every 60s with WAL-style append.
- M11 stopword list must include rust/shell common words (`error`, `cargo`, `the`, `expected`, `found`).

### Decision Audit Trail

| ID | Decision | Voices | Auto/Taste | Status |
|----|----------|--------|------------|--------|
| D1 | Add M9.5 immutable accepted-snapshot table | Codex/Eng | Auto | Applied |
| D2 | Replace aggregate +20% with per-error-class lift | CEO/Codex | Auto | Applied |
| D3 | Defer M16 sync out of L5 | Codex | Auto | Applied |
| D4 | Defer M14 chat REPL out of L5 | CEO/Codex/DX | Auto | Applied |
| D5 | Rename M12 LoRA → Modelfile prompt-bake | CEO/Codex/DX | Auto | Applied |
| D6 | Fix `f.action`→`f.verdict`, `error_hash`→`hash` on ErrorRecord | Eng/Codex | Auto | Applied (inline) |
| D7 | Move M11-03 to ipc.rs feedback handler | Codex | Auto | Applied |
| D8 | Replace `IpcRequest::Foo` with method-string envelopes | Codex | Auto | Applied |
| D9 | kNN-over-accepts replaces StyleProfile examples | CEO | **Taste** | Pending user |
| D10 | Bump pre-flight from 50 → 300 events | CEO/Codex | Auto | Applied |
| D11 | Add `--seed-from-history` + `doctor` for TTHW | DX | Auto | Applied |
| D12 | Drop M12 entirely (qwen2.5 obsolete by ship time) | CEO | **Taste** | Pending user |

### Pending taste decisions for user

- **D9 — kNN vs StyleProfile examples.** Current plan: build StyleProfile (M10) AND inject 3 accepted pairs (M11). Codex+CEO recommend: keep M10 *only* for terseness/tool-prior summary; do example retrieval via simple kNN-by-kind lookup. Saves ~40% of M10 LoC and removes ngram-stability risk. Default if no decision: **adopt kNN, downsize M10**.
- **D12 — Drop M12 entirely.** By the time M11 ships and 14-day dogfood completes, qwen2.5-coder:7b will likely be obsolete (Llama 4 / Qwen 3 etc). Modelfile MESSAGE prompt-bake adds little vs M11 in-context injection. Default if no decision: **keep M12 as gated optional**.

### Updated Goal (replaces top-of-doc)

**Goal.** v0.5.0 captures accept/reject feedback but does nothing with it. L5 closes the learning loop: per-error-class acceptance rate climbs measurably because suggestions reuse what you accepted before. Twin reuses your prior accepts via kNN; StyleProfile shapes terseness/tool prior.

**Definition of "better".** On at least 3 distinct `ErrorRecord.kind` classes with ≥10 exposures each, post-M11 acceptance rate exceeds pre-M11 baseline by ≥15 absolute percentage points (or ≥30% relative, whichever is greater), AND ≥60% of accepted suggestions need no follow-up edit within 5 minutes. Cohort split via `prompt_version` field. Measured by M15. 14-day window.

---

## ORIGINAL PLAN (below — read in conjunction with REVISION NOTES above)

**Goal.** v0.5.0 captures accept/reject feedback on suggestions but does nothing with it. L5 closes the learning loop: the daemon's next suggestion is *measurably better* because it has seen what you accepted/rejected before. After this plan, the same error in week 4 yields a suggestion that matches your code style without you re-prompting.

**Definition of "better".** ~~Acceptance rate on repeated error classes climbs from baseline (M10 measurement) to +20% relative within 14 days of dogfood usage.~~ **SUPERSEDED — see Updated Goal above.** Per-error-class acceptance lift, falsifiable, measured by M15.

**Non-goals.**
- Cloud fine-tune. Stays local (Ollama / llama.cpp).
- Cross-user federated learning.
- Replacing the user. Twin *suggests*, user still applies.
- Windows.

**Style.** Same as L4 plan. Each task ≤300 LoC, ≤6 files, failing-test-first acceptance. Local model (qwen2.5-coder:7b via `mcp__ollama-dev__implement_task`) implements; main agent reviews against gate.

**Branch strategy.** One branch per milestone (M10-M16). PR per milestone. CI green before merge. Tag `v0.6.0` after M10-M12 (style-aware suggestions). Tag `v0.7.0` after M13-M16 (proactive + observable).

---

## Dependency DAG (revised Phase 4)

```
M9.5 (immutable accepts) ──> M10 (style profile) ──┐
                                                    ├──> M11 (few-shot inject) ──> v0.6.0 dogfood gate ──> M12 (Modelfile bake, optional)
M15 (metrics + baseline) ─────────────────────────┘
                          │
                          └──> M15-03 baseline snapshot MUST land before M11 ship
                                                       │
                                                       ▼
                                         M13 (proactive, gated by per-kind rate) ──> v0.7.0
~~M14 chat REPL~~ DEFERRED → BACKLOG.md
~~M16 sync~~ DEFERRED → L7 plan
```

M9.5 → M10 + M15 in parallel. M15-03 is a hard prerequisite for M11 dogfood gate (need pre/post baseline). M14 + M16 deferred per Phase 4 review.

---

## Pre-flight Assumptions (verify at M10 start)

1. `feedback_*.json` files exist and parse via M9 schema_v migration. If not, fix M6 first.
2. `ErrorRecord.last_command` populated for >80% of records. If not, M10 has noisy input.
3. Ollama `qwen2.5-coder:7b` runs locally on developer machine. If not, L5 cannot self-host inference.
4. ~~At least 50 feedback events~~ **REVISED: ≥300 accepted feedback events** with split-half ngram-stability check (top-10 phrases overlap ≥7/10 across random halves). 50 was statistically degenerate. Use `organism-cli profile --seed-from-history` to bootstrap from `~/.zsh_history` if cold.

If any assumption fails: STOP, file as blocker, do not start M10.

---

# M9.5 — Immutable accepted-suggestion snapshots (NEW PREREQUISITE — Phase 4 revision)

**Why.** Today `suggestion_<error_hash>.json` is overwritten on every regeneration; if a user accepts then daemon regenerates, the "accepted text" we'd reuse silently mutates. M10/M11/M12 all depend on stable training pairs. Without M9.5, every milestone downstream is built on lossy joins.

## TASK-M9.5-01 — `AcceptedSuggestion` type + store

- **NEW** `pub struct AcceptedSuggestion { schema_v: u32, suggestion_hash: String, error_hash: String, text: String, ts: DateTime<Utc> }` in `crates/knowledge/src/types.rs`.
- **MODIFY** `crates/knowledge/src/store.rs`. Add `put_accepted(&self, a: &AcceptedSuggestion) -> Result<()>` writing to `accepted_<suggestion_hash>.json` with atomic tempfile+rename. `get_accepted(&self, suggestion_hash: &str)`. `list_accepted_by_kind(&self, kind: &str)`.
- **MODIFY** `crates/daemon/src/ipc.rs` `handle_feedback`: when `verdict == Accepted`, snapshot the current `SuggestionRecord` text into the immutable table BEFORE returning success. Idempotent on suggestion_hash.
- **Tests**: accept feedback → `accepted_*.json` exists; subsequent suggestion regeneration does NOT alter it; missing suggestion → graceful error.
- **Acceptance**: `cargo test --workspace accepted` green; clippy clean.

## TASK-M9.5-02 — Backfill from existing data

- **NEW** `crates/client/src/main.rs::cmd_backfill_accepts`. One-shot: scans existing `feedback_*.json` for `Accepted`, joins `suggestion_<error_hash>.json` text at current state, writes immutable rows. Best-effort (skip if join target missing). Print count.
- **Acceptance**: manual on dev's own knowledge dir → row count printed; idempotent on re-run.

---

# M10 — Style profile extraction

**Why.** Without a structured representation of "what the user accepts vs rejects", the LLM has no basis for personalization. M10 turns raw `feedback_*.json` files into a queryable profile: preferred error-handling style, naming conventions, terseness, language preferences (rust vs shell), block-type bias (patch vs note).

## TASK-M10-01 — `StyleProfile` type + serde

- **NEW** `crates/cortex/src/style.rs`.
- Fields:
  ```rust
  pub struct StyleProfile {
      pub schema_v: u32,              // 1
      pub generated_at: DateTime<Utc>,
      pub feedback_count: u32,
      pub accept_rate_overall: f32,   // 0.0..=1.0
      pub by_tool: HashMap<String, ToolStats>,           // "rustc" -> stats
      pub by_block_kind: HashMap<String, BlockStats>,    // "patch"|"shell"|"note"
      pub preferred_terseness: Terseness,                // Concise|Standard|Verbose
      pub top_accepted_phrases: Vec<String>,             // top 10 ngrams in accepted suggestions
      pub top_rejected_phrases: Vec<String>,             // top 10 ngrams in rejected
  }
  pub struct ToolStats { pub accepts: u32, pub rejects: u32 }
  pub struct BlockStats { pub accepts: u32, pub rejects: u32 }
  pub enum Terseness { Concise, Standard, Verbose }  // by avg accepted suggestion line count
  ```
- All `#[derive(Debug, Clone, Serialize, Deserialize)]`. `schema_v` defaulted via M9 pattern.
- **Tests** (`#[cfg(test)] mod tests`):
  1. Default `StyleProfile::empty()` serializes + roundtrips.
  2. Manually-constructed profile with mixed tool stats roundtrips.
  3. `Terseness` serde tag uses `snake_case`.
- **Acceptance**: `cargo test -p organism-cortex style::` → 3 passed.

## TASK-M10-02 — Profile builder

- **MODIFY** `crates/cortex/src/style.rs`. Add:
  ```rust
  pub fn build_profile(feedback: &[FeedbackRecord], suggestions: &HashMap<String, String>) -> StyleProfile;
  ```
  Where `suggestions` maps `error_hash -> suggestion text` (loaded by caller from `suggestion_*.json`).
- Algorithm:
  1. accepts = `feedback.iter().filter(|f| matches!(f.verdict, Verdict::Accepted))`  <!-- REVISION: was f.action == "accept" — that field does not exist -->
  1b. **NEW (per M9.5 prerequisite):** load accepted texts from immutable `accepted_<suggestion_hash>.json` table, NOT from regenerable `suggestion_<error_hash>.json`. Join via `FeedbackRecord.suggestion_hash`.
  2. `accept_rate_overall = accepts / total`
  3. For each feedback, look up `suggestions[&f.error_hash]`. Skip if missing.
  4. Tokenize accepted suggestion text into 2-grams + 3-grams via simple whitespace split. Drop stopwords (`the`, `a`, `is`, `to`, etc. — fixed 50-word list).
  5. Top-10 by frequency → `top_accepted_phrases`. Same for rejected.
  6. `by_tool[tool] += accept|reject` looked up from the linked `ErrorRecord`. (Caller passes `tool_for_hash: HashMap<String, String>`.)
  7. `by_block_kind` same idea (caller pre-classifies suggestion text).
  8. `preferred_terseness`: avg line count of accepted suggestions. <8 = Concise, 8-20 = Standard, >20 = Verbose.
- **Tests**:
  1. Empty input → `StyleProfile::empty()` (accept_rate=0, no maps).
  2. 10 accepts of patch blocks, 0 rejects → `by_block_kind["patch"].accepts == 10`.
  3. 5 accepts on rustc, 5 rejects on npm → `by_tool` has both, accept rates correct.
  4. Single 50-line accepted suggestion → `Terseness::Verbose`.
  5. Stopword filtering: "the the the cargo build" → top phrase is "cargo build" not "the the".
- **Acceptance**: `cargo test -p organism-cortex style::build_profile` → 5 passed.

## TASK-M10-03 — Profile persistence + `organism-cli profile` command

- **MODIFY** `crates/knowledge/src/store.rs`. Add `put_style_profile(&self, p: &StyleProfile) -> Result<()>` writing to `style_profile.json` (single file, overwritten). `get_style_profile(&self) -> Result<Option<StyleProfile>>` with M9 migration shim.
- **MODIFY** `crates/protocol/src/messages.rs`. Add `IpcRequest::Profile { rebuild: bool }` and `IpcResponse::Profile { profile: StyleProfile, freshly_built: bool }`.
- **MODIFY** `crates/daemon/src/ipc.rs`. Handle `Profile`: if `rebuild`, call `build_profile(...)` over all feedback + suggestion files, persist, return `freshly_built: true`. Else load cached, return `false` flag (or build if absent).
- **MODIFY** `crates/client/src/main.rs`. Add `cmd_profile`:
  - `organism-cli profile` → human summary (accept rate, top tools, terseness, top 5 accepted phrases).
  - `organism-cli profile --rebuild` → forces refresh first.
  - `organism-cli profile --json` → raw `StyleProfile` JSON.
- **Tests**: integration test `crates/daemon/tests/profile_test.rs` seeds 5 feedback + 5 suggestion files, calls IPC, asserts profile fields populated; CLI unit test on output formatting.
- **Acceptance**: `cargo test --workspace profile` green; manual `organism-cli profile` after dogfooding shows non-zero accept rate.

## M10 Verification

```bash
# After ≥50 accumulated feedback events:
organism-cli profile --rebuild
organism-cli profile --json | jq '.accept_rate_overall, .by_tool, .preferred_terseness'
# Expect: float in [0,1], non-empty by_tool, terseness enum
```

---

# M11 — Few-shot context injection

**Why.** Profile alone changes nothing. M11 *uses* it: each `suggest` call now includes the user's preferred style as context, plus 1-3 examples of accepted suggestions for the same tool. This is in-context learning — no model retraining.

## TASK-M11-01 — Context builder

- **NEW** `crates/cortex/src/context.rs`.
- ```rust
  pub fn build_few_shot_context(
      profile: &StyleProfile,
      tool: &str,
      recent_accepts: &[(ErrorRecord, String)],  // (error, accepted_suggestion) pairs, max 3
  ) -> String;
  ```
- Output template:
  ```
  ## User style profile
  Terseness: {Concise|Standard|Verbose}
  Tool acceptance: {tool} → {accept_rate}% accepted
  Preferred phrases: {top 3 from top_accepted_phrases}

  ## Examples of suggestions this user accepted for {tool}
  ### Example 1
  Error: {error.raw_excerpt truncated to 200 chars}
  Suggestion (accepted):
  {suggestion truncated to 500 chars}

  ### Example 2
  ...
  ```
- If `recent_accepts` empty → omit Examples section entirely (don't fabricate).
- **Tests**:
  1. Empty profile + empty examples → only profile header, no Examples section.
  2. 3 examples → 3 numbered blocks present.
  3. Truncation: 5KB suggestion → output truncated to 500 chars + ellipsis.
  4. Sensitive content: profile or example containing email → assert PII redactor (M8) was invoked. (Cross-crate test; can stub redactor and assert call.)
- **Acceptance**: `cargo test -p organism-cortex context::` → 4 passed.

## TASK-M11-02 — Wire into `suggest_for_error`

- **MODIFY** `crates/cortex/src/suggest.rs`.
- Change signature:
  ```rust
  pub async fn suggest_for_error<C: LlmClient>(
      client: &C,
      store: &KnowledgeStore,
      error_key: &str,
      use_profile: bool,   // NEW, default true via wrapper
  ) -> Result<String>;
  ```
- When `use_profile`:
  1. `let profile = store.get_style_profile()?.unwrap_or_else(StyleProfile::empty);`
  2. Find recent_accepts: query feedback for `matches!(verdict, Verdict::Accepted)` joined to `ErrorRecord.kind` (NOT tool — kind is finer-grained), take 3 most recent, pair with text from immutable `accepted_<suggestion_hash>.json` table (per M9.5).  <!-- REVISION: was action="accept" matching tool -->
  2b. **kNN variant (D9, recommended):** rank candidates by Levenshtein distance on `raw_excerpt` within same `kind`, top-3.
  3. `let context = build_few_shot_context(&profile, &record.tool, &recent_accepts);`
  4. Final prompt = `{context}\n\n## Current failure\n{existing_template}`.
- Backward compat: `use_profile = false` path is exactly the old behavior. Old call sites get a thin wrapper that defaults to `true`.
- **Tests**:
  1. Empty profile + no recent accepts → prompt unchanged from baseline (regression guard).
  2. Profile present → assert prompt sent to mock LLM contains "User style profile".
  3. 3 recent accepts → prompt contains 3 "### Example" headers.
  4. `use_profile=false` → prompt identical to baseline even with profile present.
- **Acceptance**: `cargo test -p organism-cortex suggest::` green; existing M3-M9 tests still pass.

## TASK-M11-03 — Daemon refresh policy

- **MODIFY** `crates/daemon/src/ipc.rs` post-`put_feedback` (NOT `ollama_subscriber.rs` — that handles `ErrorClassified` events, not feedback. See REVISION D7.). After every Nth feedback event (default N=10, env `ORGANISM_PROFILE_REFRESH_EVERY`), call `build_profile` + persist. Best-effort, log on failure.
- Avoid hot loop: rate-limit to once per 60s minimum even if N is hit faster.
- **Tests**: integration test seeds 30 feedback events at high rate, asserts profile rebuilt at most 1x per minute and not more than `ceil(30/N)` times total.
- **Acceptance**: `cargo test -p organism-daemon profile_refresh` green.

## M11 Verification

```bash
# 1. Build profile
organism-cli profile --rebuild

# 2. Trigger known error class
organism-cli emit-terminal "cargo build" --exit-code 1 --stderr "error[E0599]: no method named 'frob'"

# 3. Suggest — should now include style context
ORGANISM_DEBUG_PROMPT=1 organism-cli suggest
# Daemon logs (~/.organism/logs/daemon.log) show prompt with "User style profile" header
```

---

# v0.6.0 dogfood gate

**Stop. Tag v0.6.0. Dogfood for 14 days minimum.**

Track via M15 (must ship in parallel before this gate):
- `suggestion_acceptance_rate` baseline (pre-M11) vs current (post-M11).
- Target: +20% relative lift on repeated error classes.

If <10% lift: M11 prompt is wrong. Tune phrasing of style header + example formatting before M12. Do NOT skip ahead.
If 10-20% lift: marginal. Decide: tune more (cheap) or proceed to M12 (expensive). User call.
If >=20%: gate passed, M12 unlocked.

---

# M12 — Local LoRA fine-tune (OPTIONAL, gated)

**Why.** In-context learning has a token budget ceiling. Once profile + 3 examples + error context exceed ~8K tokens, smaller models degrade. LoRA fine-tune on accepted patches gives durable, cheap inference.

**Gate.** Only if M11 dogfood lift was >=20% AND user accumulated ≥500 accepted suggestions. Below that, training data is too sparse — fine-tune will overfit or regress.

## TASK-M12-01 — Training data exporter

- **NEW** `crates/cortex/src/export.rs`.
- `pub fn export_training_jsonl(store: &KnowledgeStore, out: &Path) -> Result<u32>`.
- For each accepted feedback:
  - Load `ErrorRecord` + suggestion text.
  - Apply M8 PII redactor.
  - Emit JSONL row: `{"prompt": "<error context>", "completion": "<accepted suggestion>"}`.
- Returns count of rows written. Skip records with missing fields.
- **Tests**: 5 mixed feedback → 5 rows in temp file; rejected events excluded; PII redacted in output (assert no `@` substring on emails).
- **Acceptance**: `cargo test -p organism-cortex export::` → 3 passed.

## TASK-M12-02 — `organism-cli train-export` command + Ollama Modelfile generator

- **MODIFY** `crates/client/src/main.rs`. Add:
  - `organism-cli train-export --out <path>` → writes JSONL.
  - `organism-cli train-modelfile --base qwen2.5-coder:7b --out <path>` → writes Ollama Modelfile that imports JSONL as `MESSAGE`/`SYSTEM` few-shot blocks (Ollama doesn't natively LoRA but supports prompt-baked Modelfiles, which is our v1 path).
- **Note**: Actual LoRA training (via `llama.cpp finetune` or `mlx_lm.lora` on Apple Silicon) is documented in README, not automated. v1 stops at "data exported, Modelfile generated, user runs `ollama create organism-twin -f Modelfile` themselves." Automation can come later if usage justifies.
- **Tests**: temp dir round-trip; Modelfile contains `FROM qwen2.5-coder:7b` and at least one `MESSAGE` line.
- **Acceptance**: `cargo test -p organism-client train` green; manual `ollama create organism-twin -f /tmp/Modelfile && OLLAMA_MODEL=organism-twin organism-cli suggest` returns non-empty.

## TASK-M12-03 — Switchable model

- Already supported: `OLLAMA_MODEL` env var. M12-03 is documentation-only: README section "Using your trained twin" with the 3 commands above.
- **Acceptance**: README updated, manual e2e walkthrough on user's Mac succeeds.

## M12 Verification

```bash
organism-cli train-export --out /tmp/twin.jsonl
wc -l /tmp/twin.jsonl                         # >= 500 expected
organism-cli train-modelfile --out /tmp/Modelfile
ollama create organism-twin -f /tmp/Modelfile
OLLAMA_MODEL=organism-twin organism-cli suggest
# A/B compare acceptance over next 7 days vs baseline qwen2.5-coder:7b.
```

---

# M13 — Proactive suggestion

**Why.** Today user must run `organism-cli suggest`. M13: when classifier sees a high-confidence familiar error (`occurrences >= 3` AND profile shows >=70% accept rate for that tool), daemon proactively writes a suggestion + emits a desktop notification (macOS `osascript`, Linux `notify-send`).

## TASK-M13-01 — Notification crate (best-effort)

- **NEW** `crates/daemon/src/notify.rs`. `pub fn notify(title: &str, body: &str) -> Result<()>`.
- macOS: `osascript -e 'display notification "..." with title "..."'`
- Linux: `notify-send "title" "body"`
- Missing binary → `Ok(())`, log warn. Never crash.
- **Tests**: stub the command runner via trait; assert correct args constructed for each platform.
- **Acceptance**: `cargo test -p organism-daemon notify` → 2 passed.

## TASK-M13-02 — Trigger logic in ollama_subscriber

- **MODIFY** `crates/daemon/src/ollama_subscriber.rs`.
- After persisting a fresh suggestion, check:
  ```rust
  let p = store.get_style_profile()?.unwrap_or_default();
  let tool_rate = p.by_tool.get(&record.tool).map(|s| s.accept_rate()).unwrap_or(0.0);
  if record.occurrences >= 3 && tool_rate >= 0.7 {
      notify::notify(
          "organism: suggestion ready",
          &format!("{} (occ {})", record.last_command, record.occurrences),
      )?;
  }
  ```
- Gated by env `ORGANISM_NOTIFY=1` (default off — opt-in to avoid surprise pop-ups).
- **Tests**: feed mock subscriber 3 occurrences with high-accept profile → assert notify called once.
- **Acceptance**: green; manual: trigger `cargo build` failure 3x → notification appears.

---

# M14 — `organism-cli chat`

**Why.** Single-shot `suggest` is brittle for multi-turn debugging. M14: `organism-cli chat` enters a REPL that maintains conversation state with the local model, seeded with the failing error + style context. Each turn appends to history. `:apply N` extracts plan from turn N. `:save` persists transcript.

## TASK-M14-01 — Chat session struct + history

- **NEW** `crates/cortex/src/chat.rs`.
- `ChatSession { history: Vec<Turn>, error_key: String, profile: StyleProfile }`.
- `pub async fn turn<C: LlmClient>(&mut self, client: &C, user_msg: &str) -> Result<&str>` — appends user turn, builds prompt from history, calls LLM, appends assistant turn, returns response.
- Token budget guard: if history > 6K tokens (rough char-count heuristic ~24K chars), drop oldest non-system turns.
- **Tests**: 5-turn conversation; history grows; assert oldest turn dropped after exceeding budget.
- **Acceptance**: `cargo test -p organism-cortex chat::` → 3 passed.

## TASK-M14-02 — REPL in client

- **MODIFY** `crates/client/src/main.rs`. `cmd_chat`:
  - Reads stdin line-by-line. Each non-`:` line → IPC `Chat { error_key, message, session_id }`.
  - `:apply N` → reads turn N, runs M7 multi-block extractor, stages all plans.
  - `:save <path>` → writes transcript JSON.
  - `:quit` → exits.
- **MODIFY** protocol + daemon to add `Chat` IPC message; daemon owns `HashMap<SessionId, ChatSession>` with 1h TTL.
- **Tests**: scripted stdin via `assert_cmd`; verify multi-turn echo works.
- **Acceptance**: `cargo test -p organism-client chat` green; manual REPL feels conversational.

---

# M15 — Local metrics (MUST land before v0.6.0 gate)

**Why.** L5's success criterion is measurable lift. Without metrics there's no proof.

## TASK-M15-01 — Metrics struct + sink

- **NEW** `crates/daemon/src/metrics.rs`.
- ```rust
  pub struct Metrics {
      pub suggestions_total: u64,
      pub suggestions_cached: u64,
      pub feedback_accept: u64,
      pub feedback_reject: u64,
      pub by_tool: HashMap<String, ToolMetrics>,
      pub since: DateTime<Utc>,
  }
  ```
- Behind `Arc<RwLock<Metrics>>` shared across subscribers.
- Persisted hourly to `metrics_snapshot.json`. Reloaded on daemon start.
- **Tests**: 5 increments → snapshot equals expected values; reload from disk preserves counts.
- **Acceptance**: `cargo test -p organism-daemon metrics::` → 3 passed.

## TASK-M15-02 — Wire counters

- **MODIFY** `ollama_subscriber.rs` (suggest events), `ipc.rs` (feedback events). Increment on every suggest (cache hit/miss) and feedback.
- **Tests**: integration: 3 suggests + 2 accepts + 1 reject → metrics struct matches.
- **Acceptance**: green.

## TASK-M15-03 — `organism-cli stats` command

- Daily/weekly summaries:
  ```
  organism-cli stats
  organism-cli stats --since 7d --json
  organism-cli stats --baseline 2026-05-01    # accept rate before/after a date
  ```
- Includes `acceptance_rate_by_tool` and overall.
- **Tests**: format unit tests + integration over seeded metrics file.
- **Acceptance**: green; manual after dogfooding shows real numbers.

---

# M16 — Cross-machine sync (OPTIONAL)

**Why.** User has laptop + desktop. Today, profile/feedback live on each machine separately. M16: opt-in encrypted sync via user-chosen git remote (no centralized server).

**Gate.** Only if user explicitly requests. Default off. v0.7.0 ships without this.

## TASK-M16-01 — Encrypted bundle export

- `organism-cli sync export --out <bundle.tar.gz.enc> --passphrase-env ORGANISM_SYNC_KEY`.
- Bundles: knowledge dir minus `*.tmp`. AES-256-GCM via `age` crate (rage). Passphrase-derived key.
- **Acceptance**: roundtrip test, bundle decrypts to identical tree.

## TASK-M16-02 — Import + merge

- `organism-cli sync import <bundle> --passphrase-env ...`.
- Merge strategy: last-writer-wins on `last_seen`. Feedback events append-only.
- **Acceptance**: two seeded knowledge dirs → import → assert union with correct merge.

## TASK-M16-03 — Git-remote helper script

- `scripts/sync-via-git.sh`: clone user's private repo, decrypt bundles, commit, push. Documentation only — not automated by daemon (avoid surprise network).
- **Acceptance**: manual walkthrough on two Macs.

---

# Critical Files Map

| Phase | File | Action |
|-------|------|--------|
| M10 | `crates/cortex/src/style.rs` | NEW |
| M10 | `crates/knowledge/src/store.rs` | MODIFY (`put/get_style_profile`) |
| M10 | `crates/protocol/src/messages.rs` | MODIFY (Profile request/response) |
| M10 | `crates/daemon/src/ipc.rs` | MODIFY (Profile handler) |
| M10 | `crates/client/src/main.rs` | MODIFY (cmd_profile) |
| M11 | `crates/cortex/src/context.rs` | NEW |
| M11 | `crates/cortex/src/suggest.rs` | MODIFY (use_profile path) |
| M11 | `crates/daemon/src/ollama_subscriber.rs` | MODIFY (refresh policy) |
| M12 | `crates/cortex/src/export.rs` | NEW |
| M12 | `crates/client/src/main.rs` | MODIFY (train-export, train-modelfile) |
| M12 | `README.md` | MODIFY (twin training section) |
| M13 | `crates/daemon/src/notify.rs` | NEW |
| M13 | `crates/daemon/src/ollama_subscriber.rs` | MODIFY (notify trigger) |
| M14 | `crates/cortex/src/chat.rs` | NEW |
| M14 | `crates/protocol/src/messages.rs` | MODIFY (Chat IPC) |
| M14 | `crates/client/src/main.rs` | MODIFY (cmd_chat REPL) |
| M15 | `crates/daemon/src/metrics.rs` | NEW |
| M15 | `crates/daemon/src/ollama_subscriber.rs` + `ipc.rs` | MODIFY (counter wiring) |
| M15 | `crates/client/src/main.rs` | MODIFY (cmd_stats) |
| M16 | `crates/client/src/main.rs` | MODIFY (cmd_sync export/import) |
| M16 | `Cargo.toml` workspace | MODIFY (add `age` dep) |
| M16 | `scripts/sync-via-git.sh` | NEW |

---

# Pitfalls Pre-Flagged

- **M10 stopword list** must include rust/shell common words (`error`, `cargo`, `the`) AND english fillers. Otherwise top phrases are noise.
- **M10 feedback → suggestion linking** depends on M6's `error_hash` field. Verify M6 actually populates it before computing top phrases. Audit one `feedback_*.json` by hand first.
- **M11 prompt token budget**: qwen2.5-coder:7b context is 32K but degrades past ~8K. Hard-cap context section at 4K chars with truncation, not soft-suggest.
- **M11 PII**: profile + examples include user's accepted suggestions which may contain leaked code. Run M8 redactor on EVERY field of `build_few_shot_context` output, not just at LLM boundary, since profile could also be exported via M16.
- **M12 LoRA scope**: do NOT automate `llama.cpp finetune` invocation in v1. It requires user's GPU/MPS setup, downloads gigabytes, takes hours. Document as user-run command. Daemon stays small.
- **M13 notifications**: macOS Sonoma+ requires Terminal.app (or osascript bundle) to have Notification permission. First trigger may silently fail. Document this.
- **M14 chat sessions**: 1h TTL is arbitrary. If user `:save`s after 1h gap, transcript should still load from disk. Persist eagerly, not just on `:save`.
- **M15 baseline measurement** requires recording `pre-M11` acceptance rate. Capture at M15-03 ship time and store as `metrics_baseline.json`. Without it, M11 dogfood gate is unfalsifiable.
- **M16 encryption**: `age` crate is fine; do NOT roll custom crypto. Reject any local model suggestion that does so.
- **M16 merge**: last-writer-wins on `last_seen` will lose data if clocks drift between machines >1min. Document. NTP requirement.
- **General**: if M10 acceptance data is <50 events, profile is degenerate (single-tool noise). Hard-block M11 ship until threshold met.

---

# Verification Matrix (end-to-end)

| Capability | Closed by | Proof command |
|------------|-----------|---------------|
| User has queryable style profile | M10 | `organism-cli profile --json` non-empty |
| Suggestions adapt to user style | M11 | `ORGANISM_DEBUG_PROMPT=1 organism-cli suggest` shows profile in prompt |
| Acceptance rate measurable | M15 | `organism-cli stats --baseline <date>` shows lift % |
| Local twin model usable | M12 | `OLLAMA_MODEL=organism-twin organism-cli suggest` works |
| Familiar errors trigger proactively | M13 | 3rd repeat of same error → desktop notification |
| Multi-turn debugging | M14 | `organism-cli chat` REPL maintains context across turns |
| Cross-machine portability | M16 | Bundle exported on Mac A imports cleanly on Mac B |

---

# Subagent Dispatch Template (per task)

```
SYSTEM: You are implementing TASK-MNN-NN per PLAN-L5-DIGITAL-TWIN.md.

GOAL: <copy "Behavior" verbatim>

TESTS (write FIRST, must compile and FAIL before code added):
<copy test list verbatim>

CONSTRAINTS (per CLAUDE.md):
- No `unwrap()` outside `#[cfg(test)]`. Use `OnceLock` + `.expect("literal")`.
- All public types: `#[derive(Debug, Clone, Serialize, Deserialize)]`.
- `anyhow::Result` in binaries, `thiserror` in libs.
- `#[tokio::test]` async, `#[test]` sync.
- Knowledge tests use `tempfile::TempDir` not `~/.organism/`.
- All new persisted types include `schema_v: u32` with `#[serde(default = "default_schema_v")]`.
- All cross-trust-boundary text passes through M8 PII redactor.

OUTPUT: full file contents per modified/new file. No prose.
```

---

# Next Action

1. Verify pre-flight assumptions (1 hour, manual).
2. Spawn 2 parallel branches: `feat/m10-style-profile` and `feat/m15-metrics`. Each is independent.
3. Use Haiku/local-LLM subagents per task with template above. Main agent reviews diffs vs acceptance gate.
4. Land M15 first (it must precede M11 dogfood gate).
5. Land M10. Then M11 sequentially (depends on M10 + M15).
6. Tag `v0.6.0`. Dogfood 14 days. Capture acceptance lift.
7. Decide M12 (only if gate cleared).
8. M13 + M14 in parallel after v0.6.0. Tag `v0.7.0`.
9. M16 only on explicit user request.
