#!/usr/bin/env python3
"""Stand-in coding agent for tests/e2e.rs.

`anyr <agent>` execs this in place of the real binary (ANYROUTER_<AGENT>_PATH).
It reads the env and files anyr wrote the way that agent does, then sends one
request to the AnyRouter base URL, so the mock server sees exactly the
credential the real agent would send. FAKE_AGENT picks the agent and
FAKE_AGENT_VERSION the upstream behaviour where versions differ.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.request


def pi_resolve(value: str, version: str) -> str:
    # Pi runs a leading "!" as a shell command and uses its stdout.
    if value.startswith("!"):
        return subprocess.run(
            value[1:], shell=True, capture_output=True, text=True, check=False
        ).stdout.strip()
    # Pi 0.73 (@mariozechner): a plain string is an env var name, else a literal.
    if version == "old":
        return os.environ.get(value) or value
    # Pi 0.79.4+ (@earendil-works): a plain string is the key itself.
    return value


def pi() -> tuple[str, str, dict]:
    version = os.environ.get("FAKE_AGENT_VERSION", "new")
    agent_dir = os.environ["PI_CODING_AGENT_DIR"]
    with open(os.path.join(agent_dir, "models.json"), encoding="utf-8") as f:
        provider = json.load(f)["providers"]["anyrouter"]
    args = sys.argv[1:]
    assert args[args.index("--provider") + 1] == "anyrouter", args
    model = args[args.index("--model") + 1]
    key = pi_resolve(provider["apiKey"], version)
    url = provider["baseUrl"].rstrip("/") + "/chat/completions"
    return url, key, {"model": model, "messages": [{"role": "user", "content": "hi"}]}


def claude() -> tuple[str, str, dict]:
    # Claude Code sends ANTHROPIC_AUTH_TOKEN as a bearer token.
    url = os.environ["ANTHROPIC_BASE_URL"].rstrip("/") + "/v1/messages"
    body = {"model": os.environ.get("ANTHROPIC_MODEL", ""), "max_tokens": 8}
    return url, os.environ["ANTHROPIC_AUTH_TOKEN"], body


def codex() -> tuple[str, str, dict]:
    # Codex reads the provider from `-c key=value` overrides; env_key names the
    # variable that holds the key.
    conf = {}
    args = sys.argv[1:]
    for i, a in enumerate(args):
        if a == "-c" and i + 1 < len(args):
            k, _, v = args[i + 1].partition("=")
            conf[k] = v.strip('"')
    assert conf.get("model_provider") == "anyrouter", conf
    base = conf["model_providers.anyrouter.base_url"]
    key = os.environ[conf["model_providers.anyrouter.env_key"]]
    return base.rstrip("/") + "/responses", key, {"model": conf.get("model", "")}


def main() -> int:
    agent = os.environ["FAKE_AGENT"]
    url, key, body = {"pi": pi, "claude": claude, "codex": codex}[agent]()
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        sys.stdout.write(resp.read().decode())
    return 0


if __name__ == "__main__":
    sys.exit(main())
