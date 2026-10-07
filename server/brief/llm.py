"""A minimal OpenAI-compatible chat client for the brief generator.

The brief used to be produced only by a full Hermes agent (`hermes -z`). That agent existed for
the tool-calling half of the job: reading a team page to find a published contact address. The
tailoring half -- verdict, key skills, rewrites, gaps, notes -- is text in, JSON out, and needs
no tools at all, so it goes straight to the same DeepSeek endpoint Hermes was configured to use.
One HTTP POST, no session, no skill loading, no tool loop.

The API key is read from `DEEPSEEK_API_KEY`, or from the credential pool Hermes already keeps at
`~/.hermes/auth.json`, so the two stay in step. The key is never printed.

    from llm import chat_json
    brief = chat_json([{"role": "system", "content": sys}, {"role": "user", "content": user}])
"""
from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request

DEFAULT_BASE = "https://api.deepseek.com/v1"
DEFAULT_MODEL = "deepseek-flash"
AUTH_PATH = os.path.expanduser("~/.hermes/auth.json")


class LLMError(RuntimeError):
    """Any failure that should be written to the brief as `status: failed`."""


def _credential():
    """(key, base_url) from the environment first, then from Hermes' credential pool."""
    key = os.environ.get("DEEPSEEK_API_KEY")
    base = os.environ.get("DEEPSEEK_BASE_URL")
    if key:
        return key, (base or DEFAULT_BASE).rstrip("/")
    try:
        with open(AUTH_PATH, encoding="utf-8") as fh:
            auth = json.load(fh)
        pool = (auth.get("credential_pool") or {}).get("deepseek") or []
        entry = pool[0] if pool else {}
    except Exception as exc:  # noqa: BLE001 -- a missing pool is a setup problem, not a crash
        raise LLMError(f"could not read a DeepSeek credential: {exc}") from exc
    key = entry.get("access_token")
    if not key:
        raise LLMError("no DeepSeek API key: set DEEPSEEK_API_KEY or run `hermes auth`")
    return key, (base or entry.get("base_url") or DEFAULT_BASE).rstrip("/")


def chat_message(messages, *, tools=None, model=None, max_tokens=6000, temperature=0.2,
                 timeout=180, response_json=False):
    """One completion, returning the raw assistant message (content and `tool_calls`).

    `response_json` and `tools` are kept apart on purpose: JSON mode and tool calls do not mix,
    so a tool loop sends neither until it wants the final object. Note that DeepSeek's thinking
    mode rejects a forced `tool_choice` (only `auto` is accepted).
    """
    key, base = _credential()
    payload = {
        "model": model or os.environ.get("DEEPSEEK_MODEL") or DEFAULT_MODEL,
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
    }
    if tools:
        payload["tools"] = tools
    if response_json:
        payload["response_format"] = {"type": "json_object"}
    request = urllib.request.Request(
        base + "/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"})
    # A throttled or briefly unavailable endpoint is not a failed brief: retry the wire error a
    # couple of times before giving up. 4xx other than 429 is the caller's fault and is raised
    # at once.
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                data = json.load(response)
            break
        except urllib.error.HTTPError as exc:
            if exc.code in (429, 500, 502, 503, 504) and attempt < 2:
                time.sleep(2 * (attempt + 1))
                continue
            detail = exc.read()[:400].decode("utf-8", "replace")
            raise LLMError(f"DeepSeek HTTP {exc.code}: {detail}") from exc
        except Exception as exc:  # noqa: BLE001
            raise LLMError(f"DeepSeek request failed: {exc}") from exc
    try:
        return data["choices"][0]["message"]
    except (KeyError, IndexError, TypeError) as exc:
        raise LLMError(f"unexpected DeepSeek response: {str(data)[:300]}") from exc


def chat(messages, *, response_json=True, **kwargs):
    """One chat completion. Returns the assistant's text."""
    return chat_message(messages, response_json=response_json, **kwargs).get("content") or ""


def parse_json(content):
    """The JSON object in the model's reply.

    Tolerates a code fence, prose around the object, and trailing content: DeepSeek's JSON mode
    occasionally appends a second object or a closing sentence, and a strict parse of that is an
    `Extra data` error even though the first object is exactly what was asked for. `raw_decode`
    stops at the end of the first value, so only the prefix is ever considered.
    """
    text = (content or "").strip()
    if text.startswith("```"):
        parts = text.split("```")
        if len(parts) >= 3:
            text = parts[1]
        if text.lstrip().lower().startswith("json"):
            text = text.lstrip()[4:]
    text = text.strip()
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        if start == -1:
            raise LLMError("the model did not return JSON") from None
        try:
            data, _ = json.JSONDecoder().raw_decode(text, start)
        except json.JSONDecodeError as exc:
            raise LLMError(f"the model returned invalid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise LLMError("the model's JSON was not an object")
    return data


def chat_json(messages, *, attempts=3, **kwargs):
    """Chat and parse, retrying the occasional empty or non-JSON reply.

    DeepSeek's JSON mode is not quite a guarantee: once in a while the content comes back empty
    and the answer is only in `reasoning_content`. Because the request is idempotent and the
    whole point is an unattended generator, one or two retries is cheaper than a failed brief.
    """
    last = None
    for attempt in range(attempts):
        try:
            return parse_json(chat(messages, **kwargs))
        except LLMError as exc:
            last = exc
            if attempt + 1 < attempts:
                time.sleep(1.5 * (attempt + 1))
    raise last
