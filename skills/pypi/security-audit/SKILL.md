---
name: pypi-security-audit
description: Perform a comprehensive supply chain security audit of a Python (PyPI) package source code. Use this skill when asked to audit, review, or certify a Python package for security.
---

# PyPI Package — Full Supply Chain Security Audit

You are performing a security audit of a Python package distributed via PyPI. Your job is to certify that this release contains no supply chain attacks, backdoors, or malicious code.

You MUST be exhaustive. A sophisticated attacker will hide malicious code in the most boring-looking file — a type stub, an `__init__.py`, a test utility, a docs config. Skipping even one file means a potential backdoor goes undetected.

## Critical: Do Not Trust Reputation

A package being well-known or widely-used is irrelevant from a security perspective. Supply chain attacks specifically target popular packages because they have more impact. Treat every package — whether it's requests, flask, numpy, or an unknown library — with the same level of scrutiny. Analyze the actual code, not the name.

## Important: Use Your Judgment

The checklist below covers known attack patterns in the Python/PyPI ecosystem. But attackers are creative — they invent new techniques constantly. If you see ANYTHING that feels wrong, unusual, or out of place during your review, flag it even if it doesn't match a listed pattern. Trust your instincts as a security analyst. Some things to watch for beyond the checklist:

- Code that is unnecessarily complex for what it does
- Patterns you haven't seen before in legitimate packages
- Anything that makes you think "why would a package need this?"
- Logic that only makes sense if the author is trying to hide something
- Code quality that is inconsistent — one file is clean and professional, another is obfuscated or oddly written
- Functionality that overlaps with what the OS or standard library already provides (why reimplement hashing, encoding, or networking primitives?)

## Procedure

Follow these steps in exact order. Complete each step fully before moving to the next.

### Step 1: Full File Inventory

Run this command to get every source file:
```bash
find . -type f \( -name "*.py" -o -name "*.pyx" -o -name "*.pxd" -o -name "setup.*" -o -name "*.toml" -o -name "*.cfg" -o -name "*.ini" -o -name "*.pth" -o -name "*.sh" -o -name "Makefile" \) | grep -v __pycache__ | sort
```

Save this list. At the end, your `files_reviewed` array MUST contain every file from this list. If any file is missing from your review, your audit is incomplete and cannot be trusted.

### Step 2: Build Configuration Audit

Read every build/config file:
- `pyproject.toml`
- `setup.py` (if exists)
- `setup.cfg` (if exists)
- `MANIFEST.in` (if exists)
- Any `.pth` files (these execute code on Python startup)

Check for:
- Custom build backends — `hatchling`, `flit-core`, `setuptools`, `poetry-core`, `maturin` are standard. Anything else is suspicious.
- `[project.scripts]`, `[project.gui-scripts]`, `[project.entry-points]` — do these point to code that makes sense for this package?
- `[tool.setuptools.cmdclass]` or custom install commands that override `install`, `develop`, `build`, `egg_info`
- In setup.py: ANY code that runs at import time (top-level network calls, file reads, subprocess, os.system). setup.py should only define metadata and call `setup()`.
- Build-time dependencies that seem unrelated to building (e.g., `requests` as a build dependency for a pure Python package)

### Step 3: Automated Pattern Scan

Run these grep commands across the entire source tree. Record EVERY match — you will investigate each one in Step 4.

```bash
echo "=== Dynamic execution ==="
grep -rn "exec\s*(" . --include="*.py"
grep -rn "eval\s*(" . --include="*.py"
grep -rn "compile\s*(" . --include="*.py" | grep -v "re\.\|pattern\.\|regex\|ast\."

echo "=== Encoding/obfuscation ==="
grep -rn "base64" . --include="*.py"
grep -rn "bytes\.fromhex\|bytearray\.fromhex" . --include="*.py"
grep -rn "codecs\.decode" . --include="*.py"
grep -rn "\[::-1\]" . --include="*.py"
grep -rn "chr(" . --include="*.py" | grep -v "test\|#"
grep -rn "\\\\x[0-9a-f]" . --include="*.py"

echo "=== Dynamic imports ==="
grep -rn "__import__" . --include="*.py"
grep -rn "importlib" . --include="*.py"

echo "=== Process/network ==="
grep -rn "subprocess\|os\.system\|os\.popen\|os\.exec" . --include="*.py"
grep -rn "socket\.\|urllib\.\|http\.client\|requests\.\|httpx\.\|aiohttp\." . --include="*.py"
grep -rn "ctypes\|cffi\|CDLL" . --include="*.py"

echo "=== Credential/filesystem ==="
grep -rn "os\.environ" . --include="*.py"
grep -rn "\.ssh\|\.aws\|\.config\|\.netrc\|\.gnupg\|\.docker\|\.kube" . --include="*.py"
grep -rn "SECRET\|TOKEN\|PASSWORD\|CREDENTIAL\|API_KEY" . --include="*.py" | grep -vi "test\|doc\|example\|comment"
grep -rn "getpass" . --include="*.py"

echo "=== Sensitive file access ==="
grep -rn "open\s*(" . --include="*.py" | grep -v "test\|doc\|example\|README\|fixture"
grep -rn "pathlib.*read_text\|pathlib.*read_bytes\|Path.*read_" . --include="*.py"

echo "=== Pickle/deserialization ==="
grep -rn "pickle\|marshal\|shelve\|yaml\.load\|yaml\.unsafe_load" . --include="*.py"
grep -rn "jsonpickle\|dill" . --include="*.py"
```

