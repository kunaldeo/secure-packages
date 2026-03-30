# secure-packages

Supply chain security gate for third-party packages. The first release supports Python packages from PyPI, with automated security analysis via Gemini CLI, per-version caching in PostgreSQL, and both an interactive TUI and a CI-friendly CLI.

This project was directly inspired by Andrej Karpathy's March 24 tweet about the LiteLLM PyPI supply chain attack. Seeing how much damage a single `pip install` could do made me worry about my own projects and pushed me to build a practical approval gate instead of relying on blind trust in package updates.

<img width="440" height="129" alt="and-tweet" src="https://github.com/user-attachments/assets/5b0daf69-9a12-442d-a94e-c5bfc77e67d5" />

## How it works

![secure-packages](https://github.com/user-attachments/assets/7125d4ac-a0b4-4f04-9f34-9a6b0ed05365)

```
requirements.txt ──> sp-client ──> sp-server ──> Gemini CLI ──> verdict
                     (resolve)     (fetch)    (audit)        (approve/reject)
```

1. `sp-client` resolves your `requirements.txt` to exact versions (via `uv` or `pip`)
2. Server fetches package source from the supported registry, runs Gemini CLI with purpose-built security audit skills
3. Results are stored per-version — re-checking the same version returns cached results instantly
4. For version updates, diffs against the last approved version instead of re-scanning everything

The first release starts with Python packages from PyPI. More package ecosystems will be added soon.

## Technical architecture

Two technology choices shaped the system:

1. `gemini-3-flash` gives this project a large context window and fast analysis, which makes whole-package review and detailed version-diff review practical even as package size grows.
2. Gemini CLI in non-interactive `-p` mode, combined with reusable skills, automates a large part of the audit flow without requiring a custom harness for prompt orchestration, structured output enforcement, and review workflows.

That combination keeps the implementation simple while still scaling to packages of very different sizes. Instead of hand-building a complex analysis runner, secure-packages leans on Gemini CLI skills to drive repeatable audits and diff reviews with structured outputs.

Efficiency is a core part of the design. For PyPI packages, secure-packages keeps a persistent record of each project version it has reviewed, tied to the fetched source artifact and its SHA256 hash. If the same package version appears again, the system can return the cached verdict immediately instead of re-running analysis. When a new version of an already reviewed package arrives, it compares against the last approved version and focuses on the diff, which keeps repeat reviews fast and cost-efficient without sacrificing coverage where it matters most.

## Why this is different

Traditional package security tools are often strongest at known-vulnerability matching, license checks, dependency graph policies, or simple rule-based scanning. Those are useful, but they can miss supply chain attacks that hide in install hooks, build scripts, obfuscated setup logic, malicious version bumps, or subtle changes introduced between two otherwise legitimate releases.

secure-packages is built to close that gap:

- It reviews the actual package source, not just the package name and version against a database.
- It uses full-package audits for first-time reviews and diff-focused audits for later versions, which is faster and more targeted than rescanning everything every time.
- It produces a concrete approval decision for a package version: `approved`, `rejected`, or `needs_review`.
- It keeps a persistent audit record per reviewed version, so teams do not keep paying the same analysis cost for the same artifact.
- It is designed to sit directly in development and release workflows, not just generate an offline report someone may never read.

## Quick start

### Docker (recommended)

```bash
export GEMINI_API_KEY=your-key
docker compose up -d
```

Migrations run automatically. Server listens on `http://localhost:8080`.

### From source

```bash
docker compose up -d postgres
cargo build --release

./target/release/secure-packages migrate
SP_SERVER__ADMIN_TOKEN=yourtoken RUST_LOG=info ./target/release/secure-packages serve
```

## CLI

### Interactive TUI

```bash
sp-client check -r requirements.txt
```

Launches a live-updating terminal UI with real-time analysis progress:

```
 sp-client ── requirements.txt ── 5 packages ── 12.4s ⠹
  Package                  Version      Status          Risk  Summary
  certifi                  2026.2.25    ✓ approved      0.00  CA bundle, no code execution
  charset-normalizer       3.4.6        ✗ rejected      1.00  Supply chain attack in setup.py
  idna                     3.11         ✓ approved      0.00  No issues found
▸ requests                 2.33.0       ✓ approved      0.05  Standard HTTP library
  urllib3                  2.6.3        ⠹ analyzing     -
 3 approved  1 rejected  1 pending
  j/k  navigate   Enter  details   /  filter   g/G  top/bottom   q  quit
```

Press `Enter` on any package to drill into full analysis details — risk score with visual bar, LLM reasoning, individual findings with severity and file locations, diff summaries for version updates.

Key bindings follow vim conventions: `j/k` to navigate, `g/G` for top/bottom, `/` to filter, `Esc` to go back, `q` to quit.

### CI / non-interactive mode

Non-interactive output activates automatically when stdout is piped, or with `--json`, `--no-wait`, or `--no-tui`:

```bash
# JSON output for scripting
sp-client check -r requirements.txt --json

# Exit codes
# 0 = all approved
# 1 = any rejected or failed
# 2 = still pending

# Poll until all resolved
sp-client check -r requirements.txt --no-tui --interval 15

# Strict mode: also fail on packages flagged for human review
sp-client check -r requirements.txt --fail-on-review
```

This mode is intended for automation. `sp-client` can run inside CI/CD pipelines, container builds, or release jobs and return a machine-friendly result without opening the interactive TUI. That makes it straightforward to gate merges or deployments on package approval status.

Typical CI/CD flow:

1. Run `sp-client check -r requirements.txt --no-tui` during the build.
2. Let the command poll until every package reaches a terminal state.
3. Fail the pipeline on exit code `1` when any package is rejected or analysis fails.
4. Optionally fail on `needs_review` with `--fail-on-review` for stricter environments.
5. Use `--json` when you want to archive results, post summaries, or feed another policy step.

Example pipeline step:

```bash
sp-client check -r requirements.txt --no-tui --interval 15 --fail-on-review
```

Because the client is non-interactive in this mode, it works cleanly in GitHub Actions, GitLab CI, Jenkins, Buildkite, and similar systems where a simple exit code and structured output are more useful than a live terminal UI.

### Other commands

```bash
# View detailed analysis for a specific package version
sp-client details requests 2.33.0

# Server-side one-off analysis (no requirements file needed)
./target/release/secure-packages analyze --package flask --version 3.1.1
```

## Analysis

The server uses two Gemini CLI skills depending on context:

**Full security audit** — first time a package version is seen. Inventories all source files, runs pattern scans for known attack vectors (exfiltration, credential theft, obfuscation, install hooks, unsafe deserialization), then reviews every file with the LLM. Produces a verdict with per-finding severity, file locations, and confidence scores.

**Diff security review** — when a newer version of a previously approved package is submitted. Creates a synthetic git repo with old and new source, diffs them, and focuses analysis on what changed. Catches attacks hidden in seemingly innocent version bumps.

Both skills enforce structured JSON output with explicit verdict rules:
- **approved** — only info/low severity findings
- **rejected** — any critical/high severity finding with confidence > 0.8
- **needs_review** — medium severity, or high with low confidence

## Coming soon

A hosted secure-packages service is planned so `sp-client` can be used directly without running your own server stack. That service will also maintain a global verified package list so teams can benefit from shared audit results instead of starting from scratch.

## Configuration

Config loaded from `config/default.toml`, overridden by environment variables with `SP_` prefix:

```bash
SP_DATABASE__URL=postgres://user:pass@host:5432/db
SP_DATABASE__RUN_MIGRATIONS=true
SP_SERVER__PORT=8080
SP_SERVER__ADMIN_TOKEN=secret
SP_ANALYSIS__GEMINI_MODEL=gemini-3-flash-preview
SP_ANALYSIS__GEMINI_TIMEOUT_SECONDS=300
SP_CACHE__SOURCE_CACHE_DIR=./data/cache
GEMINI_API_KEY=your-key
```

## Tests

Rust formatting:

```bash
cargo fmt --check
```

Unit and integration tests:

```bash
cargo test
```

The `sp-db` test suite uses `sqlx::test` and requires a running PostgreSQL instance plus a `DATABASE_URL` environment variable. Example:

```bash
export DATABASE_URL=postgres://user:pass@localhost:5432/secure_packages
cargo test
```

If `DATABASE_URL` is not set, the database-backed integration tests will fail even though the rest of the workspace tests may still pass.

## API

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/packages/check` | Bulk check + auto-trigger analysis |
| GET | `/api/v1/packages` | List tracked packages |
| GET | `/api/v1/packages/{name}/versions` | List versions for a package |
| GET | `/api/v1/packages/{name}/versions/{version}` | Full analysis details |
| POST | `/api/v1/packages/{name}/versions/{version}/override` | Admin override (requires Bearer token) |
| GET | `/health` | Health check |

## Requirements

- Rust 1.88+
- PostgreSQL 17
- Gemini CLI (`npm install -g @google/gemini-cli`)
- `uv` or `pip` (for dependency resolution)

## Project structure

```
crates/
  sp-core/             Shared types, traits, errors
  sp-analysis/         Gemini CLI runner, analysis orchestrator
  sp-db/               PostgreSQL repos, models, migrations
  sp-registry-pypi/    PyPI client, source fetching, PEP 440 parsing
  sp-server/           Axum API server, apalis job queue
  sp-client/           CLI + interactive TUI (ratatui)
skills/
  pypi/
    security-audit/          Full source audit skill
    diff-security-review/    Version diff review skill
```