### Step 4: File-by-File Source Review

Go through EVERY Python file from the Step 1 inventory. For each file:

1. Read the complete file contents
2. Understand its purpose — does it belong in a package of this type?
3. Cross-reference against Step 3 grep results — investigate every flagged line in full context
4. Check for these known attack patterns:

**Data Exfiltration**
- Network connections to hardcoded URLs, IPs, or domains
- DNS lookups used as data channels (`socket.getaddrinfo` with encoded data as hostname)
- HTTP requests in `__init__.py` or module-level code that runs on import
- Data written to world-readable temp files for later collection
- Exfiltration disguised as "telemetry", "analytics", "compatibility checks", "version checks", "error reporting"

**Credential Theft**
- Reading `~/.ssh/id_rsa`, `~/.aws/credentials`, `~/.config/gcloud`, `~/.docker/config.json`, `~/.kube/config`
- Browser profile/cookie access (`~/.mozilla`, `~/.config/google-chrome`, `~/Library/Application Support`)
- Bulk `os.environ` access — dumping all env vars or filtering for secrets (`SECRET`, `TOKEN`, `KEY`, `PASSWORD`, `DATABASE`, `AWS_`)
- Reading specific tokens: `AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, `SLACK_TOKEN`, `DATABASE_URL`, `API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`
- Logging or printing secrets (e.g., `log.debug("key=%s", secret_key)`)

**Obfuscated Payloads**
- `base64.b64decode` → `exec`/`eval` chain
- `bytes.fromhex()` → `exec`
- `codecs.decode('rot_13')` building code strings
- `chr()` chains assembling strings character by character
- Reversed strings: `"edoc_suoicilam"[::-1]`
- String concatenation building import names: `__import__("o"+"s")`
- Unusually large string literals (>500 chars of hex, base64, or escaped chars)
- Lambda chains or nested comprehensions that obscure intent
- Variable names that mislead: `_check_compatibility`, `_validate_env`, `_telemetry_init`

**Install-Time Execution**
- `setup.py` running code at module level (not inside `if __name__ == "__main__"`)
- Custom `cmdclass` overriding `install`/`develop`/`build`/`egg_info` commands
- `pyproject.toml` build hooks that execute arbitrary code
- Post-install scripts or `.pth` files
- `conftest.py` or pytest plugins that run code on collection

**Security Weakening**
- Changing security defaults (httponly, secure, samesite cookie flags defaulting to insecure values)
- Disabling certificate verification (`verify=False`, `CERT_NONE`)
- Weakening hash algorithms (sha256 → md5, removing HMAC)
- Adding `# nosec`, `# noqa: S`, `type: ignore` to suppress security linting
- Broadening CORS, CSP, or other security headers

**Unsafe Deserialization**
- `pickle.loads` on untrusted data
- `yaml.load` without `SafeLoader`
- `marshal.loads`, `shelve` with untrusted input
- Custom `__reduce__` methods that execute code during unpickling

**Native Code / FFI**
- `ctypes.CDLL` loading shared libraries from unusual paths
- `cffi` bindings that execute shell commands or access the filesystem
- Inline C code in `.pyx` files doing things unrelated to the package

**Hidden Functionality**
- Code that does something unrelated to the package's purpose
- Conditional execution based on hostname, username, date, time, or environment (`if os.getenv("CI")`, `if platform.node() == "target"`)
- Code that activates only on specific dates or after a delay
- Dormant code paths that are never called by the package but could be invoked externally

**Dependency Attacks**
- Typosquatted dependency names (e.g., `reqeusts`, `python-dateutil` vs `dateutil`)
- Dependencies pinned to exact versions with unusual commit hashes
- Dependencies loaded from URLs instead of PyPI
- Unnecessary dependencies that add attack surface

5. **Beyond the checklist**: Flag anything else that looks suspicious, unusual, or out of place — even if it doesn't match any pattern above. Novel attacks won't be in any checklist.

### Step 5: Final Report

Your ENTIRE response MUST be a single JSON object — no prose, no markdown, no explanation before or after. Do not wrap in code fences. Do not write any text outside the JSON. The system that calls you will fail if you output anything other than raw JSON.

```json
{
  "verdict": "approved|rejected|needs_review",
  "risk_score": 0.0-1.0,
  "reasoning": "One paragraph explaining your overall assessment and methodology",
  "files_reviewed": ["every/file/you/read.py", "pyproject.toml"],
  "files_skipped": [],
  "grep_hits": 0,
  "findings": [
    {
      "severity": "critical|high|medium|low|info",
      "file_path": "relative/path.py",
      "line_range": "10-15",
      "description": "Detailed explanation of what you found, the actual code, and why it is or is not a risk",
      "confidence": 0.0-1.0,
      "category": "exfiltration|credential_theft|obfuscation|dynamic_execution|install_hook|security_weakening|deserialization|native_code|filesystem_access|dependency|backdoor|other|legitimate"
    }
  ]
}
```

Verdict rules:
- **rejected**: Any finding with severity critical or high AND confidence > 0.8
- **needs_review**: Any finding with severity medium, OR severity high with confidence < 0.8
- **approved**: Only info or low severity findings, or no findings at all

IMPORTANT: Every grep hit from Step 3 MUST appear as a finding entry (even if category is "legitimate"). This proves you investigated every match. If you found 50 grep hits, there must be at least 50 findings.

CRITICAL REMINDER: Output ONLY the JSON object. No preamble. No summary. No markdown fences. Just `{...}`.
